//! 服务目录管理写操作 + 组件状态，供 manager「新增服务向导」与「运营仪表盘」。
//!
//! * `GET  /admin/browse?path=`          — 目录浏览（只读列出，供向导选 data_root）。
//! * `POST /admin/services`              — 登记新服务（写 catalog XML + 热重载）。
//! * `POST /admin/pg-test`               — PostGIS 连接测试（不落盘）。
//! * `GET  /admin/components`            — 组件探测（PG/PROJ/缓存/服务数），供仪表盘。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use crate::admin::http::{authorize, json};
use crate::state::AppState;

/// 允许向导登记的服务类型（决定 catalog 子目录与文档形态）。
/// layer 类写 `<layer>` 文档（JSON 直转 XML）；3dtiles 写 `<TilesetService>`
/// 文档（元素式，一个数据集目录 = 一个服务），两者都落在 `<catalog.root>/<kind>/`。
const KINDS: &[&str] = &["cog", "vector", "pmtiles", "mbtiles", "postgis", "3dtiles", "arcgis"];

/// 允许「配置编辑」读写的 kind = 向导登记类（3dtiles 为 TilesetService XML，
/// 原样文本读写 + 通用 XML 语法校验，不依赖文档结构）。
const CFG_KINDS: &[&str] = KINDS;

/// `GET /admin/browse?path=<dir>` — 列出一个目录（一层）。
/// `path` 缺省为 `server.data_root`（未配置时退回 catalog 根）；配置了 data_root
/// 时任何 `path` 都要过前缀守卫。返回 `{ path, parent, root, catalog_root, entries }`。
pub async fn browse(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    RawQuery(qs): RawQuery,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let configured_root = default_browse_root(&state);
    let override_root = crate::admin::discover::query_data_root(qs.as_deref());
    let root = override_root
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| configured_root.clone());
    let requested = qs
        .as_deref()
        .and_then(|q| q.split('&').find_map(|p| {
            p.split_once('=')
                .filter(|(k, _)| k.trim().eq_ignore_ascii_case("path"))
                .map(|(_, v)| crate::admin::discover::decode_param(v.trim()))
        }))
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(s))
        .unwrap_or_else(|| root.clone());
    let requested = match crate::admin::discover::guard_data_root_with(
        &state,
        &requested,
        override_root.as_deref(),
    ) {
        Ok(p) => p,
        Err(r) => return r,
    };

    let _meta = match std::fs::metadata(&requested) {
        Ok(m) if m.is_dir() => m,
        Ok(_) => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("不是目录：{}", requested.display()) }),
            )
        }
        Err(e) => {
            return json(
                StatusCode::NOT_FOUND,
                json!({ "error": format!("目录不可访问：{}（{e}）", requested.display()) }),
            )
        }
    };

    let mut entries: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&requested) {
        for e in rd.flatten() {
            let p = e.path();
            let is_dir = p.is_dir();
            let m = e.metadata().ok();
            let modified_ms = m
                .as_ref()
                .and_then(|mm| mm.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            entries.push(json!({
                "name": e.file_name().to_string_lossy().to_string(),
                "is_dir": is_dir,
                "size": m.as_ref().map(|mm| mm.len()).unwrap_or(0),
                "modified_ms": modified_ms,
            }));
        }
    }
    entries.sort_by(|a, b| {
        // 目录在前，再按名称。
        match (a["is_dir"].as_bool(), b["is_dir"].as_bool()) {
            (Some(true), Some(false)) => std::cmp::Ordering::Less,
            (Some(false), Some(true)) => std::cmp::Ordering::Greater,
            _ => a["name"]
                .as_str()
                .cmp(&b["name"].as_str()),
        }
    });

    let parent = requested
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    json(
        StatusCode::OK,
        json!({
            "path": requested.display().to_string(),
            "parent": parent,
            "root": root.display().to_string(),
            "catalog_root": resolve_catalog_root(&state).display().to_string(),
            "entries": entries,
        }),
    )
}

/// `POST /admin/services` — 登记新服务：layer 类把请求体（Layer 文档 JSON）转成
/// XML，3dtiles 转成 `<TilesetService>` 元素文档，写入
/// `<catalog.root>/<kind>/<name>.xml` 后热重载。数据路径受 `data_root` 约束。
pub async fn create(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    RawQuery(qs): RawQuery,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let obj = match body.as_object() {
        Some(o) if !o.is_empty() => o,
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "请求体必须是非空 JSON 对象（Layer 文档）" }),
            )
        }
    };
    let name = match obj.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => sanitize_name(n),
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 name 字段" }),
            )
        }
    };
    if name.is_empty() {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": "非法的服务名（仅允许文件名，禁止路径分隔符）" }),
        );
    }
    let kind = match obj.get("kind").and_then(|v| v.as_str()) {
        Some(k) if KINDS.contains(&k) => k.to_string(),
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("kind 必须是 {} 之一", KINDS.join("/")) }),
            )
        }
    };
    // source.type 应与 kind 一致（宽容校验）。
    if let Some(src) = obj.get("source").and_then(|v| v.as_object()) {
        let src_type = src.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if !src_type.is_empty() && src_type != kind {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("source.type（{src_type}）与 kind（{kind}）不一致") }),
            );
        }
    }

    // data_root 约束（计划 §9.2）：登记的数据必须落在配置的数据根之内。
    // 对象存储 URI（s3://…）不是本地路径，交给 storage 层自身的白名单。
    let data_path = if kind == "3dtiles" {
        obj.get("root").and_then(|v| v.as_str()).unwrap_or_default()
    } else {
        obj.get("source")
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
    };
    if !data_path.is_empty() && !data_path.contains("://") {
        let override_root = crate::admin::discover::query_data_root(qs.as_deref());
        if let Err(r) = crate::admin::discover::guard_data_root_laxed_with(
            &state,
            Path::new(data_path),
            override_root.as_deref(),
        ) {
            return r;
        }
    }

    let dir = resolve_catalog_root(&state).join(&kind);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("创建目录失败：{}（{e}）", dir.display()) }),
        );
    }
    let file = dir.join(format!("{name}.xml"));
    // 同名重发布：先备份旧文件，写盘/热重载失败时恢复，避免把已生效的图层配置弄丢。
    let backup = dir.join(format!("{name}.xml.bak"));
    let had_old = file.exists();
    if had_old {
        if let Err(e) = std::fs::copy(&file, &backup) {
            return json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("备份旧配置失败：{}（{e}）", backup.display()) }),
            );
        }
    }
    let xml = if kind == "3dtiles" {
        match tileset_xml(&name, obj) {
            Ok(s) => s,
            Err(msg) => {
                if had_old {
                    let _ = std::fs::rename(&backup, &file);
                }
                return json(StatusCode::BAD_REQUEST, json!({ "error": msg }));
            }
        }
    } else {
        crate::xmlcfg::to_xml(&body, "layer")
    };
    if let Err(e) = std::fs::write(&file, xml.as_bytes()) {
        if had_old {
            let _ = std::fs::rename(&backup, &file);
        }
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("写盘失败：{}（{e}）", file.display()) }),
        );
    }

    // 热重载：重读配置 + 重扫目录 + 重建服务集。
    match state.refresh_catalog(true).await {
        Ok(report) => {
            // layer 类进 served，3dtiles 进 tilesets。
            let served = report.served.iter().any(|n| n == &name)
                || report.tilesets.iter().any(|n| n == &name);
            let skipped = report
                .skipped
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.reason.clone());
            let _ = std::fs::remove_file(&backup);
            json(
                StatusCode::OK,
                json!({
                    "msg": format!("服务已登记：{kind}/{name}"),
                    "name": name,
                    "kind": kind,
                    "served": served,
                    "skip_reason": skipped,
                    "report": report,
                }),
            )
        }
        Err(e) => {
            // 文件已写入但重载失败；恢复旧配置（无旧文件则删除新文件）以便重试。
            if had_old {
                let _ = std::fs::rename(&backup, &file);
            } else {
                let _ = std::fs::remove_file(&file);
            }
            json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("登记后重载失败：{e}") }),
            )
        }
    }
}

/// `POST /admin/pg-test` — PostGIS 连接测试（仅探测，不落盘、不建池）。
pub async fn pg_test(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let dsn = match body.get("dsn").and_then(|v| v.as_str()) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 dsn 字段（postgres://user:pass@host:port/db）" }),
            )
        }
    };
    match gisserver::pg_source::PgPool::new(&dsn, 1).await {
        Ok(pool) => match pool.get().await {
            Ok(client) => match client.query_opt("SELECT 1", &[]).await {
                Ok(_) => json(
                    StatusCode::OK,
                    json!({ "ok": true, "detail": "连接成功，SELECT 1 通过" }),
                ),
                Err(e) => json(
                    StatusCode::OK,
                    json!({ "ok": false, "detail": format!("连接可用但查询失败：{e}") }),
                ),
            },
            Err(e) => json(
                StatusCode::OK,
                json!({ "ok": false, "detail": format!("获取连接失败：{e}") }),
            ),
        },
        Err(e) => json(
            StatusCode::OK,
            json!({ "ok": false, "detail": format!("连接失败：{e}") }),
        ),
    }
}

/// `POST /admin/pg-query` — PostGIS 普通表数据查询（只读，临时池 size=1）。
///
/// body `{dsn, schema, table, limit?}`：对指定表执行 `SELECT *`（带 limit），
/// 返回 `{columns, rows}`，其中 rows 为二维数组（与 columns 对齐）。
/// 用于 manager 的「数据查询服务」：不发布为图层，仅返回表数据供前端展示。
pub async fn pg_query(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let dsn = match body.get("dsn").and_then(|v| v.as_str()) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 dsn 字段" }),
            )
        }
    };
    let schema = body
        .get("schema")
        .and_then(|v| v.as_str())
        .unwrap_or("public")
        .trim()
        .to_string();
    let table = match body.get("table").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            return json(
                StatusCode::BAD_REQUEST,
                json!({ "error": "缺少 table 字段" }),
            )
        }
    };
    let limit = body
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(100)
        .clamp(1, 1000);
    match gisserver::pg_source::PgPool::new(&dsn, 1).await {
        Ok(pool) => match pool.get().await {
            Ok(client) => {
                // 先用 LIMIT 0 准备语句取列名（空表也能拿到列结构），再按 limit 查数据。
                let col_sql = format!(
                    "SELECT * FROM \"{}\".\"{}\" LIMIT 0",
                    schema.replace('"', "\"\""),
                    table.replace('"', "\"\"")
                );
                let stmt = match client.prepare(&col_sql).await {
                    Ok(s) => s,
                    Err(e) => {
                        return json(
                            StatusCode::BAD_GATEWAY,
                            json!({ "error": format!("获取表结构失败：{e}") }),
                        );
                    }
                };
                let columns: Vec<String> = stmt.columns().iter().map(|c| c.name().to_string()).collect();
                let sql = format!(
                    "SELECT * FROM \"{}\".\"{}\" LIMIT {}",
                    schema.replace('"', "\"\""),
                    table.replace('"', "\"\""),
                    limit
                );
                match client.query(&sql, &[]).await {
                    Ok(rows) => {
                        let data: Vec<Vec<Value>> = rows
                            .iter()
                            .map(|r| {
                                (0..r.len())
                                    .map(|i| {
                                        // 按文本取列，避免类型反序列化失败；JSON 化交给前端。
                                        r.try_get::<usize, Option<String>>(i)
                                            .ok()
                                            .flatten()
                                            .map(Value::String)
                                            .unwrap_or(Value::Null)
                                    })
                                    .collect()
                            })
                            .collect();
                        json(
                            StatusCode::OK,
                            json!({
                                "schema": schema,
                                "table": table,
                                "columns": columns,
                                "rows": data,
                                "row_count": data.len(),
                                "limit": limit,
                            }),
                        )
                    }
                    Err(e) => json(
                        StatusCode::BAD_GATEWAY,
                        json!({ "error": format!("查询失败：{e}") }),
                    ),
                }
            }
            Err(e) => json(
                StatusCode::BAD_GATEWAY,
                json!({ "error": format!("获取连接失败：{e}") }),
            ),
        },
        Err(e) => json(
            StatusCode::BAD_GATEWAY,
            json!({ "error": format!("连接失败：{e}") }),
        ),
    }
}

/// `GET /admin/components` — 组件探测，供运营仪表盘组件卡。
pub async fn components(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let pg = state.pg_registry().is_some();
    let proj = gisserver::projection::proj().is_some();
    let catalog_root = resolve_catalog_root(&state);
    let catalog_exists = catalog_root.exists();
    let tilesets_dir = state.config.tilesets_dir();
    let tilesets_dir_exists = tilesets_dir.exists();
    let served_layers = state.layer_names().len();
    let tilesets = state.tileset_names().len();
    json(
        StatusCode::OK,
        json!({
            "map_api_up": true,
            "pg_enabled": pg,
            "proj_loaded": proj,
            "catalog_root": catalog_root.display().to_string(),
            "catalog_root_exists": catalog_exists,
            "tilesets_dir": tilesets_dir.display().to_string(),
            "tilesets_dir_exists": tilesets_dir_exists,
            "served_layers": served_layers,
            "tilesets": tilesets,
            "cache_mib": state.config.server.cache_mib,
            "cache_ttl_secs": state.config.server.cache_ttl_secs,
        }),
    )
}

/// `GET /admin/services/:kind/:name` — 返回该服务当前配置文件原文
/// （XML 文本，原样返回，保留注释与格式）。供 manager「编辑配置」加载。
pub async fn get_config(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    AxumPath((kind, name)): AxumPath<(String, String)>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    if !CFG_KINDS.contains(&kind.as_str()) {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("不支持的 kind：{kind}（仅 {kinds}）", kinds = CFG_KINDS.join("/")) }),
        );
    }
    let name = sanitize_name(&name);
    if name.is_empty() {
        return json(StatusCode::BAD_REQUEST, json!({ "error": "非法的服务名" }));
    }
    let root = resolve_catalog_root(&state);
    let file = resolve_service_file(&root, &kind, &name);
    match std::fs::read_to_string(&file) {
        Ok(text) => json(
            StatusCode::OK,
            json!({
                "kind": kind,
                "name": name,
                "path": file.display().to_string(),
                "xml": text,
            }),
        ),
        Err(e) => json(
            StatusCode::NOT_FOUND,
            json!({ "error": format!("读取配置失败：{}（{e}）", file.display()) }),
        ),
    }
}

/// `PUT /admin/services/:kind/:name` — 接收编辑后的 XML 文本，语法校验后回写
/// 并热重载；写盘前自动备份 `.bak`，重载失败回滚。供 manager「编辑配置」保存。
pub async fn update_config(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    AxumPath((kind, name)): AxumPath<(String, String)>,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    if !CFG_KINDS.contains(&kind.as_str()) {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("不支持的 kind：{kind}（仅 {kinds}）", kinds = CFG_KINDS.join("/")) }),
        );
    }
    let name = sanitize_name(&name);
    if name.is_empty() {
        return json(StatusCode::BAD_REQUEST, json!({ "error": "非法的服务名" }));
    }
    let xml = match body.get("xml").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => return json(StatusCode::BAD_REQUEST, json!({ "error": "缺少 xml 字段或为空" })),
    };
    // 语法校验：仅拦住解析错误；不强制字段结构（尊重现有各类配置自由度）。
    if let Err(e) = crate::xmlcfg::validate(&xml) {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("XML 语法错误：{e}") }),
        );
    }
    let root = resolve_catalog_root(&state);
    let file = resolve_service_file(&root, &kind, &name);
    if !file.exists() {
        return json(
            StatusCode::NOT_FOUND,
            json!({ "error": format!("配置文件不存在：{}", file.display()) }),
        );
    }
    // 写盘前备份，便于回滚。
    let bak = file.with_extension("xml.bak");
    let _ = std::fs::copy(&file, &bak);
    if let Err(e) = std::fs::write(&file, xml.as_bytes()) {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("写盘失败：{}（{e}）", file.display()) }),
        );
    }
    match state.refresh_catalog(true).await {
        Ok(report) => {
            // layer 类记在 served；3dtiles 记在 tilesets。
            let served =
                report.served.iter().any(|n| n == &name) || report.tilesets.iter().any(|n| n == &name);
            let skipped = report
                .skipped
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.reason.clone());
            json(
                StatusCode::OK,
                json!({
                    "msg": format!("已更新：{kind}/{name}"),
                    "served": served,
                    "skip_reason": skipped,
                    "report": report,
                }),
            )
        }
        Err(e) => {
            let _ = std::fs::copy(&bak, &file);
            json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("更新后重载失败，已回滚：{e}") }),
            )
        }
    }
}

/// `DELETE /admin/services/:kind/:name` — 硬删除一个服务：先备份配置到
/// `<name>.xml.bak`（可恢复），再移除配置文件，清理登记簿管理态
/// （避免以「deleted 回收站」形态残留），最后热重载。
///
/// 与 `state=deleted`（软删除 / 回收站，保留配置与数据、仅停止服务）区分：
/// 此接口是物理移除配置文件。
pub async fn delete_service(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    AxumPath((kind, name)): AxumPath<(String, String)>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    if !CFG_KINDS.contains(&kind.as_str()) {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("不支持的 kind：{kind}（仅 {kinds}）", kinds = CFG_KINDS.join("/")) }),
        );
    }
    let name = sanitize_name(&name);
    if name.is_empty() {
        return json(StatusCode::BAD_REQUEST, json!({ "error": "非法的服务名" }));
    }
    let root = resolve_catalog_root(&state);
    let file = resolve_service_file(&root, &kind, &name);
    if !file.exists() {
        return json(
            StatusCode::NOT_FOUND,
            json!({ "error": format!("配置文件不存在：{}", file.display()) }),
        );
    }
    // 备份到 .bak（先备份再删，便于误删恢复）。
    let bak = file.with_extension("xml.bak");
    if let Err(e) = std::fs::copy(&file, &bak) {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("备份失败：{}（{e}）", bak.display()) }),
        );
    }
    if let Err(e) = std::fs::remove_file(&file) {
        let _ = std::fs::remove_file(&bak);
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("删除失败：{}（{e}）", file.display()) }),
        );
    }
    // 清理登记簿管理态（回收站残留）。失败仅告警，不阻断（文件已删）。
    if let Err(e) = state.admin.registry.remove(&name).await {
        tracing::warn!("delete {kind}/{name}: 清理登记簿失败（忽略）：{e}");
    }
    match state.refresh_catalog(true).await {
        Ok(report) => json(
            StatusCode::OK,
            json!({
                "msg": format!("已删除：{kind}/{name}（备份：{}）", bak.display()),
                "backup": bak.display().to_string(),
                "report": report,
            }),
        ),
        Err(e) => {
            // 重载失败：尝试恢复文件。
            let _ = std::fs::copy(&bak, &file);
            json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("删除后重载失败，已回滚：{e}") }),
            )
        }
    }
}

/// `POST /admin/services/:kind/:name/rename` — 重命名一个服务：body
/// `{"name": "newname"}`。重命名配置文件（`<name>.xml` → `<newname>.xml`）
/// 并同步更新文件内部的服务名标识（layer 类的 `<layer name>` 属性 /
/// 3dtiles 的 `<Name>` 元素），再把登记簿管理态（如有非默认态）更名，最后热重载。
pub async fn rename_service(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    AxumPath((kind, name)): AxumPath<(String, String)>,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    if !CFG_KINDS.contains(&kind.as_str()) {
        return json(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("不支持的 kind：{kind}（仅 {kinds}）", kinds = CFG_KINDS.join("/")) }),
        );
    }
    let name = sanitize_name(&name);
    if name.is_empty() {
        return json(StatusCode::BAD_REQUEST, json!({ "error": "非法的服务名" }));
    }
    let newname = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.is_empty() => sanitize_name(n),
        _ => return json(StatusCode::BAD_REQUEST, json!({ "error": "缺少 name 字段或为空" })),
    };
    if newname == name {
        return json(StatusCode::BAD_REQUEST, json!({ "error": "新名与原名相同" }));
    }
    let root = resolve_catalog_root(&state);
    let old_file = resolve_service_file(&root, &kind, &name);
    let new_file = resolve_service_file(&root, &kind, &newname);
    if !old_file.exists() {
        return json(
            StatusCode::NOT_FOUND,
            json!({ "error": format!("配置文件不存在：{}", old_file.display()) }),
        );
    }
    if new_file.exists() {
        return json(
            StatusCode::CONFLICT,
            json!({ "error": format!("目标名已存在：{}", new_file.display()) }),
        );
    }
    // 读原文，更新内部服务名标识（保持其余内容与格式不变）。
    let text = match std::fs::read_to_string(&old_file) {
        Ok(t) => t,
        Err(e) => {
            return json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("读取失败：{}（{e}）", old_file.display()) }),
            )
        }
    };
    let new_text = match rewrite_config_name(&text, &kind, &newname) {
        Some(t) => t,
        None => {
            return json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "无法解析配置文件，无法更新内部服务名" }),
            )
        }
    };
    // 先写新文件（内部名已更新），再删旧文件；任一步失败回滚。
    if let Err(e) = std::fs::write(&new_file, new_text.as_bytes()) {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("写新文件失败：{}（{e}）", new_file.display()) }),
        );
    }
    if let Err(e) = std::fs::remove_file(&old_file) {
        let _ = std::fs::remove_file(&new_file);
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("删除旧文件失败：{}（已回滚，{e}）", old_file.display()) }),
        );
    }
    // 登记簿管理态更名（如有非默认态）。
    if let Err(e) = state.admin.registry.rename(&name, &newname).await {
        tracing::warn!("rename {kind}/{name}->{newname}: 登记簿更名失败（忽略）：{e}");
    }
    match state.refresh_catalog(true).await {
        Ok(report) => json(
            StatusCode::OK,
            json!({
                "msg": format!("已重命名：{kind}/{name} → {newname}"),
                "kind": kind,
                "name": newname,
                "served": report.served.iter().any(|n| n == &newname)
                    || report.tilesets.iter().any(|n| n == &newname),
                "report": report,
            }),
        ),
        Err(e) => {
            // 回滚：删新、恢复旧、登记簿回滚。
            let _ = std::fs::remove_file(&new_file);
            let _ = std::fs::write(&old_file, text.as_bytes());
            if let Err(e2) = state.admin.registry.rename(&newname, &name).await {
                tracing::warn!("rename 回滚登记簿失败（忽略）：{e2}");
            }
            json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("重命名后重载失败，已回滚：{e}") }),
            )
        }
    }
}

/// 渲染 `<TilesetService>` 文档。登记簿按**子元素文本**读取（`collect_elements`
/// 忽略属性），所以这里必须写元素式 XML，不能走 `to_xml` 的属性式约定。
fn tileset_xml(name: &str, obj: &serde_json::Map<String, Value>) -> Result<String, String> {
    let field = |k: &str| -> String {
        obj.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let root = field("root");
    if root.is_empty() {
        return Err("3dtiles 登记缺少 root 字段（3D Tiles 数据集根目录）".to_string());
    }
    let entry = {
        let e = field("entry");
        if e.is_empty() {
            "tileset.json".to_string()
        } else {
            e
        }
    };
    if entry.contains("..") {
        return Err("非法的 entry（入口文件名不允许 ..）".to_string());
    }
    let title = {
        let t = field("title");
        if t.is_empty() { name.to_string() } else { t }
    };
    let esc = |s: &str| crate::xmlcfg::escape(s);
    let doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TilesetService>\n  <Name>{}</Name>\n  <Title>{}</Title>\n  <Root>{}</Root>\n  <Entry>{}</Entry>\n</TilesetService>\n",
        esc(name),
        esc(&title),
        esc(&root),
        esc(&entry)
    );
    Ok(doc)
}

/// 解析服务配置文件路径：`<root>/<kind>/<name>.xml`。
fn resolve_service_file(root: &Path, kind: &str, name: &str) -> PathBuf {
    root.join(kind).join(format!("{name}.xml"))
}

/// catalog 根目录（相对进程工作目录解析为绝对路径）。
fn resolve_catalog_root(state: &AppState) -> PathBuf {
    let root = &state.config.catalog.root;
    let cwd = std::env::current_dir().unwrap_or_default();
    cwd.join(root)
}

/// `GET /admin/browse` 的缺省起点：配了 `server.data_root` 就用它（数据面只
/// 应在 data_root 内活动），否则退回 catalog 根目录（向导仍能选图层配置目录）。
fn default_browse_root(state: &AppState) -> PathBuf {
    match state
        .config
        .server
        .data_root
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        Some(r) => Path::new(r)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(r)),
        None => resolve_catalog_root(state),
    }
}

/// 仅保留文件名安全字符（去分隔符），防目录穿越。
fn sanitize_name(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .chars()
        .filter(|c| *c != '/' && *c != '\\' && *c != ':')
        .collect()
}

/// 重命名时同步更新配置文件内部的服务名标识，保持文件其余内容与格式不变。
///
/// * layer 类（cog/vector/pmtiles/mbtiles/postgis）：根元素 `<layer>` 的
///   `name` 属性（服务名即 `<layer name>`）。
/// * 3dtiles：`<TilesetService>` 内的 `<Name>` 元素（服务名由 `<Name>` 决定，
///   缺省取文件名 stem）。
///
/// 返回 `None` 表示无法定位服务名标识（文件解析失败），调用方据此报错。
fn rewrite_config_name(text: &str, kind: &str, newname: &str) -> Option<String> {
    if kind == "3dtiles" {
        rewrite_tileset_name(text, newname)
    } else {
        rewrite_layer_name_attr(text, newname)
    }
}

/// 在根元素的开标签内替换 / 插入 `name="..."` 属性（layer 类服务）。
/// 保持文件其余内容 / 格式不变（注释、缩进、兄弟元素均不动）。
fn rewrite_layer_name_attr(text: &str, newname: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        if bytes[i] == b'<' {
            let rest = &text[i..];
            if rest.starts_with("<?") || rest.starts_with("<!") {
                match rest.find('>') {
                    Some(p) => {
                        i = i + p + 1;
                        continue;
                    }
                    None => return None,
                }
            }
            let nc = bytes.get(i + 1).copied()?;
            if !(nc.is_ascii_alphabetic() || nc == b'_') {
                i += 1;
                continue;
            }
            // 开标签结束 '>'。
            let gt = text[i..].find('>').map(|p| i + p)?;
            let open = &text[i..gt]; // 不含 '>'
            let newval = newname.replace('"', "&quot;");

            // 1) 跳过标签名（开标签第一个 token），定位到属性区起点。
            let mut p = 1usize; // 跳过 '<'
            while p < open.len()
                && (open.as_bytes()[p].is_ascii_alphanumeric()
                    || open.as_bytes()[p] == b'_'
                    || open.as_bytes()[p] == b':')
            {
                p += 1;
            }
            // 2) 在属性区逐个扫描，定位 `name=` 属性（带引号的值）。
            let mut at: Option<(usize, usize)> = None;
            while p < open.len() {
                // 跳过属性间空白。
                while p < open.len()
                    && (open.as_bytes()[p] == b' '
                        || open.as_bytes()[p] == b'\t'
                        || open.as_bytes()[p] == b'\n'
                        || open.as_bytes()[p] == b'\r')
                {
                    p += 1;
                }
                if p >= open.len() {
                    break;
                }
                // 读属性名。
                let start = p;
                while p < open.len()
                    && (open.as_bytes()[p].is_ascii_alphanumeric()
                        || open.as_bytes()[p] == b'_'
                        || open.as_bytes()[p] == b':')
                {
                    p += 1;
                }
                let aname = &open[start..p];
                if aname.is_empty() {
                    break;
                }
                // 跳过空白再读 '='。
                while p < open.len()
                    && (open.as_bytes()[p] == b' '
                        || open.as_bytes()[p] == b'\t'
                        || open.as_bytes()[p] == b'\n'
                        || open.as_bytes()[p] == b'\r')
                {
                    p += 1;
                }
                if p < open.len() && open.as_bytes()[p] == b'=' {
                    p += 1;
                    while p < open.len()
                        && (open.as_bytes()[p] == b' '
                            || open.as_bytes()[p] == b'\t')
                    {
                        p += 1;
                    }
                    if p < open.len()
                        && (open.as_bytes()[p] == b'"' || open.as_bytes()[p] == b'\'')
                    {
                        let q = open.as_bytes()[p];
                        let vstart = p + 1;
                        let vend = open[vstart..].find(char::from(q)).map(|x| vstart + x)?;
                        if aname == "name" {
                            at = Some((vstart, vend));
                            break;
                        }
                        p = vend + 1;
                    } else {
                        break;
                    }
                } else {
                    break; // 属性名后无 '='，到达开标签末尾
                }
            }
            let new_open = if let Some((vstart, vend)) = at {
                format!("{}{}{}", &open[..vstart], newval, &open[vend..])
            } else {
                // 无 name 属性：在标签名后插入。
                let tag_end = open[1..]
                    .find(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r')
                    .map(|x| 1 + x)
                    .unwrap_or(open.len());
                format!("{} name=\"{}\"{}", &open[..tag_end], newval, &open[tag_end..])
            };
            return Some(format!("{}{}>{}", &text[..i], new_open, &text[gt + 1..]));
        }
        i += 1;
    }
    None
}

/// 在 3D Tiles 服务配置中重命名 `<Name>文本</Name>` 元素（大小写不敏感）。
/// 保持文件其余内容与格式不变。
fn rewrite_tileset_name(text: &str, newname: &str) -> Option<String> {
    // 定位 `<Name ...>` 开标签（大小写不敏感），避开 `<namespace>` 之类。
    let lower = text.to_lowercase();
    let open_tag = {
        let mut found = None;
        let mut from = 0usize;
        while let Some(p) = lower[from..].find("<name") {
            let abs = from + p;
            let after = lower[abs + 5..].chars().next();
            if matches!(after, Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r')) {
                found = Some(abs);
                break;
            }
            from = abs + 5;
        }
        found?
    };
    let gt = text[open_tag..].find('>').map(|p| open_tag + p)?;
    let content_start = gt + 1;
    let close_rel = lower[content_start..].find("</name>")?;
    let content_end = content_start + close_rel;
    Some(format!(
        "{}{}{}",
        &text[..content_start],
        newname,
        &text[content_end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_layer_name_replaces_attribute() {
        let src = r#"<layer name="old" title="t" src_crs="EPSG:4326"><source type="cog" path="a.tif"/></layer>"#;
        let out = rewrite_config_name(src, "cog", "new").unwrap();
        assert!(out.contains(r#"name="new""#), "应更新 name 属性：{out}");
        assert!(!out.contains(r#"name="old""#), "不应残留旧名：{out}");
        // 其余内容保持不变
        assert!(out.contains(r#"title="t""#));
        assert!(out.contains("a.tif"));
        assert!(out.contains("EPSG:4326"));
    }

    #[test]
    fn rewrite_layer_name_inserts_when_missing() {
        let src = r#"<layer title="t"><source type="cog" path="a.tif"/></layer>"#;
        let out = rewrite_config_name(src, "cog", "new").unwrap();
        assert!(out.contains(r#"name="new""#), "应插入 name 属性：{out}");
        assert!(out.contains(r#"title="t""#));
    }

    #[test]
    fn rewrite_layer_name_multiline_opening() {
        let src = "<layer\n  name=\"old\"\n  title=\"t\">\n  <source type=\"cog\" path=\"a.tif\"/>\n</layer>";
        let out = rewrite_config_name(src, "cog", "new").unwrap();
        assert!(out.contains(r#"name="new""#));
        assert!(!out.contains(r#"name="old""#));
    }

    #[test]
    fn rewrite_tileset_name_replaces_element() {
        let src = "<TilesetService>\n  <Name>old</Name>\n  <Root>D:\\x</Root>\n</TilesetService>";
        let out = rewrite_config_name(src, "3dtiles", "new").unwrap();
        assert!(out.contains("<Name>new</Name>"), "应更新 <Name>：{out}");
        assert!(!out.contains("<Name>old</Name>"));
        assert!(out.contains(r#"<Root>D:\x</Root>"#));
    }

    #[test]
    fn tileset_xml_roundtrips_through_registry_parser() {
        let body = json!({
            "kind": "3dtiles",
            "title": "园区 & <倾斜> 摄影",
            "root": r"D:\data\3dtiles\kmczc"
        });
        let xml = tileset_xml("kmczc", body.as_object().unwrap()).unwrap();
        let svc = crate::tilesets::parse_tileset_xml(&xml, Path::new("kmczc.xml")).unwrap();
        assert_eq!(svc.name, "kmczc");
        assert_eq!(svc.title, "园区 & <倾斜> 摄影", "实体转义必须能还原");
        assert_eq!(svc.root, PathBuf::from(r"D:\data\3dtiles\kmczc"));
        assert_eq!(svc.entry, "tileset.json", "entry 缺省要落进文档");

        // 缺 root / entry 穿越：都在写盘前拒绝。
        assert!(tileset_xml("a", json!({}).as_object().unwrap()).is_err());
        let bad = json!({ "root": "D:\\d\\x", "entry": "../../etc/passwd" });
        assert!(tileset_xml("a", bad.as_object().unwrap()).is_err());
    }

    #[test]
    fn rewrite_config_name_rejects_garbage() {
        // 无根元素的情况返回 None，调用方据此报错而非静默产出错误文件。
        assert!(rewrite_config_name("not xml at all <<<", "cog", "new").is_none());
    }
}
