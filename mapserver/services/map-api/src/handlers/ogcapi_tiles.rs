//! OGC API - Tiles handler: `/ogc/tiles/v1` — landing, conformance,
//! tileMatrixSets, tiled collections, tileset metadata and the tile bytes
//! themselves (delegated to the WMS tile renderer / MVT encoder).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;
use gisserver::ogcapi_tiles::{TiledCollection, tile_matrix_set, tile_matrix_sets, tileset, tilesets};
use gisserver::wms_common::tile::{TileMatrixSet, strip_size_suffix};

/// The mount point (kept in one place so links and routes cannot drift).
pub const BASE_PATH: &str = "/ogc/tiles/v1";

fn json_response(body: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

fn plain(status: StatusCode, msg: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain")],
        msg,
    )
        .into_response()
}

/// Grids advertised by `/tileMatrixSets`: both presets (256px default) plus
/// every custom grid from the server config (cached in the catalog metadata).
fn advertised_grids(state: &AppState) -> Arc<Vec<TileMatrixSet>> {
    Arc::clone(&state.catalog_meta().advertised_grids)
}

/// One tile-serving layer as an OGC API tiled collection (catalog metadata,
/// built once — see `crate::cache::CatalogMeta`).
fn tiled_collection(state: &AppState, name: &str) -> Option<TiledCollection> {
    state.catalog_meta().oat_tiled.iter().find(|c| c.id == name).cloned()
}

/// All tile-serving layers (every published layer can produce tiles).
fn tiled_collections(state: &AppState) -> Arc<Vec<TiledCollection>> {
    Arc::clone(&state.catalog_meta().oat_tiled)
}

/// GET /ogc/tiles/v1 — landing page.
pub async fn landing(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let doc = state.cache.doc(format!("oat-landing/{base}"), || gisserver::ogcapi_tiles::landing(&base));
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/conformance
pub async fn conformance(State(state): State<Arc<AppState>>) -> Response {
    let doc = state.cache.doc("oat-conformance".to_string(), gisserver::ogcapi_tiles::conformance);
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/tileMatrixSets
pub async fn tile_matrix_sets_list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let grids = advertised_grids(&state);
    let doc = state.cache.doc(format!("oat-tms-list/{base}"), || tile_matrix_sets(&base, &grids));
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/tileMatrixSets/{tileMatrixSetId}
pub async fn tile_matrix_set_one(
    State(state): State<Arc<AppState>>,
    Path(tms_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    // Match by the full config id (`wh_custom_grid_4522`) *or* the stripped
    // display id (`wh_custom_grid`, which WMTS GetCapabilities advertises), so
    // clients that only know the caps identifier can still resolve the grid and
    // its proj4 definition here.
    let Some(g) = advertised_grids(&state)
        .iter()
        .find(|g| g.id == tms_id || strip_size_suffix(&g.id) == tms_id)
        .cloned()
    else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("tile matrix set '{tms_id}' is not advertised"),
        );
    };
    // 真实数据包络（conf.cdi 派生，存于 layer.extent，源 CRS 同 g.crs）。
    // 网格自身的 extent 从 TileOrigin 起算，PGIS 带号原点下会把原点到数据之间
    // 的大片空白也框进来 → 前端按网格几何中心 fit 会落到无瓦区（白屏）。这里
    // 补一个 bbox，前端据此 fit 到真实数据范围即可首屏出图。取瓦寻址仍用网格
    // 自身的 origin+resolutions，bbox 只用于定位视图。
    let bbox = {
        let ls = state.layers.read().unwrap_or_else(|e| e.into_inner());
        ls.iter()
            .find(|l| {
                l.def
                    .grids
                    .iter()
                    .any(|g| g == &tms_id || strip_size_suffix(g) == tms_id)
            })
            .and_then(|l| l.def.extent)
    };
    let doc = state.cache.doc(format!("oat-tms/{base}/{tms_id}"), || {
        let s = tile_matrix_set(&base, &g, gisserver::projection::crs_proj4(&g.crs).as_deref());
        match bbox {
            Some(b) => with_bbox(s, b),
            None => s,
        }
    });
    json_response((*doc).clone())
}

/// 把 conf.cdi 数据范围作为 `bbox:[minx,miny,maxx,maxy]`（网格 CRS）注入
/// tileMatrixSet JSON。解析失败原样返回，不影响既有响应。
fn with_bbox(json_str: String, b: [f64; 4]) -> String {
    match serde_json::from_str::<serde_json::Value>(&json_str) {
        Ok(mut v) => {
            if let Some(o) = v.as_object_mut() {
                o.insert("bbox".into(), serde_json::json!([b[0], b[1], b[2], b[3]]));
            }
            v.to_string()
        }
        Err(_) => json_str,
    }
}

/// GET /ogc/tiles/v1/collections
pub async fn collections(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let cols = tiled_collections(&state);
    let doc = state.cache.doc(format!("oat-collections/{base}"), || {
        gisserver::ogcapi_tiles::collections(&base, &cols)
    });
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/collections/{collectionId}
pub async fn collection(
    State(state): State<Arc<AppState>>,
    Path(collection_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let Some(c) = tiled_collection(&state, &collection_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("collection '{collection_id}' is not published"),
        );
    };
    let doc = state.cache.doc(format!("oat-collection/{base}/{collection_id}"), || {
        gisserver::ogcapi_tiles::collection(&base, &c)
    });
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/collections/{collectionId}/tiles
pub async fn tilesets_list(
    State(state): State<Arc<AppState>>,
    Path(collection_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let Some(c) = tiled_collection(&state, &collection_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("collection '{collection_id}' is not published"),
        );
    };
    let doc = state.cache.doc(format!("oat-tilesets/{base}/{collection_id}"), || tilesets(&base, &c));
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/collections/{collectionId}/tiles/{tileMatrixSetId}
pub async fn tileset_one(
    State(state): State<Arc<AppState>>,
    Path((collection_id, tms_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let Some(c) = tiled_collection(&state, &collection_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("collection '{collection_id}' is not published"),
        );
    };
    let Some(layer) = state.layer(&collection_id) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{collection_id}'"));
    };
    let Some(grid) = state.grid_for_layer(&layer, &tms_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("collection '{collection_id}' does not publish grid '{tms_id}'"),
        );
    };
    let doc = state.cache.doc(format!("oat-tileset/{base}/{collection_id}/{tms_id}"), || {
        tileset(&base, &c, &grid)
    });
    json_response((*doc).clone())
}

/// GET /ogc/tiles/v1/collections/{collectionId}/tiles/{tileMatrixSetId}/{tileMatrix}/{tileRow}/{tileCol}
///
/// Rows are top-left (XYZ convention), matching the OGC API - Tiles spec.
/// The tile itself is rendered by the same code path as `/tiles` (raster) or
/// `/mvt` (vector) — including the output cache.
pub async fn tile(
    State(state): State<Arc<AppState>>,
    Path((collection_id, tms_id, z, row, col)): Path<(String, String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(col) = crate::handlers::parse_coord(&col) else {
        return plain(StatusCode::BAD_REQUEST, format!("bad tile column '{col}'"));
    };
    let Some(layer) = state.layer_arc(&collection_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("collection '{collection_id}' is not published"),
        );
    };
    let Some(grid) = state.grid_for_layer(&layer, &tms_id) else {
        return plain(
            StatusCode::NOT_FOUND,
            format!("collection '{collection_id}' does not publish grid '{tms_id}'"),
        );
    };
    let Some(lvl) = grid.level(z) else {
        return plain(StatusCode::NOT_FOUND, format!("no tile matrix {z}"));
    };
    if col >= lvl.matrix_w || row >= lvl.matrix_h {
        return plain(
            StatusCode::NOT_FOUND,
            format!("tile {z}/{col}/{row} out of range"),
        );
    }
    let key = format!("oat/{collection_id}/{}/{z}/{row}/{col}", grid.id);
    if let Some(hit) = state.cache.output_get(&key).await {
        return crate::handlers::tile_response(hit, &headers, "public, max-age=86400");
    }
    let is_vector = layer.is_vector();
    let state2 = Arc::clone(&state);
    let res = tokio::task::spawn_blocking(move || {
        if is_vector {
            crate::tiles::mvt_tile(&layer, &grid, z, col, row)
        } else {
            crate::tiles::raster_tile(&state2, &layer, &grid, z, col, row)
        }
    })
    .await;
    match res {
        Ok(Ok(t)) => {
            state.cache.output_insert(&key, t.clone());
            crate::handlers::tile_response(t, &headers, "public, max-age=86400")
        }
        // OGC API - Tiles 同样要区分「该地址无瓦 / 接口不匹配」与真故障：
        // 一律 400 会把越界地址说成参数错误，也会把解码失败说成客户端问题。
        Ok(Err(e)) => plain(crate::handlers::tile_error_status(&e), e),
        Err(e) => plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("tile render join: {e}"),
        ),
    }
}
