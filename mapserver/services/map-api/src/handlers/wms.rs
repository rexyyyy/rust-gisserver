//! WMS 1.1.1 / 1.3.0 handler (GetCapabilities / GetMap / GetFeatureInfo).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Map as JsonMap;
use gisserver::wms_protocol::wms::{self, CapLayer, WmsError};
use gisserver::wms_protocol::kvp;

use crate::state::AppState;
use crate::tiles;

/// GET /wms?...
pub async fn handle(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let kv = gisserver::wms_protocol::parse_kvp(query.as_deref().unwrap_or(""));
    let get = |k: &str| gisserver::wms_protocol::kvp::lookup(&kv, k);
    let request = get("request").unwrap_or("").to_ascii_uppercase();
    let version = wms::negotiate_version(get("version"));
    let base = crate::handlers::base_url(&state, &headers);

    match request.as_str() {
        "GETCAPABILITIES" => {
            // Capabilities are immutable per catalog: render once per
            // (version, base) per TTL window instead of per request.
            let caps = state.cache.doc(format!("wms-caps/{version}/{base}"), || {
                wms::capabilities_xml(&cap_layers(&state), &version, &base)
            });
            crate::handlers::xml_response((*caps).clone()).into_response()
        }
        "GETMAP" => get_map(Arc::clone(&state), &kv, &headers, &version).await,
        "GETFEATUREINFO" => {
            get_feature_info(Arc::clone(&state), &kv, &version).await
        }
        "GETLEGENDGRAPHIC" => get_legend_graphic(Arc::clone(&state), &kv, &version).await,
        "" => crate::handlers::text_response(
            StatusCode::BAD_REQUEST,
            "text/plain",
            "missing REQUEST parameter".into(),
        ),
        other => {
            let (body, ct) = wms::exception_xml(
                &version,
                Some("InvalidRequest"),
                &format!("unknown REQUEST '{other}'"),
            );
            crate::handlers::text_response(StatusCode::BAD_REQUEST, ct, body).into_response()
        }
    }
}

/// 把任意 CRS 的范围转为 EPSG:4326 度数（给 capabilities 的
/// EX_GeographicBoundingBox / LatLonBoundingBox 用）。地理坐标系直接返回；
/// 投影坐标系取四角变换后取包络，PROJ 未加载或不支持时返回 None。
pub(crate) fn geographic_bounds(bounds: gisserver::wms_common::Bbox, crs: &str) -> Option<gisserver::wms_common::Bbox> {
    use gisserver::wms_common::crs::is_geographic;
    if is_geographic(crs) {
        return Some(bounds);
    }
    let tr = gisserver::projection::Transformer::new_any(crs, "EPSG:4326", gisserver::projection::proj()).ok()?;
    let pts = [
        (bounds.minx, bounds.miny),
        (bounds.minx, bounds.maxy),
        (bounds.maxx, bounds.miny),
        (bounds.maxx, bounds.maxy),
    ];
    let mut minx = f64::INFINITY;
    let mut miny = f64::INFINITY;
    let mut maxx = f64::NEG_INFINITY;
    let mut maxy = f64::NEG_INFINITY;
    let mut any = false;
    for (x, y) in tr.to_source_batch(&pts) {
        if x.is_finite() && y.is_finite() {
            any = true;
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
        }
    }
    if any {
        Some(gisserver::wms_common::Bbox { minx, miny, maxx, maxy })
    } else {
        None
    }
}

fn cap_layers(state: &AppState) -> Vec<CapLayer> {
    state
        .layers
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|l| {
            let raw_bc = l.bounds_crs();
            let bc = if raw_bc.is_empty() { l.def.src_crs.clone() } else { raw_bc };
            CapLayer {
                name: l.name().to_string(),
                title: if l.def.title.is_empty() {
                    l.name().to_string()
                } else {
                    l.def.title.clone()
                },
                bounds: l.bounds(),
                src_crs: l.def.src_crs.clone(),
                // MBTiles/GeoJSON 的 bounds 是 4326 度数而 src_crs 可能是 3857 瓦片方案：
                // 只在两者不同时显式登记，相同则留空由协议层回退 src_crs。
                bounds_crs: if bc == l.def.src_crs {
                    String::new()
                } else {
                    bc.clone()
                },
                // PostGIS：bounds() 为 None（图层运行时不持有 PG 范围），从
                // 启动时探测过的 PgSourceDef.bounds 补（恒为 EPSG:4326 度数）。
                geo_bounds: l
                    .bounds()
                    .and_then(|b| geographic_bounds(b, &bc))
                    .or_else(|| {
                        let reg = state.pg_registry()?;
                        let (_, pg) = reg.find(l.name())?;
                        pg.bounds
                            .map(|b| gisserver::wms_common::Bbox::new(b[0], b[1], b[2], b[3]))
                    }),
                queryable: true,
            }
        })
        .collect()
}

/// GetLegendGraphic: render a small PNG swatch for a layer's style
/// (GeoServer KVP subset: LAYER / STYLE / RULE / FORMAT / WIDTH / HEIGHT).
async fn get_legend_graphic(
    state: Arc<AppState>,
    kv: &[(String, String)],
    version: &str,
) -> Response {
    let get = |k: &str| kvp::lookup(kv, k);
    let layer_name = get("layer").unwrap_or("");
    let Some(layer) = state.layer_arc(layer_name) else {
        return wms_error_response(
            version,
            &WmsError::coded(
                "LayerNotDefined",
                format!("layer '{layer_name}' is not defined"),
            ),
        );
    };
    let format = get("format").unwrap_or("image/png");
    if !format.eq_ignore_ascii_case("image/png") {
        return wms_error_response(
            version,
            &WmsError::coded(
                "InvalidFormat",
                format!("legend format '{format}' is not supported (only image/png)"),
            ),
        );
    }
    let style = get("style").unwrap_or("default");
    if !(style.eq_ignore_ascii_case("default") || style.is_empty()) {
        return wms_error_response(
            version,
            &WmsError::coded("StyleNotDefined", format!("style '{style}' is not defined")),
        );
    }
    let rule = get("rule").filter(|s| !s.is_empty()).map(|s| s.to_string());
    let width: u32 = get("width")
        .and_then(|w| w.parse().ok())
        .unwrap_or(crate::legend::DEF_W);
    let height: u32 = get("height")
        .and_then(|h| h.parse().ok())
        .unwrap_or_else(|| crate::legend::auto_height(&layer, rule.as_deref()));
    let res = tokio::task::spawn_blocking(move || {
        crate::legend::legend_png(&layer, width, height, rule.as_deref())
    })
    .await;
    match res {
        Ok(Ok(png)) => (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "image/png"),
                (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            png,
        )
            .into_response(),
        Ok(Err(e)) => wms_error_response(version, &WmsError::internal(e)),
        Err(e) => wms_error_response(version, &WmsError::internal(format!("legend join: {e}"))),
    }
}

/// GetMap: rasterize + composite + PNG encode on the **blocking pool**
/// (`spawn_blocking`) so a big map render never occupies a tokio worker.
///
/// WMS GetMap is intentionally NOT routed through the output cache: the key
/// space (arbitrary BBOX/WIDTH/HEIGHT/LAYERS/CRS combinations) is unbounded,
/// unlike the discrete z/x/y tile addresses.
async fn get_map(
    state: Arc<AppState>,
    kv: &[(String, String)],
    headers: &HeaderMap,
    version: &str,
) -> Response {
    let layer_names = state.layer_names();
    let configured: Vec<String> = layer_names.clone();
    // The first configured layer supplies the source CRS for validation.
    let src_crs = layer_names
        .first()
        .and_then(|n| state.layer(n))
        .map(|l| l.def.src_crs.clone())
        .unwrap_or_else(|| "EPSG:4326".into());

    let params = match wms::parse_get_map(version, kv, &configured, &src_crs) {
        Ok(p) => p,
        Err(e) => return wms_error_response(version, &e),
    };

    // Render the first requested layer (documented: multi-layer compositing is
    // not implemented; all requested layers must exist).
    let layer = match state.layer_arc(&params.layers[0]) {
        Some(l) => l,
        None => {
            return wms_error_response(
                version,
                &WmsError::coded(
                    "LayerNotDefined",
                    format!("layer '{}' is not defined", params.layers[0]),
                ),
            )
        }
    };

    // PostGIS layer: features are QUERIED (bbox -> SQL) instead of read from
    // an in-memory FeatureCollection, then rendered through the same vector
    // rasterizer (SLD styled path when the layer carries a `.sld`).
    if layer.is_postgis() {
        return pg_get_map(state, layer, &params, version).await;
    }

    let res = tokio::task::spawn_blocking(move || {
        let mut rgba = render_get_map(&state, &layer, &params)?;
        // TRANSPARENT defaults to FALSE -> composite onto BGCOLOR.
        if !params.transparent {
            gisserver::renderer::png::composite_over_bg(&mut rgba, params.bgcolor);
        }
        gisserver::renderer::png::encode_rgba(&rgba, params.width, params.height)
    })
    .await;
    match res {
        Ok(Ok(png)) => {
            let etag = format!("\"{:x}\"", crate::handlers::fnv1a(&png));
            if let Some(inm) = headers.get(axum::http::header::IF_NONE_MATCH) {
                if inm.to_str().map(|v| v.trim() == etag).unwrap_or(false) {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
            }
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, "image/png"),
                    (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
                    (axum::http::header::ETAG, etag.as_str()),
                ],
                png,
            )
                .into_response()
        }
        Ok(Err(e)) => wms_error_response(version, &WmsError::internal(e)),
        Err(e) => wms_error_response(version, &WmsError::internal(format!("render join: {e}"))),
    }
}

/// Transform a request bbox (in `crs`) into an EPSG:4326 envelope. `None`
/// when the transform fails (invalid CRS / coordinates out of range).
fn bbox_to_4326(crs: &str, bbox: [f64; 4]) -> Option<[f64; 4]> {
    let [minx, miny, maxx, maxy] = bbox;
    if crs == gisserver::wms_common::crs::EPSG_4326 || gisserver::wms_common::crs::is_crs84(crs) {
        return Some(bbox);
    }
    let t = gisserver::projection::Transformer::new_any(crs, gisserver::wms_common::crs::EPSG_4326, gisserver::projection::proj())
        .ok()?;
    let mut out: Option<[f64; 4]> = None;
    for (x, y) in [(minx, miny), (maxx, miny), (maxx, maxy), (minx, maxy)] {
        let (lon, lat) = t.to_source(x, y)?;
        out = Some(match out {
            Some([a, b, c, d]) => [a.min(lon), b.min(lat), c.max(lon), d.max(lat)],
            None => [lon, lat, lon, lat],
        });
    }
    out
}

/// PostGIS GetMap: query the frame's features (SQL bbox filter, source limit
/// applied) and rasterize through the same vector path as file layers.
async fn pg_get_map(
    state: Arc<AppState>,
    layer: Arc<crate::layer::LayerRuntime>,
    params: &wms::GetMapParams,
    version: &str,
) -> Response {
    let Some((pool, def)) = state
        .pg_registry()
        .as_ref()
        .and_then(|r| r.find(layer.name()))
        .map(|(p, d)| (p, d.clone()))
    else {
        return wms_error_response(
            version,
            &WmsError::internal("postgis layer has no pool (registry disabled?)"),
        );
    };
    let Some(bbox_4326) = bbox_to_4326(&params.crs, params.bbox.to_array()) else {
        return wms_error_response(
            version,
            &WmsError::coded("InvalidBoundingBox", "bbox outside the CRS valid range"),
        );
    };
    // Frame feature cap: the source `limit` when configured, else a hard
    // default so a country-wide GetMap cannot pull the whole table.
    let limit = def.limit.map(i64::from).unwrap_or(50_000);
    let (sql, qp) = gisserver::pg_source::sql::feature_sql(&def, Some(bbox_4326), None, None, limit, 0);
    let rows = match gisserver::pg_source::sql::query_features(&pool, &sql, &gisserver::pg_source::sql::as_params(&qp))
        .await
    {
        Ok(r) => r,
        Err(e) => return wms_error_response(version, &WmsError::internal(format!("postgis query: {e}"))),
    };
    let fc = serde_json::json!({
        "type": "FeatureCollection",
        "features": rows.into_iter().map(crate::handlers::ensure_feature_id).collect::<Vec<_>>(),
    });
    let p = params.clone();
    let res = tokio::task::spawn_blocking(move || {
        let mut rgba = layer.render_vector(&fc, &p.crs, p.bbox.to_array(), p.width, p.height);
        if !p.transparent {
            gisserver::renderer::png::composite_over_bg(&mut rgba, p.bgcolor);
        }
        gisserver::renderer::png::encode_rgba(&rgba, p.width, p.height)
    })
    .await;
    match res {
        Ok(Ok(png)) => (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "image/png"),
                (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            png,
        )
            .into_response(),
        Ok(Err(e)) => wms_error_response(version, &WmsError::internal(e)),
        Err(e) => wms_error_response(version, &WmsError::internal(format!("render join: {e}"))),
    }
}

fn render_get_map(
    state: &AppState,
    layer: &crate::layer::LayerRuntime,
    params: &wms::GetMapParams,
) -> Result<Vec<u8>, String> {
    // Vector layers render through the vector rasterizer (SLD styled path
    // when the layer carries a `<layer>.sld`, legacy flat style otherwise).
    if let Some(fc) = &layer.vector_fc {
        return Ok(layer.render_vector(
            fc,
            &params.crs,
            params.bbox.to_array(),
            params.width,
            params.height,
        ));
    }
    let (cog, src) = match (&layer.cog, &layer.source) {
        (Some(c), Some(s)) => (c, s.as_ref()),
        _ => return Err("layer has no raster source".into()),
    };
    let style = tiles::raster_style(layer);
    let req = gisserver::renderer::RenderRequest {
        bbox: params.bbox.to_array(),
        crs: &params.crs,
        src_crs: &layer.def.src_crs,
        width: params.width,
        height: params.height,
        resample: gisserver::renderer::Resample::Nearest,
        style,
    };
    let transformer = gisserver::projection::Transformer::new_any(req.crs, req.src_crs, gisserver::projection::proj())
        .map_err(|e| format!("projection: {e}"))?;
    gisserver::renderer::render_with_cog(&req, cog, src, Some(&state.tile_cache), &transformer)
        .map_err(|e| e.to_string())
}

/// GetFeatureInfo: sample the raster at the query pixel and return text/plain.
/// The raster read runs on the **blocking pool** (see `get_map`).
async fn get_feature_info(
    state: Arc<AppState>,
    kv: &[(String, String)],
    version: &str,
) -> Response {
    let get = |k: &str| gisserver::wms_protocol::kvp::lookup(kv, k);
    let layer_name = get("query_layers")
        .or_else(|| get("layers"))
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let Some(layer) = state.layer_arc(&layer_name) else {
        return wms_error_response(version, &WmsError::coded("LayerNotDefined", format!("layer '{layer_name}' is not defined")));
    };
    let (ik, jk) = if version.starts_with("1.3") { ("i", "j") } else { ("x", "y") };
    let parse_px = |k: &str| -> Result<u32, WmsError> {
        get(k)
            .ok_or_else(|| WmsError::coded("InvalidPoint", format!("missing {}", k.to_uppercase())))?
            .trim()
            .parse()
            .map_err(|_| WmsError::coded("InvalidPoint", format!("invalid {}", k.to_uppercase())))
    };
    let (i, j) = match (parse_px(ik), parse_px(jk)) {
        (Ok(i), Ok(j)) => (i, j),
        (Err(e), _) | (_, Err(e)) => return wms_error_response(version, &e),
    };
    let crs_raw = get("crs").or_else(|| get("srs")).unwrap_or("EPSG:4326");
    let bbox_raw = get("bbox").unwrap_or("");
    // WMS 1.3.0 is northing-first (`BBOX = miny,minx,maxy,maxx`) except CRS:84
    // (lon/lat by definition); 1.1.1 is `minx,miny,maxx,maxy`. The protocol
    // crate normalizes GetMap the same way; GetFeatureInfo must do the same
    // for the sample math below.
    let bbox = if version.starts_with("1.3") && !gisserver::wms_common::crs::is_crs84(crs_raw) {
        let v: Vec<f64> = bbox_raw.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if v.len() == 4 {
            Some(gisserver::wms_common::Bbox::new(v[1], v[0], v[3], v[2]))
        } else {
            None
        }
    } else {
        gisserver::wms_common::Bbox::parse(bbox_raw)
    };
    let width: u32 = get("width").and_then(|w| w.parse().ok()).unwrap_or(0);
    let height: u32 = get("height").and_then(|h| h.parse().ok()).unwrap_or(0);
    if bbox.is_none() || width == 0 || height == 0 {
        return wms_error_response(version, &WmsError::plain("missing BBOX/WIDTH/HEIGHT for GetFeatureInfo"));
    }
    if i >= width || j >= height {
        return wms_error_response(version, &WmsError::coded("InvalidPoint", "query point outside the map"));
    }
    // INFO_FORMAT negotiation (GeoServer's common set).
    let info_format = get("info_format").unwrap_or("text/plain");
    if !matches!(
        info_format.to_ascii_lowercase().as_str(),
        "text/plain" | "text/html" | "application/json"
    ) {
        return wms_error_response(
            version,
            &WmsError::coded(
                "InvalidFormat",
                format!("INFO_FORMAT '{info_format}' is not supported"),
            ),
        );
    }
    let bbox_arr = bbox.unwrap().to_array();
    // Vector layers: hit-test features at the click point and return their
    // attributes (GeoServer's GetFeatureInfo on a vector layer).
    if layer.vector_fc.is_some() {
        return match vector_feature_info(
            Arc::clone(&state),
            Arc::clone(&layer),
            crs_raw.to_string(),
            bbox_arr,
            width,
            height,
            i,
            j,
            info_format,
            &layer_name,
        )
        .await
        {
            Ok(resp) => resp,
            Err(e) => wms_error_response(version, &WmsError::coded("NotFound", e)),
        };
    }
    // PostGIS layer: hit-test is a point query (`ST_Intersects`) against the
    // database instead of an in-memory scan.
    if layer.is_postgis() {
        return match pg_feature_info(
            Arc::clone(&state),
            Arc::clone(&layer),
            crs_raw.to_string(),
            bbox_arr,
            width,
            height,
            i,
            j,
            info_format,
            &layer_name,
        )
        .await
        {
            Ok(resp) => resp,
            Err(e) => wms_error_response(version, &WmsError::coded("NotFound", e)),
        };
    }
    match sample_value(
        state,
        layer,
        crs_raw.to_string(),
        bbox.unwrap().to_array(),
        width,
        height,
        i,
        j,
    )
    .await
    {
        Ok(v) => {
            let body = format!("{layer_name}: value = {v}\n");
            (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "text/plain")], body).into_response()
        }
        Err(e) => wms_error_response(version, &WmsError::internal(e)),
    }
}

/// Vector GetFeatureInfo: hit-test features at the click pixel (blocking
/// pool) and format the first hit's attributes per INFO_FORMAT.
async fn vector_feature_info(
    _state: Arc<AppState>,
    layer: Arc<crate::layer::LayerRuntime>,
    crs: String,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    i: u32,
    j: u32,
    info_format: &str,
    layer_name: &str,
) -> Result<Response, String> {
    let fc = layer
        .vector_fc
        .clone()
        .ok_or_else(|| "layer has no vector source".to_string())?;
    let fmt = info_format.to_ascii_lowercase();
    let res = tokio::task::spawn_blocking(move || vector_hit_blocking(&fc, &crs, bbox, width, height, i, j)).await;
    let props = match res {
        Ok(r) => r?,
        Err(e) => return Err(format!("hit join: {e}")),
    };
    Ok(info_response(&fmt, layer_name, props))
}

/// Format GetFeatureInfo attributes per INFO_FORMAT. `None` props = "nothing
/// under the click": 200 with empty content (GeoServer semantics).
fn info_response(fmt: &str, layer_name: &str, props: Option<JsonMap<String, serde_json::Value>>) -> Response {
    let Some(props) = props else {
        let body = if fmt == "application/json" { "{}" } else { "" };
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, if fmt == "application/json" { "application/json" } else { "text/plain; charset=utf-8" })],
            body,
        )
            .into_response();
    };
    if fmt == "application/json" {
        let body = serde_json::to_string_pretty(&props).unwrap_or_else(|_| "{}".into());
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
    }
    // text/plain and text/html share the simple "key = value" body.
    let mut body = format!("{layer_name}\n");
    for (k, v) in &props {
        body.push_str(&format!("{k} = {}\n", json_scalar(v)));
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}

/// PostGIS GetFeatureInfo: click pixel -> lon/lat -> `ST_Intersects` point
/// query; the first hit's attributes are formatted per INFO_FORMAT.
async fn pg_feature_info(
    state: Arc<AppState>,
    layer: Arc<crate::layer::LayerRuntime>,
    crs: String,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    i: u32,
    j: u32,
    info_format: &str,
    layer_name: &str,
) -> Result<Response, String> {
    let Some((pool, def)) = state
        .pg_registry()
        .as_ref()
        .and_then(|r| r.find(layer.name()))
        .map(|(p, d)| (p, d.clone()))
    else {
        return Err("postgis layer has no pool".to_string());
    };
    let fmt = info_format.to_ascii_lowercase();
    // Click pixel -> geo point (request CRS -> EPSG:4326); same pixel math
    // as the vector hit test.
    let [minx, miny, maxx, maxy] = bbox;
    let dx = (maxx - minx) / width as f64;
    let dy = (maxy - miny) / height as f64;
    let ox = minx + (i as f64 + 0.5) * dx;
    let oy = maxy - (j as f64 + 0.5) * dy;
    let (lon, lat) = if crs == gisserver::wms_common::crs::EPSG_4326 || gisserver::wms_common::crs::is_crs84(&crs) {
        (ox, oy)
    } else {
        let transformer = gisserver::projection::Transformer::new_any(
            &crs,
            gisserver::wms_common::crs::EPSG_4326,
            gisserver::projection::proj(),
        )
        .map_err(|e| e.to_string())?;
        transformer
            .to_source(ox, oy)
            .ok_or_else(|| "point outside valid CRS range".to_string())?
    };
    let (sql, mut qp) = gisserver::pg_source::sql::point_feature_sql(&def);
    qp[0] = gisserver::pg_source::sql::SqlParam::F(lon);
    qp[1] = gisserver::pg_source::sql::SqlParam::F(lat);
    let rows = gisserver::pg_source::sql::query_features(&pool, &sql, &gisserver::pg_source::sql::as_params(&qp))
        .await
        .map_err(|e| format!("postgis query: {e}"))?;
    let props = rows
        .first()
        .and_then(|f| f.get("properties"))
        .and_then(|p| p.as_object())
        .cloned();
    Ok(info_response(&fmt, layer_name, props))
}

/// Flatten a JSON scalar for text output.
fn json_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Hit test: click pixel → geo point (request CRS → EPSG:4326) → first
/// feature covering it. Tolerances are 8 canvas pixels expressed in degrees.
fn vector_hit_blocking(
    fc: &serde_json::Value,
    crs: &str,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    i: u32,
    j: u32,
) -> Result<Option<JsonMap<String, serde_json::Value>>, String> {
    // Err is reserved for invalid requests (bad CRS / transform failure);
    // "nothing under the click" is Ok(None) -> 200 with an empty body,
    // matching GeoServer's GetFeatureInfo semantics.
    let transformer = gisserver::projection::Transformer::new_any(crs, gisserver::wms_common::crs::EPSG_4326, gisserver::projection::proj())
        .map_err(|e| e.to_string())?;
    let [minx, miny, maxx, maxy] = bbox;
    let dx = (maxx - minx) / width as f64;
    let dy = (maxy - miny) / height as f64;
    let ox = minx + (i as f64 + 0.5) * dx;
    let oy = maxy - (j as f64 + 0.5) * dy;
    let (lon, lat) = transformer
        .to_source(ox, oy)
        .ok_or_else(|| "point outside valid CRS range".to_string())?;
    let tol_x = dx * 8.0;
    let tol_y = dy * 8.0;
    let features = fc
        .get("features")
        .and_then(|f| f.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    for feat in features {
        let Some(geom) = feat.get("geometry").filter(|g| !g.is_null()) else {
            continue;
        };
        if geom_hit(geom, lon, lat, tol_x, tol_y) {
            return Ok(Some(
                feat.get("properties")
                    .and_then(|p| p.as_object())
                    .cloned()
                    .unwrap_or_default(),
            ));
        }
    }
    Ok(None)
}

/// Point/line/polygon hit test in lon/lat space with pixel-derived tolerance.
fn geom_hit(geom: &serde_json::Value, lon: f64, lat: f64, tol_x: f64, tol_y: f64) -> bool {
    let gtype = geom.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let coords = geom.get("coordinates");
    let pos = |c: &serde_json::Value| -> Option<(f64, f64)> {
        let a = c.as_array()?;
        Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
    };
    let near = |x: f64, y: f64| (x - lon).abs() <= tol_x && (y - lat).abs() <= tol_y;
    let seg_hit = |p0: (f64, f64), p1: (f64, f64)| -> bool {
        // Distance from the point to the segment, normalized by tolerance.
        let vx = p1.0 - p0.0;
        let vy = p1.1 - p0.1;
        let wx = lon - p0.0;
        let wy = lat - p0.1;
        let len2 = vx * vx + vy * vy;
        let t = if len2 <= 1e-18 {
            0.0
        } else {
            ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
        };
        let cx = p0.0 + t * vx;
        let cy = p0.1 + t * vy;
        ((cx - lon) / tol_x).powi(2) + ((cy - lat) / tol_y).powi(2) <= 1.0
    };
    let ring_hit = |ring: &[(f64, f64)]| -> bool {
        // Ray casting in the lon/lat plane (sufficient for click hit tests).
        let mut inside = false;
        for k in 0..ring.len() {
            let (x0, y0) = ring[k];
            let (x1, y1) = ring[(k + 1) % ring.len()];
            if (y0 > lat) != (y1 > lat) {
                let x = x0 + (lat - y0) / (y1 - y0) * (x1 - x0);
                if x > lon {
                    inside = !inside;
                }
            }
        }
        inside
    };
    let line_hit = |line: &serde_json::Value| -> bool {
        let pts: Vec<(f64, f64)> = line
            .as_array()
            .map(|a| a.iter().filter_map(pos).collect())
            .unwrap_or_default();
        pts.windows(2).any(|s| seg_hit(s[0], s[1]))
            || pts.first().map(|p| near(p.0, p.1)).unwrap_or(false)
    };
    match gtype {
        "Point" => coords.and_then(pos).map(|(x, y)| near(x, y)).unwrap_or(false),
        "MultiPoint" => coords
            .and_then(|c| c.as_array())
            .map(|a| a.iter().filter_map(pos).any(|(x, y)| near(x, y)))
            .unwrap_or(false),
        "LineString" => coords.map(line_hit).unwrap_or(false),
        "MultiLineString" => coords
            .and_then(|c| c.as_array())
            .map(|a| a.iter().any(line_hit))
            .unwrap_or(false),
        "Polygon" => coords
            .and_then(|c| c.as_array())
            .map(|rings| {
                rings.iter().any(|r| {
                    r.as_array()
                        .map(|a| a.iter().filter_map(pos).collect::<Vec<_>>())
                        .map(|ring| ring.len() >= 3 && ring_hit(&ring))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        "MultiPolygon" => coords
            .and_then(|c| c.as_array())
            .map(|polys| {
                polys.iter().any(|p| {
                    p.as_array().map(|rings| {
                        rings.iter().any(|r| {
                            r.as_array()
                                .map(|a| a.iter().filter_map(pos).collect::<Vec<_>>())
                                .map(|ring| ring.len() >= 3 && ring_hit(&ring))
                                .unwrap_or(false)
                        })
                    }).unwrap_or(false)
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Sample band 0 at output pixel (i, j) of a GetMap frame (blocking pool).
async fn sample_value(
    state: Arc<AppState>,
    layer: Arc<crate::layer::LayerRuntime>,
    crs: String,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    i: u32,
    j: u32,
) -> Result<f64, String> {
    let res = tokio::task::spawn_blocking(move || {
        sample_value_blocking(&state, &layer, &crs, bbox, width, height, i, j)
    })
    .await;
    match res {
        Ok(r) => r,
        Err(e) => Err(format!("sample join: {e}")),
    }
}

fn sample_value_blocking(
    state: &AppState,
    layer: &crate::layer::LayerRuntime,
    crs: &str,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    i: u32,
    j: u32,
) -> Result<f64, String> {
    let (cog, src) = match (&layer.cog, &layer.source) {
        (Some(c), Some(s)) => (c, s.as_ref()),
        _ => return Err("layer has no raster source".into()),
    };
    let transformer = gisserver::projection::Transformer::new_any(crs, &layer.def.src_crs, gisserver::projection::proj())
        .map_err(|e| e.to_string())?;
    let [minx, miny, maxx, maxy] = bbox;
    let dx = (maxx - minx) / width as f64;
    let dy = (maxy - miny) / height as f64;
    let ox = minx + (i as f64 + 0.5) * dx;
    let oy = maxy - (j as f64 + 0.5) * dy;
    let (sx, sy) = transformer
        .to_source(ox, oy)
        .ok_or_else(|| "point outside valid CRS range".to_string())?;
    let native = cog.levels[0].geo;
    let (u, v) = native.geo_to_pix(sx, sy);
    if u < 0.0 || v < 0.0 || u >= cog.native_width() as f64 || v >= cog.native_height() as f64 {
        return Err("point outside the raster".into());
    }
    let level = &cog.levels[0];
    let tx = (u as u32) / level.tile_w;
    let ty = (v as u32) / level.tile_h;
    let tile = gisserver::cog::read_tile_rgba(level, src, tx, ty, Some(&state.tile_cache))
        .map_err(|e| e.to_string())?;
    let px = (v as u32 - ty * level.tile_h) as usize;
    let py = (u as u32 - tx * level.tile_w) as usize;
    let idx = (px * level.tile_w as usize + py) * 4;
    Ok(tile[idx] as f64)
}

fn wms_error_response(version: &str, e: &WmsError) -> Response {
    let (body, ct) = wms::exception_xml(version, e.code.as_deref(), &e.message);
    crate::handlers::text_response(StatusCode::BAD_REQUEST, ct, body).into_response()
}
