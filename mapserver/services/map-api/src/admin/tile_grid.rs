//! 解析 ArcGIS `Conf.xml`（或同构的 `tile_schema.xml`）为发布向导可用的瓦片网格定义。
//!
//! `POST /admin/tile-grid` body `{path}`：`path` 可以是 `Conf.xml` 文件本身、
//! 缓存根目录，或缓存根的上一层目录（与 [`gisserver::arcgis_cache::conf::CacheDocs::load`]
//! 的定位规则一致）。发布向导也允许选择同构的瓦片方案文件（如 `tile_schema.xml`，
//! 只有 `TileCacheInfo`、没有缓存包/cdi）：此时直接按该文件解析，缺少 `conf.cdi`
//! 时用 origin 推导兜底包络。只抽取本系统发布/预览所需字段：CRS、左上原点、
//! 瓦片尺寸、LOD 阶梯与对齐后的包络，忽略 ESRI 专有标记。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use crate::admin::discover::guard_data_root_laxed;
use crate::admin::http::{authorize, json as json_resp};
use crate::state::AppState;

/// `POST /admin/tile-grid`
pub async fn tile_grid(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }

    let raw = body
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(raw) = raw else {
        return bad("缺少 path 字段（Conf.xml 或其所在目录）");
    };

    let raw_path = PathBuf::from(raw);
    if !raw_path.exists() {
        return bad(&format!("路径不存在：{raw}"));
    }
    // 允许直接选 Conf.xml：取其所在目录交给 CacheDocs 定位。
    let dir: PathBuf = if raw_path.is_file() {
        raw_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| raw_path.clone())
    } else {
        raw_path.clone()
    };

    let dir = match guard_data_root_laxed(&state, &dir) {
        Ok(p) => p,
        Err(r) => return r,
    };

    // 常规路径：按 ArcGIS 缓存目录定位 Conf.xml + conf.cdi。
    // 兜底路径：用户直接选了瓦片方案文件（如 tile_schema.xml，只有 TileCacheInfo、
    // 没有缓存包/cdi）——此时目录定位不到 Conf.xml，直接按该文件解析。
    let (cache_dir, info, bounds) = match gisserver::arcgis_cache::conf::CacheDocs::load(&dir) {
        Ok(d) => {
            let b = d.cdi.as_ref().map(|c| c.bounds);
            (d.cache_dir, d.info, b)
        }
        Err(e) => {
            if raw_path.is_file() && raw_path.extension().map(|x| x.eq_ignore_ascii_case("xml")).unwrap_or(false) {
                match read_xml_file(&raw_path).and_then(|t| gisserver::arcgis_cache::conf::CacheInfo::from_xml(&t)) {
                    Ok(info) => (dir.clone(), info, None),
                    Err(e2) => {
                        return bad(&format!(
                            "读取 ArcGIS 瓦片配置失败：{}；按 {} 解析瓦片方案也失败：{e2}",
                            e,
                            raw_path.display()
                        ))
                    }
                }
            } else {
                return bad(&format!("读取 ArcGIS 瓦片配置失败：{e}"));
            }
        }
    };

    // 优先用 cdi 包络；缺失时按 origin 与 CRS 推导兜底包络（纯瓦片方案无缓存包，
    // 无 bundle 可探测）。经纬度 CRS 常用 origin [-180,90]，此时直接用全球经纬度
    // 范围；其余按最细级瓦片跨度对齐 origin。
    let bounds = bounds.or_else(|| fallback_bounds(&info));
    let spec = match gisserver::arcgis_cache::grid::derive(&info, "probe", bounds) {
        Ok(s) => s,
        Err(e) => return bad(&format!("解析瓦片网格失败：{e}")),
    };

    let lods: Vec<Value> = spec
        .levels
        .iter()
        .enumerate()
        .map(|(i, z)| {
            json!({
                "level": z,
                "scale": spec.scales.get(i).copied().unwrap_or(0.0),
                "resolution": spec.resolutions.get(i).copied().unwrap_or(f64::NAN),
            })
        })
        .collect();

    json_resp(
        StatusCode::OK,
        json!({
            "source": raw,
            "cache_dir": cache_dir.to_string_lossy(),
            "crs": spec.crs,
            "wkid": info.wkid,
            "origin": spec.origin,
            "extent": spec.extent,
            "tile_px": spec.tile_px,
            "tile_cols": info.tile_cols,
            "tile_rows": info.tile_rows,
            "dpi": info.dpi,
            "levels": spec.levels,
            "resolutions": spec.resolutions,
            "scales": spec.scales,
            "matrix_w": spec.matrix_w,
            "matrix_h": spec.matrix_h,
            "min_zoom": info.min_zoom(),
            "max_zoom": info.max_zoom(),
            "bbox": bounds,
            "lods": lods,
        }),
    )
}

/// 兜底读取 XML 文件文本（兼容 UTF-8 BOM / UTF-16LE / UTF-16BE）。
/// `gisserver::arcgis_cache::conf` 内部有同名实现但非 pub，这里按同规则自实现，
/// 供直接解析 `tile_schema.xml` 这类纯瓦片方案文件使用。
fn read_xml_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {}: {e}", path.display()))?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes[3..].to_vec()).map_err(|e| e.to_string());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let (le, body) = if bytes.starts_with(&[0xFF, 0xFE]) {
            (true, &bytes[2..])
        } else {
            (false, &bytes[2..])
        };
        let units: Vec<u16> = body
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return String::from_utf16(&units).map_err(|e| e.to_string());
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// 缺少 `conf.cdi` / bundle 探测时的包络兜底：经纬度 CRS（4326/4490/4214/4258）
/// 按全球经纬度范围；其余投影 CRS 按最细级瓦片跨度对齐到 origin 的满幅网格。
/// 返回 `[minx, miny, maxx, maxy]`。
fn fallback_bounds(info: &gisserver::arcgis_cache::conf::CacheInfo) -> Option<[f64; 4]> {
    let [ox, oy] = info.origin;
    if gisserver::wms_common::crs::is_geographic(&info.crs) {
        return Some([ox.min(-180.0), oy.min(-90.0), 180.0, 90.0]);
    }
    // 投影 CRS：以 origin 为左上，向右、向下各铺一个「米制全球跨度」，
    // 至少能容纳常见高斯/UTM 带数据。
    let world = 40_075_016.685_578_488 / 2.0; // ~20037508 m
    Some([ox, oy - world, ox + world, oy])
}

fn bad(msg: &str) -> Response {
    json_resp(StatusCode::BAD_REQUEST, json!({ "error": msg }))
}
