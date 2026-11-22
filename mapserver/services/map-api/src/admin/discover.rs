//! 只读数据发现与元数据探查：`rust-gisserver` 的 Workspace → Store → Resource
//! 组织层在服务平面对应的唯一接口。
//!
//! * `GET /admin/capabilities`               — 数据类型 × 服务类型权威矩阵。
//! * `GET /admin/probe?path=`                — 单个数据实体的元数据（bbox/源CRS）。
//! * `GET /admin/discover?path=&data_type=`  — 按 Store 类型扫描目录列出实体。
//! * `POST /admin/pg-tables`                 — PostGIS `geometry_columns` 清单。
//!
//! 全部只读：不写盘、不建常驻池、不生成瓦片、不导入要素、不重投影落库。
//! 读的是文件头 / 归档元数据 / `tileset.json` / `geometry_columns`，属于计划 §7.1
//! 允许的「读元数据探测 bbox 与源 CRS」，不是数据处理。rust-gisserver 的服务模型
//! （Layer/SourceKind）不因本模块改变。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use crate::admin::http::{authorize, json};
use crate::state::AppState;

/// 单个目录条目的上限（含递归扫描时累计的实体数），防大目录一次吐爆。
const MAX_ENTRIES: usize = 2000;
/// 递归扫描的最大层深（`depth` 参数上限）。
const MAX_DEPTH: usize = 4;
/// GeoJSON 整体读入的体积上限；超过则只登记不探测（读要素属全量解析）。
const MAX_TEXT_SOURCE: u64 = 256 * 1024 * 1024;

/// Store 类型 → 匹配的实体扩展名（`3dtiles_dir` 是目录型，单独判定）。
const STORE_TYPES: &[(&str, &[&str])] = &[
    ("cog_dir", &["tif", "tiff"]),
    ("geojson_dir", &["geojson", "json"]),
    ("shapefile_dir", &["shp"]),
    ("pmtiles_dir", &["pmtiles"]),
    ("mbtiles_dir", &["mbtiles"]),
    ("3dtiles_dir", &[]),
    // ArcGIS 预切片缓存：目录内（或一级子目录）有 Conf.xml + _alllayers；
    // 也接受 .tpk（Server 紧凑缓存包）与 .tpkx（Pro Compact Tile Package）。
    ("arcgis_dir", &["tpk", "tpkx"]),
    // 静态 MVT 瓦片金字塔（z/x/y.pbf）：仅发现，暂无对应服务源。
    ("mvt_dir", &[]),
];

/// 3D Tiles 数据集的入口文件名（目录内存在该文件即认为是一个 3D Tiles 实体）。
const TILESET_ENTRY: &str = "tileset.json";

/// 数据类型 → (rust-gisserver kind, 可发布服务类型, 默认勾选)。
///
/// 口径来自代码实测（`tiles.rs::raster_tile` / `wms.rs::render_get_map` /
/// `wfs.rs` / `ogcapi_features.rs` / `mvt.rs`），而非文档宣传：
/// * PMTiles / MBTiles 归档走 WMTS / XYZ 直出，**WMS GetMap 会拒绝**（无栅格源）；
/// * 归档若是矢量（tile_type=MVT / format=pbf），只有 `mvt_tile` 认它，WMTS/XYZ 反被拒；
/// * WMTS 与 XYZ 同走 `raster_tile`，能力必然一致；
/// * 静态 GeoJSON 矢量层 **WFS / OGC API-Features 可用**（`vector_fc` 是一等公民）；
/// * PostGIS 层不支持 WMTS（`raster_tile` 对 PostGIS 返回「无栅格源」）。
fn data_type_caps(data_type: &str) -> (&'static str, &'static [&'static str], &'static [&'static str]) {
    match data_type {
        "cog" => ("cog", &["wms", "wmts", "xyz"], &["wms", "wmts"]),
        "geojson" => (
            "vector",
            &["wms", "wmts", "xyz", "mvt", "wfs", "oaf"],
            &["mvt"],
        ),
        // 栅格归档：raster_tile 读通（WMTS/XYZ 同源），另可整档 Range 下载。
        "pmtiles" => ("pmtiles", &["wmts", "xyz"], &["wmts"]),
        // 矢量归档：只有 mvt_tile 认它，raster_tile 会拒绝非 PNG 归档。
        "pmtiles_mvt" => ("pmtiles", &["mvt"], &["mvt"]),
        "mbtiles" => ("mbtiles", &["wmts", "xyz"], &["wmts"]),
        "mbtiles_mvt" => ("mbtiles", &["mvt"], &["mvt"]),
        "3dtiles" => ("3dtiles", &["3dtiles"], &["3dtiles"]),
        "arcgis" => ("arcgis", &["wmts", "xyz"], &["wmts"]),
        "postgis_table" => ("postgis", &["wms", "mvt", "wfs", "oaf"], &["mvt", "wms"]),
        // PostGIS 普通表（无 geometry 列）：不可发布为图层服务，仅提供数据查询服务。
        "pg_table" => ("", &[], &[]),
        // 静态 shapefile：rust-gisserver 无 shp 读取器，计划 §6 标「将来支持」。
        "shapefile" => ("", &[], &[]),
        // 静态 MVT 瓦片目录：无对应 SourceKind，仅发现。
        "mvt" => ("", &[], &[]),
        _ => ("", &[], &[]),
    }
}

/// `GET /admin/capabilities` — 组织层要用的权威矩阵：Store 类型、数据类型到
/// rust-gisserver kind 的映射、每类可发布的服务类型与默认勾选。
pub async fn capabilities(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let rows: Vec<Value> = [
        "cog",
        "geojson",
        "pmtiles",
        "pmtiles_mvt",
        "mbtiles",
        "mbtiles_mvt",
        "3dtiles",
        "arcgis",
        "mvt",
        "postgis_table",
        "pg_table",
        "shapefile",
    ]
    .iter()
    .map(|dt| {
        let (kind, supported, defaults) = data_type_caps(dt);
        json!({
            "data_type": dt,
            "mapserver_kind": kind,
            "publishable": !kind.is_empty(),
            "service_types": supported,
            "default_service_types": defaults,
            "note": match *dt {
                "shapefile" => "rust-gisserver 无静态 shapefile 读取器：可发现、不可发布（计划标『将来支持』）",
                "pmtiles" | "mbtiles" => "栅格归档：WMTS/XYZ 直读 + 整档下载；WMS GetMap 不支持归档类",
                "pmtiles_mvt" | "mbtiles_mvt" => "矢量归档：仅 /mvt 直读，WMTS/XYZ/WMS 均不支持",
                "postgis_table" => "PostGIS 层不支持 WMTS/XYZ（无栅格源）",
                "pg_table" => "PostGIS 普通表（无 geometry 列）：不可发布图层服务，仅提供数据查询",
                "arcgis" => "ArcGIS 预切片缓存：读缓存自带 (z,col,row)，宣 WMTS/XYZ",
                "mvt" => "静态 MVT 瓦片目录：可发现，暂无对应服务源，暂不支持发布",
                _ => "",
            },
        })
    })
    .collect();
    json(
        StatusCode::OK,
        json!({
            "store_types": STORE_TYPES
                .iter()
                .map(|(t, exts)| json!({"store_type": t, "extensions": exts}))
                .collect::<Vec<_>>(),
            "data_types": rows,
            "service_types": ["wms", "wmts", "xyz", "wfs", "mvt", "oaf", "3dtiles"],
            "data_root": state.config.server.data_root.clone(),
        }),
    )
}

/// `GET /admin/probe?path=<file|dir>` — 读单个实体的元数据。
pub async fn probe(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    RawQuery(qs): RawQuery,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let raw = match query_param(qs.as_deref(), "path") {
        Some(p) if !p.is_empty() => p,
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 path 参数（要探查的数据文件或目录）" }),
            )
        }
    };
    let override_root = query_data_root(qs.as_deref());
    let path = match guard_data_root_with(&state, Path::new(&raw), override_root.as_deref()) {
        Ok(p) => p,
        Err(r) => return r,
    };
    json(StatusCode::OK, probe_one(&state, &path))
}

/// `GET /admin/discover?path=<dir>&data_type=<store 类型>&depth=<1..4>` —
/// 只读扫描一个 Store 目录，列出其中的数据实体并逐个探查元数据。
pub async fn discover(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    RawQuery(qs): RawQuery,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let q = qs.as_deref();
    let raw = match query_param(q, "path") {
        Some(p) if !p.is_empty() => p,
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 path 参数（Store 目录）" }),
            )
        }
    };
    let override_root = query_data_root(q);
    let dir = match guard_data_root_with(&state, Path::new(&raw), override_root.as_deref()) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !dir.is_dir() {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("不是目录：{}", dir.display()) }),
        );
    }
    let store_type = query_param(q, "data_type").unwrap_or_default();
    let exts: &[&str] = STORE_TYPES
        .iter()
        .find(|(t, _)| *t == store_type)
        .map(|(_, e)| *e)
        .unwrap_or(&[]);
    let wants_tileset = store_type == "3dtiles_dir";
    let wants_arcgis = store_type == "arcgis_dir";
    let wants_mvt = store_type == "mvt_dir";
    let depth = query_param(q, "depth")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_DEPTH);

    let mut entries: Vec<Value> = Vec::new();
    scan_dir(
        &state,
        &dir,
        exts,
        wants_tileset,
        wants_arcgis,
        wants_mvt,
        depth,
        &mut entries,
    );
    let count = entries.len();
    json(
        StatusCode::OK,
        json!({
            "path": display_path(&dir),
            "store_type": store_type,
            "depth": depth,
            "count": count,
            "truncated": count >= MAX_ENTRIES,
            "entries": entries,
        }),
    )
}

/// `POST /admin/pg-tables`（body `{"dsn":..,"schema":..}`）— PostGIS 空间表清单。
///
/// 只读 `geometry_columns` 目录视图，不碰业务表、不算几何、不建常驻池：临时池
/// size=1 用完即弃，因此 map-api 自身有没有配 PG 都与此无关（计划 §1 硬约束）。
/// DSN 走请求体而非查询串，避免口令进 URL 与访问日志；响应不回显 DSN。
pub async fn pg_tables(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let dsn = match body.get("dsn").and_then(|v| v.as_str()) {
        Some(d) if !d.trim().is_empty() => d.trim().to_string(),
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 dsn 字段（postgres://user:pass@host:port/db）" }),
            )
        }
    };
    let schema = body
        .get("schema")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let (entries, truncated) = match pg_table_entries(&dsn, &schema).await {
        Ok(r) => r,
        Err(e) => {
            return json(
                StatusCode::BAD_GATEWAY,
                json!({ "error": format!("PostGIS 元数据读取失败：{e}") }),
            )
        }
    };
    let count = entries.len();
    json(
        StatusCode::OK,
        json!({
            "schema_filter": schema,
            "count": count,
            "truncated": truncated,
            "tables": entries,
        }),
    )
}

/// `geometry_columns` 查询并组装发现条目：`schema` 非空时按模式过滤（参数绑定，
/// 不拼接 SQL）。返回的条目字段与 `base()` 一致，便于组织层同构处理。
///
/// 除空间表（`geometry_columns` 登记的）外，还返回同一模式下**无 geometry 列**的
/// 普通表（`data_type=pg_table`），供组织层提供「数据查询服务」入口。普通表
/// `publishable=false`，不参与图层发布。
async fn pg_table_entries(dsn: &str, schema: &str) -> Result<(Vec<Value>, bool), String> {
    let (kind, supported, defaults) = data_type_caps("postgis_table");
    let pool = gisserver::pg_source::PgPool::new(dsn, 1)
        .await
        .map_err(|e| e.to_string())?;
    let client = pool.get().await.map_err(|e| e.to_string())?;
    // coord_dimension 在 PostGIS 2.x 是 smallint、3.x 起改 varchar，统一 ::text
    // 拉平后由代码解析，避免按整型读取在 PostGIS 3.x 上反序列化 panic。
    const SQL: &str = "select f_table_schema, f_table_name, f_geometry_column, \
                       coord_dimension::text as coord_dimension, srid, type \
                       from geometry_columns \
                       where ($1 = '' or f_table_schema = $1) \
                       order by f_table_schema, f_table_name \
                       limit $2";
    let rows = client
        .query(SQL, &[&schema, &(MAX_ENTRIES as i64 + 1)])
        .await
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::with_capacity(rows.len());
    for r in rows.iter() {
        let sch: String = r.get(0);
        let tbl: String = r.get(1);
        let geom_col: String = r.get(2);
        let dim_txt: String = r.get(3);
        let dim: i16 = dim_txt.trim().parse().unwrap_or(2);
        let srid: i32 = r.get(4);
        let gtype: String = r.get(5);
        // 四至：只读元数据探测，对每张表查 `ST_Extent`（标识符经双引号引用防注入）。
        // 空表返回 NULL → None；几何列/表名均来自 geometry_columns（可信），仍按标识符引用。
        let bbox = client
            .query_opt(
                &format!(
                    "SELECT ST_Extent({})::text FROM {}.{}",
                    quote_ident(&geom_col),
                    quote_ident(&sch),
                    quote_ident(&tbl)
                ),
                &[],
            )
            .await
            .ok()
            .flatten()
            .and_then(|row| row.try_get::<usize, Option<String>>(0).ok().flatten())
            .and_then(|s| parse_box(&s));
        entries.push(json!({
            // 目录内身份：schema.table，与发布表单里的 table/schema 字段一致。
            "path": format!("{sch}.{tbl}"),
            "name": tbl,
            "data_type": "postgis_table",
            "kind": kind,
            "publishable": !kind.is_empty(),
            // PostGIS 层四至已在发现阶段求取（ST_Extent），无需人工填写。
            "needs_bbox": false,
            "bbox": bbox,
            "src_crs": format!("EPSG:{srid}"),
            "service_types": supported,
            "default_service_types": defaults,
            "meta": {
                "schema": sch,
                "table": tbl,
                "geometry_column": geom_col,
                "coord_dimension": dim,
                "srid": srid,
                "geometry_type": gtype,
            },
            "reason": Value::Null,
        }));
    }

    // 普通表：information_schema.tables 中不在 geometry_columns 里的 BASE TABLE。
    // 只做元数据登记（schema/table），不算行数、不读数据，保持发现阶段只读轻量。
    // 排除系统对象：pg_catalog/information_schema/topology/tiger 模式，以及
    // PostGIS 元数据表（spatial_ref_sys 等）——它们不是业务数据，不应进数据图层列表。
    const PLAIN_SQL: &str = "select t.table_schema, t.table_name \
                             from information_schema.tables t \
                             where ($1 = '' or t.table_schema = $1) \
                               and t.table_type = 'BASE TABLE' \
                               and t.table_schema not in ('pg_catalog', 'information_schema', 'topology', 'tiger') \
                               and t.table_name not in ( \
                                 'spatial_ref_sys', 'geography_columns', 'geometry_columns', \
                                 'raster_columns', 'raster_overviews' \
                               ) \
                               and not exists ( \
                                 select 1 from geometry_columns g \
                                 where g.f_table_schema = t.table_schema \
                                   and g.f_table_name = t.table_name \
                               ) \
                             order by t.table_schema, t.table_name \
                             limit $2";
    let plain_rows = client
        .query(PLAIN_SQL, &[&schema, &(MAX_ENTRIES as i64 + 1)])
        .await
        .map_err(|e| e.to_string())?;
    for r in plain_rows.iter() {
        let sch: String = r.get(0);
        let tbl: String = r.get(1);
        entries.push(json!({
            "path": format!("{sch}.{tbl}"),
            "name": tbl,
            "data_type": "pg_table",
            "kind": "",
            "publishable": false,
            "needs_bbox": false,
            "bbox": Value::Null,
            "src_crs": Value::Null,
            "service_types": [],
            "default_service_types": [],
            "meta": {
                "schema": sch,
                "table": tbl,
            },
            "reason": Value::Null,
        }));
    }
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    Ok((entries, truncated))
}

/// Quote a SQL identifier, doubling embedded double quotes.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// 解析 `ST_Extent::text` 的 `"BOX(minx miny,maxx maxy)"` 为 `[minx,miny,maxx,maxy]`。
fn parse_box(s: &str) -> Option<Vec<f64>> {
    let inner = s.trim().strip_prefix("BOX(")?.strip_suffix(')')?;
    let mut it = inner.split(',');
    let mins: Vec<&str> = it.next()?.split_whitespace().collect();
    let maxs: Vec<&str> = it.next()?.split_whitespace().collect();
    if mins.len() != 2 || maxs.len() != 2 {
        return None;
    }
    Some(vec![
        mins[0].parse().ok()?,
        mins[1].parse().ok()?,
        maxs[0].parse().ok()?,
        maxs[1].parse().ok()?,
    ])
}

/// 扫描一个目录（一层），命中实体就探查；`depth > 1` 时继续下钻子目录。
fn scan_dir(
    state: &AppState,
    dir: &Path,
    exts: &[&str],
    wants_tileset: bool,
    wants_arcgis: bool,
    wants_mvt: bool,
    depth: usize,
    out: &mut Vec<Value>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    names.sort();
    for p in names {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let is_dir = p.is_dir();
        if is_dir {
            if wants_tileset && p.join(TILESET_ENTRY).is_file() {
                out.push(probe_one(state, &p));
                continue;
            }
            if wants_arcgis && is_arcgis_cache(&p) {
                out.push(probe_arcgis(&p));
                continue;
            }
            if wants_mvt && is_mvt_pyramid(&p) {
                out.push(probe_mvt(&p));
                continue;
            }
            if depth > 1 {
                scan_dir(
                    state,
                    &p,
                    exts,
                    wants_tileset,
                    wants_arcgis,
                    wants_mvt,
                    depth - 1,
                    out,
                );
            }
            continue;
        }
        if exts.is_empty() {
            // 目录型 Store（3dtiles）也接受直接指向 tileset.json 的文件路径。
            if wants_tileset
                && p.file_name()
                    .map(|n| n.eq_ignore_ascii_case(TILESET_ENTRY))
                    .unwrap_or(false)
            {
                out.push(probe_one(state, &p));
            }
            continue;
        }
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        // ArcGIS 打包缓存 .tpk / .tpkx：按缓存探，而不是当未知扩展名。
        if wants_arcgis && (ext == "tpk" || ext == "tpkx") {
            out.push(probe_arcgis(&p));
            continue;
        }
        let matches = exts.iter().any(|x| x.eq_ignore_ascii_case(&ext));
        if matches {
            out.push(probe_one(state, &p));
        }
    }
}

/// 目录（或其一级子目录）是否为一份 ArcGIS 预切片缓存：以 `Conf.xml` 可解析为准。
fn is_arcgis_cache(dir: &Path) -> bool {
    gisserver::arcgis_cache::conf::CacheDocs::load(dir).is_ok()
}

/// 是否为静态 MVT 瓦片金字塔：抽 `z/x` 前几个分支确认存在 `.pbf` 叶子，避免全目录扫描。
fn is_mvt_pyramid(dir: &Path) -> bool {
    let numeric_children = |p: &Path| -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(p) else {
            return Vec::new();
        };
        rd.flatten()
            .map(|e| e.path())
            .filter(|q| {
                q.is_dir()
                    && q.file_name()
                        .map(|n| !n.is_empty() && n.to_string_lossy().chars().all(|c| c.is_ascii_digit()))
                        .unwrap_or(false)
            })
            .take(3)
            .collect()
    };
    numeric_children(dir).iter().any(|z| {
        numeric_children(z).iter().any(|x| {
            std::fs::read_dir(x)
                .map(|rd| {
                    rd.flatten().any(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext.eq_ignore_ascii_case("pbf"))
                            .unwrap_or(false)
                            && e.path().is_file()
                    })
                })
                .unwrap_or(false)
        })
    })
}

/// ArcGIS 缓存的资源名：缓存目录通常叫 `Layers`，本身没有业务含义，取它的
/// 上级目录名；目录名里的汉字转拼音首字母，其余字符原样保留。
fn arcgis_resource_name(cache_dir: &Path) -> String {
    let is_container = cache_dir
        .file_name()
        .map(|n| {
            let s = n.to_string_lossy();
            s.eq_ignore_ascii_case("layers") || s == "图层"
        })
        .unwrap_or(false);
    let named = if is_container {
        cache_dir.parent().unwrap_or(cache_dir)
    } else {
        cache_dir
    };
    let raw = named
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    pinyin_initials(&raw)
}

/// 汉字 → 拼音首字母；ASCII 及其他字符原样保留（如 `官渡区_CH` → `gdq_CH`）。
fn pinyin_initials(s: &str) -> String {
    use pinyin::ToPinyin;
    let mut out = String::with_capacity(s.len());
    for (ch, py) in s.chars().zip(s.to_pinyin()) {
        match py {
            Some(p) => out.push_str(p.first_letter()),
            None => out.push(ch),
        }
    }
    out
}

/// ArcGIS 预切片缓存：读 `Conf.xml` / `conf.cdi`，给出缓存范围、WKID 与层级。
fn probe_arcgis(path: &Path) -> Value {
    let name = arcgis_resource_name(path);
    let docs = match gisserver::arcgis_cache::conf::CacheDocs::load(path) {
        Ok(d) => d,
        Err(e) => {
            let mut v = finish(
                path,
                "arcgis",
                json!({}),
                Some(format!("ArcGIS 缓存解析失败：{e}")),
            );
            v["name"] = json!(name);
            return v;
        }
    };
    let bbox = docs.cdi.as_ref().map(|c| c.bounds);
    let def = &docs.def;
    let wkid = if def.wkid > 0 { Some(def.wkid) } else { docs.cdi.as_ref().map(|c| c.wkid).filter(|w| *w > 0) };
    let mut v = if bbox.is_none() {
        manual(
            path,
            "arcgis",
            wkid,
            json!({ "cache_dir": display_path(&docs.cache_dir) }),
            "未读到 conf.cdi 的全范围，请在发布表单手工填写四至".into(),
        )
    } else {
        let (kind, supported, defaults) = data_type_caps("arcgis");
        base(path, "arcgis", kind, bbox, wkid, supported, defaults)
    };
    v["name"] = json!(name);
    v["source"] = json!(display_path(path));
    if let Some(meta) = v.get_mut("meta").and_then(|m| m.as_object_mut()) {
        meta.insert("cache_dir".into(), json!(display_path(&docs.cache_dir)));
        meta.insert("storage_format".into(), json!(def.storage_format));
        meta.insert("packet_size".into(), json!(def.packet_size));
        meta.insert("min_zoom".into(), json!(def.min_zoom()));
        meta.insert("max_zoom".into(), json!(def.max_zoom()));
        // 原生网格：发布时固化进账本，前端展示/核对与引擎运行时同源（定义记录）。
        meta.insert("crs".into(), json!(def.crs));
        meta.insert("tile_cols".into(), json!(def.tile_cols));
        meta.insert("tile_rows".into(), json!(def.tile_rows));
        meta.insert("dpi".into(), json!(def.dpi));
        meta.insert("origin".into(), json!(def.origin));
        // 定义记录来源与瓦片根目录名（管理面据此展示「这份缓存的配置来自哪」）
        meta.insert("config_kind".into(), json!(def.kind.as_str()));
        meta.insert("tile_root".into(), json!(def.tile_root));
        meta.insert("definition_id".into(), json!(def.id));
        meta.insert(
            "lods".into(),
            json!(def
                .lods
                .iter()
                .map(|l| json!({
                    "level": l.level,
                    "scale": l.scale,
                    "resolution": l.resolution,
                }))
                .collect::<Vec<_>>()),
        );
    }
    v
}

/// 静态 MVT 瓦片目录：可发现，但 rust-gisserver 暂无对应的服务源。
fn probe_mvt(path: &Path) -> Value {
    finish(
        path,
        "mvt",
        json!({}),
        Some("静态 MVT 瓦片目录：已发现，暂无对应服务源，暂不支持发布".into()),
    )
}

/// 探查单个实体：读文件头 / 归档元数据，得到数据类型、bbox、源 CRS、可发布性。
/// 任何读取失败都只降级为 `publishable:false + reason`，不中断整体发现。
fn probe_one(state: &AppState, path: &Path) -> Value {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // 目录（或 tileset.json 文件）= 3D Tiles 数据集。
    if path.is_dir() || ext == "json" && name.eq_ignore_ascii_case(TILESET_ENTRY) {
        return probe_3dtiles(path);
    }

    match ext.as_str() {
        "tif" | "tiff" => probe_cog(state, path),
        "geojson" | "json" => probe_geojson(path),
        "shp" => finish(
            path,
            "shapefile",
            json!({}),
            Some("rust-gisserver 无静态 shapefile 读取器：仅发现，发布属『将来支持』（计划 §6 边界）".into()),
        ),
        "pmtiles" => probe_pmtiles(path),
        "mbtiles" => probe_mbtiles(path),
        _ => finish(path, "unknown", json!({}), Some(format!("不认识的扩展名：.{ext}"))),
    }
}

/// COG / GeoTIFF：读 TIFF 头的 ModelTiepoint/ModelPixelScale + GeoKeyDirectory。
fn probe_cog(state: &AppState, path: &Path) -> Value {
    let uri = path.display().to_string();
    let s3 = state.config.server.s3.as_ref().map(|c| c.to_storage());
    match gisserver::storage::source_from_uri_with_s3(&uri, s3.as_ref())
        .and_then(|src| gisserver::cog::Cog::parse(src.as_ref()).map(|cog| (cog, src)))
    {
        Ok((cog, _)) => {
            let lvl = &cog.levels[0];
            let g = lvl.geo;
            let (x1, y0) = g.pix_to_geo(lvl.width as f64, lvl.height as f64);
            let bbox = [g.origin_x.min(x1), y0.min(g.origin_y), g.origin_x.max(x1), y0.max(g.origin_y)];
            let data_type = "cog";
            let (kind, supported, defaults) = data_type_caps(data_type);
            let mut v = base(path, data_type, kind, Some(bbox), cog.srid, supported, defaults);
            v["meta"] = json!({
                "width": lvl.width,
                "height": lvl.height,
                "tile_w": lvl.tile_w,
                "tile_h": lvl.tile_h,
                "levels": cog.levels.len(),
                "srid": cog.srid,
            });
            v
        }
        Err(e) => finish(path, "cog", json!({}), Some(format!("COG 头解析失败：{e}"))),
    }
}

/// GeoJSON：解析 FeatureCollection 取要素并集 bbox（规范强制 4326）。
fn probe_geojson(path: &Path) -> Value {
    let data_type = "geojson";
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(u64::MAX);
    if len > MAX_TEXT_SOURCE {
        return manual(
            path,
            data_type,
            Some(4326),
            json!({"size": len}),
            format!(
                "文件 {} MiB 超过元数据探测上限，未探测 bbox（可在发布表单手工填写）",
                len / 1024 / 1024
            ),
        );
    }
    match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
    {
        Ok(fc) => {
            let bbox = gisserver::vector_query::geom_bbox_fc(&fc);
            let (kind, supported, defaults) = data_type_caps(data_type);
            let mut v = base(path, data_type, kind, bbox, Some(4326), supported, defaults);
            let features = gisserver::vector_query::features(&fc).len();
            let geom_type = gisserver::vector_query::primary_geometry_type(&fc).unwrap_or("");
            v["meta"] = json!({
                "features": features,
                "geometry_type": geom_type,
                "size": len,
            });
            v
        }
        Err(e) => finish(path, data_type, json!({}), Some(format!("GeoJSON 解析失败：{e}"))),
    }
}

/// PMTiles：只读 127 字节头，得到 bounds（e7 定点）、瓦片类型与缩放区间。
fn probe_pmtiles(path: &Path) -> Value {
    match gisserver::pmtiles::read::PmtilesReader::open(path) {
        Ok(r) => {
            let h = r.header();
            let bbox = [
                h.min_lon_e7 as f64 / 1e7,
                h.min_lat_e7 as f64 / 1e7,
                h.max_lon_e7 as f64 / 1e7,
                h.max_lat_e7 as f64 / 1e7,
            ];
            let is_mvt = h.tile_type == gisserver::pmtiles::tile_type::MVT;
            let data_type = if is_mvt { "pmtiles_mvt" } else { "pmtiles" };
            let (kind, supported, defaults) = data_type_caps(data_type);
            let mut v = base(path, data_type, kind, Some(bbox), Some(4326), supported, defaults);
            v["meta"] = json!({
                "tile_type": tile_type_name(h.tile_type),
                "min_zoom": h.min_zoom,
                "max_zoom": h.max_zoom,
                "num_addressed_tiles": h.num_addressed_tiles,
            });
            v
        }
        Err(e) => finish(path, "pmtiles", json!({}), Some(format!("PMTiles 头解析失败：{e}"))),
    }
}

/// MBTiles：只读 SQLite `metadata` 表（bounds / format / zoom）。
fn probe_mbtiles(path: &Path) -> Value {
    let uri = path.display().to_string();
    match crate::mbtiles::MbtilesMeta::open(&uri) {
        Ok(meta) => {
            let bbox = meta.bounds().map(|b| [b.minx, b.miny, b.maxx, b.maxy]);
            let fmt = meta.format();
            let is_mvt = matches!(fmt.as_str(), "pbf" | "protobuf" | "mvt");
            let data_type = if is_mvt { "mbtiles_mvt" } else { "mbtiles" };
            let (kind, supported, defaults) = data_type_caps(data_type);
            let mut v = base(path, data_type, kind, bbox, Some(4326), supported, defaults);
            let (minz, maxz) = meta.zoom_range().unwrap_or((0, 0));
            v["meta"] = json!({
                "format": fmt,
                "min_zoom": minz,
                "max_zoom": maxz,
                "name": meta.metadata.get("name").cloned().unwrap_or_default(),
            });
            v
        }
        Err(e) => finish(path, "mbtiles", json!({}), Some(format!("MBTiles 打开失败：{e}"))),
    }
}

/// 3D Tiles：读入口 `tileset.json` 的 `root.boundingVolume`（region 为 WGS84 弧度）。
fn probe_3dtiles(path: &Path) -> Value {
    let entry = if path.is_dir() {
        path.join(TILESET_ENTRY)
    } else {
        path.to_path_buf()
    };
    let text = match std::fs::read_to_string(&entry) {
        Ok(t) => t,
        Err(e) => return finish(path, "3dtiles", json!({}), Some(format!("入口 tileset.json 读取失败：{e}"))),
    };
    let doc: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return finish(path, "3dtiles", json!({}), Some(format!("tileset.json 解析失败：{e}"))),
    };
    let bv = doc.get("root").and_then(|r| r.get("boundingVolume"));
    let (bbox, crs) = match bv {
        Some(v) if v.get("region").and_then(|x| x.as_array()).map(|a| a.len() >= 4) == Some(true) => {
            let a = v["region"].as_array().unwrap();
            let g = |i: usize| a[i].as_f64().unwrap_or(0.0).to_degrees();
            (
                Some([g(0), g(1), g(2), g(3)]),
                Some(4326u32),
            )
        }
        // bbox / sphere 是本地坐标系（未声明 transform），不能当经纬度用。
        Some(v) if v.get("bbox").is_some() || v.get("sphere").is_some() => (None, None),
        _ => (None, None),
    };
    if crs.is_none() && bbox.is_none() {
        let mut v = manual(
            path,
            "3dtiles",
            None,
            json!({"entry": display_path(&entry)}),
            "tileset.json 无可读 boundingVolume.region（本地坐标 bbox 无法换算为经纬度），请在发布表单手工填写".into(),
        );
        v["source"] = json!(display_path(path));
        return v;
    }
    let (kind, supported, defaults) = data_type_caps("3dtiles");
    let mut v = base(path, "3dtiles", kind, bbox, crs, supported, defaults);
    // 根目录即服务 Root：指向数据集目录（tileset.json 的父目录）。
    v["source"] = json!(display_path(path));
    v["meta"] = json!({
        "tileset_version": doc.get("asset").and_then(|a| a.get("version")).and_then(|v| v.as_str()).unwrap_or(""),
        "has_transform": doc.get("root").and_then(|r| r.get("transform")).is_some(),
    });
    v
}

/// 组装探查结果的公共字段。
fn base(
    path: &Path,
    data_type: &str,
    kind: &str,
    bbox: Option<[f64; 4]>,
    srid: Option<u32>,
    supported: &[&str],
    defaults: &[&str],
) -> Value {
    let bbox = bbox.filter(|b| b[0] < b[2] && b[1] < b[3]);
    json!({
        "path": display_path(path),
        "name": path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        "data_type": data_type,
        "kind": kind,
        // 可发布性只看「服务平面认不认这个类型」：四至缺失只让发布表单手工填写，
        // 不构成拒绝（PostGIS 表本来就不由元数据探测出四至）。
        "publishable": !kind.is_empty(),
        "needs_bbox": !kind.is_empty() && bbox.is_none(),
        "bbox": match bbox {
            Some([a, b, c, d]) => json!([a, b, c, d]),
            None => Value::Null,
        },
        "src_crs": srid.map(|s| format!("EPSG:{s}")),
        "service_types": supported,
        "default_service_types": defaults,
        "meta": {},
        "reason": Value::Null,
    })
}

/// 类型可识别、元数据探测不到（超大 GeoJSON / 本地坐标 3D Tiles）：
/// 仍可发布，缺的四至由发布表单手工填写，因此走 `base` 而不是 `finish`。
fn manual(path: &Path, data_type: &str, srid: Option<u32>, meta: Value, reason: String) -> Value {
    let (kind, supported, defaults) = data_type_caps(data_type);
    let mut v = base(path, data_type, kind, None, srid, supported, defaults);
    v["meta"] = meta;
    v["reason"] = json!(reason);
    v
}

/// 不可发布 / 探测失败的结果：保留实体身份，给出原因。
fn finish(path: &Path, data_type: &str, meta: Value, reason: Option<String>) -> Value {
    let (kind, supported, defaults) = data_type_caps(data_type);
    json!({
        "path": display_path(path),
        "name": path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        "data_type": data_type,
        "kind": kind,
        "publishable": false,
        "needs_bbox": false,
        "bbox": Value::Null,
        "src_crs": Value::Null,
        "service_types": supported,
        "default_service_types": defaults,
        "meta": meta,
        "reason": reason,
    })
}

/// 回显给管理层的路径：去掉 Windows `canonicalize()` 加上的 `\\?\`（含 `\\?\UNC\`）前缀。
///
/// 守卫仍拿带前缀的真身做前缀校验；去前缀后的路径再进来会被同一个 `canonicalize()`
/// 还原到同一实体，因此穿越校验不受影响，但目录快照与 `Layer` XML 里的路径人类可读。
fn display_path(p: &Path) -> String {
    let raw = p.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    }
}

/// **外部传入路径**的编码归一化（幂等）。
///
/// 只用于「路径来自系统之外」的地方：HTTP 路径参数、catalog 登记的外部数据路径等。
/// 系统内部生成的路径（配置默认值、拼接出的子路径）不适用。
///
/// 修的是两类 mojibake：传统编码字节被逐字节误读为 Latin-1；UTF-8 字节被 GBK 误读。
/// 正常 UTF-8 路径原样返回（判定保守，见 `gisserver::encoding::fix_path`）。
fn norm_input_path(p: &str) -> String {
    gisserver::encoding::fix_path(p)
}

/// `data_root` 路径守卫（计划 §9.2：文件浏览器必须限定在 data_root 内）。
///
/// * 解析为绝对路径（realpath 级 `canonicalize`），杜绝 `..` / 符号链接穿越；
/// * 未配置 `data_root` 时不设限（零配置向后兼容）；
/// * 配置后要求目标在 data_root 之内，否则 403；
/// * 管理请求可用 `data_root` 查询参数按工作空间覆盖，改 UI 后立即生效，无需重启。
pub(crate) fn guard_data_root_with(
    state: &AppState,
    raw: &Path,
    data_root_override: Option<&str>,
) -> Result<PathBuf, Response> {
    // 外部传入路径先过编码归一化（幂等）：修复 GBK 字节被误读为 Latin-1 /
    // UTF-8 被 GBK 误读两类 mojibake，正常 UTF-8 路径原样通过，再 canonicalize。
    let raw = std::path::PathBuf::from(norm_input_path(&raw.to_string_lossy()));
    let abs = raw
        .canonicalize()
        .map_err(|e| json(StatusCode::NOT_FOUND, json!({ "error": format!("路径不可访问：{}（{e}）", raw.display()) })))?;
    within_data_root(state, &abs, data_root_override)
}

/// 同上，但允许路径末段尚不存在（登记一个还没落盘的数据文件）。
pub(crate) fn guard_data_root_laxed_with(
    state: &AppState,
    raw: &Path,
    data_root_override: Option<&str>,
) -> Result<PathBuf, Response> {
    // 编码归一化同 guard_data_root（幂等，正常路径无感）。
    let raw = std::path::PathBuf::from(norm_input_path(&raw.to_string_lossy()));
    let abs = canonicalize_loose(&raw)
        .map_err(|e| json(StatusCode::NOT_FOUND, json!({ "error": format!("路径不可访问：{}（{e}）", raw.display()) })))?;
    within_data_root(state, &abs, data_root_override)
}

/// 前缀校验：绝对路径必须落在配置或管理请求覆盖的 `data_root` 内。
fn within_data_root(
    state: &AppState,
    abs: &Path,
    data_root_override: Option<&str>,
) -> Result<PathBuf, Response> {
    let root = data_root_override
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
    let Some(root) = root else {
        return Ok(abs.to_path_buf());
    };
    let root_path = std::path::PathBuf::from(norm_input_path(&root));
    let root_abs = root_path
        .canonicalize()
        .map_err(|e| json(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("data_root 不可访问：{root}（{e}）") })))?;
    if abs.starts_with(&root_abs) {
        Ok(abs.to_path_buf())
    } else {
        Err(json(
            StatusCode::FORBIDDEN,
            json!({ "error": format!("超出 data_root：{}（允许根：{}）", abs.display(), root_abs.display()) }),
        ))
    }
}

/// realpath 到最近存在的祖先，尾部缺失段按 `..` 语义归一化后拼回。
///
/// 尾部若把 `..` 用到实存祖先之上（`C:\data\..\Windows\x` 这类），返回错误：
/// 无法在不访问文件系统的前提下确定其真身，宁可拒绝也不能让前缀校验被骗过。
fn canonicalize_loose(raw: &Path) -> std::io::Result<PathBuf> {
    if let Ok(abs) = raw.canonicalize() {
        return Ok(abs);
    }
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = raw.to_path_buf();
    loop {
        match cur.canonicalize() {
            Ok(base) => {
                let mut stack: Vec<std::ffi::OsString> = Vec::new();
                for name in tail.iter().rev() {
                    if name == ".." {
                        if stack.pop().is_none() {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "路径中的 .. 超出根目录",
                            ));
                        }
                    } else if name != "." {
                        stack.push(name.clone());
                    }
                }
                let mut out = base;
                for name in stack {
                    out.push(name);
                }
                return Ok(out);
            }
            Err(e) => {
                if let Some(name) = cur.file_name() {
                    tail.push(name.to_os_string());
                }
                if !cur.pop() {
                    return Err(e);
                }
            }
        }
    }
}

/// 管理请求可用 `data_root` 查询参数覆盖配置中的数据根（manager 按工作空间传入）。
pub(crate) fn query_data_root(qs: Option<&str>) -> Option<String> {
    query_param(qs, "data_root")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 从原始查询串取参数（percent 解码；Windows 路径的 `\` 与 `:` 会被编码）。
fn query_param(qs: Option<&str>, key: &str) -> Option<String> {
    qs?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k.trim().eq_ignore_ascii_case(key) {
            Some(decode_param(v.trim()))
        } else {
            None
        }
    })
}

/// 最小 percent 解码（仅 `%XX`；`+` 保留原样，路径里不是空格语义）。
/// 目录浏览与发现接口共用：管理面向来收的是原始查询串，不做整串 form 解码。
pub(crate) fn decode_param(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// PMTiles tile_type 数值 → 可读名。
fn tile_type_name(t: u8) -> &'static str {
    match t {
        gisserver::pmtiles::tile_type::MVT => "mvt",
        gisserver::pmtiles::tile_type::PNG => "png",
        gisserver::pmtiles::tile_type::JPEG => "jpeg",
        gisserver::pmtiles::tile_type::WEBP => "webp",
        gisserver::pmtiles::tile_type::AVIF => "avif",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decodes_windows_paths() {
        assert_eq!(
            decode_param("D%3A%5Cdata%5Cwh.tif"),
            r"D:\data\wh.tif"
        );
        assert_eq!(decode_param("plain/path"), "plain/path");
        // 结尾的孤立 % 与非法十六进制按原样保留，不 panic。
        assert_eq!(decode_param("a%"), "a%");
        assert_eq!(decode_param("%zz"), "%zz");
    }

    #[test]
    fn query_param_is_case_insensitive() {
        assert_eq!(
            query_param(Some("depth=3&Path=%2Ftmp"), "path").as_deref(),
            Some("/tmp")
        );
        assert_eq!(query_param(Some("a=1"), "path"), None);
        assert_eq!(query_param(None, "path"), None);
    }

    #[test]
    fn arcgis_name_uses_cache_parent_and_pinyin_initials() {
        // 缓存目录叫 Layers 时取上级目录名。
        assert_eq!(
            arcgis_resource_name(Path::new("E:/data/1/arcgis_cah/guandu_CH_GDXZQ/Layers")),
            "guandu_CH_GDXZQ"
        );
        // 中文目录名转拼音首字母，其余字符原样保留。
        assert_eq!(
            arcgis_resource_name(Path::new("E:/data/1/arcgis_cah/官渡区_CH/图层")),
            "gdq_CH"
        );
        assert_eq!(pinyin_initials("官渡区"), "gdq");
        assert_eq!(pinyin_initials("abc-123"), "abc-123");
    }

    #[test]
    fn matrix_matches_code_capabilities() {
        // 归档类不得声明 WMS（render_get_map 会拒绝无栅格源的层）。
        let (_, sp, _) = data_type_caps("pmtiles");
        assert!(!sp.contains(&"wms"), "PMTiles 归档不应支持 WMS GetMap");
        let (_, sp, _) = data_type_caps("mbtiles");
        assert!(!sp.contains(&"wms"), "MBTiles 归档不应支持 WMS GetMap");
        // PostGIS 不得声明 WMTS（raster_tile 对 PostGIS 返回无栅格源）。
        let (_, sp, _) = data_type_caps("postgis_table");
        assert!(!sp.contains(&"wmts"), "PostGIS 层不应支持 WMTS 瓦片");
        // 静态矢量层 WFS / OAF 已打通（vector_fc 是一等公民）。
        let (kind, sp, _) = data_type_caps("geojson");
        assert_eq!(kind, "vector");
        assert!(sp.contains(&"wfs") && sp.contains(&"oaf"));
        // 矢量归档追加 MVT。
        let (kind, sp, def) = data_type_caps("pmtiles_mvt");
        assert_eq!(kind, "pmtiles");
        assert!(sp.contains(&"mvt"));
        assert_eq!(def, &["mvt"]);
        // 矢量归档不能声明栅格通道：raster_tile 对非 PNG 归档直接报错。
        let (_, sp, _) = data_type_caps("mbtiles_mvt");
        assert!(!sp.contains(&"wmts") && !sp.contains(&"xyz"));
        // WMTS 与 XYZ 同走 raster_tile，能力必然成对。
        for dt in ["cog", "geojson", "pmtiles", "mbtiles"] {
            let (_, sp, _) = data_type_caps(dt);
            assert_eq!(
                sp.contains(&"wmts"),
                sp.contains(&"xyz"),
                "{dt}：WMTS 与 XYZ 能力不一致"
            );
        }
    }

    #[test]
    fn shapefile_is_discoverable_but_not_publishable() {
        let (kind, sp, _) = data_type_caps("shapefile");
        assert!(kind.is_empty(), "shapefile 没有对应的 SourceKind");
        assert!(sp.is_empty());
        let v = finish(Path::new("/d/roads.shp"), "shapefile", json!({}), Some("将来支持".into()));
        assert_eq!(v["publishable"], json!(false));
        assert_eq!(v["data_type"], json!("shapefile"));
    }

    #[test]
    fn missing_bbox_stays_publishable_but_is_flagged() {
        // 零宽高 bbox：类型可发布，只是四至要人工填。
        let v = base(
            Path::new("/d/empty.tif"),
            "cog",
            "cog",
            Some([0.0, 0.0, 0.0, 0.0]),
            Some(4326),
            &["wmts"],
            &["wmts"],
        );
        assert_eq!(v["publishable"], json!(true));
        assert_eq!(v["needs_bbox"], json!(true), "退化 bbox 应要求人工填四至");
        assert_eq!(v["bbox"], Value::Null);
        let v = base(
            Path::new("/d/nobounds.mbtiles"),
            "mbtiles",
            "mbtiles",
            None,
            Some(4326),
            &["wmts"],
            &["wmts"],
        );
        assert_eq!(v["publishable"], json!(true), "取不到四至不等于不能发布");
        assert_eq!(v["needs_bbox"], json!(true));
    }

    #[test]
    fn manual_probe_failure_keeps_the_kind_publishable() {
        // 元数据探测失败（超大 GeoJSON / 本地坐标 3D Tiles）：仍可发布 + 带原因。
        let v = manual(
            Path::new("/d/huge.geojson"),
            "geojson",
            Some(4326),
            json!({"size": 1}),
            "超过探测上限".into(),
        );
        assert_eq!(v["publishable"], json!(true));
        assert_eq!(v["needs_bbox"], json!(true));
        assert_eq!(v["kind"], json!("vector"));
        assert_eq!(v["reason"], json!("超过探测上限"));
        // 真解析失败才是硬拒绝。
        let v = finish(
            Path::new("/d/broken.pmtiles"),
            "pmtiles",
            json!({}),
            Some("头解析失败".into()),
        );
        assert_eq!(v["publishable"], json!(false));
        assert_eq!(v["needs_bbox"], json!(false));
    }

    #[test]
    fn reported_paths_drop_the_verbatim_prefix() {
        // Windows realpath 带 `\\?\`；回显与落库都必须是可读的普通路径。
        assert_eq!(
            display_path(Path::new(r"\\?\E:\data\a.geojson")),
            r"E:\data\a.geojson"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\fs\share\a.tif")),
            r"\\fs\share\a.tif"
        );
        // 非 verbatim 路径原样返回。
        assert_eq!(display_path(Path::new("/d/a.pmtiles")), "/d/a.pmtiles");

        // 去前缀不改变实体：同一个文件 canonicalize 回来仍是守卫认得的真身。
        let dir = std::env::temp_dir().join(format!("ms-disp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.geojson");
        std::fs::write(&f, "{}").unwrap();
        let shown = display_path(&f.canonicalize().unwrap());
        assert_eq!(canonicalize_loose(Path::new(&shown)).unwrap(), f.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loose_canonicalize_keeps_missing_tail() {
        let dir = std::env::temp_dir().join(format!("ms-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 末两级都不存在：realpath 到 dir，再拼回 sub/file.tif。
        let p = dir.join("sub").join("file.tif");
        let abs = canonicalize_loose(&p).unwrap();
        assert!(abs.ends_with(Path::new("sub").join("file.tif")), "{abs:?}");
        assert!(abs.starts_with(dir.canonicalize().unwrap()));
        // `..` 在实存祖先之后被归一化，不能借它逃出 data_root。
        let tricky = dir.join("sub").join("..").join("file.tif");
        let abs2 = canonicalize_loose(&tricky).unwrap();
        assert!(abs2.ends_with("file.tif"), "{abs2:?}");
        assert!(!abs2.to_string_lossy().contains(".."));
        // 借 `..` 逃出：realpath 后落在 dir 之外，前缀校验据此拒绝。
        let esc = canonicalize_loose(&dir.join("..").join("escape.tif")).unwrap();
        assert!(!esc.starts_with(dir.canonicalize().unwrap()), "{esc:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tileset_region_converts_radians_to_degrees() {
        let dir = std::env::temp_dir().join(format!("ms-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let region = [1.919862f64, -0.226893, 1.936893, -0.200713, 0.0, 100.0];
        let doc = json!({
            "asset": {"version": "1.1"},
            "root": {"boundingVolume": {"region": region}}
        });
        std::fs::write(dir.join(TILESET_ENTRY), serde_json::to_vec(&doc).unwrap()).unwrap();
        let v = probe_3dtiles(&dir);
        let bbox = v["bbox"].as_array().cloned().unwrap_or_default();
        assert_eq!(bbox.len(), 4, "region 应换算为四至：{v}");
        assert!(bbox[0].as_f64().unwrap().abs() > 100.0, "弧度需转成度：{bbox:?}");
        assert_eq!(v["src_crs"], json!("EPSG:4326"));
        assert_eq!(v["kind"], json!("3dtiles"));
        assert_eq!(v["service_types"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
