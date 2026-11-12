//! API Key 管理：为 r-mapserver 生成/管理访问凭证（名称 + Key）。
//!
//! 两种用途与 r-mapserver 现有令牌护栏一一对应：
//! * `admin` —— manager → server 管理面，请求头 `X-Admin-Token`；
//!   「应用到引擎」把 key 写入 `sys_engine.admin_token`，管理代理立即带上。
//! * `data`  —— server → 客户端数据面，`X-Data-Token` / `Authorization: Bearer`
//!   / `?token=`；可复制下发给不同客户端。
//!
//! r-mapserver 从同一共享库定时读取启用中的 key 作为附加认可凭证
//! （见 map-api `apikeys` 模块），主令牌仍是「是否启用校验」的开关。
//! 明文存储是为了管理端可随时复制分发；库在内网，访问受登录态 +
//! `map:apikey:list` / `map:apikey:edit` 约束。
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use chrono::{NaiveDate, NaiveDateTime};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::resp::Resp;
use crate::state::{conn, AppState};

/// API Key 域路由（由 `handlers::router` 统一装配；axum 0.8 花括号参数）。
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // 静态路径在前，参数路径在后；`rotate` / `apply` 为 4 段，先于 3 段通配
        .route("/map/apikeys", get(list).post(create))
        .route("/map/apikeys/{keyId}/rotate", post(rotate))
        .route("/map/apikeys/{keyId}/apply", post(apply))
        .route("/map/apikeys/{keyId}", axum::routing::put(update).delete(remove))
}

/// 生成/编辑请求体（前端 plus-ui 统一用 camelCase：keyName / expiresAt）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyBody {
    #[serde(default)]
    pub key_name: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub status: String,
    /// `YYYY-MM-DD HH:mm:ss` / `YYYY-MM-DDTHH:mm` / `YYYY-MM-DD`（空串 = 永不过期）
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub remark: String,
}

/// 生成一枚 CSPRNG 密钥：`rms_<purpose>_<32 字节 base64url>`。
///
/// 32 字节随机数（256 bit 熵）不可预测；前缀标明用途，便于运维区分与日志检索。
fn new_key(purpose: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    format!("rms_{purpose}_{token}")
}

fn validate(b: &ApiKeyBody) -> AppResult<()> {
    let name = b.key_name.trim();
    if name.is_empty() || name.chars().count() > 50 {
        return Err(AppError::Param("名称不能为空且不超过 50 个字符".into()));
    }
    if b.purpose != "admin" && b.purpose != "data" {
        return Err(AppError::Param("用途只能是 admin 或 data".into()));
    }
    if !b.status.is_empty() && b.status != "0" && b.status != "1" {
        return Err(AppError::Param("状态只能是 0（启用）或 1（停用）".into()));
    }
    if b.remark.chars().count() > 250 {
        return Err(AppError::Param("备注不超过 250 个字符".into()));
    }
    Ok(())
}

/// 解析前端传来的有效期；空值 = 永不过期，纯日期按当天 23:59:59 处理。
fn parse_exp(raw: &Option<String>) -> AppResult<Option<NaiveDateTime>> {
    let Some(s) = raw else { return Ok(None) };
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let normalized = s.replace('T', " ");
    NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M"))
        .or_else(|_| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(23, 59, 59).unwrap())
        })
        .map(Some)
        .map_err(|e| AppError::Param(format!("有效期格式错误：{e}（用 YYYY-MM-DD HH:mm:ss）")))
}

fn fmt_dt(v: Option<NaiveDateTime>) -> String {
    v.map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// 用途对应的使用说明（前端生成后直接展示/复制）。
fn usage_hint(purpose: &str, key: &str) -> Value {
    if purpose == "admin" {
        json!({
            "header": "X-Admin-Token",
            "sample": format!("X-Admin-Token: {key}"),
            "note": "manager 连接 server 的管理面凭证。点「应用到引擎」会写入默认引擎的 admin_token；server 侧需在 mapserver.xml 配置 <admin token=\"主令牌\"/> 才会启用校验（主令牌与本库中启用的 admin key 均被认可）。",
        })
    } else {
        json!({
            "header": "X-Data-Token",
            "sample": format!("X-Data-Token: {key}"),
            "bearer": format!("Authorization: Bearer {key}"),
            "query": format!("?token={key}"),
            "note": "下发给客户端访问 /tiles /wms /wmts /mvt 等数据面。server 侧需在 mapserver.xml 的 <server> 内配置 <data_token>主令牌</data_token> 才会启用校验（主令牌与本库中启用的 data key 均被认可）。",
        })
    }
}

const COLS: &str = "key_id, key_name, api_key, purpose, status, expires_at, remark, create_by, create_time, update_time";

fn row_json(r: &tokio_postgres::Row) -> Value {
    let key_id: String = r.get("key_id");
    let key_name: String = r.get("key_name");
    let api_key: String = r.get("api_key");
    let purpose: String = r.get("purpose");
    let status: String = r.get("status");
    let expires: Option<NaiveDateTime> = r.get("expires_at");
    let remark: Option<String> = r.get("remark");
    let create_by: Option<String> = r.get("create_by");
    let create_time: Option<NaiveDateTime> = r.get("create_time");
    let update_time: Option<NaiveDateTime> = r.get("update_time");
    let expired = expires
        .map(|d| d < chrono::Local::now().naive_local())
        .unwrap_or(false);
    json!({
        "keyId": key_id,
        "keyName": key_name,
        "apiKey": api_key,
        "purpose": purpose,
        "purposeName": if purpose == "admin" { "管理面（manager→server）" } else { "客户端（server→client）" },
        "status": status,
        "expiresAt": fmt_dt(expires),
        "expired": expired,
        "effective": status == "0" && !expired,
        "remark": remark.unwrap_or_default(),
        "createBy": create_by.unwrap_or_default(),
        "createTime": fmt_dt(create_time),
        "updateTime": fmt_dt(update_time),
        "usage": usage_hint(&purpose, &api_key),
    })
}

async fn fetch_one(state: &AppState, id: &str) -> AppResult<Value> {
    let sql = format!("SELECT {COLS} FROM sys_api_key WHERE key_id = $1");
    let c = conn(&state.pool).await?;
    let row = c
        .query_opt(&sql, &[&id])
        .await?
        .ok_or_else(|| AppError::Msg(format!("API Key 不存在：{id}")))?;
    Ok(row_json(&row))
}

/// `GET /map/apikeys` — 全部密钥（含明文，供管理端复制；页面默认打码）。
pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let sql = format!("SELECT {COLS} FROM sys_api_key ORDER BY create_time DESC, key_id");
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[]).await?;
    let keys: Vec<Value> = rows.iter().map(row_json).collect();
    let total = keys.len();
    Ok(Resp::ok(json!({ "keys": keys, "total": total })))
}

/// `POST /map/apikeys` — 生成一枚新密钥，返回完整明文（仅此次响应与列表可见）。
pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(mut body): Json<ApiKeyBody>,
) -> AppResult<Json<Resp<Value>>> {
    validate(&body)?;
    body.key_name = body.key_name.trim().to_string();
    if body.status.is_empty() {
        body.status = "0".into();
    }
    let expires = parse_exp(&body.expires_at)?;
    let key = new_key(&body.purpose);
    let id = uuid::Uuid::new_v4().to_string();
    let c = conn(&state.pool).await?;
    let dup: i64 = c
        .query_one("SELECT count(*) FROM sys_api_key WHERE key_name = $1", &[&body.key_name])
        .await?
        .get(0);
    if dup > 0 {
        return Err(AppError::Msg(format!("名称「{}」已存在", body.key_name)));
    }
    c.execute(
        "INSERT INTO sys_api_key (key_id, key_name, api_key, purpose, status, expires_at, remark, create_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, '')",
        &[&id, &body.key_name, &key, &body.purpose, &body.status, &expires, &body.remark],
    )
    .await?;
    Ok(Resp::ok_msg(fetch_one(&state, &id).await?, "API Key 已生成"))
}

/// `PUT /map/apikeys/{keyId}` — 改名称/用途/状态/有效期/备注（不改 key 本身）。
pub async fn update(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Json(mut body): Json<ApiKeyBody>,
) -> AppResult<Json<Resp<Value>>> {
    validate(&body)?;
    body.key_name = body.key_name.trim().to_string();
    if body.status.is_empty() {
        body.status = "0".into();
    }
    let expires = parse_exp(&body.expires_at)?;
    let c = conn(&state.pool).await?;
    let dup: i64 = c
        .query_one(
            "SELECT count(*) FROM sys_api_key WHERE key_name = $1 AND key_id <> $2",
            &[&body.key_name, &id],
        )
        .await?
        .get(0);
    if dup > 0 {
        return Err(AppError::Msg(format!("名称「{}」已存在", body.key_name)));
    }
    let n = c
        .execute(
            "UPDATE sys_api_key SET key_name=$2, purpose=$3, status=$4, expires_at=$5, \
             remark=$6, update_time=now() WHERE key_id=$1",
            &[&id, &body.key_name, &body.purpose, &body.status, &expires, &body.remark],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("API Key 不存在：{id}")));
    }
    Ok(Resp::ok(fetch_one(&state, &id).await?))
}

/// `POST /map/apikeys/{keyId}/rotate` — 重置密钥（旧 key 立即失效）。
pub async fn rotate(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let row = c
        .query_opt("SELECT purpose FROM sys_api_key WHERE key_id = $1", &[&id])
        .await?
        .ok_or_else(|| AppError::Msg(format!("API Key 不存在：{id}")))?;
    let purpose: String = row.get("purpose");
    let key = new_key(&purpose);
    c.execute(
        "UPDATE sys_api_key SET api_key=$2, update_time=now() WHERE key_id=$1",
        &[&id, &key],
    )
    .await?;
    Ok(Resp::ok_msg(fetch_one(&state, &id).await?, "密钥已重置，旧 Key 立即失效"))
}

/// `DELETE /map/apikeys/{keyId}` — 删除密钥。
pub async fn remove(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let n = c
        .execute("DELETE FROM sys_api_key WHERE key_id = $1", &[&id])
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("API Key 不存在：{id}")));
    }
    Ok(Resp::ok(json!({ "keyId": id })))
}

/// `POST /map/apikeys/{keyId}/apply` — 应用密钥。
///
/// * `admin`：写入默认引擎 `sys_engine.admin_token`，manager 代理 /map/* 立即带上
///   `X-Admin-Token`（这就是「manager 连接 server」场景）；
/// * `data`：不写引擎，返回三种客户端携带方式与配置说明（供复制下发）。
pub async fn apply(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let row = c
        .query_opt(
            "SELECT key_name, api_key, purpose, status, expires_at FROM sys_api_key WHERE key_id = $1",
            &[&id],
        )
        .await?
        .ok_or_else(|| AppError::Msg(format!("API Key 不存在：{id}")))?;
    let key_name: String = row.get("key_name");
    let api_key: String = row.get("api_key");
    let purpose: String = row.get("purpose");
    let status: String = row.get("status");
    let expires: Option<NaiveDateTime> = row.get("expires_at");
    let expired = expires
        .map(|d| d < chrono::Local::now().naive_local())
        .unwrap_or(false);
    if status != "0" || expired {
        return Err(AppError::Msg("该 Key 已停用或过期，请先启用/续期后再应用".into()));
    }
    if purpose != "admin" {
        return Ok(Resp::ok_msg(
            json!({
                "applied": false,
                "keyName": key_name,
                "usage": usage_hint(&purpose, &api_key),
            }),
            "客户端 Key 无需写入引擎；请复制下方用法下发给客户端",
        ));
    }
    let engine = c
        .query_opt(
            "SELECT engine_id FROM sys_engine WHERE is_default = '1' AND status = '0' \
             ORDER BY sort, engine_id LIMIT 1",
            &[],
        )
        .await?;
    let engine_id: String = match engine {
        Some(r) => r.get("engine_id"),
        None => "map-api".to_string(),
    };
    let n = c
        .execute(
            "UPDATE sys_engine SET admin_token = $2, update_time = now() WHERE engine_id = $1",
            &[&engine_id, &api_key],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!(
            "默认引擎「{engine_id}」不存在，请先在「服务引擎管理」登记引擎"
        )));
    }
    Ok(Resp::ok_msg(
        json!({
            "applied": true,
            "engineId": engine_id,
            "keyName": key_name,
            "usage": usage_hint(&purpose, &api_key),
        }),
        "已写入默认引擎，manager 连接 server 将立即使用该 Key",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_prefixed_and_unique() {
        let a = new_key("admin");
        let b = new_key("data");
        assert!(a.starts_with("rms_admin_"));
        assert!(b.starts_with("rms_data_"));
        assert_eq!(a.len(), "rms_admin_".len() + 43); // 32B base64url(无填充)
        assert_ne!(a, new_key("admin"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn validate_accepts_only_known_purposes_and_status() {
        let ok = ApiKeyBody {
            key_name: "客户端A".into(),
            purpose: "data".into(),
            status: "0".into(),
            expires_at: None,
            remark: String::new(),
        };
        assert!(validate(&ok).is_ok());
        let mut bad = ApiKeyBody { purpose: "root".into(), ..ok };
        assert!(validate(&bad).is_err());
        bad = ApiKeyBody {
            key_name: "x".into(),
            purpose: "admin".into(),
            status: "9".into(),
            expires_at: None,
            remark: String::new(),
        };
        assert!(validate(&bad).is_err());
        bad = ApiKeyBody {
            key_name: "".into(),
            purpose: "admin".into(),
            status: "0".into(),
            expires_at: None,
            remark: String::new(),
        };
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn expiry_parser_handles_date_datetime_and_empty() {
        assert_eq!(parse_exp(&None).unwrap(), None);
        assert_eq!(parse_exp(&Some("  ".into())).unwrap(), None);
        let d = parse_exp(&Some("2026-12-31".into())).unwrap().unwrap();
        assert_eq!(d.format("%H:%M:%S").to_string(), "23:59:59");
        let t = parse_exp(&Some("2026-12-31T08:30".into())).unwrap().unwrap();
        assert_eq!(t.format("%Y-%m-%d %H:%M:%S").to_string(), "2026-12-31 08:30:00");
        assert!(parse_exp(&Some("not-a-date".into())).is_err());
    }

    #[test]
    fn body_deserializes_camel_case_from_ui() {
        // 前端 plus-ui 发 keyName / expiresAt；少了 rename_all 会静默变成空值，
        // 表现为「名称不能为空」且排查困难，这里锁死契约。
        let b: ApiKeyBody = serde_json::from_str(
            r#"{"keyName":"client-a","purpose":"data","status":"0","expiresAt":"2026-12-31","remark":""}"#,
        )
        .unwrap();
        assert_eq!(b.key_name, "client-a");
        assert_eq!(b.expires_at.as_deref(), Some("2026-12-31"));
        assert!(validate(&b).is_ok());
    }

    #[test]
    fn usage_hint_matches_purpose() {
        let a = usage_hint("admin", "rms_admin_abc");
        assert_eq!(a["header"], "X-Admin-Token");
        let d = usage_hint("data", "rms_data_abc");
        assert_eq!(d["header"], "X-Data-Token");
        assert_eq!(d["query"], "?token=rms_data_abc");
    }
}
