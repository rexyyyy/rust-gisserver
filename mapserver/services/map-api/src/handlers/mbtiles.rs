//! MBTiles handler: serve pre-rendered tiles from a `.mbtiles` SQLite package.
//!
//! * `GET /mbtiles/{name}`             -> archive metadata JSON (format, bounds,
//!   minzoom/maxzoom, raw `metadata` table)
//! * `GET /mbtiles/{name}/{z}/{x}/{y}` -> tile bytes, addressed in **XYZ**
//!   (the handler flips `y` to the MBTiles TMS row internally). A `.png`/`.pbf`
//!   suffix is tolerated (matchit captures it with the number, so `parse_coord`
//!   strips it).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// GET /mbtiles/{name} — archive metadata.
pub async fn info(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let Some(layer) = state.layer(&name) else {
        return plain(StatusCode::NOT_FOUND, format!("no mbtiles layer '{name}'"));
    };
    let Some(mb) = &layer.mbtiles else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("'{name}' is not an MBTiles layer"),
        );
    };
    let body = serde_json::to_string_pretty(&mb.to_json()).unwrap_or_else(|_| "{}".into());
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// GET /mbtiles/{name}/{z}/{x}/{y}[.ext] — tile bytes (XYZ addressing).
pub async fn tile(
    State(state): State<Arc<AppState>>,
    Path((name, z, x, y)): Path<(String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(y) = crate::handlers::parse_coord(&y) else {
        return plain(StatusCode::BAD_REQUEST, format!("bad tile row '{y}'"));
    };
    let Some(layer) = state.layer(&name) else {
        return plain(StatusCode::NOT_FOUND, format!("no mbtiles layer '{name}'"));
    };
    let Some(mb) = &layer.mbtiles else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("'{name}' is not an MBTiles layer"),
        );
    };
    // Clone the cheap Arc for the blocking read; keep `mb` borrowed for
    // content-type resolution afterwards.
    let mb_owned = Arc::clone(mb);
    let res = tokio::task::spawn_blocking(move || mb_owned.read_tile(z, x, y)).await;
    match res {
        Ok(Ok(Some(bytes))) => {
            let out = crate::mbtiles::tile_out(mb, bytes);
            crate::handlers::tile_response(out, &headers, "public, max-age=86400")
        }
        Ok(Ok(None)) => plain(
            StatusCode::NOT_FOUND,
            format!("no tile {z}/{x}/{y} in '{name}'"),
        ),
        Ok(Err(e)) => plain(StatusCode::INTERNAL_SERVER_ERROR, format!("mbtiles {name}: {e}")),
        Err(e) => plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mbtiles read join: {e}"),
        ),
    }
}

fn plain(status: StatusCode, msg: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain")],
        msg,
    )
        .into_response()
}
