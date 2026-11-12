//! PMTiles archive serving: `/pmtiles/{name}` with HTTP Range support, so
//! `pmtiles://` clients (e.g. the pmtiles JS library) can read archives
//! directly. The name resolves to a layer of source type `pmtiles`, or to an
//! entry in the config `archives:` map.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

pub async fn archive(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    // A configured standalone archive wins over a layer of the same name.
    let path = if let Some(p) = state.config.archives.get(&name) {
        p.clone()
    } else if let Some(l) = state.layer(&name) {
        match &l.def.source {
            wms_common::layer::SourceKind::Pmtiles { path } => path.clone(),
            _ => {
                return (
                    StatusCode::NOT_FOUND,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    format!("'{name}' is not a PMTiles layer"),
                )
                    .into_response()
            }
        }
    } else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            format!("no archive '{name}'"),
        )
            .into_response();
    };

    // Range-aware file serving: only the requested byte window is read (the
    // old code `fs::read` the entire archive into memory per request -- GB-scale
    // archives OOM'd the server). Full (range-less) reads stream in 1 MiB
    // chunks. IO runs on the blocking pool.
    crate::handlers::file_range_response(
        std::path::PathBuf::from(path),
        &headers,
        "application/x-pmtiles",
    )
    .await
}
