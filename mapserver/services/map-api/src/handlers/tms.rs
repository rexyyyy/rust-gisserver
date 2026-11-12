//! TMS 1.0.0 handler: `/tms/1.0.0/{layer}/{z}/{x}/{y}.png`.
//!
//! TMS addresses tiles from the BOTTOM-left; we flip y to the top-left grid
//! convention before rendering. A grid-selector variant
//! `/tms/1.0.0/{layer}/{grid}/{z}/{x}/{y}` serves a non-default published grid
//! (e.g. geographic `WorldCRS84Quad` / srs=4326), mirroring the WMTS REST route.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::layer::LayerRuntime;
use crate::state::AppState;
use wms_common::tile::TileMatrixSet;

/// GET /tms/1.0.0/{layer}/{z}/{x}/{y}.png (y is TMS-style, bottom-up).
pub async fn tile(
    State(state): State<Arc<AppState>>,
    Path((layer_name, z, x, y)): Path<(String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(y) = crate::handlers::parse_coord(&y) else {
        return plain(StatusCode::BAD_REQUEST, format!("bad tile row '{y}'"));
    };
    let Some(layer) = state.layer_arc(&layer_name) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{layer_name}'"));
    };
    let Some(grid) = state.default_grid(&layer) else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "layer has no tile grid");
    };
    serve(Arc::clone(&state), layer, grid, layer_name, z, x, y, &headers).await
}

/// GET /tms/1.0.0/{layer}/{grid}/{z}/{x}/{y}.png (explicit published grid).
pub async fn tile_grid(
    State(state): State<Arc<AppState>>,
    Path((layer_name, grid_id, z, x, y)): Path<(String, String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(y) = crate::handlers::parse_coord(&y) else {
        return plain(StatusCode::BAD_REQUEST, format!("bad tile row '{y}'"));
    };
    let Some(layer) = state.layer_arc(&layer_name) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{layer_name}'"));
    };
    let Some(grid) = state.advertised_grid(&layer, &grid_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("layer '{layer_name}' does not publish grid '{grid_id}'"),
        );
    };
    serve(Arc::clone(&state), layer, grid, layer_name, z, x, y, &headers).await
}

/// Render one TMS tile (bottom-up y, flipped to the grid's top-left row).
///
/// Output-cache hit -> serve bytes directly. Miss -> decode + resample + PNG
/// encode runs on the **blocking pool** (`spawn_blocking`): the render is
/// CPU-heavy and would otherwise occupy a tokio worker, queueing /health and
/// every other route behind it. Under load the async worker count caps
/// concurrency at the core count; the blocking pool does not.
async fn serve(
    state: Arc<AppState>,
    layer: Arc<LayerRuntime>,
    grid: TileMatrixSet,
    layer_name: String,
    z: u32,
    x: u32,
    y: u32,
    headers: &HeaderMap,
) -> Response {
    let Some(lvl) = grid.level(z) else {
        return plain(StatusCode::NOT_FOUND, format!("no zoom level {z}"));
    };
    if x >= lvl.matrix_w || y >= lvl.matrix_h {
        return plain(
            StatusCode::NOT_FOUND,
            format!("tile {z}/{x}/{y} out of range"),
        );
    }
    // TMS y-flip: bottom-up y -> top-left row.
    let row = lvl.matrix_h - 1 - y;
    let key = format!("tms/{layer_name}/{}/{z}/{x}/{y}", grid.id);
    if let Some(hit) = state.cache.output_get(&key).await {
        return crate::handlers::tile_response(hit, headers, "public, max-age=86400");
    }
    let st = Arc::clone(&state);
    let res = tokio::task::spawn_blocking(move || {
        crate::tiles::raster_tile(&st, &layer, &grid, z, x, row)
    })
    .await;
    match res {
        Ok(Ok(t)) => {
            state.cache.output_insert(&key, t.clone());
            crate::handlers::tile_response(t, headers, "public, max-age=86400")
        }
        Ok(Err(e)) => plain(crate::handlers::tile_error_status(&e), e),
        Err(e) => plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("tile render join: {e}"),
        ),
    }
}

fn plain(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, [(axum::http::header::CONTENT_TYPE, "text/plain")], msg.into()).into_response()
}
