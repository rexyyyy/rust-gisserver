//! Request-scoped i18n for user-visible `{code,msg}` responses.
//!
//! The frontend injects `X-Language: zh_CN|en_US` on every request. Instead of
//! threading a locale through every handler, this module translates the `msg`
//! field at the response layer: handlers keep producing Chinese source text,
//! and `tr()` maps it to English (exact match first, then parameterised prefix,
//! then Chinese fallback). See docs/i18n-and-comment-style.md section 3.
use std::collections::HashMap;
use std::sync::OnceLock;

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::header::{HeaderMap, HeaderValue, CONTENT_ENCODING, CONTENT_LANGUAGE, CONTENT_LENGTH, CONTENT_TYPE, VARY};
use axum::middleware::Next;
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;

const ZH_SRC: &str = include_str!("../locales/zh_CN.json");
const EN_SRC: &str = include_str!("../locales/en_US.json");
/// Guard against buffering very large list responses just to translate one field.
const MAX_JSON: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    ZhCn,
    EnUs,
}

impl Lang {
    /// `X-Language` wins, then `Accept-Language`, then Simplified Chinese.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let raw = headers
            .get("x-language")
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .or_else(|| headers.get("accept-language").and_then(|v| v.to_str().ok()))
            .unwrap_or("zh_CN");
        if raw.to_ascii_lowercase().starts_with("en") {
            Lang::EnUs
        } else {
            Lang::ZhCn
        }
    }
}

#[derive(Deserialize, Default)]
struct EnCatalog {
    #[serde(default)]
    keys: HashMap<String, String>,
    #[serde(default)]
    exact: HashMap<String, String>,
    #[serde(default)]
    prefix: Vec<(String, String)>,
}

fn zh_catalog() -> &'static HashMap<String, String> {
    static CELL: OnceLock<HashMap<String, String>> = OnceLock::new();
    CELL.get_or_init(|| serde_json::from_str(ZH_SRC).expect("invalid locales/zh_CN.json"))
}

fn en_catalog() -> &'static EnCatalog {
    static CELL: OnceLock<EnCatalog> = OnceLock::new();
    CELL.get_or_init(|| serde_json::from_str(EN_SRC).expect("invalid locales/en_US.json"))
}

/// Semantic-key lookup for new code. Prefer this over embedding Chinese literals.
#[allow(dead_code)]
pub fn t(lang: Lang, key: &str) -> String {
    let table: &HashMap<String, String> = match lang {
        Lang::ZhCn => zh_catalog(),
        Lang::EnUs => &en_catalog().keys,
    };
    table.get(key).cloned().unwrap_or_else(|| key.to_string())
}

/// Translate an existing Chinese `msg`. Unmapped text stays Chinese (fallbackLocale).
pub fn tr(text: &str) -> String {
    let catalog = en_catalog();
    if let Some(translated) = catalog.exact.get(text) {
        return translated.clone();
    }
    for (prefix, translated) in &catalog.prefix {
        if let Some(rest) = text.strip_prefix(prefix.as_str()) {
            return format!("{translated}{}", tr(rest));
        }
    }
    text.to_string()
}

/// Recursively translate every string value inside a JSON tree via `tr()`.
/// Object keys and non-string values are never touched.
fn translate_strings(v: &mut Value) {
    match v {
        Value::String(s) => *s = tr(s),
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                translate_strings(item);
            }
        }
        Value::Object(obj) => {
            for (_k, val) in obj.iter_mut() {
                translate_strings(val);
            }
        }
        _ => {}
    }
}

/// axum middleware: localize the top-level `msg` of JSON responses for en_US.
///
/// Only small, uncompressed JSON bodies are touched; static files, streams and
/// already-compressed responses pass through untouched.
pub async fn i18n_response(req: Request, next: Next) -> Response {
    let lang = Lang::from_headers(req.headers());
    let mut res = next.run(req).await;
    let is_json = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return res;
    }

    // JSON bodies may differ per request language, so caches must key on the
    // language headers. CORS appends its own Vary entries after this layer.
    res.headers_mut()
        .append(VARY, HeaderValue::from_static("X-Language, Accept-Language"));
    let content_language = match lang {
        Lang::ZhCn => "zh-CN",
        Lang::EnUs => "en-US",
    };
    res.headers_mut()
        .insert(CONTENT_LANGUAGE, HeaderValue::from_static(content_language));

    if lang == Lang::ZhCn || res.headers().contains_key(CONTENT_ENCODING) {
        return res;
    }
    if let Some(len) = res
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
    {
        if len > MAX_JSON {
            return res;
        }
    }

    let (mut parts, body) = res.into_parts();
    let bytes = match to_bytes(body, MAX_JSON).await {
        Ok(bytes) => bytes,
        Err(_) => {
            parts.headers.remove(CONTENT_LENGTH);
            let fallback = Body::from("{\"code\":500,\"msg\":\"Response too large to localize\"}");
            return Response::from_parts(parts, fallback);
        }
    };
    let mut value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };
    // Recursively translate all string values in the JSON tree. This handles
    // manager's `msg`, map-api admin's `error`, and any other string field
    // that carries user-visible text. Pure-number/boolean/null/array/object
    // shapes are left untouched.
    translate_strings(&mut value);
    let out = serde_json::to_vec(&value).unwrap_or_else(|_| bytes.to_vec());
    parts.headers.remove(CONTENT_LENGTH);
    parts.headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Response::from_parts(parts, Body::from(out))
}

/// Helper for tests / probes: build a header map for a language.
#[allow(dead_code)]
pub fn probe_headers(lang: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-language", HeaderValue::from_str(lang).unwrap_or(HeaderValue::from_static("zh_CN")));
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_catalogs_are_isomorphic() {
        let zh = zh_catalog();
        let en = &en_catalog().keys;
        assert_eq!(zh.len(), en.len(), "zh/en key counts differ");
        for key in zh.keys() {
            assert!(en.contains_key(key), "missing en_US key: {key}");
        }
        assert_eq!(t(Lang::ZhCn, "ok.operation"), "操作成功");
        assert_eq!(t(Lang::EnUs, "ok.operation"), "Operation successful");
    }

    #[test]
    fn exact_messages_are_translated() {
        assert_eq!(tr("操作成功"), "Operation successful");
        assert_eq!(tr("用户不存在"), "User does not exist");
        assert_eq!(tr("接口不存在"), "Endpoint not found");
    }

    #[test]
    fn parameterised_prefix_keeps_suffix() {
        assert_eq!(tr("没有权限：map:user:edit"), "Permission denied: map:user:edit");
        assert_eq!(tr("引擎不存在：engine-a"), "Engine not found: engine-a");
    }

    #[test]
    fn unmapped_text_falls_back_to_chinese() {
        assert_eq!(tr("某个未登记的提示"), "某个未登记的提示");
    }

    #[test]
    fn language_resolution() {
        assert_eq!(Lang::from_headers(&probe_headers("en_US")), Lang::EnUs);
        assert_eq!(Lang::from_headers(&probe_headers("zh_CN")), Lang::ZhCn);
        assert_eq!(Lang::from_headers(&HeaderMap::new()), Lang::ZhCn);
        let mut accept = HeaderMap::new();
        accept.insert("accept-language", HeaderValue::from_static("en-US,en;q=0.9"));
        assert_eq!(Lang::from_headers(&accept), Lang::EnUs);
    }

    // End-to-end: does i18n_response middleware actually translate JSON msg?
    #[tokio::test]
    async fn middleware_translates_msg_for_en() {
        use axum::routing::get;
        use axum::Router;

        async fn handler() -> axum::Json<Value> {
            let mut obj = serde_json::Map::new();
            obj.insert("code".into(), Value::Number(200.into()));
            obj.insert("msg".into(), Value::String("登录成功".into()));
            obj.insert("data".into(), Value::Number(1.into()));
            axum::Json(Value::Object(obj))
        }

        let app = Router::new()
            .route("/login", get(handler))
            .layer(axum::middleware::from_fn(i18n_response));

        // Spawn server on random port, test via HTTP
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // en_US
        let client = reqwest::Client::new();
        let url = format!("http://{}/login", addr);
        let r = client.get(&url).header("x-language", "en_US").send().await.unwrap();
        let body: Value = r.json().await.unwrap();
        eprintln!("en_US body: {}", body);
        assert_eq!(body["msg"].as_str().unwrap(), "Signed in", "en_US should translate loginsuccess");

        // zh_CN
        let r2 = client.get(&url).header("x-language", "zh_CN").send().await.unwrap();
        let body2: Value = r2.json().await.unwrap();
        assert_eq!(body2["msg"].as_str().unwrap(), "登录成功", "zh_CN should stay Chinese");
    }
}
