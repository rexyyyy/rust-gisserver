//! Quantized-Mesh 1.1 地形服务 handler（Cesium `CesiumTerrainProvider` 原生格式）。
//!
//! 数据源：`<catalog.root>/terrain/*.terrainpack`（GWTQ 归档，gis-wt 底座
//! G10b 编码产出，一个文件 = 一个服务）。
//!
//! 路由：
//! - `GET /terrain`                          — 服务目录（JSON）
//! - `GET /terrain/{name}/layer.json`        — quantized-mesh 清单
//! - `GET /terrain/{name}/{z}/{x}/{y}.terrain` — 单瓦（gzip 透传）
//!
//! layer.json 声明 `scheme: "tms"`（Cesium 瓦片方案 y 自南向北）；归档目录
//! 按 slippy（XYZ 顶行原点）y 存储，寻址时翻转 `slippyY = 2^z - 1 - tmsY`。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::state::AppState;
use crate::terrain::TerrainPack;

const TERRAIN_CONTENT_TYPE: &str = "application/vnd.quantized-mesh";
const TERRAIN_CACHE_CONTROL: &str = "public, max-age=3600";

/// 取服务（未知名字触发一次目录重扫，新增归档无需重启）。
fn get_pack(state: &AppState, name: &str) -> Option<Arc<TerrainPack>> {
    if let Some(p) = state
        .terrains
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
    {
        return Some(p);
    }
    let dir = state.config.terrain_dir();
    let (reg, errs) = crate::terrain::TerrainRegistry::load_dir(&dir);
    for e in &errs {
        tracing::warn!("{e}");
    }
    let found = reg.get(name);
    *state.terrains.write().unwrap_or_else(|e| e.into_inner()) = reg;
    found
}

fn not_found(msg: String) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        msg,
    )
        .into_response()
}

/// `GET /terrain` — 已配置的地形服务目录。
pub async fn list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = crate::handlers::base_url(&state, &headers);
    // 顺带重扫一次目录，保证目录页反映磁盘现状。
    let dir = state.config.terrain_dir();
    let (reg, errs) = crate::terrain::TerrainRegistry::load_dir(&dir);
    for e in &errs {
        tracing::warn!("{e}");
    }
    *state.terrains.write().unwrap_or_else(|e| e.into_inner()) = reg;

    let reg = state.terrains.read().unwrap_or_else(|e| e.into_inner());
    let services: Vec<serde_json::Value> = reg
        .names()
        .iter()
        .filter_map(|name| {
            let p = reg.get(name)?;
            Some(json!({
                "name": name,
                "format": "quantized-mesh-1.1",
                "projection": if p.geodetic() { "EPSG:4326" } else { "EPSG:3857" },
                "minzoom": p.min_zoom(),
                "maxzoom": p.max_zoom(),
                "tiles": p.dir.len(),
                "bounds": p.bounds(),
                "layer_json": format!("{base}/terrain/{name}/layer.json"),
            }))
        })
        .collect();
    let body = json!({
        "count": services.len(),
        "root": dir.display().to_string(),
        "services": services,
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into()),
    )
        .into_response()
}

/// `GET /terrain/{name}/layer.json` — quantized-mesh 清单。
///
/// `available` 按层级给出每级已编码瓦片的包围矩形（tms y）；低于 minzoom
/// 的层补空数组（Cesium 以 available[ level ] 索引取用）。
pub async fn layer_json(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(pack) = get_pack(&state, &name) else {
        return not_found(format!(
            "未找到地形服务 '{name}'（见 GET /terrain）"
        ));
    };
    let base = crate::handlers::base_url(&state, &headers);

    // 每级已编码瓦片的包围矩形（slippy y -> tms y 翻转）。
    let mut per_level: BTreeMap<u32, (u32, u32, u32, u32)> = BTreeMap::new();
    for &(z, x, y) in pack.dir.keys() {
        let rows = 1u32 << z;
        let tms_y = rows - 1 - y;
        let e = per_level
            .entry(z)
            .or_insert((x, x, tms_y, tms_y));
        e.0 = e.0.min(x);
        e.1 = e.1.max(x);
        e.2 = e.2.min(tms_y);
        e.3 = e.3.max(tms_y);
    }
    let minz = pack.min_zoom();
    let maxz = pack.max_zoom();
    let mut available: Vec<serde_json::Value> = Vec::with_capacity((maxz + 1) as usize);
    for z in 0..=maxz {
        match per_level.get(&z) {
            Some(&(x0, x1, y0, y1)) => available.push(json!([{
                "start": { "x": x0, "y": y0, "level": z },
                "end":   { "x": x1, "y": y1, "level": z },
            }])),
            None => available.push(json!([])),
        }
    }
    let _ = minz;

    let mut doc = json!({
        "tilejson": "2.1.0",
        "name": name,
        "format": "quantized-mesh-1.1",
        "version": "1.0.0",
        "projection": if pack.geodetic() { "EPSG:4326" } else { "EPSG:3857" },
        "scheme": "tms",
        "available": available,
        "bounds": pack.bounds(),
        "tiles": [format!("{base}/terrain/{name}/{{z}}/{{x}}/{{y}}.terrain")],
    });
    if pack.with_normals() {
        doc["extensions"] = json!("octvertexnormals");
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, TERRAIN_CACHE_CONTROL),
        ],
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into()),
    )
        .into_response()
}

/// `GET /terrain/{name}/{z}/{x}/{y}.terrain` — 单瓦（gzip 透传）。
pub async fn tile(
    State(state): State<Arc<AppState>>,
    Path((name, z, x, yseg)): Path<(String, u32, u32, String)>,
    req_headers: HeaderMap,
) -> Response {
    let Some(y_tms) = crate::handlers::parse_coord(&yseg) else {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("非法 y 坐标段 '{yseg}'"),
        )
            .into_response();
    };
    if z > 24 {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "z 超出范围（<=24）".to_string(),
        )
            .into_response();
    }
    let Some(pack) = get_pack(&state, &name) else {
        return not_found(format!("未找到地形服务 '{name}'（见 GET /terrain）"));
    };
    // tms -> slippy（归档按 XYZ 顶行原点存储）。
    let y_slippy = (1u32 << z) - 1 - y_tms;
    let read = tokio::task::spawn_blocking(move || pack.read_tile(z, x, y_slippy)).await;
    match read {
        Ok(Ok(Some(bytes))) => {
            // gzip 归档 blob 原样透传（gzip magic 判定），浏览器/Cesium 自动解压。
            let is_gzip = bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b;
            let out = crate::tiles::TileOut {
                bytes,
                content_type: TERRAIN_CONTENT_TYPE.to_string(),
                content_encoding: if is_gzip { Some("gzip".into()) } else { None },
            };
            crate::handlers::tile_response(out, &req_headers, TERRAIN_CACHE_CONTROL)
        }
        Ok(Ok(None)) => not_found(format!("瓦片不存在：{name}/{z}/{x}/{y_tms}")),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("读取地形瓦片失败：{e}"),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("瓦片读取任务失败：{e}"),
        )
            .into_response(),
    }
}
