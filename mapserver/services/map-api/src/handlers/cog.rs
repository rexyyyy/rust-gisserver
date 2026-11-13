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

    // A suffix range (`bytes=-N`) needs the representation length to resolve,
    // so the header is parsed against that length rather than up front.
    let range = match (
        headers.contains_key(axum::http::header::RANGE),
        source.len(),
    ) {
        (true, Some(total)) => crate::handlers::parse_range(&headers, total)
            .map(|(start, end)| (start, end, total)),
        (true, None) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                "range requests need a known length".to_string(),
            )
                .into_response()
        }
        (false, _) => None,
    };

    match range {
        Some((start, end, total)) => {
            let end = end.min(total - 1);
            if start > end {
                // RFC 9110 §14.4.1: a 416 must carry the unsatisfied-range
                // form, which is also what `file_range_response` emits.
                let content_range = format!("bytes */{total}");
                return (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/plain"),
                        (axum::http::header::CONTENT_RANGE, content_range.as_str()),
                    ],
                    format!("range start {start} is past the end of {total} bytes"),
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
