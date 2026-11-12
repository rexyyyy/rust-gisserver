//! MVT handler: `/mvt/{layer}/{z}/{x}/{y}.pbf`, grid variant, TileJSON, style.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::cache::TileBuild;
use crate::state::AppState;
use gisserver::pg_source::sql::SqlParam;
use gisserver::wms_common::layer::SourceKind;

/// GET /mvt/{layer}/{z}/{x}/{y}.pbf (Web Mercator default grid).
pub async fn tile(
    State(state): State<Arc<AppState>>,
    Path((layer_name, z, x, y)): Path<(String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(y) = crate::handlers::parse_coord(&y) else {
        return plain(StatusCode::BAD_REQUEST, format!("bad tile row '{y}'"));
    };
    serve(Arc::clone(&state), layer_name, None, z, x, y, &headers).await
}

/// GET /mvt/{layer}/{tms}/{z}/{x}/{y}.pbf (explicit grid id).
pub async fn tile_grid(
    State(state): State<Arc<AppState>>,
    Path((layer_name, tms, z, x, y)): Path<(String, String, u32, u32, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(y) = crate::handlers::parse_coord(&y) else {
        return plain(StatusCode::BAD_REQUEST, format!("bad tile row '{y}'"));
    };
    serve(Arc::clone(&state), layer_name, Some(tms), z, x, y, &headers).await
}

/// Output-cache hit -> serve bytes directly. Miss -> encode runs on the
/// **blocking pool** (`spawn_blocking`): MVT encoding clones and walks the
/// whole FeatureCollection, which must not occupy a tokio worker.
///
/// The cache key carries the resolved grid id: the file-vector encoder is now
/// grid-aware (projected sources keep their own CRS via `encode_projected`),
/// so the same `z/x/y` under different coordinate interfaces yields different
/// bytes and must not collide in the output cache.
async fn serve(
    state: Arc<AppState>,
    layer_name: String,
    tms: Option<String>,
    z: u32,
    x: u32,
    y: u32,
    headers: &HeaderMap,
) -> Response {
    let Some(layer) = state.layer_arc(&layer_name) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{layer_name}'"));
    };
    // --- PostGIS dynamic path: tiles are QUERIED, not encoded from memory.
    // Two coordinate interfaces are served (see `serve_pg`):
    //   * WebMercatorQuad / 3857      -> geometry reprojected to EPSG:3857
    //   * <source SRID> / EPSG:<srid> -> tiles stay in the ORIGINAL projection
    if matches!(layer.def.source, SourceKind::Postgis(_)) {
        let Some((pool, def)) = state
            .pg_registry()
            .as_ref()
            .and_then(|r| r.find(&layer_name))
            .map(|(p, d)| (p, d.clone()))
        else {
            return plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("postgis layer '{layer_name}' has no pool (registry disabled?)"),
            );
        };
        // 发布时选定的网格（OGC 标准 / 天地图 / Conf.xml 自定义）优先：切片包络
        // 必须与该网格的 origin 与分辨率一致，否则前端按自定义 LOD 寻址会错位。
        // `3857` 显式接口除外——那条路径走 ST_TileEnvelope 的原生 Web Mercator。
        let grid = if matches!(
            tms.as_deref(),
            Some("3857") | Some("EPSG:3857") | Some("WebMercatorQuad")
        ) {
            None
        } else {
            tms.as_deref()
                .and_then(|t| state.grid_for_layer(&layer, t))
                .or_else(|| state.default_grid(&layer))
        };
        return serve_pg(state, layer_name, pool, &def, tms.as_deref(), grid, z, x, y, &headers)
            .await;
    }
    // Resolve the effective grid: the requested `tms` interface when given, else
    // the layer's default. The file-vector MVT path is grid-aware (a projected
    // source stays in its own projection via `gisserver::mvt::encode_projected`), so the
    // grid must both reach `mvt_tile` and key the output cache.
    let grid_id = tms
        .clone()
        .unwrap_or_else(|| state.config.default_grid_id(&layer.def));
    let grid = state
        .grid_for_layer(&layer, &grid_id)
        .unwrap_or_else(|| gisserver::wms_common::tile::TileMatrixSet::web_mercator_quad(layer.def.tile_px));
    if let Some(lvl) = grid.level(z) {
        if x >= lvl.matrix_w || y >= lvl.matrix_h {
            return plain(StatusCode::NOT_FOUND, format!("tile {z}/{x}/{y} out of range"));
        }
    }
    let key = format!("mvt/{layer_name}/{}/{z}/{x}/{y}", grid.id);
    if let Some(hit) = state.cache.output_get(&key).await {
        return crate::handlers::pbf_response(hit, headers, "public, max-age=86400");
    }
    let res = tokio::task::spawn_blocking(move || crate::tiles::mvt_tile(&layer, &grid, z, x, y)).await;
    match res {
        Ok(Ok(t)) => {
            // Cold path: gzip once, then cache + serve the compressed bytes.
            let t = crate::tiles::gzip_pbf(t.bytes);
            state.cache.output_insert(&key, t.clone());
            crate::handlers::pbf_response(t, headers, "public, max-age=86400")
        }
        Ok(Err(e)) => plain(crate::handlers::tile_error_status(&e), e),
        Err(e) => plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mvt render join: {e}"),
        ),
    }
}

/// GET /mvt/{layer}/tilejson.json
pub async fn tilejson(
    State(state): State<Arc<AppState>>,
    Path(layer_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(layer) = state.layer(&layer_name) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{layer_name}'"));
    };
    // PostGIS layers own their TileJSON (dual coordinate interfaces).
    if matches!(layer.def.source, SourceKind::Postgis(_)) {
        return pg_tilejson(state, layer_name, None, headers).await;
    }
    let grid = state.default_grid(&layer);
    let (minzoom, maxzoom, bounds) = match &grid {
        Some(g) => {
            let zs: Vec<u32> = g.levels.iter().map(|l| l.z).collect();
            let min = *zs.first().unwrap_or(&0);
            let max = *zs.last().unwrap_or(&0);
            (min, max, g.full_extent().unwrap_or([-180.0, -85.0, 180.0, 85.0]))
        }
        None => (0, 24, [-180.0, -85.0, 180.0, 85.0]),
    };
    let base = crate::handlers::base_url(&state, &headers);
    let template = format!("{base}/mvt/{layer_name}/{{z}}/{{x}}/{{y}}.pbf");
    // TileJSON is immutable per (layer, base): render once per TTL window.
    let doc = state.cache.doc(format!("mvt-tilejson/{base}/{layer_name}"), || {
        let tj = gisserver::mvt::tilejson(&template, &layer_name, minzoom as u8, maxzoom as u8, bounds);
        serde_json::to_string(&tj).unwrap_or_else(|_| "{}".into())
    });
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        (*doc).clone(),
    )
        .into_response()
}

/// GET /mvt/{layer}/style.json -- the configured vector style served verbatim
/// (client-side MapLibre style), or a minimal default.
pub async fn style(
    State(state): State<Arc<AppState>>,
    Path(layer_name): Path<String>,
) -> Response {
    let Some(layer) = state.layer(&layer_name) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{layer_name}'"));
    };
    // Style is immutable per layer: render once per TTL window (no base URL
    // in the document, so the key is layer-only).
    let body = state.cache.doc(format!("mvt-style/{layer_name}"), || {
        match layer.style.as_ref().and_then(|s| s.vector.as_ref()) {
            Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".into()),
            None => serde_json::json!({
                "version": 8,
                "name": layer_name,
                "sources": { "layer": { "type": "vector", "tiles": ["/mvt/{layer}/{z}/{x}/{y}.pbf".replace("{layer}", &layer_name)] } },
                "layers": [
                    { "id": "points", "type": "circle", "source": "layer", "paint": { "circle-color": "#d43", "circle-radius": 4 } },
                    { "id": "lines", "type": "line", "source": "layer", "paint": { "line-color": "#284" } },
                    { "id": "fills", "type": "fill", "source": "layer", "paint": { "fill-color": "#5a5", "fill-opacity": 0.6 } }
                ]
            })
            .to_string(),
        }
    });
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        (*body).clone(),
    )
        .into_response()
}

/// Serve one dynamic MVT tile for a PostGIS layer.
///
/// Coordinate interfaces (the `tms` path segment selects; omitted = 3857):
///
/// | interface | segment values                     | geometry in tile       |
/// |-----------|------------------------------------|------------------------|
/// | 经纬度    | `3857` / `EPSG:3857` / `WebMercatorQuad` | EPSG:3857 (reprojected) |
/// | 原投影    | `<srid>` / `EPSG:<srid>` (e.g. `4543`)   | source SRID, no transform |
///
/// Both interfaces go through the foyer output cache
/// (`mvt-pg/{layer}/{crs}/{z}/{x}/{y}`), so repeated requests never hit the
/// database within the TTL window. P0 adds: singleflight for concurrent cold
/// misses, a negative cache for empty tiles, and once-per-build gzip of the
/// pbf payload (hot hits serve compressed bytes verbatim).
async fn serve_pg(
    state: Arc<AppState>,
    layer_name: String,
    pool: std::sync::Arc<gisserver::pg_source::PgPool>,
    def: &gisserver::pg_source::PgSourceDef,
    tms: Option<&str>,
    grid: Option<gisserver::wms_common::tile::TileMatrixSet>,
    z: u32,
    x: u32,
    y: u32,
    headers: &axum::http::HeaderMap,
) -> Response {
    enum Iface {
        /// Standard XYZ over EPSG:3857 (MapLibre / OpenLayers / Cesium).
        WebMercator,
        /// A published grid: the tile envelope comes from its own origin and
        /// resolution, geometry is cut in the grid's CRS.
        Grid(gisserver::wms_common::tile::TileMatrixSet, i32),
        /// Original projection grid (see `gisserver::pg_source::grid`) — legacy path used
        /// when the layer publishes no custom grid.
        Projected,
    }
    let srid_str = def.srid.to_string();
    let iface = match &grid {
        // Published grid in a non-Web-Mercator CRS: authoritative.
        Some(g) if g.crs != gisserver::wms_common::crs::EPSG_3857 => {
            match epsg_of(&g.crs) {
                Some(code) => Iface::Grid(g.clone(), code),
                None => {
                    return plain(
                        StatusCode::BAD_REQUEST,
                        format!("grid '{}' has unsupported crs '{}'", g.id, g.crs),
                    )
                }
            }
        }
        // Web Mercator grid, or no grid at all: keep the ST_TileEnvelope path
        // unless the caller explicitly asked for the source CRS interface.
        _ => {
            if matches!(tms, Some(s) if s == srid_str || s == format!("EPSG:{}", def.srid)) {
                Iface::Projected
            } else {
                Iface::WebMercator
            }
        }
    };
    if let Some(mz) = def.minzoom {
        if z < u32::from(mz) {
            return plain(StatusCode::NOT_FOUND, format!("zoom {z} below minzoom {mz}"));
        }
    }
    if let Some(mz) = def.maxzoom {
        if z > u32::from(mz) {
            return plain(StatusCode::NOT_FOUND, format!("zoom {z} above maxzoom {mz}"));
        }
    }
    let layer_id = def.layer_id_or(&layer_name);
    let crs_tag = match &iface {
        Iface::WebMercator => "3857".to_string(),
        Iface::Grid(g, _) => g.id.clone(),
        Iface::Projected => def.srid.to_string(),
    };
    let key = format!("mvt-pg/{layer_name}/{crs_tag}/{z}/{x}/{y}");
    // P0: positive cache holds gzip'd pbf — a hot hit is a pure byte copy.
    if let Some(hit) = state.cache.output_get(&key).await {
        return crate::handlers::pbf_response(hit, headers, "public, max-age=86400");
    }
    // P0: negative cache — an empty tile stays 404 without a DB round-trip.
    if state.cache.empty_get(&key) {
        return plain(StatusCode::NOT_FOUND, "empty tile".to_string());
    }

    // Build the SQL before joining the flight: pure, cheap work that should
    // never be repeated by waiters. Only the actual query is coalesced.
    enum Query {
        Tile(String),
        Env(String, Vec<SqlParam>),
    }
    let query = match iface {
        Iface::WebMercator => Query::Tile(gisserver::pg_source::sql::tile_sql(&pool, def, &layer_id)),
        Iface::Grid(g, out_srid) => {
            let Some([minx, miny, maxx, maxy]) = g.tile_bounds(z, x, y) else {
                return plain(StatusCode::NOT_FOUND, format!("tile {z}/{x}/{y} out of grid range"));
            };
            Query::Env(
                gisserver::pg_source::sql::tile_sql_env(def, &layer_id, out_srid),
                envelope_params(def, [minx, miny, maxx, maxy], z),
            )
        }
        Iface::Projected => {
            let Some([minx, miny, maxx, maxy]) = gisserver::pg_source::grid::tile_bounds(z, x, y) else {
                return plain(StatusCode::NOT_FOUND, format!("tile {z}/{x}/{y} out of grid range"));
            };
            Query::Env(
                gisserver::pg_source::sql::tile_sql_projected(def, &layer_id),
                envelope_params(def, [minx, miny, maxx, maxy], z),
            )
        }
    };

    // P0 singleflight: exactly ONE database query per cold tile, however many
    // clients request it simultaneously. The leader writes the positive or
    // negative cache entry before publishing the result to every waiter.
    let build_key = key.clone();
    let build_state = Arc::clone(&state);
    let outcome = state
        .cache
        .tile_flights
        .run(&key, move || {
            let pool = Arc::clone(&pool);
            async move {
                let res = match query {
                    Query::Tile(sql) => gisserver::pg_source::sql::query_tile(&pool, &sql, z, x, y).await,
                    Query::Env(sql, params) => {
                        gisserver::pg_source::sql::query_tile_env(&pool, &sql, &params).await
                    }
                };
                match res {
                    Ok(Some(bytes)) if !bytes.is_empty() => {
                        // gzip once on the cold path; cache + serve the
                        // compressed bytes (see `tiles::gzip_pbf`).
                        let tile = crate::tiles::gzip_pbf(bytes);
                        build_state.cache.output_insert(&build_key, tile.clone());
                        TileBuild::Tile(tile)
                    }
                    Ok(_) => {
                        build_state.cache.empty_insert(&build_key);
                        TileBuild::Empty
                    }
                    Err(e) => {
                        // Walk the source chain: tokio-postgres' top Display
                        // is terse ("db error"); the cause carries the detail.
                        let mut msg = format!("postgis tile query: {e}");
                        let mut src = std::error::Error::source(&e);
                        while let Some(s) = src {
                            msg.push_str(&format!(" | {s}"));
                            src = s.source();
                        }
                        // Errors are shared by this burst but NEVER cached:
                        // a transient pool/DB failure must not become a 500
                        // for the whole TTL window.
                        TileBuild::Error(msg)
                    }
                }
            }
        })
        .await;

    match &*outcome {
        TileBuild::Tile(tile) => crate::handlers::pbf_response(
            tile.clone(),
            headers,
            "public, max-age=86400",
        ),
        TileBuild::Empty => plain(StatusCode::NOT_FOUND, "empty tile".to_string()),
        TileBuild::Error(msg) => plain(StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
    }
}

/// GET /mvt/{layer}/{tms}/tilejson.json -- TileJSON per coordinate interface.
///
/// The projected variant advertises `"crs": "EPSG:<srid>"` and bounds in the
/// source SRID's map units; the 3857 variant keeps standard 4326 bounds.
pub async fn tilejson_grid(
    State(state): State<Arc<AppState>>,
    Path((layer_name, tms)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Response {
    pg_tilejson(state, layer_name, Some(tms), headers).await
}

/// TileJSON for a PostGIS layer. `tms = None` -> 3857 (default interface).
///
/// P2: rendering probes the database (bounds + catalog field types, ~90 ms
/// on this deployment). The rendered document is memoized in the foyer `docs`
/// cache under `(base, layer, interface)`; concurrent cold renders coalesce
/// into one through `CacheHub::doc_async`. The shared TTL and
/// `scope=docs` purge refresh it just like any other metadata document.
async fn pg_tilejson(
    state: Arc<AppState>,
    layer_name: String,
    tms: Option<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(layer) = state.layer(&layer_name) else {
        return plain(StatusCode::NOT_FOUND, format!("no layer '{layer_name}'"));
    };
    if !matches!(layer.def.source, SourceKind::Postgis(_)) {
        return plain(
            StatusCode::BAD_REQUEST,
            format!("'{layer_name}' is not a postgis layer"),
        );
    }
    let pg_reg = state.pg_registry();
    let Some((pool, def)) = pg_reg
        .as_ref()
        .and_then(|r| r.find(&layer_name))
        .map(|(p, d)| (p, d.clone()))
    else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "postgis registry disabled".to_string());
    };
    // 发布网格优先：TileJSON 必须宣告与切片一致的 crs/grid/原点/分辨率，
    // 前端（OpenLayers）据此建自定义 TileGrid；显式 `3857` 接口除外。
    let grid = if matches!(
        tms.as_deref(),
        Some("3857") | Some("EPSG:3857") | Some("WebMercatorQuad")
    ) {
        None
    } else {
        tms.as_deref()
            .and_then(|t| state.grid_for_layer(&layer, t))
            .or_else(|| state.default_grid(&layer))
    };
    let base = crate::handlers::base_url(&state, &headers);
    let key = format!(
        "mvt-tilejson-pg/{base}/{layer_name}/{}",
        tms.as_deref().unwrap_or("_default")
    );
    if let Some(doc) = state.cache.doc_get(&key) {
        return tilejson_response((*doc).clone());
    }
    let doc = state
        .cache
        .doc_async(key, move || async move {
            render_pg_tilejson(pool, def, grid, tms, base, layer_name).await
        })
        .await;
    tilejson_response((*doc).clone())
}

fn tilejson_response(doc: String) -> Response {
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        doc,
    )
        .into_response()
}

/// The database-bound half of [`pg_tilejson`], executed once per cache window
/// (and once per burst of concurrent cold requests).
async fn render_pg_tilejson(
    pool: std::sync::Arc<gisserver::pg_source::PgPool>,
    def: gisserver::pg_source::PgSourceDef,
    grid: Option<gisserver::wms_common::tile::TileMatrixSet>,
    tms: Option<String>,
    base: String,
    layer_name: String,
) -> String {
    let mut extra = serde_json::Map::new();
    let (crs_tag, template, bounds, crs_member, zrange) = match &grid {
        Some(g) if g.crs != gisserver::wms_common::crs::EPSG_3857 => {
            // 网格 CRS 与数据 SRID 一致时直接取源包络；否则退回网格满幅范围。
            let same_crs = epsg_of(&g.crs) == Some(def.srid);
            let b = if same_crs {
                gisserver::pg_source::sql::query_bounds(&pool, &gisserver::pg_source::sql::bounds_sql_src(&def))
                    .await
                    .ok()
                    .flatten()
                    .or_else(|| g.full_extent())
                    .unwrap_or([
                        -gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                        -gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                        gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                        gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                    ])
            } else {
                g.full_extent().unwrap_or([-180.0, -90.0, 180.0, 90.0])
            };
            let zs: Vec<u32> = g.levels.iter().map(|l| l.z).collect();
            let zmin = *zs.first().unwrap_or(&0);
            let zmax = *zs.last().unwrap_or(&0);
            extra.insert("grid".into(), serde_json::json!(g.id));
            extra.insert("tileSize".into(), serde_json::json!(g.tile_w));
            extra.insert("origin".into(), serde_json::json!([g.origin_x, g.origin_y]));
            extra.insert(
                "resolutions".into(),
                serde_json::json!(g.levels.iter().map(|l| l.resolution).collect::<Vec<_>>()),
            );
            (
                g.id.clone(),
                format!("{base}/mvt/{layer_name}/{}/{{z}}/{{x}}/{{y}}.pbf", g.id),
                b,
                Some(g.crs.clone()),
                (zmin, zmax),
            )
        }
        _ => {
            let same_srid = matches!(
                tms,
                Some(s) if s == def.srid.to_string() || s == format!("EPSG:{}", def.srid)
            );
            if same_srid {
                let b = gisserver::pg_source::sql::query_bounds(&pool, &gisserver::pg_source::sql::bounds_sql_src(&def))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or([
                        -gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                        -gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                        gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                        gisserver::pg_source::grid::PROJECTED_WORLD_HALF,
                    ]);
                (
                    def.srid.to_string(),
                    format!("{base}/mvt/{layer_name}/{srid}/{{z}}/{{x}}/{{y}}.pbf", srid = def.srid),
                    b,
                    Some(format!("EPSG:{}", def.srid)),
                    (u32::from(def.minzoom.unwrap_or(0)), u32::from(def.maxzoom.unwrap_or(22))),
                )
            } else {
                let b = gisserver::pg_source::sql::query_bounds(&pool, &gisserver::pg_source::sql::bounds_sql(&def))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or([-180.0, -85.0, 180.0, 85.0]);
                (
                    "3857".to_string(),
                    format!("{base}/mvt/{layer_name}/{{z}}/{{x}}/{{y}}.pbf"),
                    b,
                    None,
                    (u32::from(def.minzoom.unwrap_or(0)), u32::from(def.maxzoom.unwrap_or(22))),
                )
            }
        }
    };
    // vector_layers field typing (one catalog query per cache window; P2).
    // SQL/view layers resolve columns from the row description instead of
    // information_schema.
    let fields = match &def.from {
        Some(f) => {
            gisserver::pg_source::sql::discover_from_fields(&pool, f, &def.geometry_column)
                .await
                .unwrap_or_default()
        }
        None => {
            gisserver::pg_source::sql::discover_fields(&pool, &def.schema, &def.table, &def.geometry_column)
                .await
                .unwrap_or_default()
        }
    };
    let mut field_map = serde_json::Map::new();
    for (name, kind) in fields {
        field_map.insert(name, serde_json::Value::String(kind.to_string()));
    }
    let (minzoom, maxzoom) = zrange;
    let mut doc = serde_json::json!({
        "tilejson": "3.0.0",
        "name": layer_name,
        "tiles": [template],
        "minzoom": minzoom,
        "maxzoom": maxzoom,
        "bounds": bounds,
        "vector_layers": [{
            "id": def.layer_id_or(&layer_name),
            "minzoom": minzoom,
            "maxzoom": maxzoom,
            "fields": field_map,
        }],
    });
    if let Some(crs) = crs_member {
        doc["crs"] = serde_json::Value::String(crs);
    }
    // TileJSON 的 grid 必须是可直接查 `tileMatrixSets/{id}` 的网格 id；默认 WebMercator
    // 接口内部用 `3857` 作 crs 标签，不能原样写进 grid（客户端会拿它查网格 404）。
    let grid_id = match crs_tag.as_str() {
        "3857" | "EPSG:3857" => "WebMercatorQuad".to_string(),
        other => other.to_string(),
    };
    doc["grid"] = serde_json::Value::String(grid_id);
    if let Some(obj) = doc.as_object_mut() {
        for (k, v) in extra {
            obj.insert(k, v);
        }
    }
    doc.to_string()
}

fn plain(status: StatusCode, msg: String) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        msg,
    )
        .into_response()
}

/// `EPSG:4490` / `urn:ogc:def:crs:EPSG::4490` → `4490`.
fn epsg_of(crs: &str) -> Option<i32> {
    let digits: String = crs
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

/// Envelope params for `tile_sql_env` / `tile_sql_projected`: `$1..$4` are the
/// tile bbox in the grid CRS, `$5` (only when simplify is configured) is zoom.
fn envelope_params(
    def: &gisserver::pg_source::PgSourceDef,
    bbox: [f64; 4],
    z: u32,
) -> Vec<SqlParam> {
    let [minx, miny, maxx, maxy] = bbox;
    let mut params = vec![
        SqlParam::F(minx),
        SqlParam::F(miny),
        SqlParam::F(maxx),
        SqlParam::F(maxy),
    ];
    if def.simplify.is_some() {
        params.push(SqlParam::I32(z as i32));
    }
    params
}
