//! WMTS 1.0.0 handler: KVP GetCapabilities / GetTile, RESTful tiles,
//! standalone TileMatrixSet documents.

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use wms_protocol::wmts::{self, WmtsCapLayer, WmtsError};

use crate::state::AppState;

/// GET /wmts?...
pub async fn kvp(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let kv = wms_protocol::parse_kvp(query.as_deref().unwrap_or(""));
    let get = |k: &str| wms_protocol::kvp::lookup(&kv, k);
    let request = get("request").unwrap_or("").to_ascii_uppercase();
    let base = crate::handlers::base_url(&state, &headers);

    match request.as_str() {
        "GETCAPABILITIES" => {
            let xml = wmts_caps(&state, &base);
            crate::handlers::xml_response((*xml).clone()).into_response()
        }
        "GETTILE" => get_tile_kvp(Arc::clone(&state), &kv, &headers).await,
        "" => wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError::missing("missing REQUEST", "REQUEST"),
        ),
        other => wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError {
                http: 400,
                code: "InvalidParameterValue".into(),
                text: format!("unknown REQUEST '{other}'"),
                locator: Some("REQUEST".into()),
            },
        ),
    }
}

/// GET /wmts/1.0.0/WMTSCapabilities.xml
pub async fn caps(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = crate::handlers::base_url(&state, &headers);
    let xml = wmts_caps(&state, &base);
    crate::handlers::xml_response((*xml).clone()).into_response()
}

/// WMTS capabilities document, cached per base URL (immutable per catalog).
fn wmts_caps(state: &AppState, base: &str) -> Arc<String> {
    state.cache.doc(format!("wmts-caps/{base}"), || {
        wmts::capabilities_xml(&cap_layers(state), &all_grids(state), base)
    })
}

/// GET /wmts/rest/{layer}/{style}/{tms}/{z}/{row}/{col}.png|.pbf
pub async fn rest_tile(
    State(state): State<Arc<AppState>>,
    Path((layer, style, tms, z, row, col)): Path<(String, String, String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let ext = col.rsplit('.').next().unwrap_or("png").to_ascii_lowercase();
    let Some(col) = crate::handlers::parse_coord(&col) else {
        return crate::handlers::text_response(
            StatusCode::BAD_REQUEST,
            "text/plain",
            format!("bad tile column '{col}'"),
        )
        .into_response();
    };
    let format = if ext == "pbf" {
        "application/vnd.mapbox-vector-tile"
    } else {
        "image/png"
    };
    serve_tile(
        Arc::clone(&state),
        layer,
        style,
        tms,
        z,
        row,
        col,
        format.to_string(),
        &headers,
    )
    .await
}

/// GET /tileMatrixSets/{id}
pub async fn tile_matrix_set(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    // Cached per grid id (documents are immutable per catalog). An unknown
    // id renders to the empty string, which doubles as the "not found"
    // sentinel — negatives are never cached (a grid could appear later).
    let xml = state.cache.doc(format!("wmts-tms/{id}"), || {
        // Custom map first; the guard is dropped before the fallback so
        // `grid_for_layer` can re-lock `custom_grids` without recursion.
        let from_custom = state
            .custom_grids
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        let g = match from_custom {
            Some(g) => Some(g),
            None => {
                let guard = state.layers.read().unwrap_or_else(|e| e.into_inner());
                guard
                    .iter()
                    .filter_map(|l| state.grid_for_layer(l, &id))
                    .next()
            }
        };
        g.map(|g| wmts::tile_matrix_set_xml(&g))
            .unwrap_or_default()
    });
    if xml.is_empty() {
        return crate::handlers::text_response(
            StatusCode::NOT_FOUND,
            "text/plain",
            format!("no TileMatrixSet '{id}'"),
        );
    }
    crate::handlers::xml_response((*xml).clone()).into_response()
}

fn cap_layers(state: &AppState) -> Vec<WmtsCapLayer> {
    state
        .layers
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|l| {
            // WGS84BoundingBox 规范要求必须是经纬度。投影 CRS（如 EPSG:4522）
            // 的配置 bounds 是米单位平面坐标，直接写入会让客户端把它当 4326
            // transformExtent 得到 NaN 污染视图——这里先转成 4326 度数包络。
            // PROJ 未加载 / 不支持时 geographic_bounds 返回 None，则不下发该元素
            // （好过下发错误的投影坐标）。
            let raw_bc = l.bounds_crs();
            let bc = if raw_bc.is_empty() {
                l.def.src_crs.clone()
            } else {
                raw_bc
            };
            let geo = l.bounds().and_then(|b| crate::handlers::wms::geographic_bounds(b, &bc));
            WmtsCapLayer {
                name: l.name().to_string(),
                title: if l.def.title.is_empty() {
                    l.name().to_string()
                } else {
                    l.def.title.clone()
                },
                grids: l.def.grids.clone(),
                bounds: geo,
            }
        })
        .collect()
}

fn all_grids(state: &AppState) -> Vec<wms_common::tile::TileMatrixSet> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for l in state.layers.read().unwrap_or_else(|e| e.into_inner()).iter() {
        for g in state.grids_for_layer(l) {
            if !seen.contains(&g.id) {
                seen.push(g.id.clone());
                out.push(g);
            }
        }
    }
    out
}

async fn get_tile_kvp(
    state: Arc<AppState>,
    kv: &[(String, String)],
    headers: &HeaderMap,
) -> Response {
    let parsed = match wmts::parse_get_tile(kv) {
        Ok(p) => p,
        Err(e) => return wmts_error(StatusCode::BAD_REQUEST, &e),
    };
    let (layer, style, tms, z, row, col, format) = parsed;
    serve_tile(state, layer, style, tms, z, row, col, format, headers).await
}

/// WMTS GetTile / REST tile: validate, then output-cache lookup, then render
/// on the **blocking pool** (`spawn_blocking`) so tokio workers stay free.
async fn serve_tile(
    state: Arc<AppState>,
    layer_name: String,
    style: String,
    tms_id: String,
    z: u32,
    row: u32,
    col: u32,
    format: String,
    headers: &HeaderMap,
) -> Response {
    let Some(layer) = state.layer_arc(&layer_name) else {
        return wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError::invalid(format!("no layer '{layer_name}'"), "LAYER"),
        );
    };
    if style != "default" && !style.is_empty() {
        return wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError::invalid(format!("no style '{style}'"), "STYLE"),
        );
    }
    let Some(grid) = state.grid_for_layer(&layer, &tms_id) else {
        return wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError::invalid(format!("no TileMatrixSet '{tms_id}'"), "TILEMATRIXSET"),
        );
    };
    let Some(lvl) = grid.level(z) else {
        return wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError::invalid(format!("no TileMatrix {z}"), "TILEMATRIX"),
        );
    };
    if col >= lvl.matrix_w || row >= lvl.matrix_h {
        return wmts_error(
            StatusCode::BAD_REQUEST,
            &WmtsError {
                http: 400,
                code: "TileOutOfRange".into(),
                text: format!("tile {z}/{row}/{col} out of range"),
                locator: Some("TILEROW".into()),
            },
        );
    }

    let key = format!("{layer_name}/{tms_id}/{z}/{row}/{col}/{format}");
    if let Some(hit) = state.cache.output_get(&key).await {
        return crate::handlers::tile_response(hit, headers, "public, max-age=86400");
    }
    let st = Arc::clone(&state);
    // MVT format -> vector tiles; PNG -> raster.
    let is_vector =
        format == "application/vnd.mapbox-vector-tile" || format == "application/x-protobuf";
    let res = tokio::task::spawn_blocking(move || {
        if is_vector {
            crate::tiles::mvt_tile(&layer, &grid, z, col, row)
        } else {
            crate::tiles::raster_tile(&st, &layer, &grid, z, col, row)
        }
    })
    .await;
    match res {
        Ok(Ok(t)) => {
            state.cache.output_insert(&key, t.clone());
            crate::handlers::tile_response(t, headers, "public, max-age=86400")
        }
        Ok(Err(e)) => {
            // 瓦片错误按「请求问题 / 服务端故障」分流（见 tile_error_status）：
            // 该地址没有瓦 -> 404 + TileOutOfRange；无法兑现的 CRS、端点与数据
            // 类型不符 -> 400 + InvalidParameterValue；其余才是 500。一律报 500
            // 会让客户端重试永远不可能成功的请求，也会让管理端能力探活把
            // 「不支持」读成「服务坏了」。
            let status = crate::handlers::tile_error_status(&e);
            let err = if status == StatusCode::NOT_FOUND {
                WmtsError::out_of_range(e)
            } else if status.is_client_error() {
                // 栅格/矢量归档类型不符、不支持的 format 都是 FORMAT 参数的问题；
                // 只有无法兑现的 CRS 才指向 TILEMATRIXSET。
                let locator = if e.contains("CRS") { "TILEMATRIXSET" } else { "FORMAT" };
                WmtsError::invalid(e, locator)
            } else {
                WmtsError::internal(e)
            };
            wmts_error(status, &err)
        }
        Err(e) => wmts_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &WmtsError::internal(format!("tile render join: {e}")),
        ),
    }
}

fn wmts_error(status: StatusCode, e: &WmtsError) -> Response {
    let xml = wmts::exception_xml(e);
    crate::handlers::text_response(
        status,
        "application/xml; charset=utf-8",
        xml,
    )
    .into_response()
}
