//! Admin HTTP surface consumed by the manager system:
//!
//! * `GET  /admin/services`                  — catalog scan + registry states
//! * `PUT  /admin/services/:kind/:name/state`— stop / pause / restore / delete
//! * `POST /admin/reload`                    — hot reload (rescan + rebuild)
//! * `GET  /api/usage`                       — access counters (day/week/month/year)
//!
//! When `admin.token` is configured, all four require the `X-Admin-Token`
//! header (the manager backend is the intended — and only — caller on the
//! intranet; the token is a seatbelt, not an auth system). The master token
//! or any enabled `admin` key from the shared `sys_api_key` table is accepted
//! (rust-gisserver generates them; see `crate::apikeys`).

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::admin::usage;
use crate::state::AppState;

pub(crate) fn json(status: StatusCode, v: serde_json::Value) -> Response {
    (status, Json(v)).into_response()
}

/// Token seatbelt for the admin surface.
///
/// 配置 `admin.token` 后启用校验：主令牌 **或** rust-gisserver `sys_api_key` 中
/// 启用且未过期的 `admin` 用途 key 均被认可（后者由 `apikeys` 定时同步，
/// 最多 15 秒生效）。未配置主令牌时保持零配置不设防，向后兼容。
pub(crate) fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    if let Some(tok) = &state.config.admin.token {
        let got = headers
            .get("x-admin-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let ok = got == tok || state.api_keys.is_valid("admin", got);
        if !ok {
            return Err(json(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"error": "missing/invalid X-Admin-Token"}),
            ));
        }
    }
    Ok(())
}

/// Protocol endpoint URLs for one service (test-browse / one-click copy in
/// the manager UI). `base` comes from the request Host (or config override);
/// `layers` 是调用方按磁盘重建的当前图层清单（`state.config.layers` 只是启动快照）。
pub fn endpoints_for(
    state: &AppState,
    base: &str,
    kind: &str,
    name: &str,
    layers: &[gisserver::wms_common::layer::Layer],
) -> Vec<serde_json::Value> {
    // 把引擎基址作为 `?base=` 查询值；保留 `/` 与 `:` 便于阅读，
    // 其余会破坏 query 结构的字符按 percent-encoding 转义。
    fn query_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
                | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => out.push(b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    let b = base.trim_end_matches('/');
    let enc_base = query_escape(b);
    let mut v: Vec<(String, String)> = Vec::new();
    let layer_defs = kind != "3dtiles"
        && kind != "pmtiles"
        && kind != "mbtiles"
        && kind != "terrain"
        && kind != "arcgis";
    if layer_defs {
        let def = layers.iter().find(|l| l.name == name);
        // Sample GetMap bbox: the configured extent when known.
        let bbox = def
            .and_then(|l| l.extent)
            .map(|[a, b2, c, d]| format!("{a:.4},{b2:.4},{c:.4},{d:.4}"))
            .unwrap_or_else(|| "100,36,108,40".to_string());
        v.push((
            "WMS GetCapabilities".into(),
            format!("{b}/wms?service=WMS&version=1.3.0&request=GetCapabilities"),
        ));
        v.push((
            "WMS GetMap 预览".into(),
            format!(
                "{b}/wms?service=WMS&version=1.1.1&request=GetMap&layers={name}&styles=&srs=EPSG:4326&bbox={bbox}&width=768&height=512&format=image/png"
            ),
        ));
        let src = def.map(|l| &l.source);
        // WMTS / XYZ 同走 raster_tile：PostGIS 层无栅格源（实测报错），不宣传。
        if !matches!(src, Some(gisserver::wms_common::layer::SourceKind::Postgis(_))) {
            v.push((
                "WMTS Capabilities".into(),
                format!("{b}/wmts/1.0.0/WMTSCapabilities.xml"),
            ));
            v.push((
                "XYZ 瓦片模板".into(),
                format!("{b}/tiles/{name}/{{z}}/{{x}}/{{y}}.png"),
            ));
        }
        v.push((
            "图例 GetLegendGraphic".into(),
            format!("{b}/wms?service=WMS&version=1.1.1&request=GetLegendGraphic&format=image/png&layer={name}"),
        ));
        let is_vector = matches!(
            src,
            Some(gisserver::wms_common::layer::SourceKind::Vector { .. })
                | Some(gisserver::wms_common::layer::SourceKind::Postgis(_))
        );
        // 首个网格决定 MVT 的寻址口径：非 Web Mercator 时模板要带网格段。
        let grid_id = def
            .and_then(|l| l.grids.first().cloned())
            .unwrap_or_else(|| "WebMercatorQuad".to_string());
        let native_grid = gisserver::wms_common::tile::strip_size_suffix(&grid_id) != "WebMercatorQuad";
        if is_vector {
            v.push((
                "MVT 矢量瓦片模板".into(),
                if native_grid {
                    format!("{b}/mvt/{name}/{grid_id}/{{z}}/{{x}}/{{y}}.pbf")
                } else {
                    format!("{b}/mvt/{name}/{{z}}/{{x}}/{{y}}.pbf")
                },
            ));
            v.push(("TileJSON".into(), format!("{b}/mvt/{name}/tilejson.json")));
            v.push((
                "WFS GetCapabilities".into(),
                format!("{b}/wfs?service=WFS&version=2.0.0&request=GetCapabilities"),
            ));
            v.push((
                "OGC API - Features".into(),
                format!("{b}/ogc/features/v1/collections/{name}/items"),
            ));
        }
        // 通用 /viewer 只按 WebMercator XYZ 寻址；默认网格是地理/投影自定义网格
        // （WorldCRS84Quad、CGCS2000Quad、Conf.xml 派生网格）时必须用 viewer-arcgis，
        // 它读 `/ogc/tiles/v1/tileMatrixSets/{grid}` 的原生 LOD 与原点。
        // PostGIS 层没有栅格源，预览改用矢量瓦片页 viewer-mvt（同样的网格读取逻辑）。
        v.push((
            "内置预览（OpenLayers）".into(),
            if matches!(src, Some(gisserver::wms_common::layer::SourceKind::Postgis(_))) {
                format!("/preview/viewer_mvt.html?layer={name}&kind=postgis&grid={grid_id}&base={enc_base}")
            } else if native_grid {
                format!("/preview/viewer_arcgis.html?layer={name}&grid={grid_id}&base={enc_base}")
            } else {
                format!("/preview/viewer.html?layer={name}&base={enc_base}")
            },
        ));
    } else if kind == "pmtiles" || kind == "mbtiles" {
        // 归档类能力按 tiles.rs 实测：栅格归档走 raster_tile（WMTS/XYZ 同源），
        // 矢量归档只有 mvt_tile 认它；两者都不参与 WMS GetMap（无栅格源）。
        // 扩展名按归档实际格式取：MBTiles 读 metadata.format，PMTiles 读 header.tile_type。
        let ext = state
            .layer(name)
            .and_then(|l| match (&l.pmtiles, &l.mbtiles) {
                (Some(pm), _) => {
                    use gisserver::pmtiles::tile_type as pt;
                    Some(match pm.tile_type() {
                        pt::MVT => "pbf",
                        pt::PNG => "png",
                        pt::JPEG => "jpg",
                        pt::WEBP => "webp",
                        pt::AVIF => "avif",
                        _ => "png",
                    })
                }
                (_, Some(mb)) => {
                    let fmt = mb.format();
                    Some(match fmt.as_str() {
                        "pbf" | "protobuf" | "mvt" => "pbf",
                        "png" => "png",
                        "jpg" | "jpeg" => "jpg",
                        "webp" => "webp",
                        "gif" => "gif",
                        "avif" => "avif",
                        "" => "png",
                        _ => "png",
                    })
                }
                _ => None,
            })
            .unwrap_or("png");
        let is_vector = ext == "pbf";
        let tpl = if kind == "pmtiles" {
            format!("{b}/tiles/{name}/{{z}}/{{x}}/{{y}}")
        } else {
            format!("{b}/mbtiles/{name}/{{z}}/{{x}}/{{y}}")
        };
        v.push((
            format!("{} 归档信息", if kind == "pmtiles" { "PMTiles" } else { "MBTiles" }),
            format!("{b}/{kind}/{name}"),
        ));
        if is_vector {
            v.push((
                "MVT 矢量瓦片模板".into(),
                format!("{b}/mvt/{name}/{{z}}/{{x}}/{{y}}.pbf"),
            ));
            v.push(("TileJSON".into(), format!("{b}/mvt/{name}/tilejson.json")));
        } else {
            v.push((
                "WMTS Capabilities".into(),
                format!("{b}/wmts/1.0.0/WMTSCapabilities.xml"),
            ));
            v.push(("XYZ 瓦片模板".into(), format!("{tpl}.{ext}")));
        }
        v.push((
            "内置预览（OpenLayers）".into(),
            format!("/preview/viewer.html?tiles={}&base={enc_base}", query_escape(&tpl)),
        ));
    } else if kind == "arcgis" {
        // ArcGIS 预切片缓存走 raster_tile 的 read-through 分支：只宣 WMTS / TMS /
        // XYZ 三个瓦片前端（地址就是缓存自己的 (z, col, row)）；无栅格波段
        // 参与 WMS GetMap，也不出 MVT。网格用注入的 `arcgis_<name>`。
        v.push((
            "WMTS Capabilities".into(),
            format!("{b}/wmts/1.0.0/WMTSCapabilities.xml"),
        ));
        v.push((
            "XYZ 瓦片模板".into(),
            format!("{b}/tiles/{name}/{{z}}/{{x}}/{{y}}.png"),
        ));
        v.push((
            "TMS 瓦片模板".into(),
            format!("{b}/tms/1.0.0/{name}/{{z}}/{{x}}/{{y}}"),
        ));
        v.push((
            "内置预览（OpenLayers）".into(),
            // ArcGIS 缓存是源坐标系下的自定义网格，必须用专用的 viewer-arcgis
            // （读 tileMatrixSets/{grid} 的原生 LOD 与原点）；通用 /viewer 按
            // WebMercator XYZ 寻址，打开只会是空白底图。
            format!("/preview/viewer_arcgis.html?layer={name}&grid=arcgis_{name}&base={enc_base}"),
        ));
    } else if kind == "3dtiles" {
        v.push(("tileset.json".into(), format!("{b}/3dtiles/{name}/tileset.json")));
        v.push((
            "内置预览（Cesium）".into(),
            format!("/preview/cesium.html?name={name}&base={enc_base}"),
        ));
    } else if kind == "terrain" {
        // Quantized-Mesh 1.1 地形服务（.terrainpack 归档）
        v.push(("layer.json".into(), format!("{b}/terrain/{name}/layer.json")));
        v.push((
            "瓦片模板（.terrain）".into(),
            format!("{b}/terrain/{name}/{{z}}/{{x}}/{{y}}.terrain"),
        ));
        v.push((
            "内置预览（Cesium 三维地形）".into(),
            format!("/preview/cesium.html?terrain={name}&base={enc_base}"),
        ));
    }
    // 独立引擎模式没有 manager 托管预览页：能力列表只给数据端点，不给死链。
    if !state.manager.is_managed() {
        v.retain(|(name, _)| !name.starts_with("内置预览"));
    }
    v.into_iter()
        .map(|(name, url)| serde_json::json!({"name": name, "url": url}))
        .collect()
}

/// `GET /admin/services` — everything the catalog can see, merged with the
/// registry's management state, the live serving flag and protocol endpoints.
pub async fn services(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let states = state.admin.registry.fetch_states().await.unwrap_or_default();
    let scan = state.catalog_scan();
    let cur_layers = state.current_layers();
    let serving_layers = state.layer_names();
    let serving_tilesets = state.tileset_names();
    let base = crate::handlers::base_url(&state, &headers);

    let mut rows: Vec<serde_json::Value> = Vec::new();
    let tileset_reg = state.tilesets.read().unwrap_or_else(|e| e.into_inner());
    for s in &scan {
        let st = states.get(&s.name).cloned().unwrap_or_else(|| "active".into());
        let serving = match s.kind.as_str() {
            "3dtiles" => serving_tilesets.iter().any(|n| n == &s.name),
            _ => serving_layers.iter().any(|n| n == &s.name),
        };
        // 在用但未在服时的原因说明：3D Tiles 取登记时的校验结论；文件类
        // （cog/vector/pmtiles/mbtiles/arcgis）检查数据路径存在性；PostGIS 需要连
        // 库才能定位（不在列表接口做，指去体检）。
        let note = if !serving && st == "active" {
            match s.kind.as_str() {
                "3dtiles" => tileset_reg
                    .get(&s.name)
                    .and_then(|t| t.unavailable_reason.clone())
                    .or_else(|| Some("加载失败（详见体检）".to_string())),
                "cog" | "vector" | "pmtiles" | "mbtiles" | "arcgis" => {
                    let path = cur_layers.iter().find(|l| l.name == s.name).map(|l| match &l.source {
                        gisserver::wms_common::layer::SourceKind::Cog { path }
                        | gisserver::wms_common::layer::SourceKind::Vector { path }
                        | gisserver::wms_common::layer::SourceKind::Pmtiles { path }
                        | gisserver::wms_common::layer::SourceKind::Mbtiles { path }
                        | gisserver::wms_common::layer::SourceKind::Arcgis { path } => path.clone(),
                        _ => String::new(),
                    });
                    // ArcGIS 缓存的 path 是目录不是文件，措辞分开才不会把运维
                    // 引到「去找一个数据文件」的错误方向。
                    let what = if s.kind == "arcgis" { "缓存目录" } else { "数据文件" };
                    match path {
                        Some(p) if !p.is_empty() && !std::path::Path::new(&p).exists() => {
                            Some(format!("{what}不存在：{p}"))
                        }
                        _ => Some(format!("{what}打开失败（详见体检）")),
                    }
                }
                _ => Some("PG 连接失败或加载失败（详见体检）".to_string()),
            }
        } else {
            None
        };
        let endpoints = endpoints_for(&state, &base, &s.kind, &s.name, &cur_layers);
        // 瓦片网格类型：取该图层发布用的默认网格，标出它是 OGC 预设（标准）
        // 还是自定义。与坐标系是否标准 EPSG 正交（如 4490 是标准 CRS、但
        // arcgis_<图层> 是自定义网格）。manager 透传本对象，前端据此渲染标签。
        let grid = cur_layers
            .iter()
            .find(|l| l.name == s.name)
            .map(|l| {
                let gid = state.config.default_grid_id(l);
                serde_json::json!({
                    "id": gid,
                    "crs": l.src_crs,
                    "standard": gisserver::wms_common::tile::is_preset_id(&gid),
                })
            });
        rows.push(serde_json::json!({
            "name": s.name,
            "kind": s.kind,
            "title": s.title,
            "source": s.source,
            "state": st,
            "serving": serving,
            "note": note,
            "grid": grid,
            "endpoints": endpoints,
        }));
    }
    drop(tileset_reg);
    // Quantized-Mesh 地形服务（kind=terrain）：不走向导登记（与 3dtiles 同理），
    // 直接从 TerrainRegistry 出清单行，管理端服务列表即可展示/预览/复制 layer.json。
    {
        let terrain_reg = state.terrains.read().unwrap_or_else(|e| e.into_inner());
        for name in terrain_reg.names() {
            let st = states.get(&name).cloned().unwrap_or_else(|| "active".into());
            let pack = terrain_reg.get(&name);
            let serving = pack.is_some();
            let note = if !serving && st == "active" {
                Some("terrainpack 归档读取失败（详见服务日志）".to_string())
            } else {
                None
            };
            let (source, meta) = match &pack {
                Some(t) => (
                    t.path.display().to_string(),
                    serde_json::json!({
                        "bounds": t.bounds(),
                        "minzoom": t.min_zoom(),
                        "maxzoom": t.max_zoom(),
                        "tiles": t.dir.len(),
                    }),
                ),
                None => (String::new(), serde_json::json!({})),
            };
            let endpoints = endpoints_for(&state, &base, "terrain", &name, &cur_layers);
            rows.push(serde_json::json!({
                "name": name,
                "kind": "terrain",
                "title": name,
                "source": source,
                "state": st,
                "serving": serving,
                "note": note,
                "endpoints": endpoints,
                "meta": meta,
            }));
        }
    }
    json(
        StatusCode::OK,
        serde_json::json!({
            "registry_mode": state.admin.registry.mode(),
            "count": rows.len(),
            "services": rows,
        }),
    )
}

#[derive(Deserialize)]
pub struct StateBody {
    pub state: String,
}

/// `PUT /admin/services/:kind/:name/state` — body `{"state": "..."}`.
/// `active` restores (including from the deleted trash bin). Applies the new
/// state immediately (rebuild + purge) so no restart is needed.
pub async fn set_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((kind, name)): Path<(String, String)>,
    body: Option<Json<StateBody>>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let Some(Json(body)) = body else {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "body required: {\"state\": \"active|stopped|paused|deleted\"}"}),
        );
    };
    let new_state = body.state;
    if !crate::admin::registry::STATES.contains(&new_state.as_str()) {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": format!("invalid state '{new_state}' (use active|stopped|paused|deleted)")}),
        );
    }
    // Locate the service to persist accurate kind/title/source.
    let scan = state.catalog_scan();
    let Some(svc) = scan.iter().find(|s| s.name == name && s.kind == kind).cloned() else {
        return json(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("service '{kind}/{name}' not found in catalog")}),
        );
    };
    if let Err(e) = state
        .admin
        .registry
        .set_state(&name, &svc.kind, &svc.title, &svc.source, &new_state)
        .await
    {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("persist state: {e}")}),
        );
    }
    // Apply now: rebuild served sets from the current catalog + purge.
    match state.refresh_catalog(false).await {
        Ok(report) => json(
            StatusCode::OK,
            serde_json::json!({
                "ok": true,
                "service": {"name": name, "kind": kind},
                "state": new_state,
                "serving": report.served.iter().any(|n| n == &name)
                    || report.tilesets.iter().any(|n| n == &name),
            }),
        ),
        Err(e) => json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("apply: {e}")}),
        ),
    }
}

/// `POST /admin/reload` — re-read `mapserver.xml` + catalog from disk,
/// sync the registry, rebuild the serving sets and purge caches.
pub async fn reload(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    match state.refresh_catalog(true).await {
        Ok(report) => json(StatusCode::OK, serde_json::to_value(&report).unwrap_or_default()),
        Err(e) => json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("reload: {e}")}),
        ),
    }
}

/// `GET /api/usage?period=day|week|month|year&group_by=name|type&kind=...`
pub async fn usage_handler(State(state): State<Arc<AppState>>, headers: HeaderMap, RawQuery(qs): RawQuery) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let kv = qs
        .as_deref()
        .map(gisserver::wms_protocol::kvp::parse_kvp)
        .unwrap_or_default();
    let get = |k: &str| -> Option<String> {
        kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    };
    let hours = match get("PERIOD").as_deref() {
        None | Some("day") => 24,
        Some("week") => 24 * 7,
        Some("month") => 24 * 30,
        Some("year") => 24 * 365,
        Some(other) => {
            return json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": format!("unknown period '{other}' (use day|week|month|year)")}),
            );
        }
    };
    let by_kind = get("GROUP_BY").as_deref() == Some("type");
    let kind_filter = get("KIND");
    let (rows, source) = usage::query(&state, hours, by_kind).await;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|(k, _, _)| kind_filter.as_deref().map(|f| k == f).unwrap_or(true))
        .take(200)
        .map(|(kind, name, hits)| {
            serde_json::json!({"kind": kind, "name": name, "hits": hits})
        })
        .collect();
    json(
        StatusCode::OK,
        serde_json::json!({
            "period": get("PERIOD").unwrap_or_else(|| "day".into()),
            "hours": hours,
            "group_by": if by_kind { "type" } else { "name" },
            "source": source,
            "rows": rows,
        }),
    )
}

/// `GET /admin/access-logs?limit=200&status=4xx|2xx|3xx|5xx&kind=&name=&q=`
/// — newest-first audit query over the in-memory ring ("since restart").
pub async fn access_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(qs): RawQuery,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let kv = qs
        .as_deref()
        .map(gisserver::wms_protocol::kvp::parse_kvp)
        .unwrap_or_default();
    let get = |k: &str| -> Option<String> {
        kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    };
    let limit = get("LIMIT")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, 2000);
    let class = get("STATUS").and_then(|s| {
        let t = s.trim().trim_end_matches("xx");
        t.parse::<u8>().ok().filter(|c| (2..=5).contains(c))
    });
    let kind = get("KIND").filter(|s| !s.is_empty());
    let name = get("NAME").filter(|s| !s.is_empty());
    let q = get("Q").filter(|s| !s.is_empty());
    let rows: Vec<serde_json::Value> = state
        .admin
        .access_log
        .query(limit, class, kind.as_deref(), name.as_deref(), q.as_deref())
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();
    let (total, counts) = state.admin.access_log.stats();
    let mut status_counts = serde_json::Map::new();
    for (code, n) in counts {
        status_counts.insert(code.to_string(), serde_json::json!(n));
    }
    // Error rollup (the 4xx/5xx headline numbers for the UI).
    let err4: u64 = status_counts
        .iter()
        .filter(|(k, _)| k.starts_with('4'))
        .filter_map(|(_, v)| v.as_u64())
        .sum();
    let err5: u64 = status_counts
        .iter()
        .filter(|(k, _)| k.starts_with('5'))
        .filter_map(|(_, v)| v.as_u64())
        .sum();
    json(
        StatusCode::OK,
        serde_json::json!({
            "total": total,
            "status_counts": status_counts,
            "errors": {"client_4xx": err4, "server_5xx": err5},
            "rows": rows,
        }),
    )
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use crate::config::{ServerConfig, ServerOpts};
    use crate::manager::{ManagerLink, RunMode};
    use gisserver::wms_common::layer::{Layer, SourceKind};

    fn test_config() -> ServerConfig {
        ServerConfig {
            server: ServerOpts::default(),
            admin: Default::default(),
            manager: Default::default(),
            layers: vec![],
            archives: Default::default(),
            catalog: Default::default(),
            grids: Default::default(),
        }
    }

    /// 构造一个带真实 mbtiles layer（PNG 格式）的 ServerConfig
    fn config_with_mbtiles(fmt: &str) -> ServerConfig {
        let name = match fmt {
            "png" => "test_mbtiles_png",
            "pbf" => "test_mbtiles_pbf",
            _ => panic!("unsupported fixture fmt: {fmt}"),
        };
        let rel = match fmt {
            "png" => "fixtures/test.mbtiles",
            "pbf" => "fixtures/test-vector.mbtiles",
            _ => unreachable!(),
        };
        let path = format!("{}/../../{}", env!("CARGO_MANIFEST_DIR"), rel);
        ServerConfig {
            layers: vec![Layer {
                name: name.to_string(),
                title: name.to_string(),
                source: SourceKind::Mbtiles { path },
                src_crs: "EPSG:3857".to_string(),
                style: None,
                sld: None,
                grids: vec!["WebMercatorQuad".to_string()],
                tile_px: 256,
                extent: None,
            }],
            ..test_config()
        }
    }

    // config_with_pmtiles 留空：fixtures 目录没有 pmtiles fixture，
    // 后缀映射核心分支已被 mbtiles 三档覆盖；后续有 pmtiles fixture 时再补。

    // ────────────────────────────────────────────────────────────────────────
    // 已有：run_mode 切换预览入口
    // ────────────────────────────────────────────────────────────────────────

    /// 独立引擎没有 manager 托管预览页：能力列表只给数据端点，不给死链。
    /// 受管模式则给出 `/preview/*.html?...&base=<引擎地址>` 入口。
    #[tokio::test]
    async fn preview_endpoint_follows_run_mode() {
        let mut state = crate::AppState::new(test_config(), None).await.unwrap();

        state.manager = ManagerLink::standalone();
        let eps = endpoints_for(&state, "http://engine:8080/", "3dtiles", "demo", &[]);
        assert!(eps.iter().any(|v| v["name"].as_str() == Some("tileset.json")));
        assert!(!eps
            .iter()
            .any(|v| v["name"].as_str().unwrap_or("").starts_with("内置预览")));

        state.manager = ManagerLink {
            mode: RunMode::Managed,
            url: Some("http://127.0.0.1:6180".into()),
        };
        let eps = endpoints_for(&state, "http://engine:8080/", "3dtiles", "demo", &[]);
        let url = eps
            .iter()
            .find(|v| v["name"].as_str().unwrap_or("").starts_with("内置预览"))
            .expect("受管模式应给出预览入口")["url"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(url.starts_with("/preview/cesium.html?"), "{url}");
        assert!(url.contains("name=demo"), "{url}");
        assert!(url.contains("base=http://engine:8080"), "{url}");
    }

    // ────────────────────────────────────────────────────────────────────────
    // 新增：动态后缀映射（本次修复回归）
    // ────────────────────────────────────────────────────────────────────────

    /// MBTiles PNG：XYZ 模板后缀必须是 .png（不硬编码）
    #[tokio::test]
    async fn mbtiles_png_preserves_png_ext_in_xyz_template() {
        let state = crate::AppState::new(config_with_mbtiles("png"), None).await.unwrap();
        let eps = endpoints_for(&state, "http://engine:8080/", "mbtiles", "test_mbtiles_png", &[]);
        let xyz = eps
            .iter()
            .find(|v| v["name"].as_str() == Some("XYZ 瓦片模板"))
            .expect("raster mbtiles 必须给出 XYZ 模板");
        let url = xyz["url"].as_str().unwrap();
        assert!(url.ends_with("/{z}/{x}/{y}.png"), "png 后缀被改坏: {url}");
        assert!(!url.ends_with(".jpg"), "png fixture 不应误报 jpg: {url}");
        assert!(!url.ends_with(".pbf"), "raster fixture 不应误报 pbf: {url}");
        // 有 WMTS Capabilities
        assert!(eps.iter().any(|v| v["name"].as_str() == Some("WMTS Capabilities")));
    }

    /// MBTiles PBF（矢量）：不宣 XYZ（没意义），只宣 MVT 模板 + TileJSON
    #[tokio::test]
    async fn mbtiles_pbf_publishes_mvt_not_xyz() {
        let state = crate::AppState::new(config_with_mbtiles("pbf"), None).await.unwrap();
        let eps = endpoints_for(&state, "http://engine:8080/", "mbtiles", "test_mbtiles_pbf", &[]);

        // 不应有 XYZ 瓦片模板
        assert!(
            !eps.iter().any(|v| v["name"].as_str() == Some("XYZ 瓦片模板")),
            "vector mbtiles 不应宣 XYZ"
        );
        // 应有 MVT 模板 .pbf
        let mvt = eps
            .iter()
            .find(|v| v["name"].as_str() == Some("MVT 矢量瓦片模板"))
            .expect("vector mbtiles 必须宣 MVT");
        assert!(
            mvt["url"].as_str().unwrap().ends_with(".pbf"),
            "MVT 模板必须 .pbf: {}",
            mvt["url"].as_str().unwrap()
        );
        // 应有 TileJSON
        assert!(eps.iter().any(|v| v["name"].as_str() == Some("TileJSON")));
    }

    /// 旧 fixture test.mbtiles 的 format=png，test-vector.mbtiles 的 format=pbf——
    /// 两条服务端实际跑起来也确实按各自 metadata 格式返回正确 Content-Type。
    /// 这里通过 endpoints_for 后缀回归验证整个动态映射链路。
    #[tokio::test]
    async fn endpoints_for_reflects_real_archive_format() {
        let cfg = ServerConfig {
            layers: vec![
                Layer {
                    name: "rt_png".into(),
                    title: "".into(),
                    source: SourceKind::Mbtiles {
                        path: format!("{}/../../fixtures/test.mbtiles", env!("CARGO_MANIFEST_DIR")),
                    },
                    src_crs: "EPSG:3857".into(),
                    style: None,
                    sld: None,
                    grids: vec!["WebMercatorQuad".into()],
                    tile_px: 256,
                    extent: None,
                },
                Layer {
                    name: "rt_vec".into(),
                    title: "".into(),
                    source: SourceKind::Mbtiles {
                        path: format!(
                            "{}/../../fixtures/test-vector.mbtiles",
                            env!("CARGO_MANIFEST_DIR")
                        ),
                    },
                    src_crs: "EPSG:3857".into(),
                    style: None,
                    sld: None,
                    grids: vec!["WebMercatorQuad".into()],
                    tile_px: 256,
                    extent: None,
                },
            ],
            ..test_config()
        };
        let state = crate::AppState::new(cfg, None).await.unwrap();

        // PNG 层 → XYZ .png
        let eps_png = endpoints_for(&state, "http://e/", "mbtiles", "rt_png", &[]);
        let xyz_png = eps_png.iter().find(|v| v["name"].as_str() == Some("XYZ 瓦片模板"));
        assert!(xyz_png.is_some(), "png fixture 必须宣 XYZ");
        assert!(
            xyz_png.unwrap()["url"].as_str().unwrap().ends_with(".png"),
            "png fixture 后缀应为 .png"
        );

        // PBF 层 → MVT .pbf，不宣 XYZ
        let eps_vec = endpoints_for(&state, "http://e/", "mbtiles", "rt_vec", &[]);
        let xyz_vec = eps_vec.iter().find(|v| v["name"].as_str() == Some("XYZ 瓦片模板"));
        assert!(xyz_vec.is_none(), "vector fixture 不应宣 XYZ");
        let mvt_vec = eps_vec.iter().find(|v| v["name"].as_str() == Some("MVT 矢量瓦片模板"));
        assert!(mvt_vec.is_some(), "vector fixture 必须宣 MVT");
        assert!(
            mvt_vec.unwrap()["url"].as_str().unwrap().ends_with(".pbf"),
            "MVT 后缀应为 .pbf"
        );
    }
}
