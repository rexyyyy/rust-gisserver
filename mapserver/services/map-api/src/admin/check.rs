//! Data-source health check (`POST /admin/services/:kind/:name/check`,
//! manager scope doc §10 P1 数据源体检).
//!
//! One report per call; each probe is a `(item, ok, detail)` row so the UI can
//! render a checklist. Critical probes (data file / readability / PG connect)
//! drive the overall `ok` flag; informational rows (bounds / SRS / style) are
//! always `ok = true` with a descriptive detail.
//!
//! * file kinds (cog/vector/pmtiles/mbtiles): existence + size, then a real
//!   `LayerRuntime::open` (COG header parse, GeoJSON parse, SQLite open) —
//!   the same path startup uses, so "check passes" == "serves".
//! * postgis: fresh `PgPool::new_plain` connection test, feature count and
//!   extent probes via `gisserver::pg_source::sql` helpers (estimated first, exact
//!   fallback — mirrors startup behavior).
//! * 3dtiles: tileset.xml presence + tileset.json + payload file inventory.

use std::path::Path as FsPath;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use gisserver::wms_common::layer::SourceKind;

use crate::state::AppState;

use super::http::{authorize, json};

#[derive(Default, serde::Serialize)]
struct Report {
    name: String,
    kind: String,
    ok: bool,
    checks: Vec<CheckRow>,
}

#[derive(Clone, serde::Serialize)]
struct CheckRow {
    item: String,
    ok: bool,
    /// `critical` failures flip the overall flag; `info` rows never do.
    level: &'static str,
    detail: String,
}

impl Report {
    fn push(&mut self, item: &str, ok: bool, level: &'static str, detail: String) {
        self.checks.push(CheckRow {
            item: item.to_string(),
            ok,
            level,
            detail,
        });
    }
    fn finish(mut self) -> Report {
        self.ok = self
            .checks
            .iter()
            .all(|c| c.ok || c.level != "critical");
        self
    }
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.2} GB", n as f64 / (1 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KB", n as f64 / (1 << 10) as f64)
    } else {
        format!("{n} B")
    }
}

/// `POST /admin/services/:kind/:name/check`
pub async fn check_service(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((kind, name)): Path<(String, String)>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let mut report = Report {
        name: name.clone(),
        kind: kind.clone(),
        ..Default::default()
    };
    if kind == "3dtiles" {
        check_3dtiles(&state, &name, &mut report);
    } else {
        match state.current_layers().iter().find(|l| l.name == name) {
            Some(def) if crate::state::source_kind(&def.source) == kind => {
                check_layer(&state, def, &mut report).await
            }
            _ => {
                return json(
                    StatusCode::NOT_FOUND,
                    serde_json::json!({"error": format!("service '{kind}/{name}' not found in catalog")}),
                );
            }
        }
    }
    json(StatusCode::OK, serde_json::to_value(report.finish()).unwrap_or_default())
}

fn check_3dtiles(state: &AppState, name: &str, report: &mut Report) {
    let dir = state.config.tilesets_dir();
    let xml = dir.join(format!("{name}.xml"));
    report.push(
        "服务定义",
        xml.exists(),
        "critical",
        format!("配置文件 {}", xml.display()),
    );
    // 服务定义 XML 即数据位置的真源（Root 指向数据集根目录，Entry 为入口
    // json）。XML 缺失/不可解析时退回约定路径 {dir}/{name}，detail 里注明。
    let (root, entry) = match std::fs::read_to_string(&xml)
        .map_err(|e| e.to_string())
        .and_then(|t| crate::tilesets::parse_tileset_xml(&t, &xml))
    {
        Ok(svc) => (svc.root, svc.entry),
        Err(e) => {
            report.push(
                "tileset.json",
                false,
                "critical",
                format!("配置不可解析（{e}），无法定位数据目录"),
            );
            report.push("切片数据", false, "critical", "无数据目录，未统计".to_string());
            return;
        }
    };
    let tileset = root.join(&entry);
    if !root.is_dir() {
        report.push(
            "tileset.json",
            false,
            "critical",
            format!("数据目录指向 {}，但该目录不存在", root.display()),
        );
        report.push(
            "切片数据",
            false,
            "critical",
            format!("数据目录 {} 不存在，无法统计切片文件", root.display()),
        );
        return;
    }
    match std::fs::metadata(&tileset) {
        Ok(m) if m.is_file() => report.push(
            "tileset.json",
            true,
            "critical",
            format!("{}（{}）", tileset.display(), fmt_bytes(m.len())),
        ),
        _ => {
            report.push(
                "tileset.json",
                false,
                "critical",
                format!(
                    "数据目录指向 {}，但缺少入口文件 {}",
                    root.display(),
                    entry
                ),
            );
        }
    }
    // Payload inventory: 3D Tiles payloads (b3dm/glb/i3dm/pnts) + subtree data.
    let mut payloads = 0usize;
    let mut bytes = 0u64;
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("b3dm") | Some("glb") | Some("i3dm") | Some("pnts")
                ) {
                    payloads += 1;
                    bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    report.push(
        "切片数据",
        payloads > 0,
        "critical",
        if payloads > 0 {
            format!("{payloads} 个切片文件（{}）", fmt_bytes(bytes))
        } else {
            format!("数据目录 {} 内未发现切片文件（b3dm/glb/i3dm/pnts）", root.display())
        },
    );
}

async fn check_layer(_state: &AppState, def: &gisserver::wms_common::layer::Layer, report: &mut Report) {
    report.push(
        "服务定义",
        true,
        "info",
        format!("{}（{}）", def.title, def.src_crs),
    );

    // Data file probes.
    match &def.source {
        SourceKind::Cog { path } | SourceKind::Vector { path } | SourceKind::Pmtiles { path } | SourceKind::Mbtiles { path } => {
            if path.starts_with("http://") || path.starts_with("https://") {
                report.push("数据文件", true, "info", format!("远程数据源（{path}），惰性校验"));
            } else {
                match std::fs::metadata(path) {
                    Ok(m) if m.is_file() => report.push(
                        "数据文件",
                        true,
                        "critical",
                        format!("{path}（{}）", fmt_bytes(m.len())),
                    ),
                    Ok(_) => report.push("数据文件", false, "critical", format!("{path} 不是文件")),
                    Err(e) => report.push("数据文件", false, "critical", format!("{path}: {e}")),
                }
            }
        }
        SourceKind::Postgis(_) => {} // dedicated PG probes below
        // ArcGIS 预切片缓存：path 可以是缓存目录，也可以是 `.tpk`（ZIP 缓存包）。
        // 目录形态 `metadata().is_file()` 恒假、TPK 形态 `is_dir()` 恒假，走文件
        // 分支都会被误报。统一先经 `tpk::ensure_dir`（`.tpk` 解压、目录原样）得
        // 到真正的缓存目录，再复用 crate 的 find_conf/find_alllayers（与打开路径
        // 同一套定位规则），否则出图正常、体检却报 critical。
        SourceKind::Arcgis { path } => {
            let p = std::path::Path::new(path);
            let is_tpk = gisserver::arcgis_cache::tpk::is_tpk(p);
            if !p.exists() {
                report.push(
                    "数据文件",
                    false,
                    "critical",
                    format!("{path} 不存在（缓存目录或 .tpk 包）"),
                );
            } else {
                match gisserver::arcgis_cache::tpk::ensure_dir(p)
                    .and_then(|dir| gisserver::arcgis_cache::conf::find_conf(&dir).map(|c| (dir, c)))
                {
                    Ok((dir, conf)) => {
                        let csize = conf.metadata().map(|c| c.len()).unwrap_or(0);
                        let what = if is_tpk { "TPK 缓存包" } else { "缓存目录" };
                        report.push(
                            "数据文件",
                            true,
                            "critical",
                            format!("{what}，Conf.xml {}（{}）", fmt_bytes(csize), conf.display()),
                        );
                        let cache_dir = conf.parent().unwrap_or(&dir);
                        match gisserver::arcgis_cache::bundle::find_alllayers(cache_dir) {
                            Ok(all) => report.push(
                                "瓦片目录",
                                true,
                                "info",
                                format!("{}", all.display()),
                            ),
                            Err(e) => report.push(
                                "瓦片目录",
                                false,
                                "critical",
                                format!("{e}（缓存未切片或导出未完成）"),
                            ),
                        }
                    }
                    Err(e) => report.push("数据文件", false, "critical", e),
                }
            }
        }
    }

    // Readability: a real open (same path startup uses).
    let s3_cfg = _state.config.server.s3.as_ref().map(|c| c.to_storage());
    match &def.source {
        SourceKind::Postgis(_) => {}
        _ => match crate::layer::LayerRuntime::open(def.clone(), None, u64::MAX, s3_cfg.as_ref()) {
            Ok(_) => report.push("数据可读", true, "critical", "数据源打开成功（格式头解析通过）".into()),
            Err(e) => report.push("数据可读", false, "critical", e.to_string()),
        },
    }

    // Bounds / SRS (informational).
    report.push(
        "数据范围",
        true,
        "info",
        match def.extent {
            Some([a, b, c, d]) => format!("配置范围 [{a:.4}, {b:.4}, {c:.4}, {d:.4}]"),
            None => "未显式配置，运行时按数据计算".to_string(),
        },
    );
    report.push(
        "坐标系统",
        true,
        "info",
        format!(
            "{}（PROJ {}）",
            def.src_crs,
           if gisserver::projection::proj().is_some() { "已加载，支持动态重投影" } else { "未加载，仅原生 CRS 出图" }
        ),
    );

    // 瓦片网格类型：区分「标准/OGC 预设网格」与「自定义网格」。注意这判的是
    // TileMatrixSet（WebMercatorQuad / WorldCRS84Quad 为预设），与"坐标系是否
    // 标准 EPSG"是两个正交维度——例如 EPSG:4490 本身是标准坐标系，但按 ArcGIS
    // Conf.xml LOD 派生的 arcgis_<图层> 网格是自定义网格（OGC 未为 4490 登记预设
    // TileMatrixSet）。判定与 OGC API/WMTS 元数据共用 gisserver::wms_common::tile::is_preset_id。
    let gid = _state.config.default_grid_id(def);
    report.push(
        "瓦片网格",
        true,
        "info",
        format!(
            "{gid}（{}，坐标系 {}）",
            if gisserver::wms_common::tile::is_preset_id(&gid) { "标准/OGC 预设网格" } else { "自定义网格" },
            def.src_crs
        ),
    );

    // Style probes.
    match &def.sld {
        Some(p) => match std::fs::read_to_string(p)
            .map(|t| gisserver::renderer::sld::SldDocument::parse(&t))
        {
            Ok(Some(doc)) => {
                let rules: usize = doc.feature_type_styles.iter().map(|f| f.rules.len()).sum();
                report.push("SLD样式", true, "info", format!("{}（{rules} 条规则）", p));
            }
            _ => report.push("SLD样式", false, "info", format!("{p} 无法解析（将回退默认样式）")),
        },
        None => report.push("SLD样式", true, "info", "未配置 SLD（使用内置默认样式）".into()),
    }
    if let Some(s) = &def.style {
        let p = FsPath::new(s);
        if p.exists() {
            report.push("样式文件", true, "info", format!("{s} 存在"));
        } else {
            report.push("样式文件", false, "info", format!("{s} 不存在（将使用默认样式）"));
        }
    }

    // PostGIS dedicated probes.
    if let SourceKind::Postgis(pg) = &def.source {
        match gisserver::pg_source::PgPool::new_plain(&pg.dsn, 1).await {
            Ok(pool) => {
                report.push("PG连接", true, "critical", pg.dsn.clone());
                let target = match &pg.from {
                    Some(f) => format!("SQL: {f}"),
                    None => format!("{}/{}", pg.schema, pg.table),
                };
                match gisserver::pg_source::sql::query_count(&pool, &gisserver::pg_source::sql::count_all_sql(pg), &[]).await {
                    Ok(n) => report.push("要素数量", true, "info", format!("{target}：{n} 条要素")),
                    Err(e) => report.push("要素数量", false, "critical", format!("{target}: {e}")),
                }
                let bounds = match gisserver::pg_source::sql::query_bounds(&pool, &gisserver::pg_source::sql::bounds_sql_estimated(pg)).await {
                    Ok(Some(b)) => Some(b),
                    _ => gisserver::pg_source::sql::query_bounds(&pool, &gisserver::pg_source::sql::bounds_sql(pg)).await.ok().flatten(),
                };
                report.push(
                    "PG数据范围",
                    true,
                    "info",
                    match bounds {
                        Some([a, b, c, d]) => format!("EPSG:4326 [{a:.4}, {b:.4}, {c:.4}, {d:.4}]"),
                        None => "未取到范围（表可能为空或无统计信息）".to_string(),
                    },
                );
            }
            Err(e) => report.push("PG连接", false, "critical", format!("{}: {e}", pg.dsn)),
        }
    }
}
