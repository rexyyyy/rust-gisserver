//! Data catalog: Workspace → Store → Resource → published service.
//!
//! Boundaries (plan §1 red lines, followed strictly):
//! * Only manages **service publish & organize metadata**; all metadata goes to rust-gisserver PostgreSQL.
//! * Never reads data files or connects to business DB; data access goes through map-api's read-only discovery
//!   (`/admin/capabilities`, `/admin/discover`, `/admin/pg-tables`, `/admin/pg-test`).
//! * Publish output is rust-gisserver `Layer` / `TilesetService` doc, sent via
//!   `POST /admin/services` to write and hot-reload; rust-gisserver does not write XML, generate tiles, or import features.
//! * Soft-delete is marking only: Resource `del_flag=1`, published layer `state='deleted'` (trash, recoverable).
//!
//! Engine convention: catalog ops (discover / connection test / publish) all go through the **default engine** for now,
//! ([`crate::handlers::engine::default_engine`]); service-level read/write (config, start/stop, health,
//! ownership change) still goes through `/map/services/*` routed by ownership; both paths don't interfere.
//!
//! Capability matrix (`data_type` → `kind` + publishable service types) is fetched from map-api `/admin/capabilities`;
//! rust-gisserver does not maintain a second copy, to avoid drift between the two.
use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::handlers::{
    engine,
    mapsvc::{upstream_json, upstream_json_engine},
};
use crate::resp::Resp;
use crate::state::{conn, AppState};

/// PostGIS Store: not a directory, located by `pg_dsn` + `pg_schema` (map-api's
/// `store_types` only lists dir types, we add this one locally).
const STORE_TYPE_POSTGIS: &str = "postgis";

/// Catalog routes (assembled by `handlers::router`). Static paths first, `:id` wildcard after.
/// `GET /map/catalog/crs-tree` — CRS selection tree (WGS84 + CGCS2000),
/// used by the publish wizard to guide correct CRS selection when 'missing / suspicious'.
pub async fn crs_tree() -> AppResult<Json<Resp<Value>>> {
    let gk_zones = [33u32, 34, 35]; // 昆明及周边 3°带
    let gk: Vec<Value> = gk_zones
        .iter()
        .map(|z| {
            json!({
                "name": format!("{} 带（CM {}°E，昆明周边）", z, z * 3),
                "code": format!("EPSG:{}", 4509 + z),
            })
        })
        .collect();
    Ok(Resp::ok(json!({
        "tree": [
            {
                "name": "WGS84",
                "children": [
                    {"name": "地理坐标（WGS84 经纬度）", "code": "EPSG:4326"},
                    {"name": "Web Mercator（Web 默认投影）", "code": "EPSG:3857"}
                ]
            },
            {
                "name": "CGCS2000（2000 国家大地坐标系）",
                "children": [
                    {"name": "地理坐标（CGCS2000 经纬度）", "code": "EPSG:4490"},
                    {"name": "高斯克吕格 3°带", "children": gk}
                ]
            }
        ]
    })))
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Capability matrix (publish wizard data dictionary)
        .route("/map/catalog/capabilities", get(capabilities))
        // Workspace
        .route("/map/catalog/workspaces", get(ws_list).post(ws_add))
        .route(
            "/map/catalog/workspaces/{id}",
            put(ws_update).delete(ws_remove),
        )
        // Browse workspace data root (left tree in new data source, scoped to workspace)
        .route("/map/catalog/dirs", get(dir_list))
        // Publish wizard 'select tile config XML': parse server-side ArcGIS Conf.xml → grid params
        .route("/map/catalog/tile-grid", post(tile_grid_parse))
        // Data store
        .route("/map/catalog/stores", get(store_list).post(store_add))
        // 'Test connection' for new data source: no existing store id needed; probe directly with form params.
        .route("/map/catalog/stores/connect", post(store_connect_preflight))
        .route(
            "/map/catalog/stores/{id}/connect",
            post(store_connect),
        )
        .route(
            "/map/catalog/stores/{id}/discover",
            post(store_discover),
        )
        // PostGIS data source: import a workspace Shapefile into the DB.
        .route(
            "/map/catalog/stores/{id}/import-shp",
            post(store_import_shp),
        )
        // SSE version of the same import: front-end keeps receiving progress for large data, avoids long requests looking frozen
        .route(
            "/map/catalog/stores/{id}/import-shp/stream",
            post(store_import_shp_stream),
        )
        .route(
            "/map/catalog/stores/{id}/restore",
            post(store_restore),
        )
        .route(
            "/map/catalog/stores/{id}",
            put(store_update).delete(store_remove),
        )
        // Data layer (Resource)
        .route("/map/catalog/resources", get(resource_list))
        .route(
            "/map/catalog/resources/{id}/restore",
            post(resource_restore),
        )
        .route(
            "/map/catalog/resources/{id}/refresh",
            post(resource_refresh),
        )
        // PostGIS plain-table data query service (no layer publish, returns table data directly).
        .route(
            "/map/catalog/resources/{id}/query",
            get(resource_query),
        )
        .route("/map/catalog/resources/{id}", delete(resource_remove))
        // Publish
        .route("/map/catalog/publish", post(publish))
        .route("/map/catalog/published", get(published_list))
        .route(
            "/map/catalog/published/{id}/state",
            put(published_state),
        )
        .route(
            "/map/catalog/published/{id}/endpoints",
            get(published_endpoints),
        )
        .route("/map/catalog/published/{id}", put(published_update))
        // CRS selection tree (publish wizard CRS dict, covers common WGS84/CGCS2000 systems)
        .route("/map/catalog/crs-tree", get(crs_tree))
}

// =====================================================================
// Capability matrix (from map-api, single source of truth)
// =====================================================================

/// Publish definition for a single `data_type`.
#[derive(Debug, Clone)]
struct CapRow {
    kind: String,
    service_types: Vec<String>,
    defaults: Vec<String>,
    publishable: bool,
    note: String,
}

/// Local view of `/admin/capabilities`.
struct Caps {
    by_type: HashMap<String, CapRow>,
    store_types: Vec<String>,
}

impl Caps {
    fn is_store_type(&self, t: &str) -> bool {
        t == STORE_TYPE_POSTGIS || self.store_types.iter().any(|s| s == t)
    }

    fn get(&self, data_type: &str) -> AppResult<CapRow> {
        self.by_type
            .get(data_type)
            .cloned()
            .ok_or_else(|| AppError::Msg(format!("引擎未知数据类型：{data_type}")))?
            .check_publishable()
    }
}

impl CapRow {
    fn check_publishable(self) -> AppResult<CapRow> {
        if self.publishable {
            Ok(self)
        } else {
            Err(AppError::Msg(format!(
                "该数据类型当前不可发布：{}",
                if self.note.is_empty() {
                    "引擎无对应数据源类型"
                } else {
                    &self.note
                }
            )))
        }
    }

    /// Validate picked service types: empty → use defaults; out-of-range → reject with options listed.
    fn resolve_service_types(&self, picked: &[String]) -> AppResult<Vec<String>> {
        let list: Vec<String> = if picked.is_empty() {
            self.defaults.clone()
        } else {
            picked
                .iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        };
        if list.is_empty() {
            return Err(AppError::Param(format!(
                "至少勾选一种服务类型（可选：{}）",
                self.service_types.join("/")
            )));
        }
        if let Some(bad) = list.iter().find(|s| !self.service_types.contains(s)) {
            return Err(AppError::Param(format!(
                "服务类型 {bad} 不在该数据类型支持范围内（可选：{}）",
                self.service_types.join("/")
            )));
        }
        Ok(list)
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn arr_field(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(|x| x.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch and parse `/admin/capabilities`.
async fn load_caps(state: &AppState) -> AppResult<Caps> {
    let data = upstream_json(
        state,
        reqwest::Method::GET,
        "/admin/capabilities",
        None,
        None,
    )
    .await?;
    let mut by_type = HashMap::new();
    if let Some(rows) = data.get("data_types").and_then(|v| v.as_array()) {
        for r in rows {
            by_type.insert(
                str_field(r, "data_type"),
                CapRow {
                    kind: str_field(r, "mapserver_kind"),
                    service_types: arr_field(r, "service_types"),
                    defaults: arr_field(r, "default_service_types"),
                    publishable: r
                        .get("publishable")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    note: str_field(r, "note"),
                },
            );
        }
    }
    let store_types = data
        .get("store_types")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|s| str_field(s, "store_type"))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(Caps { by_type, store_types })
}

/// `GET /map/catalog/capabilities` — passthrough engine capability matrix for publish wizard.
pub async fn capabilities(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Resp<Value>>> {
    let data =
        upstream_json(&state, reqwest::Method::GET, "/admin/capabilities", None, None).await?;
    Ok(Resp::ok(data))
}

// =====================================================================
// Workspace
// =====================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBody {
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub engine_id: String,
    #[serde(default)]
    pub data_root: String,
    #[serde(default)]
    pub remark: String,
}

/// Normalize user-entered path: `\` → `/`, append trailing `/`, keep empty as empty.
fn normalize_data_root(raw: &str) -> String {
    let s = raw.trim().replace('\\', "/");
    if s.is_empty() {
        return String::new();
    }
    if s.ends_with('/') {
        s
    } else {
        format!("{s}/")
    }
}

/// Standard subdirs under workspace data root (auto-created before bind / modify save).
const WORKSPACE_STD_SUBDIRS: [&str; 14] = [
    "3dtiles", "arcgis_cache", "arcgis_tpk", "cog", "docs", "geojson", "mbtiles", "pmtiles",
    "postgis", "prj", "recon", "shapefile", "terrain", "xml-grid",
];

/// Ensure standard subdirs are in place before save, **only when the data root is
/// visible to the manager process itself**.
///
/// 数据目录由 map-api 独占挂载（容器里 manager 看不到 `/app/data`），数据操作
/// 一律走 map-api 接口；因此目录不可见时直接跳过，不做存在性校验。
/// 原生部署（map-api 与 manager 同机、路径可见）时仍会补齐标准子目录。
fn ensure_std_subdirs(data_root: &str) -> AppResult<()> {
    if data_root.is_empty() {
        return Ok(());
    }
    let root = std::path::Path::new(data_root);
    if !root.is_dir() {
        return Ok(());
    }
    for name in WORKSPACE_STD_SUBDIRS {
        let dir = root.join(name);
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir).map_err(|e| {
                AppError::Param(format!("创建标准子目录失败 {data_root}{name}：{e}"))
            })?;
        }
    }
    Ok(())
}

/// Workspace name = namespace ID; must be a simple identifier (used by tree nodes and URLs).
fn validate_ident(name: &str, label: &str, max: usize) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(AppError::Param(format!("{label}不能为空")));
    }
    if n.len() > max {
        return Err(AppError::Param(format!("{label}长度上限 {max} 个字符")));
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(AppError::Param(format!(
            "{label}仅允许字母、数字、下划线与短横线：{n}"
        )));
    }
    Ok(n.to_string())
}

async fn ws_rows(state: &AppState, include_deleted: bool) -> AppResult<Vec<Value>> {
    let sql = format!(
        "SELECT w.id, w.name, w.title, coalesce(w.engine_id,''), coalesce(w.data_root,''), w.del_flag, coalesce(w.remark,''), \
                (SELECT count(*) FROM sys_store s WHERE s.workspace_id = w.id AND s.del_flag = 0), \
                (SELECT count(*) FROM sys_published_layer p \
                 WHERE p.workspace_id = w.id AND p.del_flag = 0 AND p.state <> 'deleted') \
         FROM sys_workspace w {} ORDER BY w.id",
        if include_deleted { "" } else { "WHERE w.del_flag = 0" }
    );
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[]).await?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, i64>(0),
                "name": r.get::<_, String>(1),
                "title": r.get::<_, String>(2),
                "engineId": r.get::<_, String>(3),
                "dataRoot": r.get::<_, String>(4),
                "delFlag": r.get::<_, i16>(5),
                "remark": r.get::<_, String>(6),
                "storeCount": r.get::<_, i64>(7),
                "publishedCount": r.get::<_, i64>(8),
            })
        })
        .collect())
}

/// `GET /map/catalog/workspaces?includeDeleted=1`
pub async fn ws_list(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let include = query_flag(qs.as_deref(), "includeDeleted");
    let items = ws_rows(&state, include).await?;
    Ok(Resp::ok(json!({ "workspaces": items, "total": items.len() })))
}

/// Lexically check `path` is inside `root` (no filesystem access).
///
/// manager 容器不再挂载数据卷，无法 canonicalize 数据路径；这里只做词法前缀
/// 校验（拒绝 `..` 上跳）防路径穿越，真实存在性由 map-api 侧守卫。
fn lexical_inside(path: &str, root: &str) -> bool {
    fn parts(s: &str) -> Vec<&str> {
        s.split(['/', '\\'])
            .filter(|c| !c.is_empty() && *c != ".")
            .collect()
    }
    let p = parts(path);
    let r = parts(root);
    if p.iter().any(|c| *c == "..") || r.iter().any(|c| *c == "..") {
        return false;
    }
    if p.len() < r.len() {
        return false;
    }
    let eq = |a: &str, b: &str| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    p.iter().zip(r.iter()).all(|(a, b)| eq(a, b))
}

/// `GET /map/catalog/dirs?workspaceId=&path=` — List subdirs (one level) under workspace data root.
///
/// Used by the left-side tree in 'new data source'. 目录读取属于数据操作，
/// 统一代理 map-api `/admin/browse` 执行（manager 不再挂载数据卷）；
/// 这里只校验 `path` 词法上落在工作空间 `data_root` 内，防路径穿越。
pub async fn dir_list(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let workspace_id = query_i64(qs.as_deref(), "workspaceId")
        .ok_or_else(|| AppError::Param("缺少 workspaceId".to_string()))?;
    let c = conn(&state.pool).await?;
    let row = c
        .query_opt(
            "SELECT coalesce(data_root,'') FROM sys_workspace WHERE id = $1 AND del_flag = 0",
            &[&workspace_id],
        )
        .await?;
    let data_root = row
        .ok_or_else(|| AppError::Param(format!("工作空间不存在：{workspace_id}")))?
        .get::<_, String>(0);
    let data_root = data_root.trim().to_string();
    if data_root.is_empty() {
        return Err(AppError::Param("该工作空间未配置数据根目录".to_string()));
    }
    drop(c);

    let requested_raw = query_pairs(qs.as_deref())
        .into_iter()
        .find(|(k, _)| k == "path")
        .map(|(_, v)| v)
        .unwrap_or_default();
    let requested_raw = requested_raw.trim().to_string();
    let requested = if requested_raw.is_empty() {
        data_root.clone()
    } else {
        requested_raw
    };
    if !lexical_inside(&requested, &data_root) {
        return Err(AppError::Param(format!(
            "目录超出工作空间数据根目录：{}（根：{}）",
            requested, data_root
        )));
    }

    let q = format!("path={}", qs_encode(&requested));
    let data = upstream_json(&state, reqwest::Method::GET, "/admin/browse", Some(&q), None).await?;
    Ok(Resp::ok(data))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileGridBody {
    #[serde(default)]
    pub workspace_id: i64,
    #[serde(default)]
    pub path: String,
}

/// `POST /map/catalog/tile-grid` — Parse server-side ArcGIS `Conf.xml`, return the grid params the publish
/// wizard needs (CRS / origin / tile size / LOD levels). Engine reuses `arcgis-cache`
/// parsing logic (incl. DPI, tile alignment).
pub async fn tile_grid_parse(
    State(state): State<Arc<AppState>>,
    Json(form): Json<TileGridBody>,
) -> AppResult<Json<Resp<Value>>> {
    if form.workspace_id <= 0 {
        return Err(AppError::Param("缺少 workspaceId".to_string()));
    }
    let raw = form.path.trim();
    if raw.is_empty() {
        return Err(AppError::Param("请选择或填写 Conf.xml 路径".to_string()));
    }
    let c = conn(&state.pool).await?;
    let row = c
        .query_opt(
            "SELECT coalesce(data_root,'') FROM sys_workspace WHERE id = $1 AND del_flag = 0",
            &[&form.workspace_id],
        )
        .await?;
    let data_root = row
        .ok_or_else(|| AppError::Param(format!("工作空间不存在：{}", form.workspace_id)))?
        .get::<_, String>(0);
    let data_root = data_root.trim().to_string();
    if data_root.is_empty() {
        return Err(AppError::Param("该工作空间未配置数据根目录".to_string()));
    }
    if !lexical_inside(raw, &data_root) {
        return Err(AppError::Param(format!(
            "路径超出工作空间数据根目录：{}（根：{}）",
            raw, data_root
        )));
    }
    // Conf.xml 才解析网格；目录交给 map-api 自行判断。
    if let Some(ext) = std::path::Path::new(raw).extension().and_then(|e| e.to_str()) {
        if !ext.eq_ignore_ascii_case("xml") {
            return Err(AppError::Param(
                "请选择 Conf.xml 文件，或该文件所在的缓存目录".to_string(),
            ));
        }
    }
    drop(c);

    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/tile-grid",
        None,
        Some(json!({ "path": raw })),
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `POST /map/catalog/workspaces`
pub async fn ws_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspaceBody>,
) -> AppResult<Json<Resp<Value>>> {
    let name = validate_ident(&body.name, "工作区名", 64)?;
    let c = conn(&state.pool).await?;
    let n: i64 = c
        .query(
            "SELECT count(*) FROM sys_workspace WHERE name = $1 AND del_flag = 0",
            &[&name],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if n > 0 {
        return Err(AppError::Msg(format!("工作区名已存在：{name}")));
    }
    let data_root = normalize_data_root(&body.data_root);
    ensure_std_subdirs(&data_root)?;
    let id: i64 = c
        .query_opt(
            "INSERT INTO sys_workspace (name, title, engine_id, data_root, remark) VALUES ($1,$2,$3,$4,$5) RETURNING id",
            &[&name, &body.title.trim(), &body.engine_id.trim(), &data_root, &body.remark.trim()],
        )
        .await?
        .and_then(|r| r.get(0))
        .unwrap_or(0);
    Ok(Resp::ok(json!({ "id": id, "name": name })))
}

/// `PUT /map/catalog/workspaces/:id`
pub async fn ws_update(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Json(body): Json<WorkspaceBody>,
) -> AppResult<Json<Resp<Value>>> {
    let name = validate_ident(&body.name, "工作区名", 64)?;
    let c = conn(&state.pool).await?;
    let dup: i64 = c
        .query(
            "SELECT count(*) FROM sys_workspace WHERE name = $1 AND del_flag = 0 AND id <> $2",
            &[&name, &id],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if dup > 0 {
        return Err(AppError::Msg(format!("工作区名已存在：{name}")));
    }
    let data_root = normalize_data_root(&body.data_root);
    ensure_std_subdirs(&data_root)?;
    let n = c
        .execute(
            "UPDATE sys_workspace SET name=$2, title=$3, engine_id=$4, data_root=$5, remark=$6, update_time=now() \
             WHERE id=$1 AND del_flag=0",
            &[&id, &name, &body.title.trim(), &body.engine_id.trim(), &data_root, &body.remark.trim()],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("工作区不存在：{id}")));
    }
    Ok(Resp::ok(json!({ "id": id, "name": name })))
}

/// `DELETE /map/catalog/workspaces/:id` — Soft-delete; reject if Stores or published services still exist,
/// to avoid accidentally moving the entire namespace into trash with one click.
pub async fn ws_remove(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let stores: i64 = c
        .query(
            "SELECT count(*) FROM sys_store WHERE workspace_id = $1 AND del_flag = 0",
            &[&id],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if stores > 0 {
        return Err(AppError::Msg(format!(
            "工作区下仍有 {stores} 个数据存储，请先删除它们"
        )));
    }
    let live: i64 = c
        .query(
            "SELECT count(*) FROM sys_published_layer \
             WHERE workspace_id = $1 AND del_flag = 0 AND state <> 'deleted'",
            &[&id],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if live > 0 {
        return Err(AppError::Msg(format!(
            "工作区下仍有 {live} 个在线服务，请先停止或删除"
        )));
    }
    let n = c
        .execute(
            "UPDATE sys_workspace SET del_flag = 1, update_time = now() WHERE id = $1 AND del_flag = 0",
            &[&id],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("工作区不存在：{id}")));
    }
    Ok(Resp::ok(json!({ "id": id })))
}

// =====================================================================
// Data store (pointer only: directory or PostGIS connection)
// =====================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreBody {
    pub workspace_id: i64,
    pub name: String,
    pub store_type: String,
    #[serde(default)]
    pub base_path: String,
    #[serde(default)]
    pub pg_dsn: String,
    #[serde(default)]
    pub pg_schema: String,
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Clone)]
struct StoreRow {
    id: i64,
    workspace_id: i64,
    name: String,
    store_type: String,
    base_path: String,
    pg_dsn: String,
    pg_schema: String,
}

const STORE_SELECT: &str = "SELECT id, workspace_id, name, store_type, \
                           coalesce(base_path,''), coalesce(pg_dsn,''), coalesce(pg_schema,'public') \
                           FROM sys_store";

fn row_to_store(r: &tokio_postgres::Row) -> StoreRow {
    StoreRow {
        id: r.get(0),
        workspace_id: r.get(1),
        name: r.get(2),
        store_type: r.get(3),
        base_path: r.get(4),
        pg_dsn: r.get(5),
        pg_schema: r.get(6),
    }
}

async fn load_store(state: &AppState, id: i64) -> AppResult<StoreRow> {
    let sql = format!("{STORE_SELECT} WHERE id = $1 AND del_flag = 0");
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[&id]).await?;
    let row = rows.first().map(row_to_store).ok_or_else(|| {
        AppError::Msg(format!("数据存储不存在：{id}"))
    })?;
    decrypt_store_dsn(state, row)
}

/// `sys_store.pg_dsn` 以密文落库，读出后统一在这里解密（历史明文原样返回）。
fn decrypt_store_dsn(state: &AppState, mut row: StoreRow) -> AppResult<StoreRow> {
    row.pg_dsn = state.dsn.decrypt(&row.pg_dsn).map_err(|e| {
        AppError::Msg(format!("数据存储 {} 的 DSN 解密失败：{e}", row.id))
    })?;
    Ok(row)
}

/// Store form validation: legal type + required locator fields (dir needs path, PostGIS needs DSN).
fn validate_store(b: &StoreBody, caps: &Caps) -> AppResult<StoreRow> {
    let name = validate_ident(&b.name, "Store 名", 128)?;
    let store_type = b.store_type.trim().to_string();
    if !caps.is_store_type(&store_type) {
        return Err(AppError::Param(format!(
            "未知 Store 类型：{store_type}（引擎可选：{}）",
            caps.store_types.join("/")
        )));
    }
    let base_path = b.base_path.trim().to_string();
    let pg_dsn = b.pg_dsn.trim().to_string();
    let pg_schema = if b.pg_schema.trim().is_empty() {
        "public".to_string()
    } else {
        b.pg_schema.trim().to_string()
    };
    if store_type == STORE_TYPE_POSTGIS {
        // Empty = keep registered DSN (echo is masked only, can't round-trip);
        // so we don't check empty here; each add/edit caller decides whether it's required.
        if !pg_dsn.is_empty()
            && !(pg_dsn.starts_with("postgres://") || pg_dsn.starts_with("postgresql://"))
        {
            return Err(AppError::Param(
                "pgDsn 必须是 postgres:// 或 postgresql:// 连接串".into(),
            ));
        }
    } else if base_path.is_empty() {
        return Err(AppError::Param("目录型 Store 必须填写目录（basePath）".into()));
    } else if base_path.contains(';') {
        return Err(AppError::Param("basePath 含非法字符".into()));
    }
    Ok(StoreRow {
        id: 0,
        workspace_id: b.workspace_id,
        name,
        store_type,
        base_path,
        pg_dsn,
        pg_schema,
    })
}

/// Public response: DSN is masked + configured flag only — password never leaks back.
fn store_json(s: &StoreRow, extra: Option<&Value>) -> Value {
    let mut v = json!({
        "id": s.id,
        "workspaceId": s.workspace_id,
        "name": s.name,
        "storeType": s.store_type,
        "basePath": s.base_path,
        "pgSchema": s.pg_schema,
        "pgDsnConfigured": !s.pg_dsn.is_empty(),
        "pgDsnMasked": if s.pg_dsn.is_empty() { String::new() } else { crate::handlers::mapsvc::mask_conn(&s.pg_dsn) },
        "isPostgis": s.store_type == STORE_TYPE_POSTGIS,
    });
    if let Some(e) = extra {
        if let (Some(obj), Some(add)) = (v.as_object_mut(), e.as_object()) {
            for (k, val) in add {
                obj.insert(k.clone(), val.clone());
            }
        }
    }
    v
}

/// 启动时把历史明文 DSN 迁移为密文（幂等：已带 `enc:v1:` 前缀的跳过）。
pub async fn migrate_plaintext_dsns(state: &AppState) {
    let c = match conn(&state.pool).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("DSN 迁移跳过：{e:?}");
            return;
        }
    };
    let rows = match c
        .query(
            // 含软删除行：已逻辑删除的 Store 也不应留下明文口令。
            "SELECT id, coalesce(pg_dsn,'') FROM sys_store WHERE coalesce(pg_dsn,'') <> ''",
            &[],
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("DSN 迁移查询失败：{e}");
            return;
        }
    };
    let mut migrated = 0usize;
    for r in rows {
        let id: i64 = r.get(0);
        let dsn: String = r.get(1);
        if dsn.starts_with("enc:v1:") {
            continue;
        }
        match state.dsn.encrypt(&dsn) {
            Ok(enc) => {
                if let Err(e) = c
                    .execute("UPDATE sys_store SET pg_dsn=$2 WHERE id=$1", &[&id, &enc])
                    .await
                {
                    tracing::warn!(store_id = id, "DSN 迁移写库失败：{e}");
                } else {
                    migrated += 1;
                }
            }
            Err(e) => tracing::warn!(store_id = id, "DSN 迁移加密失败：{e}"),
        }
    }
    if migrated > 0 {
        tracing::info!("已将 {migrated} 条历史明文 DSN 迁移为密文");
    }
}

/// `GET /map/catalog/stores?workspaceId=` — Store list (incl. resource counts).
pub async fn store_list(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let ws = query_i64(qs.as_deref(), "workspaceId");
    let include = query_flag(qs.as_deref(), "includeDeleted");
    let page = query_i64(qs.as_deref(), "page").unwrap_or(0);
    let size = query_i64(qs.as_deref(), "pageSize").unwrap_or(0);
    let ws_id = ws.unwrap_or(0);
    let show_deleted = if include { 1 } else { 0 };
    let c = conn(&state.pool).await?;
    // Total count (unaffected by pagination)
    let total: i64 = c
        .query(
            "SELECT count(*) FROM sys_store s WHERE ($1::bigint = 0 OR s.workspace_id = $1) \
             AND ($2::int = 1 OR s.del_flag = 0)",
            &[&ws_id, &show_deleted],
        )
        .await?
        .first()
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0);
    let mut sql = String::from(
        "SELECT s.id, s.workspace_id, s.name, s.store_type, \
         coalesce(s.base_path,''), coalesce(s.pg_dsn,''), coalesce(s.pg_schema,'public'), \
         coalesce(s.remark,''), s.del_flag, \
         (SELECT count(*) FROM sys_resource r WHERE r.store_id = s.id AND r.del_flag = 0), \
         (SELECT count(*) FROM sys_resource r WHERE r.store_id = s.id AND r.del_flag = 0 \
          AND r.publishable = 1) \
         FROM sys_store s WHERE ($1::bigint = 0 OR s.workspace_id = $1) \
         AND ($2::int = 1 OR s.del_flag = 0) ORDER BY s.workspace_id, s.id",
    );
    // Server-side pagination: slice only when page/pageSize are explicitly passed (pageSize<=0 means no pagination, backward-compat)
    if size > 0 {
        let off = if page < 1 { 0 } else { (page - 1) * size };
        sql.push_str(&format!(" LIMIT {size} OFFSET {off}"));
    }
    let rows = c.query(&sql, &[&ws_id, &show_deleted]).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut s = row_to_store(r);
            // 列表只展示掩码；解密失败时降级为空串，不阻断整页。
            s.pg_dsn = state.dsn.decrypt(&s.pg_dsn).unwrap_or_else(|e| {
                tracing::warn!(store_id = s.id, "DSN 解密失败：{e}");
                String::new()
            });
            store_json(
                &s,
                Some(&json!({
                    "remark": r.get::<_, String>(7),
                    "delFlag": r.get::<_, i16>(8),
                    "resourceCount": r.get::<_, i64>(9),
                    "publishableCount": r.get::<_, i64>(10),
                })),
            )
        })
        .collect();
    Ok(Resp::ok(json!({ "stores": items, "total": total })))
}

/// `POST /map/catalog/stores`
pub async fn store_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreBody>,
) -> AppResult<Json<Resp<Value>>> {
    let caps = load_caps(&state).await?;
    let mut row = validate_store(&body, &caps)?;
    if row.store_type == STORE_TYPE_POSTGIS && row.pg_dsn.is_empty() {
        return Err(AppError::Param("PostGIS Store 必须填写连接串（pgDsn）".into()));
    }
    row.workspace_id = body.workspace_id;
    let c = conn(&state.pool).await?;
    let ws: i64 = c
        .query(
            "SELECT count(*) FROM sys_workspace WHERE id = $1 AND del_flag = 0",
            &[&row.workspace_id],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if ws == 0 {
        return Err(AppError::Msg(format!(
            "工作区不存在：{}",
            row.workspace_id
        )));
    }
    let dup: i64 = c
        .query(
            "SELECT count(*) FROM sys_store WHERE workspace_id=$1 AND name=$2 AND del_flag=0",
            &[&row.workspace_id, &row.name],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if dup > 0 {
        return Err(AppError::Msg(format!(
            "该工作区下已有同名 Store：{}",
            row.name
        )));
    }
    // DSN 密文落库；响应仍用内存里的明文做掩码，不回传。
    let stored_dsn = state
        .dsn
        .encrypt(&row.pg_dsn)
        .map_err(|e| AppError::Msg(format!("DSN 加密失败：{e}")))?;
    let id: i64 = c
        .query_opt(
            "INSERT INTO sys_store (workspace_id, name, store_type, base_path, pg_dsn, pg_schema, remark) \
             VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
            &[
                &row.workspace_id,
                &row.name,
                &row.store_type,
                &opt_text(&row.base_path),
                &opt_text(&stored_dsn),
                &row.pg_schema,
                &body.remark.trim(),
            ],
        )
        .await?
        .and_then(|r| r.get(0))
        .unwrap_or(0);
    row.id = id;
    Ok(Resp::ok(store_json(&row, None)))
}

/// `PUT /map/catalog/stores/:id` — Edit pointer; path / DSN changes do NOT auto-rescan,
/// front-end prompts 're-discover needed'. `pgDsn` empty means keep the registered DSN
/// (public response is masked only, can't round-trip).
pub async fn store_update(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Json(body): Json<StoreBody>,
) -> AppResult<Json<Resp<Value>>> {
    let caps = load_caps(&state).await?;
    let mut row = validate_store(&body, &caps)?;
    if row.store_type == STORE_TYPE_POSTGIS && row.pg_dsn.is_empty() {
        let cur = load_store(&state, id).await?;
        if cur.store_type != STORE_TYPE_POSTGIS || cur.pg_dsn.trim().is_empty() {
            return Err(AppError::Param("PostGIS Store 必须填写连接串（pgDsn）".into()));
        }
        row.pg_dsn = cur.pg_dsn;
    }
    row.id = id;
    row.workspace_id = body.workspace_id;
    let stored_dsn = state
        .dsn
        .encrypt(&row.pg_dsn)
        .map_err(|e| AppError::Msg(format!("DSN 加密失败：{e}")))?;
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_store SET workspace_id=$2, name=$3, store_type=$4, base_path=$5, \
             pg_dsn=$6, pg_schema=$7, remark=$8, update_time=now() WHERE id=$1 AND del_flag=0",
            &[
                &id,
                &row.workspace_id,
                &row.name,
                &row.store_type,
                &opt_text(&row.base_path),
                &opt_text(&stored_dsn),
                &row.pg_schema,
                &body.remark.trim(),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("数据存储不存在：{id}")));
    }
    Ok(Resp::ok(store_json(&row, None)))
}

/// `DELETE /map/catalog/stores/:id` — Soft-delete Store: all its online services stop and go to trash
/// (`state='deleted'`); resource snapshots preserved (recover with Store), data files untouched.
pub async fn store_remove(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let store = load_store(&state, id).await?;
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_store SET del_flag = 1, update_time = now() WHERE id = $1 AND del_flag = 0",
            &[&id],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("数据存储不存在：{id}")));
    }
    let ids = resource_ids_of(&state, id).await?;
    let retired = retire_resources(&state, &ids).await?;
    Ok(Resp::ok(json!({
        "id": id,
        "name": store.name,
        "resources": ids.len(),
        "retiredLayers": retired,
    })))
}

/// `POST /map/catalog/stores/:id/restore` — Restore Store pointer from trash.
/// Only clears its own `del_flag`; resources and published layers cascaded into trash stay as-is,
/// handled one by one via their own 'restore' button (one click changes one state type, bounded blast radius).
pub async fn store_restore(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_store SET del_flag = 0, update_time = now() WHERE id = $1 AND del_flag = 1",
            &[&id],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("数据存储不存在或未删除：{id}")));
    }
    Ok(Resp::ok(json!({ "id": id })))
}

// =====================================================================
// Connection test and read-only discovery
// =====================================================================

/// `POST /map/catalog/stores/:id/connect` — Reachability test (no DB write, no state change).
///
/// * PostGIS → map-api `/admin/pg-test`;
/// * Directory type → map-api `/admin/discover?depth=1`; reachable if entries are listed.
///
/// Engine request failure is swallowed, returns `{ok:false, detail}` — 'test connection' button gets readable feedback.
pub async fn store_connect(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let store = load_store(&state, id).await?;
    let probe = if store.store_type == STORE_TYPE_POSTGIS {
        upstream_json(
            &state,
            reqwest::Method::POST,
            "/admin/pg-test",
            None,
            Some(json!({ "dsn": store.pg_dsn })),
        )
        .await
    } else {
        let q = format!(
            "path={}&data_type={}&depth=1",
            qs_encode(&store.base_path),
            qs_encode(&store.store_type)
        );
        upstream_json(&state, reqwest::Method::GET, "/admin/discover", Some(&q), None).await
    };
    let out = match probe {
        Ok(v) => {
            if let Some(count) = v.get("count").and_then(|c| c.as_i64()) {
                json!({
                    "ok": true,
                    "detail": format!("目录可达，本层列出 {count} 个数据实体"),
                    "path": v.get("path").and_then(|p| p.as_str()).unwrap_or(&store.base_path),
                    "catalogRoot": v.get("catalog_root").and_then(|p| p.as_str()),
                    "truncated": v.get("truncated").and_then(|t| t.as_bool()).unwrap_or(false),
                })
            } else {
                json!({
                    "ok": v.get("ok").and_then(|o| o.as_bool()).unwrap_or(true),
                    "detail": v.get("detail").and_then(|d| d.as_str()).unwrap_or("连接成功"),
                })
            }
        }
        Err(e) => json!({ "ok": false, "detail": e.to_string() }),
    };
    Ok(Resp::ok(out))
}

/// New data-source 'test connection' body: assembled from front-end form fields; backend builds DSN and forwards to engine.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreConnectBody {
    pub host: String,
    #[serde(default = "default_pg_port")]
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
}

fn default_pg_port() -> u16 {
    54321
}

/// Percent-encode one DSN component (userinfo / dbname), keeping RFC 3986
/// unreserved characters. Host is left untouched (IPv6 literals need brackets).
fn encode_dsn_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `POST /map/catalog/stores/connect` — Connection test for new data source (no store id needed).
///
/// Front-end sends host/port/username/password/database; backend assembles `postgres://...` DSN
/// and forwards to engine `/admin/pg-test`, returns `{ok, detail}`. No DB write, no state record.
pub async fn store_connect_preflight(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreConnectBody>,
) -> AppResult<Json<Resp<Value>>> {
    let host = body.host.trim();
    if host.is_empty() {
        return Err(AppError::Param("host 不能为空".into()));
    }
    let database = body.database.trim();
    if database.is_empty() {
        return Err(AppError::Param("database 不能为空".into()));
    }
    // Password in DSN goes through request body (not URL / access log), consistent with pg-test contract.
    // userinfo / dbname 按 RFC 3986 百分号编码，否则密码含 @ : / # ? 会拼坏连接串
    // （tokio-postgres 解析 URI 时会做 percent-decode，可还原原始字符）。
    let dsn = format!(
        "postgres://{}:{}@{}:{}/{}",
        encode_dsn_component(body.username.trim()),
        encode_dsn_component(&body.password),
        host,
        body.port,
        encode_dsn_component(database)
    );
    let probe = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/pg-test",
        None,
        Some(json!({ "dsn": dsn })),
    )
    .await;
    let out = match probe {
        Ok(v) => json!({
            "ok": v.get("ok").and_then(|o| o.as_bool()).unwrap_or(true),
            "detail": v.get("detail").and_then(|d| d.as_str()).unwrap_or("连接成功"),
        }),
        Err(e) => json!({ "ok": false, "detail": e.to_string() }),
    };
    Ok(Resp::ok(out))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverBody {
    /// Recursion depth for dir-type Store (1..=4); ignored for PostGIS.
    #[serde(default)]
    pub depth: Option<i64>,
    /// PostGIS only: discover tables in this schema only (default: Store's pg_schema).
    #[serde(default)]
    pub schema: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportShpBody {
    /// Absolute path to .shp under workspace data root (picked by front-end tree).
    pub path: String,
    /// Target PostGIS table name after import (data layer name).
    pub table: String,
    /// Optional: override EPSG auto-detected from .prj.
    #[serde(default)]
    pub srid: Option<i32>,
}

/// `POST /map/catalog/stores/:id/import-shp` — Import a workspace Shapefile into this PostGIS DB.
///
/// Manager only does path/name validation and DSN forwarding; actual import is done by map-api calling `shp2pgsql`.
pub async fn store_import_shp(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Json(body): Json<ImportShpBody>,
) -> AppResult<Json<Resp<Value>>> {
    let (dsn, schema, table, shp) = prepare_import(&state, id, &body).await?;
    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/pg-import-shp",
        None,
        Some(json!({
            "dsn": dsn,
            "schema": schema,
            "shpPath": shp,
            "table": table,
            "srid": body.srid,
        })),
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `POST /map/catalog/stores/:id/import-shp/stream` — SSE version of the same import.
///
/// Large Shapefile shp2pgsql → COPY can take minutes; the sync API looks frozen to front-end users.
/// We pass map-api's SSE through directly to the browser: `stage=start/copy/done/error`,
/// `copy` events carry written row count and elapsed time, so front-end can refresh progress continuously.
pub async fn store_import_shp_stream(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Json(body): Json<ImportShpBody>,
) -> Response {
    let (dsn, schema, table, shp) = match prepare_import(&state, id, &body).await {
        Ok(v) => v,
        Err(e) => return sse_error(&e.to_string()),
    };
    let engine = engine::default_engine(&state).await;
    let base = engine.0.trim_end_matches('/').to_string();
    let mut req = state
        .map_http
        .post(format!("{base}/admin/pg-import-shp/stream"))
        .json(&json!({
            "dsn": dsn,
            "schema": schema,
            "shpPath": shp,
            "table": table,
            "srid": body.srid,
        }));
    if let Some(tok) = &engine.1 {
        req = req.header("X-Admin-Token", tok);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return sse_error(&format!("map-api 服务不可达（{base}）：{e}")),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return sse_error(&format!("map-api HTTP {status}：{txt}"));
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(resp.bytes_stream()))
        .unwrap_or_else(|_| sse_error("构建流式响应失败"))
}

/// Errors are also wrapped as SSE events (HTTP 200); front-end handles all via `stage=error`,
/// to avoid 4xx making the browser/RestTemplate declare the stream failed and drop the error reason.
fn sse_error(msg: &str) -> Response {
    let payload = json!({ "stage": "error", "error": msg }).to_string();
    let body = format!("data: {payload}\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Common pre-import validation (shared by sync and SSE paths): Store must be PostGIS,
/// table name legal, .shp must reside inside the owning workspace's data root.
/// Returns `(dsn, schema, table, shp absolute path)`.
async fn prepare_import(
    state: &AppState,
    id: i64,
    body: &ImportShpBody,
) -> AppResult<(String, String, String, String)> {
    let store = load_store(state, id).await?;
    if store.store_type != STORE_TYPE_POSTGIS {
        return Err(AppError::Param(
            "仅 PostGIS 数据源支持导入 Shapefile".to_string(),
        ));
    }
    if store.pg_dsn.trim().is_empty() {
        return Err(AppError::Param(
            "该数据源未登记 PostGIS 连接串，请先编辑补全".to_string(),
        ));
    }

    let table = body.table.trim().to_lowercase();
    validate_pg_ident(&table)?;

    // File must reside inside the Store's owning workspace data root (prevents path traversal).
    let c = conn(&state.pool).await?;
    let row = c
        .query_opt(
            "SELECT coalesce(w.data_root,'') FROM sys_store s JOIN sys_workspace w ON w.id = s.workspace_id WHERE s.id = $1 AND s.del_flag = 0",
            &[&id],
        )
        .await?;
    let data_root = row
        .ok_or_else(|| AppError::Msg(format!("数据存储不存在：{id}")))?
        .get::<_, String>(0)
        .trim()
        .to_string();
    if data_root.is_empty() {
        return Err(AppError::Param("该工作空间未配置数据根目录".to_string()));
    }
    let raw = body.path.trim();
    if raw.is_empty() {
        return Err(AppError::Param("请选择要导入的 .shp 文件".to_string()));
    }
    if !raw.to_ascii_lowercase().ends_with(".shp") {
        return Err(AppError::Param("仅支持 .shp 矢量文件".to_string()));
    }
    if !lexical_inside(raw, &data_root) {
        return Err(AppError::Param(format!(
            "文件超出工作空间数据根目录：{}（根：{}）",
            raw, data_root
        )));
    }
    // 文件存在性 / 可读性由 map-api 的导入接口判定（数据目录只在 map-api 侧挂载）。

    Ok((store.pg_dsn, store.pg_schema, table, raw.to_string()))
}

/// PostGIS identifier (import table name): starts with lowercase letter or underscore, only [A-Za-z0-9_], length ≤ 63.
fn validate_pg_ident(name: &str) -> AppResult<()> {
    if name.is_empty() {
        return Err(AppError::Param("数据图层名不能为空".to_string()));
    }
    if name.len() > 63 {
        return Err(AppError::Param("数据图层名长度上限 63 个字符".to_string()));
    }
    let mut it = name.chars();
    match it.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => {
            return Err(AppError::Param(
                "数据图层名需以字母或下划线开头".to_string(),
            ))
        }
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(AppError::Param(
            "数据图层名仅允许字母、数字与下划线".to_string(),
        ));
    }
    Ok(())
}

/// `POST /map/catalog/stores/:id/discover` — Read-only discover and refresh resource list.
///
/// Engine does read-only scan (file headers / archive metadata / `geometry_columns`); this endpoint snapshots results
/// as-is into `sys_resource` (same-name rows refresh metadata, `del_flag` untouched).
pub async fn store_discover(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    body: Option<Json<DiscoverBody>>,
) -> AppResult<Json<Resp<Value>>> {
    let store = load_store(&state, id).await?;
    let b = body.map(|Json(v)| v).unwrap_or(DiscoverBody { depth: None, schema: None });
    let entries = if store.store_type == STORE_TYPE_POSTGIS {
        let schema = b
            .schema
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| store.pg_schema.clone());
        let data = upstream_json(
            &state,
            reqwest::Method::POST,
            "/admin/pg-tables",
            None,
            Some(json!({ "dsn": store.pg_dsn, "schema": schema })),
        )
        .await?;
        arr_of(&data, "tables")
    } else {
        let depth = b.depth.unwrap_or(1).clamp(1, 4).to_string();
        let q = format!(
            "path={}&data_type={}&depth={}",
            qs_encode(&store.base_path),
            qs_encode(&store.store_type),
            depth
        );
        let data =
            upstream_json(&state, reqwest::Method::GET, "/admin/discover", Some(&q), None).await?;
        arr_of(&data, "entries")
    };
    let stored = upsert_resources(&state, store.id, &entries).await?;
    Ok(Resp::ok(json!({
        "storeId": store.id,
        "storeType": store.store_type,
        "discovered": entries.len(),
        "stored": stored,
        "resources": resources_of(&state, store.id).await?,
    })))
}

// =====================================================================
// 资源（Resource）
// =====================================================================

#[derive(Debug, Clone)]
struct ResourceRow {
    id: i64,
    store_id: i64,
    name: String,
    data_type: String,
    source: String,
    src_crs: String,
    bbox: Option<Vec<f64>>,
    publishable: i16,
    reason: String,
    meta: Value,
    crs_suspicious: bool,
    del_flag: i16,
}

const RES_SELECT: &str = "SELECT r.id, r.store_id, r.name, r.data_type, r.source, r.src_crs, \
                          r.bbox, r.publishable, coalesce(r.reason,''), r.meta::text, r.crs_suspicious, r.del_flag \
                          FROM sys_resource r";

fn row_to_resource(r: &tokio_postgres::Row) -> ResourceRow {
    let meta_text: String = r.get(9);
    ResourceRow {
        id: r.get(0),
        store_id: r.get(1),
        name: r.get(2),
        data_type: r.get(3),
        source: r.get(4),
        src_crs: r.get(5),
        bbox: r.get(6),
        publishable: r.get(7),
        reason: r.get(8),
        meta: serde_json::from_str(&meta_text).unwrap_or(Value::Null),
        crs_suspicious: r.get(10),
        del_flag: r.get(11),
    }
}

fn resource_json(x: &ResourceRow) -> Value {
    json!({
        "id": x.id,
        "storeId": x.store_id,
        "name": x.name,
        "dataType": x.data_type,
        "source": x.source,
        "srcCrs": x.src_crs,
        "bbox": x.bbox,
        "publishable": x.publishable == 1,
        "reason": x.reason,
        "crsSuspicious": x.crs_suspicious,
        "meta": x.meta,
    })
}

/// CRS sanity check: does the coord range fall within `src_crs`'s expected bounding box?
/// Returns `true` for 'suspicious' — CRS name clearly mismatches coord values, or CRS missing/unknown.
fn crs_suspicious(src_crs: &str, bbox: Option<&Vec<f64>>) -> bool {
    let epsg = match parse_epsg(src_crs) {
        Some(c) => c,
        None => return true, // 空 / LOCAL / UNKNOWN → 视为坐标系缺失，需引导
    };
    let ranges = expected_ranges(epsg);
    if ranges.is_empty() {
        return false; // 未覆盖的坐标系：不校验，放行
    };
    match bbox {
        None => false, // 无四至：无法判断，交由 needs_bbox 引导人工填写
        Some(b) if b.len() == 4 => {
            let (minx, miny, maxx, maxy) = (b[0], b[1], b[2], b[3]);
            !ranges.iter().any(|((x0, x1), (y0, y1))| {
                minx >= *x0 && maxx <= *x1 && miny >= *y0 && maxy <= *y1
            })
        }
        _ => false,
    }
}

/// Parse `EPSG:<code>` to int; empty / LOCAL / UNKNOWN returns None (treat as missing CRS).
fn parse_epsg(s: &str) -> Option<i32> {
    let s = s.trim().to_uppercase();
    if s.is_empty() || s == "LOCAL" || s == "UNKNOWN" {
        return None;
    }
    if let Some(code) = s.strip_prefix("EPSG:") {
        return code.trim().parse().ok();
    }
    s.parse().ok()
}

/// Known CRS expected coord-range candidates `((xmin,xmax),(ymin,ymax))`.
/// Same CRS may have multiple unit semantics (e.g. EPSG:3857 bbox can be in degrees
/// or in Web Mercator meters). Any one candidate match counts as legal.
fn expected_ranges(epsg: i32) -> Vec<((f64, f64), (f64, f64))> {
    match epsg {
        // Geographic coords (lat/lon)
        4326 | 4490 => vec![((-180.0, 180.0), (-90.0, 90.0))],
        // Web Mercator: accepts lat/lon semantics AND meter semantics
        3857 => vec![
            ((-180.0, 180.0), (-90.0, 90.0)),
            ((-20037508.34, 20037508.34), (-20037508.34, 20037508.34)),
        ],
        // CGCS2000 Gauss-Kruger 3° zone: EPSG = 4509 + zone (zone 34 = 4543, Kunming)
        c if (4509 + 25..=4509 + 45).contains(&c) => {
            let zone = c - 4509; // 25..45
            let base = (zone as f64) * 1_000_000.0 + 500_000.0;
            vec![((base - 1_500_000.0, base + 1_500_000.0), (0.0, 5_000_000.0))]
        }
        _ => Vec::new(),
    }
}

/// Snap discoveries into `sys_resource`: `on conflict do update` refreshes metadata only,
/// preserving user's soft-delete mark (`del_flag` not in update columns).
async fn upsert_resources(state: &AppState, store_id: i64, entries: &[Value]) -> AppResult<usize> {
    if entries.is_empty() {
        return Ok(0);
    }
    let c = conn(&state.pool).await?;
    // `::text::jsonb`: writing `$9::jsonb` alone makes Postgres infer the param itself as jsonb,
    // but tokio-postgres doesn't enable serde_json feature; String serializes to text only.
    const SQL: &str = "INSERT INTO sys_resource (store_id, name, data_type, source, src_crs, \
                       bbox, publishable, reason, meta, crs_suspicious) \
                       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9::text::jsonb,$10) \
                       ON CONFLICT (store_id, name) DO UPDATE SET \
                       data_type = EXCLUDED.data_type, source = EXCLUDED.source, \
                       src_crs = EXCLUDED.src_crs, bbox = EXCLUDED.bbox, \
                       publishable = EXCLUDED.publishable, reason = EXCLUDED.reason, \
                       meta = EXCLUDED.meta, crs_suspicious = EXCLUDED.crs_suspicious, update_time = now()";
    let mut done = 0usize;
    for e in entries {
        let name = str_field(e, "name");
        if name.is_empty() {
            continue;
        }
        let bbox = bbox_of(e);
        // Missing CRS no longer silently defaults to EPSG:4326 (that would hide the 'data has no CRS' issue):
        // Keep empty and mark suspicious; at publish time guide user to assign CRS with GIS software or pick from CRS tree.
        let src_crs = str_field(e, "src_crs");
        let data_type = str_field(e, "data_type");
        // CRS sanity: CRS name doesn't match coord range → mark suspicious (block at publish).
        // 3dtiles / terrain render per WGS 84/ECEF spec, don't need or record EPSG;
        // pg_table (no geometry) also has no CRS semantics. These three types skip suspicious check.
        let suspicious = !matches!(data_type.as_str(), "3dtiles" | "terrain" | "pg_table")
            && crs_suspicious(&src_crs, bbox.as_ref());
        let publishable = e
            .get("publishable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let reason = str_field(e, "reason");
        let source = str_field(e, "path");
        let meta = e.get("meta").cloned().unwrap_or(json!({}));
        let meta_text = meta.to_string();
        c.execute(
            SQL,
            &[
                &store_id,
                &name,
                &data_type,
                &source,
                &src_crs,
                &bbox,
                &(publishable as i16),
                &reason.chars().take(500).collect::<String>(),
                &meta_text,
                &suspicious,
            ],
        )
        .await?;
        done += 1;
    }
    Ok(done)
}

async fn resource_ids_of(state: &AppState, store_id: i64) -> AppResult<Vec<i64>> {
    let c = conn(&state.pool).await?;
    let rows = c
        .query(
            "SELECT id FROM sys_resource WHERE store_id = $1 AND del_flag = 0 ORDER BY id",
            &[&store_id],
        )
        .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

async fn resources_of(state: &AppState, store_id: i64) -> AppResult<Vec<Value>> {
    let sql = format!("{RES_SELECT} WHERE r.store_id = $1 AND r.del_flag = 0 ORDER BY r.name");
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[&store_id]).await?;
    Ok(rows.iter().map(|r| resource_json(&row_to_resource(r))).collect())
}

/// `GET /map/catalog/resources?storeId=&workspaceId=&includeDeleted=1`
/// Filter priority: storeId > workspaceId > all. When none is specified (or storeId=0),
/// list all resources; avoids `store_id = 0` always returning empty and being misdiagnosed as a broken API.
pub async fn resource_list(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let store_id = query_i64(qs.as_deref(), "storeId").unwrap_or(0);
    let workspace_id = query_i64(qs.as_deref(), "workspaceId").unwrap_or(0);
    let include = query_flag(qs.as_deref(), "includeDeleted");
    let page = query_i64(qs.as_deref(), "page").unwrap_or(0);
    let size = query_i64(qs.as_deref(), "pageSize").unwrap_or(0);
    // Scope: prefer store; then workspace; neither → list all (`$1 >= 0` always true,
    // placeholder to reuse unified param binding).
    let (scope, param): (String, i64) = if store_id > 0 {
        ("r.store_id = $1".to_string(), store_id)
    } else if workspace_id > 0 {
        (
            "r.store_id IN (SELECT id FROM sys_store WHERE workspace_id = $1)".to_string(),
            workspace_id,
        )
    } else {
        ("$1::bigint >= 0".to_string(), 0)
    };
    let del_prefix = if include { "" } else { "r.del_flag = 0 AND " };
    let c = conn(&state.pool).await?;
    // Total count (unaffected by pagination)
    let total: i64 = c
        .query(
            &format!("SELECT count(*) FROM sys_resource r WHERE {del_prefix}{scope}"),
            &[&param],
        )
        .await?
        .first()
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0);
    let mut sql = format!("{RES_SELECT} WHERE {del_prefix}{scope} ORDER BY r.name");
    // Server-side pagination: slice only when page/pageSize are explicitly passed (pageSize<=0 means no pagination, backward-compat)
    if size > 0 {
        let off = if page < 1 { 0 } else { (page - 1) * size };
        sql.push_str(&format!(" LIMIT {size} OFFSET {off}"));
    }
    let rows = c.query(&sql, &[&param]).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let x = row_to_resource(r);
            let mut v = resource_json(&x);
            v["delFlag"] = json!(x.del_flag);
            v
        })
        .collect();
    Ok(Resp::ok(json!({ "resources": items, "total": total })))
}

/// `DELETE /map/catalog/resources/:id` — Soft-delete (mark only):
/// `del_flag=1` + its published services set to `state='deleted'` (rust-gisserver stops, keeps
/// XML, goes to trash). No physical file delete, no physical DELETE.
pub async fn resource_remove(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_resource SET del_flag = 1, update_time = now() WHERE id = $1 AND del_flag = 0",
            &[&id],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("数据图层不存在：{id}")));
    }
    drop(c);
    let retired = retire_resources(&state, &[id]).await?;
    Ok(Resp::ok(json!({ "id": id, "retiredLayers": retired })))
}

/// `POST /map/catalog/resources/:id/restore` — Restore from trash: `del_flag=0`.
/// Service-side restore is done separately via `/map/catalog/published/:id/state` (avoids one click
/// changing two state types at once but only half succeeding, which is confusing).
pub async fn resource_restore(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_resource SET del_flag = 0, update_time = now() WHERE id = $1",
            &[&id],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("数据图层不存在：{id}")));
    }
    Ok(Resp::ok(json!({ "id": id, "delFlag": 0 })))
}

/// `POST /map/catalog/resources/:id/refresh` — Re-probe a single layer's metadata and refresh the ledger.
///
/// Data layers are derived from discovery; when source file/table changes, this is the single-entry re-read:
/// only calls engine read-only probes (file headers / archive metadata / `pg-tables`), no full rescan, no publish-state change.
pub async fn resource_refresh(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let (res, store) = load_resource_with_store(&state, id).await?;
    let entry = if store.store_type == STORE_TYPE_POSTGIS {
        let data = upstream_json(
            &state,
            reqwest::Method::POST,
            "/admin/pg-tables",
            None,
            Some(json!({ "dsn": store.pg_dsn, "schema": store.pg_schema })),
        )
        .await?;
        arr_of(&data, "tables")
            .into_iter()
            .find(|e| str_field(e, "name") == res.name)
            .ok_or_else(|| AppError::Msg(format!("PostGIS 中未找到表：{}", res.name)))?
    } else {
        let q = format!("path={}", qs_encode(&res.source));
        upstream_json(
            &state,
            reqwest::Method::GET,
            "/admin/probe",
            Some(&q),
            None,
        )
        .await?
    };
    let stored = upsert_resources(&state, store.id, &[entry]).await?;
    let refreshed = resources_of(&state, store.id)
        .await?
        .into_iter()
        .find(|r| r.get("id").and_then(|v| v.as_i64()) == Some(id));
    Ok(Resp::ok(json!({
        "id": id,
        "storeId": store.id,
        "stored": stored,
        "resource": refreshed,
    })))
}

/// `GET /map/catalog/resources/:id/query?limit=` — PostGIS plain-table data query service.
///
/// Open only for `data_type=pg_table` (plain tables with no geometry col): loads the owning Store's
/// DSN and table locator, forwards to engine `/admin/pg-query`, returns `{columns, rows}`.
/// Spatial layers use published services (MVT/WFS/OAF/WMS), not this endpoint.
pub async fn resource_query(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let (res, store) = load_resource_with_store(&state, id).await?;
    if store.store_type != STORE_TYPE_POSTGIS {
        return Err(AppError::Param("仅 PostGIS 数据源支持数据查询服务".into()));
    }
    if res.data_type != "pg_table" {
        return Err(AppError::Param(
            "该资源是空间图层，请发布为图层服务（MVT/WFS/OAF/WMS）后查询".into(),
        ));
    }
    let (sch, tbl) = match res.source.split_once('.') {
        Some((s, t)) if !s.is_empty() && !t.is_empty() => (s.to_string(), t.to_string()),
        _ => (
            str_field(&res.meta, "schema"),
            str_field(&res.meta, "table"),
        ),
    };
    if sch.is_empty() || tbl.is_empty() {
        return Err(AppError::Msg(format!("资源 {} 缺少 schema/table 定位", res.name)));
    }
    let limit = query_i64(qs.as_deref(), "limit").unwrap_or(100).clamp(1, 1000);
    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/pg-query",
        None,
        Some(json!({
            "dsn": store.pg_dsn,
            "schema": sch,
            "table": tbl,
            "limit": limit,
        })),
    )
    .await?;
    Ok(Resp::ok(data))
}

/// Stop and trash published services of multiple resources (local `state` + engine registry sync).
/// Returns processed layer count; per-layer failure is logged only, soft-delete is NOT rolled back (it is the authoritative intent).
async fn retire_resources(state: &AppState, resource_ids: &[i64]) -> AppResult<usize> {
    if resource_ids.is_empty() {
        return Ok(0);
    }
    let c = conn(&state.pool).await?;
    let rows = c
        .query(
            "SELECT layer_name, mapserver_kind FROM sys_published_layer \
             WHERE resource_id = ANY($1) AND del_flag = 0 AND state <> 'deleted'",
            & [&resource_ids.to_vec()],
        )
        .await?;
    let targets: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    drop(c);
    let mut done = 0usize;
    for (name, kind) in &targets {
        if let Err(e) = set_layer_state(state, kind, name, "deleted").await {
            tracing::warn!("停止服务失败（{kind}/{name}）：{e}");
            continue;
        }
        if let Ok(c) = conn(&state.pool).await {
            let _ = c
                .execute(
                    "UPDATE sys_published_layer SET state='deleted', update_time=now() \
                     WHERE layer_name=$1 AND del_flag=0",
                    &[name],
                )
                .await;
        }
        done += 1;
    }
    Ok(done)
}

/// Engine registry state change (`PUT /admin/services/:kind/:name/state`), routed by service ownership.
async fn set_layer_state(state: &AppState, kind: &str, name: &str, st: &str) -> AppResult<Value> {
    let engine = engine::resolve_for_service(state, kind, name).await;
    upstream_json_engine(
        state,
        &engine,
        reqwest::Method::PUT,
        &format!("/admin/services/{kind}/{name}/state"),
        None,
        Some(json!({ "state": st })),
    )
    .await
}

// =====================================================================
// Publish / modify publish info
// =====================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishBody {
    /// Resource to publish; when modifying publish info, the ledger row's resource is authoritative, this field can be omitted.
    #[serde(default)]
    pub resource_id: i64,
    pub layer_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub src_crs: String,
    #[serde(default)]
    pub bbox: Option<Vec<f64>>,
    #[serde(default)]
    pub service_types: Vec<String>,
    #[serde(default)]
    pub remark: String,
    /// Tile grid mode: `ogc` (standard LOD) / `tdt` (Tianditu LOD) / `custom` (Conf.xml).
    /// Empty string = unspecified, use engine default (WebMercatorQuad).
    #[serde(default)]
    pub grid_mode: String,
    /// Grid snapshot (required for `custom`; other modes can be generated by backend from presets).
    /// See [`resolve_publish_grid`] for return shape.
    #[serde(default)]
    pub grid: Option<Value>,
}

/// Source CRS: accepts `EPSG:<code>` only, consistent with service-level `Layer.src_crs`.
fn validate_crs(s: &str) -> AppResult<String> {
    let v = s.trim();
    let digits = v.strip_prefix("EPSG:").or_else(|| v.strip_prefix("epsg:"));
    match digits {
        Some(d) if !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) => {
            Ok(format!("EPSG:{}", d))
        }
        _ => Err(AppError::Param(format!(
            "坐标系须形如 EPSG:4326，当前为：{s}"
        ))),
    }
}

/// Bbox: optional (engine can compute after publish). If given, must be 4 finite values with minX<maxX.
fn normalize_bbox(b: Option<&Vec<f64>>) -> AppResult<Option<Vec<f64>>> {
    let Some(v) = b else { return Ok(None) };
    if v.len() != 4 {
        return Err(AppError::Param("四至必须是 4 个数 [minx,miny,maxx,maxy]".into()));
    }
    if v.iter().any(|x| !x.is_finite()) || v[0] >= v[2] || v[1] >= v[3] {
        return Err(AppError::Param(
            "四至无效：需满足 minx < maxx 且 miny < maxy".into(),
        ));
    }
    Ok(Some(v.clone()))
}

/// Extract bbox from discovery snapshot (keeps None when engine returns null).
fn bbox_of(e: &Value) -> Option<Vec<f64>> {
    let a = e.get("bbox").and_then(|v| v.as_array())?;
    if a.len() != 4 {
        return None;
    }
    let v: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
    if v.len() == 4 && v[0] < v[2] && v[1] < v[3] {
        Some(v)
    } else {
        None
    }
}

/// Rebuild rust-gisserver service doc from resource + Store pointer (payload for `POST /admin/services`).
///
/// Three shapes: 3D Tiles → `<TilesetService>` (root/entry); PostGIS →
/// `source.type=postgis` conn & table locator; others → `source.type=<kind>` + file path.
/// Pure function: no network, no DB read — easy to unit-test every kind.
fn build_service_body(
    res: &ResourceRow,
    store: &StoreRow,
    cap: &CapRow,
    layer_name: &str,
    title: &str,
    src_crs: &str,
    bbox: Option<&Vec<f64>>,
    grid: Option<&Value>,
) -> AppResult<Value> {
    let kind = cap.kind.as_str();
    let mut body = json!({
        "kind": kind,
        "name": layer_name,
        "title": if title.trim().is_empty() { res.name.clone() } else { title.trim().to_string() },
    });
    if kind == "3dtiles" {
        // Root must be a **directory** (dataset root), NOT a tileset.json file.
        // discover may return a dir path (parent dir containing tileset.json) or a file path directly;
        // we normalize here: if source points to tileset.json, take its parent dir as root.
        let source_path = std::path::PathBuf::from(res.source.trim());
        let root_path = if source_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case("tileset.json"))
            .unwrap_or(false)
        {
            source_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(source_path)
        } else {
            source_path
        };
        body["root"] = json!(root_path.to_string_lossy().replace('\\', "/"));
    } else if kind == "postgis" {
        let dsn = {
            let d = store.pg_dsn.trim();
            if d.is_empty() {
                return Err(AppError::Msg("该 Store 未登记 PostGIS 连接串".into()));
            }
            d
        };
        // schema/table 优先取发现阶段写入的 meta（名称含点也不会拆错）；旧资源才回退解析 source。
        let sch_meta = str_field(&res.meta, "schema");
        let tbl_meta = str_field(&res.meta, "table");
        let (sch, tbl) = if !sch_meta.is_empty() && !tbl_meta.is_empty() {
            (sch_meta, tbl_meta)
        } else {
            match res.source.split_once('.') {
                Some((s, t)) => (s.to_string(), t.to_string()),
                None => (store.pg_schema.clone(), res.source.clone()),
            }
        };
        let geometry_column = str_field(&res.meta, "geometry_column");
        if geometry_column.is_empty() {
            return Err(AppError::Msg(format!(
                "资源 {} 缺少几何列元数据，请重新发现后再发布",
                res.name
            )));
        }
        // geometry_columns.srid = 0 表示列上无 SRID 约束：此时回退发布表单的 srcCrs，
        // 仍取不到就拒绝发布——否则 ST_Transform(..., 0) 会让该图层所有瓦片查询失败。
        let meta_srid = res
            .meta
            .get("srid")
            .and_then(Value::as_i64)
            .filter(|v| *v > 0);
        let srid = match meta_srid.or_else(|| epsg_code(src_crs).map(|v| v as i64)) {
            Some(v) => v as i32,
            None => {
                return Err(AppError::Param(format!(
                    "资源 {} 的几何列未登记 SRID，请在发布表单指定源坐标系（srcCrs）",
                    res.name
                )))
            }
        };
        body["source"] = json!({
            "type": "postgis",
            "dsn": dsn,
            "schema": sch,
            "table": tbl,
            "geometry_column": geometry_column,
            "srid": srid,
        });
    } else {
        body["source"] = json!({ "type": kind, "path": res.source });
    }
    if kind != "3dtiles" {
        body["src_crs"] = json!(src_crs);
    }
    if let Some(b) = bbox {
        body["extent"] = json!(b);
    }
    // Tile grid: `grids` references grid id; non-preset grids (Tianditu / Conf.xml) also send
    // `custom_grids`; engine writes and hot-reloads, then tiles and previews use that grid.
    if kind != "3dtiles" {
        if let Some(g) = grid {
            if let Some(id) = g.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                body["grids"] = json!([id]);
                if let Some(def) = g.get("grid_def").filter(|d| d.is_object()) {
                    body["custom_grids"] = json!({ id: def });
                    if let Some(px) = def.get("tile_px").and_then(Value::as_u64) {
                        body["tile_px"] = json!(px);
                    }
                }
            }
        }
    }
    Ok(body)
}

/// Load resource + its Store (shared by publish and modify; resource must not be soft-deleted).
async fn load_resource_with_store(
    state: &AppState,
    resource_id: i64,
) -> AppResult<(ResourceRow, StoreRow)> {
    let sql = format!("{RES_SELECT} WHERE r.id = $1 AND r.del_flag = 0");
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[&resource_id]).await?;
    let res = rows
        .first()
        .map(row_to_resource)
        .ok_or_else(|| AppError::Msg(format!("数据图层不存在或已删除：{resource_id}")))?;
    let srow = c
        .query(
            &format!("{STORE_SELECT} WHERE id = $1 AND del_flag = 0"),
            &[&res.store_id],
        )
        .await?;
    let store = srow
        .first()
        .map(row_to_store)
        .ok_or_else(|| AppError::Msg(format!("所属 Store 不存在或已删除：{}", res.store_id)))?;
    let store = decrypt_store_dsn(state, store)?;
    Ok((res, store))
}

/// ArcGIS cache native grid snapshot: read CRS / tile size / origin / from `Conf.xml` at discover time,
/// LOD levels, solidified into `sys_published_layer.grid` at publish (or modify) time. Engine runtime still uses
/// the cache's own `Conf.xml`; we only do ledger record + front-end display/verification here.
fn native_grid(res: &ResourceRow) -> Option<Value> {
    if res.data_type != "arcgis" {
        return None;
    }
    let levels = res
        .meta
        .get("lods")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let crs = str_field(&res.meta, "crs");
    if crs.is_empty() && levels.as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return None;
    }
    Some(json!({
        "mode": "native",
        "crs": crs,
        "tile_cols": res.meta.get("tile_cols").cloned().unwrap_or(Value::Null),
        "tile_rows": res.meta.get("tile_rows").cloned().unwrap_or(Value::Null),
        "dpi": res.meta.get("dpi").cloned().unwrap_or(Value::Null),
        "origin": res.meta.get("origin").cloned().unwrap_or(Value::Null),
        "min_zoom": res.meta.get("min_zoom").cloned().unwrap_or(Value::Null),
        "max_zoom": res.meta.get("max_zoom").cloned().unwrap_or(Value::Null),
        "levels": levels,
    }))
}

/// Extract an EPSG code from `EPSG:4326`, `urn:ogc:def:crs:EPSG::4326` or `4326`.
fn epsg_code(crs: &str) -> Option<u32> {
    let s = crs.trim();
    s.rsplit(|c| c == ':' || c == '/')
        .next()
        .unwrap_or(s)
        .trim()
        .parse()
        .ok()
        .filter(|c| *c > 0)
}

/// Scale denominator under OGC 0.28 mm/pixel rule (for ledger/front-end display only; engine uses resolution).
fn ogc_scale(resolution: f64, crs: &str) -> f64 {
    let mpu = if epsg_code(crs) == Some(3857) {
        1.0
    } else {
        111_319.490_793_273_57
    };
    resolution * mpu / 0.00028
}

/// Build `levels: [{level, scale, resolution}]` display steps.
fn grid_level_rows(levels: &[u32], resolutions: &[f64], scales: &[f64], crs: &str) -> Vec<Value> {
    levels
        .iter()
        .enumerate()
        .map(|(i, z)| {
            let res = resolutions.get(i).copied().unwrap_or(f64::NAN);
            let scale = scales
                .get(i)
                .copied()
                .filter(|s| *s > 0.0)
                .unwrap_or_else(|| ogc_scale(res, crs));
            json!({ "level": z, "scale": scale, "resolution": res })
        })
        .collect()
}

/// Ledger snapshot for standard preset grids (WebMercatorQuad / WorldCRS84Quad / CGCS2000Quad).
/// Presets are parsed by engine internally; `grid_def` is always null.
fn preset_grid_snapshot(id: &str, crs: &str) -> Value {
    let (origin, extent, levels, resolutions): (Vec<f64>, Vec<f64>, Vec<u32>, Vec<f64>) =
        if id == "WebMercatorQuad" {
            let o = 20037508.3427892;
            let resolutions = (0..=25u32)
                .map(|z| o * 2.0 / (256.0 * (1u64 << z) as f64))
                .collect();
            (vec![-o, o], vec![-o, -o, o, o], (0..=25).collect(), resolutions)
        } else {
            let resolutions = (0..=24u32)
                .map(|z| 360.0 / (256.0 * 2.0 * (1u64 << z) as f64))
                .collect();
            (
                vec![-180.0, 90.0],
                vec![-180.0, -90.0, 180.0, 90.0],
                (0..=24).collect(),
                resolutions,
            )
        };
    let scales: Vec<f64> = resolutions.iter().map(|r| ogc_scale(*r, crs)).collect();
    let rows = grid_level_rows(&levels, &resolutions, &scales, crs);
    json!({
        "mode": "ogc",
        "id": id,
        "source": Value::Null,
        "crs": crs,
        "tile_cols": 256,
        "tile_rows": 256,
        "dpi": 96,
        "origin": origin,
        "extent": extent,
        "min_zoom": levels.first(),
        "max_zoom": levels.last(),
        "levels": rows,
        "grid_def": Value::Null,
    })
}

/// Tianditu LOD (EPSG:4490 lat/lon): origin [-180,90], 256px, levels 1–18,
/// resolution 0.703125 / 2^(level-1). Treated as custom grid (engine `custom_grids`).
fn tianditu_grid_snapshot() -> Value {
    let levels: Vec<u32> = (1..=18).collect();
    let resolutions: Vec<f64> = (0..18).map(|i| 0.703125 / (1u64 << i) as f64).collect();
    let crs = "EPSG:4490";
    let scales: Vec<f64> = resolutions.iter().map(|r| ogc_scale(*r, crs)).collect();
    let rows = grid_level_rows(&levels, &resolutions, &scales, crs);
    let origin = vec![-180.0, 90.0];
    let extent = vec![-180.0, -90.0, 180.0, 90.0];
    json!({
        "mode": "tdt",
        "id": "tianditu4490",
        "source": Value::Null,
        "crs": crs,
        "tile_cols": 256,
        "tile_rows": 256,
        "dpi": 96,
        "origin": origin,
        "extent": extent,
        "min_zoom": 1,
        "max_zoom": 18,
        "levels": rows,
        "grid_def": {
            "crs": crs,
            "origin": origin,
            "extent": extent,
            "tile_px": 256,
            "resolutions": resolutions,
            "levels": levels,
            "scales": scales,
        },
    })
}

/// Custom grid (ArcGIS `Conf.xml` parse result) → ledger snapshot + `grid_def` (for engine
/// `custom_grids` rebuild). `src_crs` must match XML's CRS — mismatch → error,
/// hint to convert CRS first; avoids tile-address vs data-coordinate misalignment.
fn custom_grid_snapshot(layer_name: &str, src_crs: &str, g: &Value) -> AppResult<Value> {
    let def = g.get("grid_def").filter(|d| d.is_object()).unwrap_or(g);
    let crs = def
        .get("crs")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Param("瓦片配置缺少 CRS".into()))?;
    if epsg_code(crs) != epsg_code(src_crs) {
        return Err(AppError::Param(format!(
            "瓦片配置的坐标系（{crs}）与数据源坐标系（{src_crs}）不一致，请先转换坐标系后再选择该 XML"
        )));
    }
    let origin = def
        .get("origin")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 2 && a.iter().all(Value::is_number))
        .cloned()
        .ok_or_else(|| AppError::Param("瓦片配置缺少有效的 origin[x,y]".into()))?;
    let extent = def
        .get("extent")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4 && a.iter().all(Value::is_number))
        .cloned()
        .ok_or_else(|| AppError::Param("瓦片配置缺少有效的 extent[4]".into()))?;
    let tile_px = def
        .get("tile_px")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0 && *v <= 4096)
        .unwrap_or(256);
    let resolutions: Vec<f64> = def
        .get("resolutions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    if resolutions.is_empty() {
        return Err(AppError::Param("瓦片配置缺少分辨率阶梯 resolutions".into()));
    }
    let levels: Vec<u32> = def
        .get("levels")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
        .unwrap_or_default();
    let levels = if levels.len() == resolutions.len() {
        levels
    } else {
        (0..resolutions.len() as u32).collect()
    };
    let scales: Vec<f64> = def
        .get("scales")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();

    let id = format!("g_{layer_name}_xml");
    let rows = grid_level_rows(&levels, &resolutions, &scales, src_crs);
    let source = g.get("source").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "mode": "custom",
        "id": id,
        "source": source,
        "crs": src_crs,
        "tile_cols": tile_px,
        "tile_rows": tile_px,
        "dpi": 96,
        "origin": origin,
        "extent": extent,
        "min_zoom": levels.first(),
        "max_zoom": levels.last(),
        "levels": rows,
        "grid_def": {
            "crs": src_crs,
            "origin": origin,
            "extent": extent,
            "tile_px": tile_px,
            "resolutions": resolutions,
            "levels": levels,
            "scales": scales,
        },
    }))
}

/// Parse and validate grid selection from publish form, return snapshot to solidify into `sys_published_layer.grid`.
///
/// * ArcGIS pre-sliced cache always uses `Conf.xml` native grid (form ignored);
/// * `ogc`：3857→WebMercatorQuad、4326→WorldCRS84Quad、4490→CGCS2000Quad，
///   other (projected) CRS rejected;
/// * `tdt`: EPSG:4490 only;
/// * `custom`: use form-supplied Conf.xml parse result; CRS must match `src_crs`.
/// Empty mode returns `None`, meaning use engine default (WebMercatorQuad).
fn resolve_publish_grid(
    res: &ResourceRow,
    layer_name: &str,
    src_crs: &str,
    mode: &str,
    form_grid: Option<&Value>,
) -> AppResult<Option<Value>> {
    if res.data_type == "arcgis" {
        return Ok(native_grid(res));
    }
    let mode = mode.trim().to_ascii_lowercase();
    if mode.is_empty() {
        return Ok(None);
    }
    let code = epsg_code(src_crs);
    match mode.as_str() {
        "ogc" => {
            let id = match code {
                Some(3857) => "WebMercatorQuad",
                Some(4326) => "WorldCRS84Quad",
                Some(4490) => "CGCS2000Quad",
                _ => {
                    return Err(AppError::Param(format!(
                        "坐标系 {src_crs} 不能使用 OGC 标准 LOD：投影坐标系只能选择「自定义 LOD」，\
                         或先转换为 EPSG:4326/4490/3857 后再发布"
                    )))
                }
            };
            Ok(Some(preset_grid_snapshot(id, src_crs)))
        }
        "tdt" => {
            if code != Some(4490) {
                return Err(AppError::Param(
                    "天地图 LOD 仅适用于 EPSG:4490（CGCS2000 经纬度）".into(),
                ));
            }
            Ok(Some(tianditu_grid_snapshot()))
        }
        "custom" => {
            let g = form_grid
                .ok_or_else(|| AppError::Param("自定义 LOD 需先选择并解析瓦片配置 XML".into()))?;
            Ok(Some(custom_grid_snapshot(layer_name, src_crs, g)?))
        }
        other => Err(AppError::Param(format!("未知的瓦片网格模式：{other}"))),
    }
}

/// Common DB write for publish/modify: upsert published layer + record owning engine.
/// Params are exactly `sys_published_layer` columns; passing them individually is clearer than wrapping in a struct, so we don't.
#[allow(clippy::too_many_arguments)]
async fn record_published(
    state: &AppState,
    res: &ResourceRow,
    store: &StoreRow,
    layer_name: &str,
    title: &str,
    src_crs: &str,
    bbox: Option<&Vec<f64>>,
    service_types: &[String],
    kind: &str,
    remark: &str,
    grid: Option<&Value>,
) -> AppResult<Value> {
    let grid_text = grid.map(|v| v.to_string());
    let c = conn(&state.pool).await?;
    let types = service_types.join(",");
    let id: i64 = c
        .query_opt(
            "INSERT INTO sys_published_layer (workspace_id, resource_id, layer_name, title, \
             src_crs, bbox, service_types, mapserver_kind, state, remark, grid) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,$10::text::jsonb) \
             ON CONFLICT (layer_name) WHERE del_flag = 0 DO UPDATE SET \
             workspace_id = EXCLUDED.workspace_id, resource_id = EXCLUDED.resource_id, \
             title = EXCLUDED.title, src_crs = EXCLUDED.src_crs, bbox = EXCLUDED.bbox, \
             service_types = EXCLUDED.service_types, mapserver_kind = EXCLUDED.mapserver_kind, \
             state = 'active', remark = EXCLUDED.remark, grid = EXCLUDED.grid, \
             del_flag = 0, update_time = now() \
             RETURNING id",
            &[
                &store.workspace_id,
                &res.id,
                &layer_name,
                &title,
                &src_crs,
                &bbox,
                &types,
                &kind,
                &remark,
                &grid_text,
            ],
        )
        .await?
        .and_then(|r| r.get(0))
        .unwrap_or(0);
    drop(c);
    // Owning engine: registration uses default engine; record it so later service ops route by ownership.
    let (base, _) = engine::default_engine(state).await;
    if let Some(eid) = engine::engine_id_by_base(state, base.trim_end_matches('/')).await {
        engine::record_service_engine(state, kind, layer_name, &eid).await;
    }
    Ok(json!({ "id": id, "layerName": layer_name, "kind": kind, "serviceTypes": service_types }))
}

/// `POST /map/catalog/publish` — Discovered Resource → map service.
///
/// Plan §7.2: form → `mapserver_kind` by `data_type` → build `Layer`/`TilesetService`
/// doc → `POST /admin/services` (engine writes XML + hot-reload) → upsert `sys_published_layer`.
pub async fn publish(
    State(state): State<Arc<AppState>>,
    Json(form): Json<PublishBody>,
) -> AppResult<Json<Resp<Value>>> {
    if form.resource_id == 0 {
        return Err(AppError::Param("缺少 resourceId".into()));
    }
    let layer_name = validate_ident(&form.layer_name, "服务名", 64)?;
    let (res, store) = load_resource_with_store(&state, form.resource_id).await?;
    let caps = load_caps(&state).await?;
    let cap = caps.get(&res.data_type)?;
    if res.publishable != 1 {
        return Err(AppError::Msg(format!(
            "资源 {} 标记为不可发布：{}",
            res.name,
            if res.reason.is_empty() { "未知原因" } else { &res.reason }
        )));
    }
    let types = cap.resolve_service_types(&form.service_types)?;
    let native_crs = res
        .meta
        .get("crs")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    // 3dtiles / terrain render per WGS 84/ECEF spec, don't need or record EPSG;
    // skip all CRS / bbox / grid validations at publish time.
    let is_3d = matches!(res.data_type.as_str(), "3dtiles" | "terrain");
    let src_crs_in = if res.data_type == "arcgis" && !native_crs.is_empty() {
        native_crs
    } else if form.src_crs.trim().is_empty() {
        res.src_crs.clone()
    } else {
        form.src_crs.clone()
    };
    if !is_3d && src_crs_in.trim().is_empty() {
        return Err(AppError::Msg(format!(
            "资源「{}」未声明坐标系，无法发布。请先用 GIS 软件赋予正确坐标系后重新导入，\
             或在坐标系树中选择一个（WGS84 系的地理/投影坐标、CGCS2000 系的地理坐标与高斯投影）。",
            res.name
        )));
    }
    let src_crs = if is_3d {
        // 3dtiles / terrain: no CRS validation, allow empty string directly
        src_crs_in.trim().to_string()
    } else {
        validate_crs(&src_crs_in)?
    };
    let bbox = if is_3d {
        // 3dtiles / terrain: no bbox validation, null directly
        None
    } else {
        normalize_bbox(form.bbox.as_ref().or(res.bbox.as_ref()))?
    };
    // CRS sanity block: stop publish when CRS name clearly mismatches coord range (or CRS missing).
    if !is_3d && crs_suspicious(&src_crs, bbox.as_ref()) {
        return Err(AppError::Msg(format!(
            "资源「{}」的坐标系（{}）与坐标值范围不匹配，疑似局部坐标或坐标系错误，已停止发布。\
             请先在 GIS 软件中核实坐标系、按真实经纬度配准后重新导入，\
             或在坐标系树中选择正确坐标系后再发布。",
            res.name, src_crs
        )));
    }
    let title = form.title.trim().to_string();
    // Grid: ArcGIS uses Conf.xml native grid; others follow form mode / custom XML parse.
    let grid = resolve_publish_grid(
        &res,
        &layer_name,
        &src_crs,
        &form.grid_mode,
        form.grid.as_ref(),
    )?;
    let body = build_service_body(
        &res,
        &store,
        &cap,
        &layer_name,
        &title,
        &src_crs,
        bbox.as_ref(),
        grid.as_ref(),
    )?;

    // Reject when same-name service already exists on engine (avoids overwriting someone else's config).
    ensure_name_free(&state, &layer_name).await?;

    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/services",
        None,
        Some(body),
    )
    .await?;
    if data.get("served").and_then(|v| v.as_bool()) != Some(true) {
        let why = data
            .get("skip_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("引擎未把该服务纳入服务集");
        return Err(AppError::Msg(format!("发布未生效：{why}")));
    }
    let out = record_published(
        &state,
        &res,
        &store,
        &layer_name,
        &title,
        &src_crs,
        bbox.as_ref(),
        &types,
        &cap.kind,
        form.remark.trim(),
        grid.as_ref(),
    )
    .await?;
    Ok(Resp::ok(json!({
        "published": out,
        "upstream": data.get("msg").cloned().unwrap_or(Value::Null),
    })))
}

/// Service-name collision check: reject if ledger has an undeleted same-name row (trash same-name rows can be reused,
/// `record_published` revives them in-place, keeping consistent with engine 're-register same-name service').
async fn ensure_name_free(state: &AppState, layer_name: &str) -> AppResult<()> {
    let c = conn(&state.pool).await?;
    let n: i64 = c
        .query(
            "SELECT count(*) FROM sys_published_layer WHERE layer_name = $1 AND del_flag = 0 AND state <> 'deleted'",
            &[&layer_name],
        )
        .await?
        .first()
        .map(|r| r.get(0))
        .unwrap_or(0);
    if n > 0 {
        return Err(AppError::Msg(format!(
            "服务名已存在且在线：{layer_name}（要改动它请用「修改发布信息」）"
        )));
    }
    Ok(())
}

// ---- Published service ledger ----

#[derive(Debug, Clone)]
struct PublishedRow {
    id: i64,
    workspace_id: i64,
    resource_id: Option<i64>,
    layer_name: String,
    title: String,
    src_crs: String,
    bbox: Option<Vec<f64>>,
    service_types: Vec<String>,
    mapserver_kind: String,
    state: String,
    remark: String,
    /// Native grid snapshot solidified at publish time (ArcGIS cache: CRS / tile size / origin / LOD).
    grid: Option<Value>,
}

const PUB_SELECT: &str = "SELECT p.id, p.workspace_id, p.resource_id, p.layer_name, p.title, \
                          p.src_crs, p.bbox, p.service_types, p.mapserver_kind, p.state, \
                          coalesce(p.remark,''), coalesce(w.name,''), coalesce(s.name,''), \
                          coalesce(r.name,''), coalesce(r.data_type,''), coalesce(r.source,''), \
                          coalesce(r.meta::text,'{}'), p.del_flag, p.grid::text \
                          FROM sys_published_layer p \
                          LEFT JOIN sys_workspace w ON w.id = p.workspace_id \
                          LEFT JOIN sys_resource r ON r.id = p.resource_id \
                          LEFT JOIN sys_store s ON s.id = r.store_id";

fn row_to_published(r: &tokio_postgres::Row) -> PublishedRow {
    let types: String = r.get(7);
    PublishedRow {
        id: r.get(0),
        workspace_id: r.get(1),
        resource_id: r.get(2),
        layer_name: r.get(3),
        title: r.get(4),
        src_crs: r.get(5),
        bbox: r.get(6),
        service_types: types
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        mapserver_kind: r.get(8),
        state: r.get(9),
        remark: r.get(10),
        grid: r
            .get::<_, Option<String>>(18)
            .and_then(|s| serde_json::from_str(&s).ok()),
    }
}

fn published_json(p: &PublishedRow, extra: &Value) -> Value {
    // geometry_type comes from extra.meta (sys_resource.meta.geometry_type)
    let geom = extra
        .get("meta")
        .and_then(|m| m.get("geometry_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    json!({
        "id": p.id,
        "workspaceId": p.workspace_id,
        "resourceId": p.resource_id,
        "layerName": p.layer_name,
        "title": p.title,
        "srcCrs": p.src_crs,
        "bbox": p.bbox,
        "serviceTypes": p.service_types,
        "mapserverKind": p.mapserver_kind,
        "state": p.state,
        "remark": p.remark,
        "grid": p.grid,
        "workspaceName": extra.get("workspaceName").cloned().unwrap_or(Value::Null),
        "storeName": extra.get("storeName").cloned().unwrap_or(Value::Null),
        "resourceName": extra.get("resourceName").cloned().unwrap_or(Value::Null),
        "dataType": extra.get("dataType").cloned().unwrap_or(Value::Null),
        "source": extra.get("source").cloned().unwrap_or(Value::Null),
        "geometryType": geom,
    })
}

fn published_extra(r: &tokio_postgres::Row) -> Value {
    let meta_text: String = r.get(16);
    json!({
        "workspaceName": r.get::<_, String>(11),
        "storeName": r.get::<_, String>(12),
        "resourceName": r.get::<_, String>(13),
        "dataType": r.get::<_, String>(14),
        "source": r.get::<_, String>(15),
        "meta": serde_json::from_str::<Value>(&meta_text).unwrap_or(json!({})),
        "delFlag": r.get::<_, i16>(17),
    })
}

/// `GET /map/catalog/published?workspaceId=&includeDeleted=1` — Published service list
/// (workspace/Store/resource ownership included, for grouped view and trash).
pub async fn published_list(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let ws = query_i64(qs.as_deref(), "workspaceId").unwrap_or(0);
    let include = query_flag(qs.as_deref(), "includeDeleted");
    let page = query_i64(qs.as_deref(), "page").unwrap_or(0);
    let size = query_i64(qs.as_deref(), "pageSize").unwrap_or(0);
    let mut where_clause = format!("($1::bigint = 0 OR p.workspace_id = $1)");
    if !include {
        where_clause.push_str(" AND p.state <> 'deleted'");
    }
    let c = conn(&state.pool).await?;
    // Total count (unaffected by pagination)
    let total: i64 = c
        .query(
            &format!("SELECT count(*) FROM sys_published_layer p WHERE {where_clause}"),
            &[&ws],
        )
        .await?
        .first()
        .map(|r| r.get::<_, i64>(0))
        .unwrap_or(0);
    let mut sql = format!("{PUB_SELECT} WHERE {where_clause} ORDER BY p.workspace_id, p.layer_name");
    // Server-side pagination: slice only when page/pageSize are explicitly passed (pageSize<=0 means no pagination, backward-compat)
    if size > 0 {
        let off = if page < 1 { 0 } else { (page - 1) * size };
        sql.push_str(&format!(" LIMIT {size} OFFSET {off}"));
    }
    let rows = c.query(&sql, &[&ws]).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| published_json(&row_to_published(r), &published_extra(r)))
        .collect();
    Ok(Resp::ok(json!({ "published": items, "total": total })))
}

/// `PUT /map/catalog/published/:id` — Modify publish info (plan §7.4).
///
/// Rebuild same resource's service doc and overwrite registration (`POST /admin/services` is idempotent
/// overwrite + hot-reload for same-name files), so rename uses `rename` then overwrite, avoiding two XML copies.
pub async fn published_update(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Json(form): Json<PublishBody>,
) -> AppResult<Json<Resp<Value>>> {
    let (p, extra) = load_published(&state, id).await?;
    let resource_id = p
        .resource_id
        .or(Some(form.resource_id).filter(|x| *x != 0))
        .ok_or_else(|| AppError::Msg("该服务没有关联资源，无法重建配置".into()))?;
    let (res, store) = load_resource_with_store(&state, resource_id).await?;
    let caps = load_caps(&state).await?;
    let cap = caps.get(&res.data_type)?;
    let types = cap.resolve_service_types(&form.service_types)?;
    // ArcGIS cache also uses Conf.xml's CRS; form override is not accepted on modify either.
    let native_crs = res
        .meta
        .get("crs")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    // Only ArcGIS cache uses Conf.xml's CRS (read-only); other types keep publish wizard input.
    let src_crs = validate_crs(if res.data_type == "arcgis" && !native_crs.is_empty() {
        &native_crs
    } else if form.src_crs.trim().is_empty() {
        &p.src_crs
    } else {
        &form.src_crs
    })?;
    let bbox = normalize_bbox(
        form.bbox
            .as_ref()
            .or(p.bbox.as_ref())
            .or(res.bbox.as_ref()),
    )?;
    let title = form.title.trim().to_string();
    let layer_name = if form.layer_name.trim().is_empty() {
        p.layer_name.clone()
    } else {
        validate_ident(&form.layer_name, "服务名", 64)?
    };
    // Grid: rebuild by new value when form explicitly submits; otherwise (only title/remark changed) use last snapshot.
    // ArcGIS always uses Conf.xml native grid.
    let grid = if form.grid_mode.trim().is_empty() && form.grid.is_none() {
        if res.data_type == "arcgis" {
            native_grid(&res)
        } else {
            p.grid.clone()
        }
    } else {
        resolve_publish_grid(
            &res,
            &layer_name,
            &src_crs,
            &form.grid_mode,
            form.grid.as_ref(),
        )?
    };

    // Rename: first rename on engine (keeps internal config IDs consistent), then overwrite content by new name.
    if layer_name != p.layer_name {
        ensure_name_free(&state, &layer_name).await?;
        upstream_json(
            &state,
            reqwest::Method::POST,
            &format!(
                "/admin/services/{}/{}/rename",
                p.mapserver_kind, p.layer_name
            ),
            None,
            Some(json!({ "name": layer_name })),
        )
        .await
        .map_err(|e| enrich(&e, &extra))?;
    }

    let body = build_service_body(
        &res,
        &store,
        &cap,
        &layer_name,
        &title,
        &src_crs,
        bbox.as_ref(),
        grid.as_ref(),
    )?;
    let data = upstream_json(&state, reqwest::Method::POST, "/admin/services", None, Some(body))
        .await
        .map_err(|e| enrich(&e, &extra))?;
    if data.get("served").and_then(|v| v.as_bool()) != Some(true) {
        let why = data
            .get("skip_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("引擎未把该服务纳入服务集");
        return Err(AppError::Msg(format!("修改未生效：{why}")));
    }

    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_published_layer SET layer_name=$2, title=$3, src_crs=$4, bbox=$5, \
             service_types=$6, mapserver_kind=$7, state='active', del_flag=0, remark=$8, grid=$9::text::jsonb, \
             update_time=now() WHERE id=$1",
            &[
                &id,
                &layer_name,
                &title,
                &src_crs,
                &bbox,
                &types.join(","),
                &cap.kind,
                &form.remark.trim(),
                &grid.as_ref().map(|v| v.to_string()),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("已发布服务不存在：{id}")));
    }
    // After rename, ownership record must follow the new name.
    if layer_name != p.layer_name {
        let (base, _) = engine::default_engine(&state).await;
        if let Some(eid) = engine::engine_id_by_base(&state, base.trim_end_matches('/')).await {
            engine::record_service_engine(&state, &cap.kind, &layer_name, &eid).await;
        }
        let _ = c
            .execute(
                "DELETE FROM sys_service_engine WHERE kind = $1 AND name = $2",
                &[&p.mapserver_kind, &p.layer_name],
            )
            .await;
    }
    Ok(Resp::ok(json!({
        "layerName": layer_name,
        "kind": cap.kind,
        "serviceTypes": types,
        "resourceId": resource_id,
    })))
}

/// When engine request fails, append local context so ops can directly tell which resource/file is the problem.
fn enrich(e: &AppError, extra: &Value) -> AppError {
    let src = extra.get("source").and_then(|v| v.as_str()).unwrap_or("");
    AppError::Msg(format!("{e}（资源数据：{src}）"))
}

async fn load_published(state: &AppState, id: i64) -> AppResult<(PublishedRow, Value)> {
    let sql = format!("{PUB_SELECT} WHERE p.id = $1");
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[&id]).await?;
    let row = rows
        .first()
        .ok_or_else(|| AppError::Msg(format!("已发布服务不存在：{id}")))?;
    Ok((row_to_published(row), published_extra(row)))
}

#[derive(Debug, Deserialize)]
pub struct StateBody {
    pub state: String,
}

/// `PUT /map/catalog/published/:id/state` — Lifecycle state (active|stopped|paused|deleted).
///
/// Same source as `/map/services/:kind/:name/state`, but also maintains the catalog ledger: setting to `active`
/// also restores soft-deleted resources back to the catalog (this is what trash 'restore' does).
pub async fn published_state(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Json(body): Json<StateBody>,
) -> AppResult<Json<Resp<Value>>> {
    let st = body.state.trim().to_ascii_lowercase();
    if !matches!(st.as_str(), "active" | "stopped" | "paused" | "deleted") {
        return Err(AppError::Param(
            "state 仅支持 active|stopped|paused|deleted".into(),
        ));
    }
    let (p, extra) = load_published(&state, id).await?;
    set_layer_state(&state, &p.mapserver_kind, &p.layer_name, &st)
        .await
        .map_err(|e| enrich(&e, &extra))?;
    let c = conn(&state.pool).await?;
    c.execute(
        "UPDATE sys_published_layer SET state = $2, update_time = now() WHERE id = $1",
        &[&id, &st],
    )
    .await?;
    if st == "active" {
        if let Some(rid) = p.resource_id {
            let _ = c
                .execute(
                    "UPDATE sys_resource SET del_flag = 0, update_time = now() WHERE id = $1",
                    &[&rid],
                )
                .await;
        }
    }
    Ok(Resp::ok(json!({ "id": id, "layerName": p.layer_name, "state": st })))
}

/// `GET /map/catalog/published/:id/endpoints` — Protocol endpoints of one published service (preview, one-click copy).
///
/// Endpoint URLs all come from engine ([`engine::aggregate_services`] aggregates across engines
/// `/admin/services`); rust-gisserver only picks the matching entry by name, does not build URLs itself,
/// to avoid drift with the 'service management' page. When the service is absent from engine catalog
/// (config deleted externally, or engine changed catalog dir), returns `found=false` + reason, not an error.
pub async fn published_endpoints(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> AppResult<Json<Resp<Value>>> {
    let (p, extra) = load_published(&state, id).await?;
    let agg = engine::aggregate_services(&state)
        .await
        .map_err(|e| enrich(&e, &extra))?;
    let hit = agg
        .get("services")
        .and_then(|v| v.as_array())
        .and_then(|list| {
            list.iter()
                .find(|s| s.get("name").and_then(|v| v.as_str()) == Some(p.layer_name.as_str()))
        });
    let Some(s) = hit else {
        return Ok(Resp::ok(json!({
            "layerName": p.layer_name,
            "state": p.state,
            "found": false,
            "serving": false,
            "endpoints": [],
            "hint": "引擎目录里已无该服务的配置：可能被外部删除或该服务从未落到本引擎。用「修改发布信息」重新落盘。"
        })));
    };
    Ok(Resp::ok(json!({
        "layerName": p.layer_name,
        "state": p.state,
        "found": true,
        "kind": s.get("kind").cloned().unwrap_or(Value::Null),
        "serving": s.get("serving").cloned().unwrap_or(json!(false)),
        "note": s.get("note").cloned().unwrap_or(Value::Null),
        "engineName": s.get("engineName").cloned().unwrap_or(Value::Null),
        "endpoints": s.get("endpoints").cloned().unwrap_or_else(|| json!([])),
    })))
}

// =====================================================================
// Utilities: query-string parse & encode (same logic as engine.rs, hand-written — no extra deps)
// =====================================================================

fn query_pairs(qs: Option<&str>) -> Vec<(String, String)> {
    qs.unwrap_or("")
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.trim().to_ascii_lowercase(), qs_decode(v.trim()))
        })
        .collect()
}

pub(crate) fn query_i64(qs: Option<&str>, key: &str) -> Option<i64> {
    let k = key.to_ascii_lowercase();
    query_pairs(qs)
        .into_iter()
        .find(|(x, _)| *x == k)
        .and_then(|(_, v)| v.parse::<i64>().ok())
}

pub(crate) fn query_flag(qs: Option<&str>, key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    query_pairs(qs)
        .into_iter()
        .find(|(x, _)| *x == k)
        .map(|(_, v)| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Store empty string as NULL (dir-type Store has no DSN, and vice versa).
fn opt_text(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn arr_of(v: &Value, key: &str) -> Vec<Value> {
    v.get(key)
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Minimal percent-encode: keep unreserved chars only, everything else → `%XX` per UTF-8 byte.
/// `\`, `:`, and Chinese in directory paths must be encoded for safe query-string transport.
fn qs_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Minimal percent-decode (`+` → space, illegal sequences kept as-is).
pub(crate) fn qs_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| (b as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(data_type: &str, source: &str, meta: Value) -> ResourceRow {
        ResourceRow {
            id: 1,
            store_id: 9,
            name: "demo".into(),
            data_type: data_type.into(),
            source: source.into(),
            src_crs: "EPSG:4326".into(),
            bbox: Some(vec![0.0, 0.0, 1.0, 1.0]),
            publishable: 1,
            reason: String::new(),
            meta,
            crs_suspicious: false,
            del_flag: 0,
        }
    }

    fn store(store_type: &str, base: &str, dsn: &str) -> StoreRow {
        StoreRow {
            id: 9,
            workspace_id: 1,
            name: "s".into(),
            store_type: store_type.into(),
            base_path: base.into(),
            pg_dsn: dsn.into(),
            pg_schema: "public".into(),
        }
    }

    fn cap(kind: &str, supported: &[&str], defaults: &[&str]) -> CapRow {
        CapRow {
            kind: kind.into(),
            service_types: supported.iter().map(|s| s.to_string()).collect(),
            defaults: defaults.iter().map(|s| s.to_string()).collect(),
            publishable: true,
            note: String::new(),
        }
    }

    #[test]
    fn file_source_reuses_discovered_path() {
        let body = build_service_body(
            &res("cog", r"D:\data\demo.tif", json!({})),
            &store("cog_dir", r"D:\data", ""),
            &cap("cog", &["wms", "wmts"], &["wms", "wmts"]),
            "demo_cog",
            "演示影像",
            "EPSG:32650",
            Some(&vec![1.0, 2.0, 3.0, 4.0]),
            None,
        )
        .unwrap();
        assert_eq!(body["kind"], json!("cog"));
        assert_eq!(body["name"], json!("demo_cog"));
        assert_eq!(body["source"]["type"], json!("cog"));
        assert_eq!(body["source"]["path"], json!(r"D:\data\demo.tif"));
        assert_eq!(body["src_crs"], json!("EPSG:32650"));
        assert_eq!(body["extent"], json!([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(body["title"], json!("演示影像"));
    }

    #[test]
    fn postgis_source_carries_table_and_srid() {
        let r = res(
            "postgis_table",
            "gis.wuhua_buildings",
            json!({"geometry_column": "geom_wgs84", "srid": 4490}),
        );
        let body = build_service_body(
            &r,
            &store("postgis", "", "postgres://u:p@h:5432/gis"),
            &cap("postgis", &["mvt", "wms", "wfs"], &["mvt", "wms"]),
            "buildings",
            "",
            "EPSG:4490",
            None,
            None,
        )
        .unwrap();
        assert_eq!(body["source"]["type"], json!("postgis"));
        assert_eq!(body["source"]["schema"], json!("gis"));
        assert_eq!(body["source"]["table"], json!("wuhua_buildings"));
        assert_eq!(body["source"]["geometry_column"], json!("geom_wgs84"));
        assert_eq!(body["source"]["srid"], json!(4490));
        assert_eq!(body["source"]["dsn"], json!("postgres://u:p@h:5432/gis"));
        // PostGIS does not write extent: engine computes via ST_Extent at startup.
        assert!(body.get("extent").is_none());
        // Title falls back to resource name when empty.
        assert_eq!(body["title"], json!("demo"));
    }

    #[test]
    fn postgis_without_dsn_is_rejected() {
        let e = build_service_body(
            &res("postgis_table", "public.t", json!({"srid": 4326})),
            &store("postgis", "", ""),
            &cap("postgis", &["mvt"], &["mvt"]),
            "t",
            "",
            "EPSG:4326",
            None,
            None,
        );
        assert!(e.is_err(), "缺 DSN 的 PostGIS Store 不能发布");
    }

    #[test]
    fn tileset_body_uses_root_and_drops_crs() {
        // 平台无关：用 Path::join 生成本机分隔符的路径，再按“服务体统一为正斜杠”的
        // 契约比较，不把 Windows 盘符 / 反斜杠写死进断言（D5）。
        let store_root = std::path::Path::new("data_root").join("region");
        let res_dir = store_root.join("city");
        let store_root = store_root.to_string_lossy().to_string();
        let res_source = res_dir.to_string_lossy().to_string();
        let body = build_service_body(
            &res("3dtiles", &res_source, json!({})),
            &store("3dtiles_dir", &store_root, ""),
            &cap("3dtiles", &["3dtiles"], &["3dtiles"]),
            "city",
            "城市模型",
            "EPSG:4326",
            None,
            None,
        )
        .unwrap();
        assert_eq!(body["kind"], json!("3dtiles"));
        // 校验点 1：root 是资源目录本身并保留 Store root 前缀（相对名 "city" 会被渲染侧
        // 解析到错误位置），且不指向 tileset.json 文件。
        let root = body["root"].as_str().unwrap();
        assert_eq!(root, res_dir.to_string_lossy().replace('\\', "/"));
        assert!(root.ends_with("/city"), "root 应保留资源目录名: {root}");
        // 校验点 2：跨平台契约——服务体里的路径分隔符统一为 '/'。
        assert!(!root.contains('\\'), "root 不应含 Windows 分隔符: {root}");
        // 校验点 3：TilesetService 文档没有 src_crs / source 字段（引擎只读 Name/Title/Root/Entry），
        // 发布 3dtiles 时必须丢掉 CRS 信息。
        assert!(body.get("src_crs").is_none());
        assert!(body.get("source").is_none());
    }

    #[test]
    fn crs_and_bbox_are_validated() {
        assert_eq!(validate_crs(" EPSG:4490 ").unwrap(), "EPSG:4490");
        assert!(validate_crs("4490").is_err());
        assert!(validate_crs("EPSG:abc").is_err());
        assert!(normalize_bbox(None).unwrap().is_none());
        assert!(normalize_bbox(Some(&vec![0.0, 0.0, 1.0])).is_err());
        assert!(normalize_bbox(Some(&vec![1.0, 0.0, 0.0, 1.0])).is_err());
        assert!(normalize_bbox(Some(&vec![0.0, 0.0, f64::NAN, 1.0])).is_err());
        assert_eq!(
            normalize_bbox(Some(&vec![0.0, 0.0, 1.0, 1.0]))
                .unwrap()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn service_types_default_and_reject() {
        let c = cap("cog", &["wms", "wmts", "xyz"], &["wms", "wmts"]);
        assert_eq!(c.resolve_service_types(&[]).unwrap(), vec!["wms", "wmts"]);
        assert_eq!(
            c.resolve_service_types(&["WMTS".into()]).unwrap(),
            vec!["wmts"]
        );
        // 归档矢量层勾 WMTS 这类越界勾选必须被拒（口径来自引擎矩阵）。
        assert!(c.resolve_service_types(&["wfs".into()]).is_err());
        assert!(c.resolve_service_types(&["".into()]).is_err());
        // 不可发布类型（shapefile）直接给原因。
        let shp = CapRow {
            kind: String::new(),
            service_types: vec![],
            defaults: vec![],
            publishable: false,
            note: "无静态 shapefile 读取器".into(),
        };
        let caps = Caps {
            by_type: HashMap::from([("shapefile".to_string(), shp)]),
            store_types: vec!["cog_dir".to_string()],
        };
        let err = caps.get("shapefile").unwrap_err().to_string();
        assert!(err.contains("无静态 shapefile 读取器"), "{err}");
        assert!(caps.is_store_type("postgis"));
        assert!(caps.is_store_type("cog_dir"));
        assert!(!caps.is_store_type("shp_dir"));
    }

    #[test]
    fn identifiers_are_constrained() {
        assert_eq!(validate_ident(" ws1 ", "工作区名", 64).unwrap(), "ws1");
        // 点号会被引擎 file_stem 截断，必须在入口就拒掉。
        assert!(validate_ident("a.b", "服务名", 64).is_err());
        assert!(validate_ident("中文", "服务名", 64).is_err());
        assert!(validate_ident("", "服务名", 64).is_err());
    }

    #[test]
    fn store_validation_requires_its_pointer() {
        let caps = Caps {
            by_type: HashMap::new(),
            store_types: vec!["cog_dir".to_string()],
        };
        let b = StoreBody {
            workspace_id: 1,
            name: "s1".into(),
            store_type: "cog_dir".into(),
            base_path: String::new(),
            pg_dsn: String::new(),
            pg_schema: String::new(),
            remark: String::new(),
        };
        assert!(validate_store(&b, &caps).is_err(), "目录型必须有 basePath");
        let pg = StoreBody {
            store_type: "postgis".into(),
            pg_dsn: "mysql://h/db".into(),
            ..b.clone()
        };
        assert!(validate_store(&pg, &caps).is_err(), "DSN 必须是 postgres 连接串");
        let ok = StoreBody {
            pg_dsn: "postgres://u@h/db".into(),
            ..pg.clone()
        };
        let row = validate_store(&ok, &caps).unwrap();
        assert_eq!(row.pg_schema, "public", "缺省 schema 回落 public");
        assert!(validate_store(&StoreBody { store_type: "shp_dir".into(), ..b }, &caps).is_err());
    }

    #[test]
    fn query_string_helpers_roundtrip_windows_paths() {
        let raw = r"D:\数据\a b.tif";
        let enc = qs_encode(raw);
        assert!(!enc.contains(' ') && !enc.contains('\\'));
        assert_eq!(qs_decode(&enc), raw);
        let qs = format!("path={}&depth=3", enc);
        assert_eq!(query_i64(Some(&qs), "depth"), Some(3));
        assert!(query_flag(Some("includeDeleted=1"), "includeDeleted"));
        assert!(!query_flag(Some("includeDeleted=0"), "includeDeleted"));
        assert_eq!(query_i64(Some("workspaceId=7"), "workspaceId"), Some(7));
    }

    #[test]
    fn bbox_snapshot_ignores_degenerate() {
        assert_eq!(bbox_of(&json!({"bbox": [0.0, 0.0, 1.0, 2.0]})).unwrap().len(), 4);
        assert!(bbox_of(&json!({"bbox": null})).is_none());
        assert!(bbox_of(&json!({"bbox": [1.0, 1.0, 1.0, 1.0]})).is_none());
        assert!(bbox_of(&json!({"bbox": [1.0, 2.0]})).is_none());
    }
}
