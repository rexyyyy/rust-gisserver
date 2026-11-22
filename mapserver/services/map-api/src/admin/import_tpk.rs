//! ArcGIS 缓存包导入（`.tpk` / `.tpkx` → 可发布的缓存目录）。
//!
//! `POST /admin/import-tpk` body `{path, name?}`：
//!
//! * `path`：**服务器上的** `.tpk` / `.tpkx` 包路径（受 `data_root` 守卫约束，
//!   与其他管理端点同一套前缀校验）；
//! * `name`：落点目录名，缺省取包名去扩展名。
//!
//! # 落点
//!
//! `{data_root}/arcgis_tpk/explode_tpk/{name}_tpk/`——即「标准子目录
//! `arcgis_tpk` 下的 `explode_tpk`」里，每份导入一个独立目录。目录内**就是**
//! 一份标准的 ArcGIS 缓存目录（`conf.xml` + `conf.cdi` + `_alllayers/`，或
//! `root.json` + `tile/`），可以直接作为 ArcGIS 缓存图层发布。
//!
//! # 为什么导入后就地解压而不是直接指向包
//!
//! 直接把 `.tpk` 当图层源（`ArcgisCache::open` 走 `tpk::ensure_dir`）在运行期
//! 是可行的，但那个解压目录在**服务自己的缓存根**下（`cache/tpk/`），不进
//! `data_root`，管理面看不到、也删不掉。导入的语义是「把这份数据收进来」，
//! 因此必须落在 `data_root` 内、且是可预期的固定位置。
//!
//! # 返回
//!
//! 导入完成后回一份**定义记录**（`CacheDefinition`）与数据范围：来源形态
//! （`conf.xml` / `root.json`）、CRS、原点、瓦片尺寸、LOD 阶梯、瓦片根目录名，
//! 以及「是否复用了已有定义记录」。这份信息与后续发布时的网格快照同源，
//! 因此前端展示与引擎运行时不会打架。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use crate::admin::discover::{guard_data_root_with, query_data_root};
use crate::admin::http::{authorize, json as json_resp};
use crate::state::AppState;

/// 导入落点相对 `data_root` 的固定位置。
const EXPLODE_SUBDIR: &str = "arcgis_tpk/explode_tpk";

/// `POST /admin/import-tpk`
pub async fn import_tpk(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    RawQuery(qs): RawQuery,
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
        return bad("缺少 path 字段（服务器上的 .tpk / .tpkx 包路径）");
    };

    // 源包必须在 data_root 内：这是数据操作，与其他管理端点同一套守卫。
    let override_root = query_data_root(qs.as_deref());
    let src = match guard_data_root_with(&state, Path::new(raw), override_root.as_deref()) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !gisserver::arcgis_cache::tpk::is_package(&src) {
        return bad(&format!(
            "不是 ArcGIS 缓存包（扩展名应为 .tpk 或 .tpkx）：{}",
            src.display()
        ));
    }

    // 落点根：{data_root}/arcgis_tpk/explode_tpk
    let root = match resolve_data_root(&state, override_root.as_deref()) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let explode_root = root.join(EXPLODE_SUBDIR);

    // 目录名：缺省取包名去扩展名；只保留安全字符，防目录穿越。
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(sanitize_name)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            sanitize_name(
                src.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("cache"),
            )
        });
    let dest = explode_root.join(format!("{name}_tpk"));

    // 解压到指定目录（内部走「临时目录 + 原子改名」，失败不留半成品）。
    let unpacked = match gisserver::arcgis_cache::tpk::unpack_to(&src, &dest) {
        Ok(u) => u,
        Err(e) => return bad(&format!("解压失败：{e}")),
    };

    // 用「打开」验证这份缓存确实可读：定义记录能解析、瓦片根目录存在、
    // 索引布局可识别。这一步与运行时出图走同一条路径（`ArcgisCache::open`）。
    let cache = match gisserver::arcgis_cache::bundle::ArcgisCache::open(&unpacked.dir) {
        Ok(c) => c,
        Err(e) => return bad(&format!("导入的缓存无法读取：{e}")),
    };

    let def = cache.info().clone();
    let bounds = cache.bounds();
    let probe = cache.probe().clone();
    let version = format!("{:?}", cache.version()).to_lowercase();

    json_resp(
        StatusCode::OK,
        json!({
            "source": src.display().to_string(),
            "dir": unpacked.dir.display().to_string(),
            "config_file": unpacked.config_file.display().to_string(),
            "package_kind": match unpacked.kind {
                gisserver::arcgis_cache::tpk::PackageKind::Tpk => "tpk",
                gisserver::arcgis_cache::tpk::PackageKind::Tpkx => "tpkx",
            },
            "entries": unpacked.entries,
            "definition_id": def.id,
            "definition_reused": cache.docs_reused(),
            "config_kind": def.kind.as_str(),
            "crs": def.crs,
            "wkid": def.wkid,
            "origin": def.origin,
            "tile_cols": def.tile_cols,
            "tile_rows": def.tile_rows,
            "dpi": def.dpi,
            "packet_size": def.packet_size,
            "storage_format": def.storage_format,
            "tile_root": def.tile_root,
            "min_zoom": def.min_zoom(),
            "max_zoom": def.max_zoom(),
            "lods": def.lods.iter().map(|l| json!({
                "level": l.level,
                "scale": l.scale,
                "resolution": l.resolution,
            })).collect::<Vec<_>>(),
            "bounds": bounds,
            "cache_version": version,
            "levels": probe.levels,
            "max_col": probe.max_col,
            "max_row": probe.max_row,
        }),
    )
}

/// 有效的数据根：优先管理请求按工作空间覆盖的 `data_root`，否则用配置里的。
fn resolve_data_root(state: &AppState, override_root: Option<&str>) -> Result<PathBuf, Response> {
    let raw = override_root
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            state
                .config
                .server
                .data_root
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
    let Some(raw) = raw else {
        return Err(bad("未配置 data_root，无法确定导入落点"));
    };
    let p = PathBuf::from(gisserver::encoding::fix_path(&raw));
    let abs = p
        .canonicalize()
        .map_err(|e| bad(&format!("data_root 不可访问：{raw}（{e}）")))?;
    Ok(abs)
}

/// 保留文件名安全字符（去分隔符与控制字符），防目录穿越。
fn sanitize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

fn bad(msg: &str) -> Response {
    json_resp(StatusCode::BAD_REQUEST, json!({ "error": msg }))
}
