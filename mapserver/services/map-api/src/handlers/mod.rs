//! HTTP handlers: router assembly + shared response helpers.

pub mod cache;
pub mod cog;
pub mod health;
pub mod mbtiles;
pub mod mvt;
pub mod ogcapi_features;
pub mod ogcapi_tiles;
pub mod pmtiles;
pub mod terrain;
pub mod threed_tiles;
pub mod tms;
pub mod wfs;
pub mod wms;
pub mod wmts;
pub mod xyz;

use std::fs::File;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use http_body::Body as HttpBody;
use tower_http::compression::{CompressionLayer, Predicate};
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, RequestId, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{MakeSpan, TraceLayer};

use crate::state::AppState;

/// Compression predicate: skip gzip for responses that are already encoded
/// or inherently incompressible — image tiles (PNG/JPEG/WebP) and binary 3D
/// Tiles / terrain chunks. Responses that already carry `Content-Encoding`
/// (MBTiles' pre-gzip'd pbf, our own cached gzip tiles) are skipped here too.
///
/// P0: **uncompressed pbf is no longer excluded**. Dynamic PostGIS tiles are
/// pre-compressed once at build time and served with `Content-Encoding: gzip`;
/// pbf from archives that are not already gzip'd now gets negotiated compression
/// by tower-http instead of being sent at full size.
#[derive(Clone, Copy)]
struct NoRecompress;

impl Predicate for NoRecompress {
    fn should_compress<B>(&self, response: &axum::http::Response<B>) -> bool
    where
        B: HttpBody,
    {
        if response.headers().contains_key(header::CONTENT_ENCODING) {
            return false;
        }
        let ct = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        !(ct.starts_with("image/") || ct.starts_with("application/octet-stream"))
    }
}

/// 数据面路由（瓦片 + 要素服务）：直接挂在根路径，由 [`router`] 通过 `merge` 接入。
/// 受 [`data_auth`] 守卫（当 `server.data_token` 配置时）。
pub fn data_router(state: Arc<AppState>) -> Router {
    // 慢请求面（WMS GetMap / WFS 大查询）：单次重出图、大范围矢量查询或
    // PostGIS 重查询可能长时间占住 worker，挂 TimeoutLayer 在超时（默认 30s）
    // 后回 408，避免单点慢请求拖垮整服务并发。瓦片 / 归档等其它数据面不在此列，
    // 维持原有低延迟路径。
    let data_slow = Router::new()
        .route("/wms", get(wms::handle))
        .route("/wms/", get(wms::handle))
        .route("/wfs", get(wfs::handle))
        .route("/wfs/", get(wfs::handle))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(30),
        ));

    Router::new()
        // OGC API - Features (`/ogc/features/v1`, GeoServer-compatible path)
        .route("/ogc/features/v1", get(ogcapi_features::landing))
        .route("/ogc/features/v1/", get(ogcapi_features::landing))
        .route("/ogc/features/v1/conformance", get(ogcapi_features::conformance))
        .route("/ogc/features/v1/api", get(ogcapi_features::api))
        .route("/ogc/features/v1/collections", get(ogcapi_features::collections))
        .route(
            "/ogc/features/v1/collections/{collection_id}",
            get(ogcapi_features::collection),
        )
        .route(
            "/ogc/features/v1/collections/{collection_id}/items",
            get(ogcapi_features::items),
        )
        .route(
            "/ogc/features/v1/collections/{collection_id}/items/{feature_id}",
            get(ogcapi_features::item),
        )
        // OGC API - Tiles (`/ogc/tiles/v1`; rows top-left like XYZ)
        .route("/ogc/tiles/v1", get(ogcapi_tiles::landing))
        .route("/ogc/tiles/v1/", get(ogcapi_tiles::landing))
        .route("/ogc/tiles/v1/conformance", get(ogcapi_tiles::conformance))
        .route("/ogc/tiles/v1/tileMatrixSets", get(ogcapi_tiles::tile_matrix_sets_list))
        .route(
            "/ogc/tiles/v1/tileMatrixSets/{tms_id}",
            get(ogcapi_tiles::tile_matrix_set_one),
        )
        .route("/ogc/tiles/v1/collections", get(ogcapi_tiles::collections))
        .route(
            "/ogc/tiles/v1/collections/{collection_id}",
            get(ogcapi_tiles::collection),
        )
        .route(
            "/ogc/tiles/v1/collections/{collection_id}/tiles",
            get(ogcapi_tiles::tilesets_list),
        )
        .route(
            "/ogc/tiles/v1/collections/{collection_id}/tiles/{tms_id}",
            get(ogcapi_tiles::tileset_one),
        )
        .route(
            "/ogc/tiles/v1/collections/{collection_id}/tiles/{tms_id}/{z}/{row}/{col}",
            get(ogcapi_tiles::tile),
        )
        // WMTS (KVP + RESTful + capabilities). matchit 0.7 `:param` captures a
        // whole segment, so the REST tile route ends at `:col` and the handler
        // strips the `.png` / `.pbf` suffix itself.
        .route("/wmts", get(wmts::kvp))
        .route("/wmts/", get(wmts::kvp))
        .route("/wmts/1.0.0/WMTSCapabilities.xml", get(wmts::caps))
        .route(
            "/wmts/rest/{layer}/{style}/{tms}/{z}/{row}/{col}",
            get(wmts::rest_tile),
        )
        .route("/tileMatrixSets/{id}", get(wmts::tile_matrix_set))
        // TMS (y-flip). A `:grid` variant (one more segment) selects a non-default
        // published grid, e.g. geographic `WorldCRS84Quad` / srs=4326.
        .route("/tms/1.0.0/{layer}/{z}/{x}/{y}", get(tms::tile))
        .route("/tms/1.0.0/{layer}/{grid}/{z}/{x}/{y}", get(tms::tile_grid))
        // XYZ alias (no flip), plus the same `:grid` selector variant.
        .route("/tiles/{layer}/{z}/{x}/{y}", get(xyz::tile))
        .route("/tiles/{layer}/{grid}/{z}/{x}/{y}", get(xyz::tile_grid))
        // MVT
        .route("/mvt/{layer}/{z}/{x}/{y}", get(mvt::tile))
        .route("/mvt/{layer}/{tms}/{z}/{x}/{y}", get(mvt::tile_grid))
        .route("/mvt/{layer}/tilejson.json", get(mvt::tilejson))
        .route("/mvt/{layer}/{tms}/tilejson.json", get(mvt::tilejson_grid))
        .route("/mvt/{layer}/style.json", get(mvt::style))
        // Archives
        .route("/pmtiles/{name}", get(pmtiles::archive))
        .route("/cog/{layer}", get(cog::archive))
        // MBTiles（SQLite 瓦片包）：归档信息 + XYZ 寻址瓦片。
        // 瓦片路由容忍 `.png` / `.pbf` 后缀（matchit 连同数字一起捕获，
        // handler 内 `parse_coord` 剥离），与 TMS / XYZ 路由行为一致。
        .route("/mbtiles/{name}", get(mbtiles::info))
        .route("/mbtiles/{name}/{z}/{x}/{y}", get(mbtiles::tile))
        // 3D Tiles 服务（配置目录下每个 *.xml 一个服务）
        .route("/3dtiles", get(threed_tiles::list))
        .route("/3dtiles/", get(threed_tiles::list))
        .route("/3dtiles/{name}", get(threed_tiles::entry))
        .route("/3dtiles/{name}/{*rest}", get(threed_tiles::file))
        // Quantized-Mesh 1.1 地形服务（<catalog.root>/terrain/*.terrainpack）
        .route("/terrain", get(terrain::list))
        .route("/terrain/", get(terrain::list))
        .route("/terrain/{name}/layer.json", get(terrain::layer_json))
        .route("/terrain/{name}/{z}/{x}/{y}", get(terrain::tile))

        .merge(data_slow)
        .with_state(state.clone())
        // 数据面鉴权：当 `server.data_token` 配置时，所有数据面（OGC 瓦片 /
        // 要素）请求必须携带令牌，否则 401。零配置（未设 data_token）时不设防，
        // 向后兼容。令牌携带方式见 `data_auth`。
        .layer(from_fn_with_state(state.clone(), data_auth))}

/// 数据面鉴权中间件：仅作用于 [`data_router`]。
///
/// 当 `server.data_token` 配置时，数据面请求必须携带令牌，否则返回 401。
/// 令牌支持三种携带方式（便于不同客户端）：
///
/// * 请求头 `X-Data-Token: <token>`
/// * `Authorization: Bearer <token>`
/// * 查询参数 `?token=<token>`（浏览器 / viewer 可直接拼在瓦片 URL 上）
///
/// 配置主令牌后，主令牌**或** r-manager 共享库 `sys_api_key` 中启用且未过期的
/// `data` key 均被认可（见 [`crate::apikeys`]，15 秒同步、用途隔离）。
/// 未配置 `data_token` 时不设防（零配置向后兼容）—— 与 `admin.token` 的
/// 「安全带」定位一致：它是令牌护栏，不是完整鉴权体系（角色 / ACL 交给网关或
/// manager）。数据面不包括 `/health` `/ready` `/viewer` `/cesium` `/api/*`
/// 等运维 / 管理路径（它们不在 [`data_router`] 内，分别由 ops / admin 面处理）。
async fn data_auth(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    let Some(tok) = &state.config.server.data_token else {
        return next.run(req).await;
    };
    // 主令牌或共享库中启用且未过期的 `data` key 均被认可（见 crate::apikeys）。
    // 收集三种携带方式后统一比对，避免逐处重复校验逻辑。
    let header_val = headers.get("x-data-token").and_then(|v| v.to_str().ok());
    let bearer_val = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let v = v.trim();
            v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))
        });
    let query_val = req.uri().query().and_then(|q| {
        q.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == "token").then_some(v)
        })
    });
    let ok = [header_val, bearer_val, query_val]
        .into_iter()
        .flatten()
        .any(|v| v == tok.as_str() || state.api_keys.is_valid("data", v));
    if ok {
        next.run(req).await
    } else {
        let mut resp = crate::admin::http::json(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "error": "missing/invalid data token (set X-Data-Token, Authorization: Bearer, or ?token=)"
            }),
        );
        resp
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        resp
    }
}

/// 运维面路由（健康 / 缓存 / 用量 / 预览 / 配置）：路径分散，由 [`router`] 通过 `merge` 接入。
pub fn ops_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ops
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        // Cache administration (stats + purge; see `cache.rs`)
        .route("/api/cache", get(cache::stats))
        .route("/api/cache/purge", axum::routing::post(cache::purge))
        // Manager integration: usage metering (`admin::http`).
        .route("/api/usage", get(crate::admin::http::usage_handler))
        // 内置预览页 + 当前配置便捷接口
        .route("/viewer", get(viewer))
        .route("/viewer-projected", get(viewer_projected))
        .route("/viewer-arcgis", get(viewer_arcgis))
        .route("/viewer-mvt", get(viewer_mvt))
        // 投影预览页依赖库（proj4 + OpenLayers）同源内嵌伺服，无需联网 CDN
        .route("/vendor/proj4.js", get(vendor_proj4_js))
        .route("/vendor/ol.js", get(vendor_ol_js))
        .route("/vendor/ol.css", get(vendor_ol_css))
        .route("/cesium", get(cesium_viewer))
        .route("/api/config", get(api_config))

        .with_state(state)}

/// 管理面路由（`/admin/*`）：统一前缀，由 [`router`] 通过 `nest("/admin", …)` 挂载。
/// 服务 CRUD 完整覆盖：GET/PUT/DELETE `/services/:kind/:name` + POST `…/rename`
/// （对齐 GeoServer REST 配置 API 的图层增删改查）。
pub fn admin_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Manager integration: admin surface + usage metering (`admin::http`).
        // Token-protected when `admin.token` is configured.
        .route("/reload", axum::routing::post(crate::admin::http::reload))
        .route("/services", get(crate::admin::http::services))
        .route(
            "/services/{kind}/{name}/state",
            axum::routing::put(crate::admin::http::set_state),
        )
        .route(
            "/services/{kind}/{name}/check",
            axum::routing::post(crate::admin::check::check_service),
        )
        .route("/access-logs", get(crate::admin::http::access_logs))
        .route("/browse", get(crate::admin::catalog_admin::browse))
        // 只读数据发现（r-manager Workspace/Store/Resource 组织层的数据面接口）
        .route("/probe", get(crate::admin::discover::probe))
        .route("/discover", get(crate::admin::discover::discover))
        .route("/capabilities", get(crate::admin::discover::capabilities))
        .route(
            "/pg-tables",
            axum::routing::post(crate::admin::discover::pg_tables),
        )
        .route(
            "/services",
            axum::routing::post(crate::admin::catalog_admin::create),
        )
        .route(
            "/services/{kind}/{name}",
            get(crate::admin::catalog_admin::get_config)
                .put(crate::admin::catalog_admin::update_config)
                .delete(crate::admin::catalog_admin::delete_service),
        )
        .route(
            "/services/{kind}/{name}/rename",
            axum::routing::post(crate::admin::catalog_admin::rename_service),
        )
        .route(
            "/pg-test",
            axum::routing::post(crate::admin::catalog_admin::pg_test),
        )
        .route(
            "/pg-query",
            axum::routing::post(crate::admin::catalog_admin::pg_query),
        )
        .route(
            "/pg-import-shp",
            axum::routing::post(crate::admin::pg_import::pg_import_shp),
        )
        // 同一导入的 SSE 版本：大 Shapefile 时前端可持续收到 stage/rows 进度
        .route(
            "/pg-import-shp/stream",
            axum::routing::post(crate::admin::pg_import::pg_import_shp_stream),
        )
        // 解析 ArcGIS Conf.xml → 瓦片网格（发布向导「选择瓦片配置 XML」）
        .route(
            "/tile-grid",
            axum::routing::post(crate::admin::tile_grid::tile_grid),
        )
        .route(
            "/components",
            get(crate::admin::catalog_admin::components),
        )
        // Style library (SLD files under <catalog.root>/styles).
        .route("/styles", get(crate::admin::styles::list))
        .route("/styles/{name}/content", get(crate::admin::styles::content))
        .route("/styles/{name}/legend.png", get(crate::admin::styles::legend))
        .route(
            "/styles/{name}",
            axum::routing::post(crate::admin::styles::upload)
                .delete(crate::admin::styles::remove),
        )

        .with_state(state)}

/// 自定义 trace span：在 tower-http 默认记录的 method / uri / status / latency 之外，
/// 额外记录 `request_id`（由最外层 SetRequestIdLayer 写入请求扩展的 `RequestId`），
/// 使逐请求日志与回传前端的 `x-request-id` 响应头对齐，便于跨服务串联排查。
/// `status` / `latency` 字段保留为空，供 `DefaultOnResponse` 在响应阶段回填。
#[derive(Clone, Copy, Default)]
struct TraceSpanMaker;

impl<B> MakeSpan<B> for TraceSpanMaker
where
    B: HttpBody,
{
    fn make_span(&mut self, request: &axum::http::Request<B>) -> tracing::Span {
        let req_id = request
            .extensions()
            .get::<RequestId>()
            .and_then(|r| r.header_value().to_str().ok())
            .unwrap_or("")
            .to_string();
        tracing::info_span!(
            "request",
            method = %request.method(),
            uri = %request.uri().path(),
            version = ?request.version(),
            request_id = %req_id,
            status = tracing::field::Empty,
            latency = tracing::field::Empty,
        )
    }
}

/// Build the application router.
///
/// 三段装配：数据面（`data_router`）与运维面（`ops_router`）直接 `merge` 到根
/// 路径；管理面（`admin_router`）以 `/admin` 为前缀 `nest` 挂载。这样服务 /
/// 运维 / 管理三类路由在装配处一目了然，也便于将来给管理面单独加鉴权 / 限流层。
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // 数据面：瓦片 + 要素服务（直接挂在根路径）
        .merge(data_router(state.clone()))
        // 运维面：健康 / 缓存 / 用量 / 预览 / 配置（分散路径）
        .merge(ops_router(state.clone()))
        // 管理面：统一 /admin 前缀
        .nest("/admin", admin_router(state.clone()))
        // Request metering for every service route (skips admin/api/ops paths).
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::admin::usage::track,
        ))
        // Compress text-ish responses only (see `NoRecompress`).
        .layer(CompressionLayer::new().compress_when(NoRecompress))
        // Cesium 等浏览器客户端跨域访问 3D Tiles 时需要 CORS
        .layer(CorsLayer::permissive())
        // 逐请求追踪：方法 / 状态码 / 耗时 + 请求 ID（与下方 x-request-id 串联）。
        .layer(TraceLayer::new_for_http().make_span_with(TraceSpanMaker))
        // 请求 ID：每个请求生成唯一 id 并经 `x-request-id` 响应头回传，作为最外层
        // 让追踪 / 用量等内层中间件都能读到，供前端 / 网关 / 日志跨服务对齐同一请求。
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid::default()))
        // CORS 会追加自己的 Vary 值，内层也可能已有 Accept-Encoding，导致多行
        // Vary；这里统一合并成一行并保证包含 Accept-Encoding —— 压缩协商与
        // 缓存 gzip 瓦片都依赖它，老旧代理只取第一行时尤其重要。
        .layer(axum::middleware::from_fn(vary_accept_encoding))
}

/// Normalize `Vary` to a single header that always includes
/// `Accept-Encoding`. `CorsLayer` appends its own Vary on the way out and the
/// tile helper sets `Accept-Encoding`, so a response can otherwise carry two
/// Vary field lines. Multiple lines are legal, but a single merged value is
/// understood by every cache/proxy and keeps the compressed representations
/// correctly separated.
async fn vary_accept_encoding(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let merged = {
        let headers = resp.headers();
        let lines: Vec<&str> = headers
            .get_all(header::VARY)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        let mut parts: Vec<&str> = Vec::new();
        for s in &lines {
            for t in s.split(',') {
                let t = t.trim();
                if !t.is_empty() && !parts.iter().any(|p| p.eq_ignore_ascii_case(t)) {
                    parts.push(t);
                }
            }
        }
        let has_ae = parts.iter().any(|p| p.eq_ignore_ascii_case("accept-encoding"));
        if has_ae && lines.len() == 1 && parts.len() == 1 {
            None
        } else {
            if !has_ae {
                parts.push("Accept-Encoding");
            }
            HeaderValue::from_str(&parts.join(", ")).ok()
        }
    };
    if let Some(v) = merged {
        resp.headers_mut().insert(header::VARY, v);
    }
    resp
}

/// Text response helper.
pub fn text_response(status: StatusCode, content_type: &str, body: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, content_type)],
        body,
    )
        .into_response()
}

/// XML response (cacheable).
pub fn xml_response(xml: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        xml,
    )
        .into_response()
}

/// Recover an HTTP status from a tile-pipeline error string.
///
/// Tile errors leave `spawn_blocking` as plain strings, so the XYZ / TMS / WMTS
/// front ends classify them through here (OGC API - Tiles already answers 400
/// for every tile failure). Reporting 500 tells the client the server is broken
/// and makes it retry a request that can never succeed -- and it also makes a
/// capability probe (r-manager walks these endpoints to decide what a layer can
/// publish) read "unsupported" as "broken", so every *request* problem answers
/// 4xx:
/// * a CRS the server cannot honour (CGCS2000 Gauss-Krüger tile on a build
///   without the PROJ backend, a UTM zone outside the accepted set) -> 400;
/// * the layer's data type is not servable on this endpoint (raster endpoint on
///   a vector archive, a layer with no raster source such as a PostGIS table on
///   WMTS) -> 400: the client picked the wrong interface;
/// * the address simply has no tile (absent from the archive, absent from the
///   ArcGIS cache's bundle slot matrix, outside the tile matrix, no such zoom
///   level) -> 404, so the client stops asking.
/// Genuine failures (decode / encode / IO / join) stay 500.
pub fn tile_error_status(msg: &str) -> StatusCode {
    if msg.contains("unsupported CRS") {
        StatusCode::BAD_REQUEST
    } else if msg.contains("not in archive")
        || msg.contains("not in cache")
        || msg.contains("out of range")
        || msg.contains("no zoom level")
    {
        StatusCode::NOT_FOUND
    } else if msg.contains("archive is not a")
        || msg.contains("archive is a vector")
        || msg.contains("layer is not a")
        || msg.contains("no raster source")
        || msg.contains("raster tiles only")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Tile response with ETag / 304 support and optional content encoding.
pub fn tile_response(
    t: crate::tiles::TileOut,
    req_headers: &HeaderMap,
    cache_control: &'static str,
) -> Response {
    let etag = format!("\"{:x}\"", fnv1a(&t.bytes));
    if let Some(inm) = req_headers.get(header::IF_NONE_MATCH) {
        if inm.to_str().map(|v| v.trim() == etag).unwrap_or(false) {
            return StatusCode::NOT_MODIFIED.into_response();
        }
    }
    let mut resp = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, t.content_type.as_str()),
            (header::CACHE_CONTROL, cache_control),
            (header::ETAG, etag.as_str()),
        ],
        t.bytes,
    )
        .into_response();
    if let Some(enc) = &t.content_encoding {
        if let Ok(v) = HeaderValue::from_str(enc) {
            resp.headers_mut().insert(header::CONTENT_ENCODING, v);
            resp.headers_mut()
                .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
        }
    }
    resp
}

/// Serve a pbf tile whose cached payload may already be gzip'd (P0).
///
/// * Client accepts gzip -> the compressed bytes go out verbatim (zero CPU,
///   `Vary: Accept-Encoding` is set by `tile_response`).
/// * Client does not accept gzip -> decompress once and return the raw pbf.
///   The response still varies on `Accept-Encoding` because the same URL has
///   two representations; tower-http leaves it alone (no gzip in the request).
/// * Raw (uncompressed) cached payload -> `tile_response` as before; the
///   compression layer negotiates gzip with the client.
pub fn pbf_response(
    t: crate::tiles::TileOut,
    req_headers: &HeaderMap,
    cache_control: &'static str,
) -> Response {
    if t.content_encoding.as_deref() == Some("gzip") && !accepts_gzip(req_headers) {
        return match crate::tiles::gunzip_pbf(&t.bytes) {
            Ok(raw) => {
                let mut plain_tile = t;
                plain_tile.bytes = raw;
                plain_tile.content_encoding = None;
                let mut resp = tile_response(plain_tile, req_headers, cache_control);
                resp.headers_mut()
                    .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
                resp
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                format!("decompress cached pbf tile: {e}"),
            )
                .into_response(),
        };
    }
    tile_response(t, req_headers, cache_control)
}

/// `Accept-Encoding` negotiation for the cached-gzip path. Handles multiple
/// header lines, comma-separated tokens, quality values and `*`.
fn accepts_gzip(headers: &HeaderMap) -> bool {
    for value in headers.get_all(header::ACCEPT_ENCODING) {
        let Ok(line) = value.to_str() else { continue };
        for token in line.split(',') {
            let mut parts = token.trim().split(';');
            let name = parts.next().unwrap_or("").trim();
            if name != "gzip" && name != "*" {
                continue;
            }
            let acceptable = parts.all(|p| {
                let p = p.trim();
                let Some(q) = p.strip_prefix("q=") else {
                    return true;
                };
                q.parse::<f32>().map(|q| q > 0.0).unwrap_or(true)
            });
            if acceptable {
                return true;
            }
        }
    }
    false
}

/// Normalize an `ST_AsGeoJSON(row)` output into a GeoJSON Feature with a
/// top-level `id`. PostGIS keeps non-geometry columns in `properties`; the
/// `id` alias lands there when PostGIS does not lift it, but WFS fids and
/// OGC API item ids expect it at the top level.
pub fn ensure_feature_id(mut f: serde_json::Value) -> serde_json::Value {
    if f.get("type").and_then(|t| t.as_str()) != Some("Feature") {
        return f;
    }
    if f.get("id").is_none() {
        let id = f.get("properties").and_then(|p| p.get("id")).cloned();
        if let Some(id) = id {
            f["id"] = id;
        }
    }
    f
}

/// FNV-1a 64-bit hash (fast, stable per payload -- enough for ETags).
pub fn fnv1a(bytes: &[u8]) -> u64 {    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Parse a `Range: bytes=a-b` header into `(start, end_inclusive)`.
pub fn parse_range(headers: &HeaderMap) -> Option<(u64, u64)> {
    let r = headers.get(header::RANGE)?.to_str().ok()?;
    let rest = r.strip_prefix("bytes=")?;
    let (a, b) = rest.split_once('-')?;
    let start = a.parse::<u64>().ok()?;
    let end = if b.is_empty() {
        None
    } else {
        Some(b.parse::<u64>().ok()?)
    };
    Some((start, end.unwrap_or(u64::MAX)))
}

/// Serve a **local file** honoring an optional `Range` header, reading only the
/// requested byte window with positioned reads. Blocking file IO runs on the
/// blocking pool.
///
/// This replaces the old `range_response(&fs::read(path))` pattern, which
/// buffered entire (GB-scale) PMTiles / 3D Tiles payloads into memory on every
/// request. Full (range-less) reads stream in 1 MiB chunks with progressive
/// allocation instead of `fs::read`'s single file-sized allocation.
pub async fn file_range_response(
    path: std::path::PathBuf,
    headers: &HeaderMap,
    content_type: &'static str,
) -> Response {
    let range = parse_range(headers);
    let path_display = path.display().to_string();
    let res = tokio::task::spawn_blocking(move || -> std::io::Result<FileRange> {
        let file = File::open(&path)?;
        let total = file.metadata()?.len();
        Ok(match range {
            Some((start, end)) if start < total => {
                let end = end.min(total - 1);
                let n = (end - start + 1) as usize;
                let mut buf = vec![0u8; n];
                storage::read_exact_at(&file, &mut buf, start)?;
                FileRange::Partial {
                    total,
                    start,
                    end,
                    bytes: buf,
                }
            }
            Some(_) => FileRange::Unsatisfiable { total },
            None => {
                let mut bytes: Vec<u8> = Vec::new();
                let mut off = 0u64;
                while off < total {
                    let n = ((total - off).min(1 << 20)) as usize;
                    let base = bytes.len();
                    bytes.resize(base + n, 0);
                    storage::read_exact_at(&file, &mut bytes[base..], off)?;
                    off += n as u64;
                }
                FileRange::Full { bytes }
            }
        })
    })
    .await;
    match res {
        Ok(Ok(FileRange::Full { bytes, .. })) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                (header::ACCEPT_RANGES, "bytes"),
            ],
            bytes,
        )
            .into_response(),
        Ok(Ok(FileRange::Partial {
            total,
            start,
            end,
            bytes,
        })) => {
            let content_range = format!("bytes {start}-{end}/{total}");
            (
                StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, content_type),
                    (header::CONTENT_RANGE, content_range.as_str()),
                    (header::ACCEPT_RANGES, "bytes"),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(Ok(FileRange::Unsatisfiable { total })) => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CONTENT_RANGE, format!("bytes */{total}").as_str()),
            ],
            String::new(),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain")],
            format!("read {path_display}: {e}"),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain")],
            format!("file read join: {e}"),
        )
            .into_response(),
    }
}

enum FileRange {
    Full {
        bytes: Vec<u8>,
    },
    Partial {
        total: u64,
        start: u64,
        end: u64,
        bytes: Vec<u8>,
    },
    Unsatisfiable {
        total: u64,
    },
}

/// The canonical base URL for this request (config override or origin from
/// the Host header).
pub fn base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(p) = &state.config.server.public_url {
        return p.clone();
    }
    if let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) {
        return format!("http://{host}");
    }
    state.base_url.clone()
}

/// The tiny built-in Leaflet viewer, served inline.
async fn viewer() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../../../../web/viewer.html").to_string(),
    )
        .into_response()
}

/// 投影坐标系（EPSG:4522 / wh_custom_grid_4522）MVT 浏览验证页：
/// 用 proj4 注册投影 CRS + 自定义 TileGrid，请求留在源投影的矢量瓦片，
/// 验证 `mvt::encode_projected` 的产出可被前端正确定位渲染。
async fn viewer_projected() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../../../../web/viewer_projected.html").to_string(),
    )
        .into_response()
}

/// 投影预览页依赖库：`proj4` UMD bundle（编译期内嵌，与运行时工作目录无关）。
async fn vendor_proj4_js() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../../../web/vendor/proj4.js").to_string(),
    )
        .into_response()
}

/// 投影预览页依赖库：OpenLayers 单文件构建（编译期内嵌）。
async fn vendor_ol_js() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../../../web/vendor/ol.js").to_string(),
    )
        .into_response()
}

/// 投影预览页样式：OpenLayers ol.css（编译期内嵌）。
async fn vendor_ol_css() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../../../web/vendor/ol.css").to_string(),
    )
        .into_response()
}

/// 内置 Cesium 三维浏览页：动态加载 `/3dtiles` 目录下的全部 3D Tiles 服务。
async fn cesium_viewer() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../../../../web/cesium.html").to_string(),
    )
        .into_response()
}

/// 内置 ArcGIS 预切片缓存浏览页：地理系（EPSG:4490）自定义网格栅格查看器。
/// 依赖 `/ogc/tiles/v1/tileMatrixSets/{grid}` 暴露的原生 resolution 建网格，
/// 避免用 scaleDenominator 反算导致的 ~5.8% 寻址偏差。
async fn viewer_arcgis() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../../../../web/viewer_arcgis.html").to_string(),
    )
        .into_response()
}

/// 内置 PostGIS 矢量瓦片预览页：按发布网格（含 Conf.xml 自定义 LOD）加载 MVT。
/// 与 `viewer_arcgis` 共用同一套网格读取口径（`/ogc/tiles/v1/tileMatrixSets`）。
async fn viewer_mvt() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../../../../web/viewer_mvt.html").to_string(),
    )
        .into_response()
}

/// Parse a tile coordinate segment that may carry an extension (e.g. `36.png`),
/// which matchit 0.7 captures together with the number.
pub fn parse_coord(s: &str) -> Option<u32> {
    let stem = s.split('.').next().unwrap_or(s);
    stem.parse().ok()
}

/// Current layer configuration as JSON (admin convenience).
async fn api_config(State(state): State<Arc<AppState>>) -> Response {
    let cfg = serde_json::to_string_pretty(&state.config).unwrap_or_else(|_| "{}".into());
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], cfg).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::util::ServiceExt;

    /// P0: raw pbf now negotiates gzip, but already-encoded payloads (our
    /// cached gzip tiles / MBTiles) are never compressed twice.
    #[test]
    fn compression_predicate_allows_raw_pbf_but_skips_encoded() {
        let resp = |ct: &'static str, enc: Option<&'static str>| {
            let mut r = axum::http::Response::new(Body::empty());
            r.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
            if let Some(e) = enc {
                r.headers_mut()
                    .insert(header::CONTENT_ENCODING, HeaderValue::from_static(e));
            }
            r
        };
        assert!(NoRecompress.should_compress(&resp("application/x-protobuf", None)));
        assert!(!NoRecompress.should_compress(&resp(
            "application/x-protobuf",
            Some("gzip")
        )));
        assert!(!NoRecompress.should_compress(&resp("image/png", None)));
        assert!(!NoRecompress.should_compress(&resp("application/octet-stream", None)));
        assert!(NoRecompress.should_compress(&resp("application/json", None)));
    }

    /// CORS must not swallow the Accept-Encoding variant declaration.
    #[tokio::test]
    async fn responses_vary_on_accept_encoding_even_with_cors() {
        let state = Arc::new(
            crate::AppState::new(
                crate::config::ServerConfig {
                    server: crate::config::ServerOpts::default(),
                    admin: Default::default(),
                    layers: vec![],
                    archives: Default::default(),
                    catalog: Default::default(),
                    grids: Default::default(),
                },
                None,
            )
            .await
            .unwrap(),
        );
        let app = router(state);
        let req = axum::http::Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let vary_lines: Vec<_> = resp.headers().get_all(header::VARY).iter().collect();
        assert_eq!(vary_lines.len(), 1, "Vary must be a single merged header");
        let vary = resp
            .headers()
            .get_all(header::VARY)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ");
        assert!(vary.to_ascii_lowercase().contains("accept-encoding"), "vary={vary}");
        assert!(vary.contains("origin"), "CORS vary must survive: {vary}");
    }

    #[test]
    fn accepts_gzip_negotiation() {
        let headers = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(v));
            h
        };
        assert!(accepts_gzip(&headers("gzip, deflate, br")));
        assert!(accepts_gzip(&headers("deflate, gzip;q=1.0")));
        assert!(accepts_gzip(&headers("*")));
        assert!(!accepts_gzip(&headers("gzip;q=0")));
        assert!(!accepts_gzip(&headers("br")));
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    /// P0: gzip-capable clients get the cached compressed bytes verbatim;
    /// others get a transparent decompression, with Vary on both paths.
    #[tokio::test]
    async fn pbf_response_decompresses_only_for_non_gzip_clients() {
        let raw: Vec<u8> = (0..8_000).map(|i| (i % 37) as u8).collect();
        let tile = crate::tiles::gzip_pbf(raw.clone());
        assert_eq!(tile.content_encoding.as_deref(), Some("gzip"));

        let mut gz = HeaderMap::new();
        gz.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        let resp = pbf_response(tile.clone(), &gz, "public, max-age=86400");
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), tile.bytes.as_slice(), "no per-request gzip");

        let resp = pbf_response(tile, &HeaderMap::new(), "public, max-age=86400");
        assert!(resp.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(
            resp.headers().get(header::VARY).and_then(|v| v.to_str().ok()),
            Some("Accept-Encoding")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), raw.as_slice());
    }

    #[tokio::test]
    async fn parameterized_routes_match() {
        let state = Arc::new(
            crate::AppState::new(
                crate::config::ServerConfig {
                    server: crate::config::ServerOpts::default(),
                    admin: Default::default(),
                    layers: vec![],
                    archives: Default::default(),
                    catalog: Default::default(),
                    grids: Default::default(),
                },
                None,
            )
            .await
            .unwrap(),
        );
        let app = router(state);
        for path in [
            "/health",
            "/tms/1.0.0/world/6/50/36",
            "/tms/1.0.0/world/6/50/36.png",
            "/tms/1.0.0/world/WorldCRS84Quad/6/50/36.png",
            "/tiles/world/6/50/36.png",
            "/tiles/world/WorldCRS84Quad/6/50/36.png",
            "/mvt/places/6/50/27.pbf",
            "/mvt/places/tilejson.json",
            "/wmts/rest/world/default/WebMercatorQuad/6/27/50.png",
            "/tileMatrixSets/WebMercatorQuad",
            "/pmtiles/cities",
            "/cog/world",
            "/mbtiles/test",
            "/mbtiles/test/6/50/36",
            "/mbtiles/test/6/50/36.png",
            "/3dtiles",
            "/3dtiles/nope",
            "/3dtiles/nope/tileset.json",
            "/terrain",
            "/terrain/nope/layer.json",
            "/terrain/nope/0/0/0.terrain",
            "/wfs",
            "/wfs?service=WFS&request=GetCapabilities",
            "/ogc/features/v1",
            "/ogc/features/v1/conformance",
            "/ogc/features/v1/api",
            "/ogc/features/v1/collections",
            "/ogc/features/v1/collections/nope",
            "/ogc/features/v1/collections/nope/items",
            "/ogc/features/v1/collections/nope/items/1",
            "/ogc/tiles/v1",
            "/ogc/tiles/v1/conformance",
            "/ogc/tiles/v1/tileMatrixSets",
            "/ogc/tiles/v1/tileMatrixSets/WebMercatorQuad",
            "/ogc/tiles/v1/collections",
            "/ogc/tiles/v1/collections/nope",
            "/ogc/tiles/v1/collections/nope/tiles",
            "/ogc/tiles/v1/collections/nope/tiles/WebMercatorQuad",
            "/ogc/tiles/v1/collections/nope/tiles/WebMercatorQuad/6/27/50",
        ] {
            let req = axum::http::Request::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            let status = resp.status();
            let body_len = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap()
                .len();
            println!("{path} -> {status} (body {body_len} bytes)");
            // A router miss is axum's default 404: empty body. Handlers that
            // legitimately 404 (empty test state) always write a message.
            assert!(
                !(status == axum::http::StatusCode::NOT_FOUND && body_len == 0),
                "route {path} did not match"
            );
        }
    }

    /// axum 0.8 路由语法守卫。
    ///
    /// 0.7 的 `:param` / `*rest` 在 matchit 0.8 下**不是编译错误**，而是
    /// **注册路由时直接 panic**（`Path segments must not start with ':'`）。
    /// 也就是说：编译通过 ≠ 能跑起来，一旦有人写回旧语法，问题只会在
    /// 启动时炸。本测试把路由表里的字符串字面量逐条扫出来，
    /// 禁止旧语法回流。
    #[test]
    fn routes_use_axum08_brace_syntax() {
        // 本文件里所有 `.route("...")` / `.nest("...")` 的路径字面量。
        let src = include_str!("mod.rs");
        let mut checked = 0usize;
        let mut line_no = 0usize;
        for line in src.lines() {
            line_no += 1;
            // 只取字符串字面量里的路径（以 / 开头），避免误伤其它字符串。
            for (idx, _) in line.match_indices('"') {
                let rest = &line[idx + 1..];
                let Some(end) = rest.find('"') else { continue };
                let s = &rest[..end];
                if !s.starts_with('/') {
                    continue;
                }
                // 旧参数语法：某一段整段是 `:name`
                let has_old_param = s
                    .split('/')
                    .any(|seg| seg.starts_with(':') && seg.len() > 1);
                // 旧通配语法：某一段整段是 `*name`
                let has_old_wild = s
                    .split('/')
                    .any(|seg| seg.starts_with('*') && seg.len() > 1);
                assert!(
                    !has_old_param,
                    "路由仍用 axum 0.7 旧参数语法 :param —— {line_no}: {s}"
                );
                assert!(
                    !has_old_wild,
                    "路由仍用 axum 0.7 旧通配语法 *rest —— {line_no}: {s}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "未扫描到任何路由字面量，守卫测试失效");
        println!("已检查 {checked} 条路径字面量");
    }

    /// The cache admin surface: stats document + purge round-trip.
    #[tokio::test]
    async fn cache_admin_endpoints() {
        let state = Arc::new(
            crate::AppState::new(
                crate::config::ServerConfig {
                    server: crate::config::ServerOpts::default(),
                    admin: Default::default(),
                    layers: vec![],
                    archives: Default::default(),
                    catalog: Default::default(),
                    grids: Default::default(),
                },
                None,
            )
            .await
            .unwrap(),
        );
        let app = router(state);

        // Stats: JSON with per-cache counters and the configured TTL.
        let req = axum::http::Request::builder()
            .uri("/api/cache")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(doc["ttl_secs"].is_number(), "stats must report the TTL");
        for key in ["output", "layers", "docs", "catalog", "features", "decoded_tiles"] {
            assert!(doc[key]["entries"].is_number(), "stats missing cache '{key}'");
        }

        // Purge all: 200 + a report naming the scope.
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/cache/purge")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(doc["scope"], "all");
        assert_eq!(doc["purges"], 1);

        // A scoped purge and an unknown scope (400).
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/cache/purge?scope=docs")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/cache/purge?scope=bogus")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    /// 瓦片链路的错误按「请求问题 / 服务端故障」分类：不支持的 CRS、端点与
    /// 数据类型不匹配都是请求问题（400），地址无瓦是 404，只有解码/编码
    /// 这类真故障留 500。探活靠这个口径区分「不支持」与「坏了」。
    #[test]
    fn tile_crs_errors_are_client_errors() {
        assert_eq!(
            tile_error_status("projection: unsupported CRS 'EPSG:4326 -> EPSG:4490'"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tile_error_status("unsupported CRS 'EPSG:4543 -> EPSG:4490': needs the PROJ backend"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tile_error_status("tile decode failed: truncated LZW stream"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        // 归档里没这张瓦 / ArcGIS 缓存空槽或区外 / 瓦号越界 / 矩阵无此层级：
        // 该地址无数据，不是故障。
        assert_eq!(
            tile_error_status("tile 12/2048/2048 not in archive"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            tile_error_status("tile 10/0/0 not in cache"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            tile_error_status("tile 2/6/3 out of range"),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            tile_error_status("no zoom level 26"),
            StatusCode::NOT_FOUND
        );
        // 端点与数据类型不匹配：客户端换接口即可，重试永远不会成功。
        assert_eq!(
            tile_error_status("layer's archive is not a PNG archive"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tile_error_status(
                "arcgis tile cache holds raster tiles only (use /wmts, /tms or /xyz)"
            ),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tile_error_status("layer's archive is not a vector archive"),
            StatusCode::BAD_REQUEST
        );
        // 对 COG / 栅格层请求 /mvt：同样是选错接口，不是服务端故障。
        assert_eq!(
            tile_error_status("layer is not a vector layer"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tile_error_status("layer's archive is a vector archive (use /mvt)"),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            tile_error_status("layer has no raster source"),
            StatusCode::BAD_REQUEST
        );
    }

    /// 数据面鉴权 `data_auth`：配置 `data_token` 后，无令牌 / 错令牌返回 401，
    /// 头令牌 / Bearer / `?token=` 三种携带方式均放行；未配置则一律放行。
    #[tokio::test]
    async fn data_auth_gates_when_token_set() {
        use crate::config::{AdminOpts, CatalogOpts, ServerConfig, ServerOpts};
        let build = |token: Option<String>| ServerConfig {
            server: ServerOpts {
                data_token: token,
                ..ServerOpts::default()
            },
            admin: AdminOpts::default(),
            layers: vec![],
            archives: Default::default(),
            catalog: CatalogOpts::default(),
            grids: Default::default(),
        };
        // 设防态
        let state = Arc::new(
            AppState::new(build(Some("secret".to_string())), None)
                .await
                .unwrap(),
        );
        let app = Router::new()
            .route("/ping", get(|| async { "ok" }))
            .with_state(state.clone())
            .layer(from_fn_with_state(state.clone(), data_auth));
        let no_tok = app
            .clone()
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(no_tok.status(), StatusCode::UNAUTHORIZED, "无令牌应 401");
        let wrong = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header("x-data-token", "wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED, "错令牌应 401");
        for (hdr, val) in [
            ("x-data-token", "secret"),
            ("authorization", "Bearer secret"),
        ] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/ping")
                        .header(hdr, val)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "{hdr} 应放行");
        }
        let qs = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ping?token=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(qs.status(), StatusCode::OK, "?token= 应放行");

        // 不设防态：任何请求都放行
        let state2 = Arc::new(AppState::new(build(None), None).await.unwrap());
        let app2 = Router::new()
            .route("/ping", get(|| async { "ok" }))
            .with_state(state2.clone())
            .layer(from_fn_with_state(state2.clone(), data_auth));
        let open = app2
            .clone()
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(open.status(), StatusCode::OK, "未配置 data_token 应放行");
    }

    /// 管理面 DELETE / rename 路由存在且能匹配（admin.token 默认关闭，故直接
    /// 进入处理器；目标不存在返回 404 + 非空 JSON，而非路由未命中的空 404）。
    #[tokio::test]
    async fn admin_delete_rename_routes_match() {
        let state = Arc::new(
            crate::AppState::new(
                crate::config::ServerConfig {
                    server: crate::config::ServerOpts::default(),
                    admin: Default::default(),
                    layers: vec![],
                    archives: Default::default(),
                    catalog: Default::default(),
                    grids: Default::default(),
                },
                None,
            )
            .await
            .unwrap(),
        );
        let app = router(state);
        // DELETE 一个不存在的服务 → 404 + 非空 body（命中处理器，非路由未命中）。
        let req = axum::http::Request::builder()
            .method("DELETE")
            .uri("/admin/services/cog/nope")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            !(status == axum::http::StatusCode::NOT_FOUND && body.is_empty()),
            "DELETE 路由应匹配"
        );
        // POST rename 一个不存在的服务 → 404 + 非空 body。
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/admin/services/cog/nope/rename")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"x"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            !(status == axum::http::StatusCode::NOT_FOUND && body.is_empty()),
            "rename 路由应匹配"
        );
    }
}
