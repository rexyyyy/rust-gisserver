//! Cache administration endpoints — the operator-facing half of the refresh
//! mechanism (TTL and the periodic housekeeping task are the other half).
//!
//! * `GET /api/cache` — live counters for every cache.
//! * `POST /api/cache/purge?scope=...` — invalidate caches now (after
//!   swapping a dataset on disk, for example, instead of waiting out the TTL).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::cache::CacheScope;
use crate::state::AppState;

/// `GET /api/cache` — cache statistics as JSON.
pub async fn stats(State(state): State<Arc<AppState>>) -> Response {
    let s = state.cache_stats();
    let body = serde_json::to_string_pretty(&s).unwrap_or_else(|_| "{}".into());
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// `POST /api/cache/purge?scope=all|output|docs|catalog|features|layers`
///
/// `scope=all` (default) also drops the decoded-tile cache. The layer index
/// repopulates lazily on the next lookup, so purging it is safe but pointless
/// unless the catalog itself changed.
pub async fn purge(State(state): State<Arc<AppState>>, RawQuery(qs): RawQuery) -> Response {
    let kv = qs
        .as_deref()
        .map(wms_protocol::kvp::parse_kvp)
        .unwrap_or_default();
    let scope_raw = wms_protocol::kvp::lookup(&kv, "scope").unwrap_or("all");
    let Some(scope) = CacheScope::parse(scope_raw) else {
        return (
            StatusCode::BAD_REQUEST,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            format!(
                "{{\"error\":\"unknown scope '{}' (use all|output|docs|catalog|features|layers)\"}}",
                scope_raw
            ),
        )
            .into_response();
    };
    let report = state.purge_caches(scope);
    // Housekeeping right after the purge so freed capacity is visible
    // immediately in the stats the caller sees next.
    state.cache_housekeeping();
    let body = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into());
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
