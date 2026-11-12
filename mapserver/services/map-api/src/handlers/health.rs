//! Health / readiness probes.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// GET /health -- liveness, plus whether the optional PROJ backend is live.
///
/// `proj` lets callers (desktop 壳、探测脚本、运维面板) know up front whether
/// arbitrary-CRS reprojection (EPSG:4490 / 高斯-克吕格 45xx / UTM) works, instead
/// of guessing from a failing GetMap. The built-in 4326<->3857 path serves
/// either way, so this flag never flips the probe to non-200.
pub async fn health() -> Response {
    let body = format!(r#"{{"status":"ok","proj":{}}}"#, projection::proj().is_some());
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// GET /ready -- readiness (layers loaded at startup, so always ready).
pub async fn ready() -> Response {
    (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], r#"{"status":"ready"}"#)
        .into_response()
}
