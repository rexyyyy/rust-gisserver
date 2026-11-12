//! COG serving: `/cog/{layer}` streams the layer's COG bytes with HTTP Range
//! support, so external COG clients can open it
//! over HTTP. Local COGs are read fully; remote `http(s)://` COGs are proxied
//! range-by-range.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use crate::state::AppState;

pub async fn archive(
    State(state): State<Arc<AppState>>,
    Path(layer_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(layer) = state.layer(&layer_name) else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            format!("no layer '{layer_name}'"),
        )
            .into_response();
    };
    let Some(source) = &layer.source else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            "layer is not a COG layer".to_string(),
        )
            .into_response();
    };

    match crate::handlers::parse_range(&headers) {
        Some((start, end)) => {
            let len = source.len();
            let Some(total) = len else {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    "range requests need a known length".to_string(),
                )
                    .into_response();
            };
            let end = end.min(total - 1);
            if start > end {
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    format!("invalid range {start}-{end}"),
                )
                    .into_response();
            }
            match source.read_range(start, (end - start + 1) as usize) {
                Ok(bytes) => {
                    let content_range = format!("bytes {start}-{end}/{total}");
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [
                            (axum::http::header::CONTENT_TYPE, "image/tiff"),
                            (axum::http::header::CONTENT_RANGE, content_range.as_str()),
                            (axum::http::header::ACCEPT_RANGES, "bytes"),
                        ],
                        bytes,
                    )
                        .into_response()
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    format!("read range: {e}"),
                )
                    .into_response(),
            }
        }
        None => {
            // Full stream: local files read whole; remote sources need a loop.
            let total = source.len();
            let mut full = Vec::new();
            let mut off = 0u64;
            loop {
                let chunk = match source.read_range(off, 1 << 20) {
                    Ok(c) => c,
                    Err(_) => break,
                };
                if chunk.is_empty() {
                    break;
                }
                full.extend_from_slice(&chunk);
                off += chunk.len() as u64;
                if let Some(t) = total {
                    if off >= t {
                        break;
                    }
                }
            }
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "image/tiff"),
                    (axum::http::header::ACCEPT_RANGES, "bytes"),
                ],
                full,
            )
                .into_response()
        }
    }
}
