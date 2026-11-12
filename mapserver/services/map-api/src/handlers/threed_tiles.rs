//! 3D Tiles 服务 handler：`/3dtiles`（服务目录）、`/3dtiles/{name}` 与
//! `/3dtiles/{name}/{path...}`（tileset.json 与全部瓦片文件直出）。
//!
//! 服务定义来自 `<catalog.root>/3dtiles` 下的 XML 配置（一个 XML = 一个服务，
//! 见 [`crate::tilesets`]）。文件响应支持 HTTP Range（供下载器断点续取），
//! 并拒绝越出数据集根目录的路径穿越。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::state::AppState;

/// `GET /3dtiles` — 已配置的 3D Tiles 服务目录（JSON）。
pub async fn list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = crate::handlers::base_url(&state, &headers);
    let services: Vec<_> = state
        .tilesets
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list()
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "title": s.title,
                "entry": s.entry,
                "root": s.root.display().to_string(),
                "available": s.available,
                "unavailable_reason": s.unavailable_reason,
                "tileset_url": format!("{base}/3dtiles/{}/{}", s.name, s.entry),
            })
        })
        .collect();
    let body = json!({
        "count": services.len(),
        "services": services,
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into()),
    )
        .into_response()
}

/// `GET /3dtiles/{name}` — 直出该服务的入口 tileset.json。
pub async fn entry(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    serve(&state, &name, "", &headers).await
}

/// `GET /3dtiles/{name}/{path...}` — 直出 tileset.json 引用的任意文件
/// （b3dm / glb / 贴图 / 二进制等）。
pub async fn file(
    State(state): State<Arc<AppState>>,
    Path((name, rest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    serve(&state, &name, &rest, &headers).await
}

async fn serve(state: &AppState, name: &str, rel: &str, headers: &HeaderMap) -> Response {
    let not_found = |msg: String| {
        (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            msg,
        )
            .into_response()
    };

    let Some(svc) = state.tilesets.read().unwrap_or_else(|e| e.into_inner()).get(name).cloned() else {
        return not_found(format!("未找到 3D Tiles 服务 '{name}'（见 GET /3dtiles）"));
    };
    // 登记可见但数据缺失的服务：404 + 明确原因（而不是模糊的文件不存在）。
    if let Some(reason) = &svc.unavailable_reason {
        return not_found(format!("3D Tiles 服务 '{name}' 不可用：{reason}"));
    }

    // 相对路径安全检查：拒绝 `..` / 绝对路径前缀穿越。
    let target = if rel.is_empty() {
        svc.root.join(&svc.entry)
    } else {
        match crate::tilesets::safe_rel_path(rel) {
            Some(p) => svc.root.join(p),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    "非法路径".to_string(),
                )
                    .into_response()
            }
        }
    };

    // 规范化后必须仍位于数据集根目录内（符号链接/大小写差异也能兜住）。
    let (canon_root, canon_target) = match (svc.root.canonicalize(), target.canonicalize()) {
        (Ok(r), Ok(t)) => (r, t),
        _ => return not_found(format!("文件不存在：{rel}")),
    };
    if !canon_target.starts_with(&canon_root) {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "非法路径（越出数据集根目录）".to_string(),
        )
            .into_response();
    }

    // Range-aware file serving: only the requested byte window is read (the
    // old code `fs::read` the whole tile file per request). IO runs on the
    // blocking pool.
    let ct = content_type(&canon_target);
    crate::handlers::file_range_response(canon_target, headers, ct).await
}

/// 按扩展名给出 3D Tiles 常用内容的 Content-Type。
fn content_type(p: &std::path::Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("json" | "gltf") => "application/json",
        Some("glb") => "model/gltf-binary",
        Some("b3dm" | "i3dm" | "pnts" | "cmpt" | "bin" | "drc") => "application/octet-stream",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ktx2") => "image/ktx2",
        Some("xml") => "text/xml; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}
