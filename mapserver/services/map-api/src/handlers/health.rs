//! Health / readiness probes.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// GET /health -- liveness, PROJ backend availability and run mode.
///
/// `proj` lets callers (desktop 壳、探测脚本、运维面板) know up front whether
/// arbitrary-CRS reprojection (EPSG:4490 / 高斯-克吕格 45xx / UTM) works.
/// `mode` / `manager` report the startup self-check: `managed` means the
/// configured manager answered and owns the UI, `standalone` means this node
/// serves data only.
pub async fn health(State(state): State<Arc<AppState>>) -> Response {
    let body = serde_json::json!({
        "status": "ok",
        "proj": gisserver::projection::proj().is_some(),
        "mode": state.manager.mode.as_str(),
        "manager": state.manager.url.as_deref(),
    })
    .to_string();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// GET /ready -- readiness (layers loaded at startup, so always ready).
pub async fn ready() -> Response {
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], r#"{"status":"ready"}"#)
        .into_response()
}
