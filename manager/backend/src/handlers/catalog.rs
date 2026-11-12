//! 数据目录：Workspace → Store → Resource → 已发布服务（对齐 GeoServer 组织层）。
//!
//! 边界（计划 §1 红线，本模块逐条遵守）：
//! * 只做**服务发布与管理**的组织元数据；组织元数据全部落 r-manager PostgreSQL。
//! * 不读数据文件、不连业务库：数据侧一律走 map-api 的只读发现面
//!   （`/admin/capabilities`、`/admin/discover`、`/admin/pg-tables`、`/admin/pg-test`）。
//! * 发布产物是 r-mapserver 的 `Layer` / `TilesetService` 文档，经
//!   `POST /admin/services` 落盘并热重载；r-manager 不写 XML、不生成瓦片、不导入要素。
//! * 软删除仅标记：Resource `del_flag=1`，已发布层 `state='deleted'`（进垃圾箱，可恢复）。
//!
//! 引擎口径：目录面（发现/连接测试/发布）当前一律走**默认引擎**
//! （[`crate::handlers::engine::default_engine`]）；服务级读写（配置原文、启停、体检、
//! 归属调整）仍由既有 `/map/services/*` 按服务归属路由，两者互不影响。
//!
//! 能力矩阵（`data_type` → `kind` + 可发布服务类型）从 map-api `/admin/capabilities`
//! 取，r-manager 不重复维护第二份矩阵，避免两处口径漂移。
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

/// PostGIS 型 Store：不是目录，`pg_dsn` + `pg_schema` 定位（map-api 的
/// `store_types` 只列目录型，这里在本地补齐这一类）。
const STORE_TYPE_POSTGIS: &str = "postgis";

/// 目录域路由（由 `handlers::router` 统一装配）。静态段在前，`:id` 通配在后。
/// `GET /map/catalog/crs-tree` — 坐标系选择树（WGS84 系 + CGCS2000 系），
/// 供发布向导在「坐标系缺失 / 可疑」时引导用户选择正确坐标系。
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
        // 能力矩阵（发布向导的数据字典）
        .route("/map/catalog/capabilities", get(capabilities))
        // 工作区
        .route("/map/catalog/workspaces", get(ws_list).post(ws_add))
        .route(
            "/map/catalog/workspaces/{id}",
            put(ws_update).delete(ws_remove),
        )
        // 工作空间数据根目录浏览（新建数据源左侧目录树，按工作区范围读取）
        .route("/map/catalog/dirs", get(dir_list))
        // 发布向导「选择瓦片配置 XML」：解析服务器上的 ArcGIS Conf.xml → 网格参数
        .route("/map/catalog/tile-grid", post(tile_grid_parse))
        // 数据存储
        .route("/map/catalog/stores", get(store_list).post(store_add))
        // 新建数据源时的「测试连接」：无需已落库的 store id，直接用表单参数探测。
        .route("/map/catalog/stores/connect", post(store_connect_preflight))
        .route(
            "/map/catalog/stores/{id}/connect",
            post(store_connect),
        )
        .route(
            "/map/catalog/stores/{id}/discover",
            post(store_discover),
        )
        // PostGIS 数据源：把工作空间内的 Shapefile 导入数据库。
        .route(
            "/map/catalog/stores/{id}/import-shp",
            post(store_import_shp),
        )
        // 同一导入的 SSE 版本：数据量大时前端持续收到进度，避免长请求看起来卡死
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
        // 数据图层（Resource）
        .route("/map/catalog/resources", get(resource_list))
        .route(
            "/map/catalog/resources/{id}/restore",
            post(resource_restore),
        )
        .route(
            "/map/catalog/resources/{id}/refresh",
            post(resource_refresh),
        )
        // PostGIS 普通表的数据查询服务（不发布图层，直接返回表数据）。
        .route(
            "/map/catalog/resources/{id}/query",
            get(resource_query),
        )
        .route("/map/catalog/resources/{id}", delete(resource_remove))
        // 发布
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
        // 坐标系选择树（发布向导的 CRS 字典，覆盖 WGS84/CGCS2000 常用系）
        .route("/map/catalog/crs-tree", get(crs_tree))
}

// =====================================================================
// 能力矩阵（来自 map-api，单一口径）
// =====================================================================

/// 一种 `data_type` 的发布口径。
#[derive(Debug, Clone)]
struct CapRow {
    kind: String,
    service_types: Vec<String>,
    defaults: Vec<String>,
    publishable: bool,
    note: String,
}

/// `/admin/capabilities` 的本地视图。
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

    /// 校验勾选的服务类型：空则取默认，越界则拒绝并列出可选值。
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

/// 拉取并解析 `/admin/capabilities`。
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

/// `GET /map/catalog/capabilities` — 原样透传引擎能力矩阵，供发布向导渲染勾选。
pub async fn capabilities(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Resp<Value>>> {
    let data =
        upstream_json(&state, reqwest::Method::GET, "/admin/capabilities", None, None).await?;
    Ok(Resp::ok(data))
}

// =====================================================================
// 工作区
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

/// 将用户输入的路径规范化：`\` → `/`，末尾补 `/`，空串保持空串。
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

/// 工作空间数据根目录下的标准子目录（绑定 / 修改 data_root 保存前自动补建）。
const WORKSPACE_STD_SUBDIRS: [&str; 13] = [
    "3dtiles", "arcgis_cah", "cog", "docs", "geojson", "mbtiles", "pmtiles", "postgis", "prj",
    "recon", "shapefile", "terrain", "xml-grid",
];

/// 保存前确保数据根目录存在且标准子目录齐全。
///
/// - `data_root` 为空（纯数据库工作区）直接放行；
/// - 根目录本身不存在 → 视为录入错误，拒绝保存（防止 create_dir_all 连带造出拼错的根）；
/// - 子目录已存在则跳过，缺哪个补哪个；建目录失败（权限等）报错不落库。
fn ensure_std_subdirs(data_root: &str) -> AppResult<()> {
    if data_root.is_empty() {
        return Ok(());
    }
    let root = std::path::Path::new(data_root);
    if !root.is_dir() {
        return Err(AppError::Param(format!(
            "数据根目录不可访问，请先创建或检查路径：{}",
            data_root.trim_end_matches('/')
        )));
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

/// 工作区名 = 命名空间标识，必须是简单标识符（树节点与 URL 都用它）。
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

/// 把路径 canonicalize 成绝对目录；文件或不可访问一律报错。
fn canonical_dir(raw: &str) -> std::io::Result<std::path::PathBuf> {
    let p = std::fs::canonicalize(raw)?;
    if !p.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "不是目录",
        ));
    }
    Ok(p)
}

/// `GET /map/catalog/dirs?workspaceId=&path=` — 列出工作空间数据根目录下的子目录（一层）。
///
/// 供「新建数据源」左侧目录树使用。数据根目录不一定在引擎 `DATA_ROOT` 之内，
/// 因此不复用 `/admin/browse` 的前缀守卫；这里只读取目录/文件元信息，不读数据内容，
/// 且 `path` 必须落在该工作空间 `data_root` 内（canonicalize 后做前缀校验，防目录穿越）。
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
    let root = canonical_dir(&data_root)
        .map_err(|_| AppError::Param(format!("数据根目录不可访问：{data_root}")))?;

    let requested_raw = query_pairs(qs.as_deref())
        .into_iter()
        .find(|(k, _)| k == "path")
        .map(|(_, v)| v)
        .unwrap_or_default();
    let requested_raw = requested_raw.trim().to_string();
    let requested = if requested_raw.is_empty() {
        root.clone()
    } else {
        canonical_dir(&requested_raw)
            .map_err(|_| AppError::Param(format!("目录不可访问：{requested_raw}")))?
    };
    if !requested.starts_with(&root) {
        return Err(AppError::Param(format!(
            "目录超出工作空间数据根目录：{}（根：{}）",
            requested.display(),
            root.display()
        )));
    }

    let mut entries: Vec<Value> = Vec::new();
    let rd = std::fs::read_dir(&requested)
        .map_err(|e| AppError::Msg(format!("读取目录失败：{}（{e}）", requested.display())))?;
    for e in rd.flatten() {
        let meta = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_dir = meta.is_dir();
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        entries.push(json!({
            "name": e.file_name().to_string_lossy(),
            "is_dir": is_dir,
            "size": if is_dir { 0 } else { meta.len() },
            "modified_ms": modified_ms,
        }));
    }
    entries.sort_by(|a, b| {
        let ad = a["is_dir"].as_bool().unwrap_or(false);
        let bd = b["is_dir"].as_bool().unwrap_or(false);
        bd.cmp(&ad).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
        })
    });

    Ok(Resp::ok(json!({
        "path": requested.to_string_lossy(),
        "parent": requested.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "root": root.to_string_lossy(),
        "entries": entries,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileGridBody {
    #[serde(default)]
    pub workspace_id: i64,
    #[serde(default)]
    pub path: String,
}

/// `POST /map/catalog/tile-grid` — 解析服务器上的 ArcGIS `Conf.xml`，返回发布
/// 向导需要的网格参数（CRS/原点/瓦片尺寸/LOD 阶梯），由引擎复用 `arcgis-cache`
/// 的解析逻辑（含 DPI、瓦片对齐），忽略 ESRI 专有标记。
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
    let root = canonical_dir(&data_root)
        .map_err(|_| AppError::Param(format!("数据根目录不可访问：{data_root}")))?;
    let path = std::fs::canonicalize(raw)
        .map_err(|_| AppError::Param(format!("路径不可访问：{raw}")))?;
    if !path.starts_with(&root) {
        return Err(AppError::Param(format!(
            "路径超出工作空间数据根目录：{}（根：{}）",
            path.display(),
            root.display()
        )));
    }
    let is_xml = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("xml"))
        .unwrap_or(false);
    if path.is_file() && !is_xml {
        return Err(AppError::Param(
            "请选择 Conf.xml 文件，或该文件所在的缓存目录".to_string(),
        ));
    }
    drop(c);

    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/tile-grid",
        None,
        Some(json!({ "path": plain_abs_path(&path) })),
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

/// `DELETE /map/catalog/workspaces/:id` — 软删除；仍有 Store 或已发布服务时拒绝，
/// 避免一次误点把整个命名空间下的目录连带打进垃圾箱。
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
// 数据存储（纯指针：目录 或 PostGIS 连接）
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
    rows.first().map(row_to_store).ok_or_else(|| {
        AppError::Msg(format!("数据存储不存在：{id}"))
    })
}

/// Store 表单校验：类型合法 + 定位字段齐备（目录要路径，PostGIS 要 DSN）。
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
        // 留空 = 沿用已登记的连接串（回显只有掩码，无法原样回填）；
        // 因此这里不判空，由新增/编辑各自的调用点决定「必须有」。
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

/// 对外回显：DSN 只给掩码 + 是否已配置，绝不回传口令。
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

/// `GET /map/catalog/stores?workspaceId=` — Store 清单（含资源计数）。
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
    // 总数（不受分页影响）
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
    // 后端分页：仅当显式传 page/pageSize 时切片（pageSize<=0 视为不分页，向下兼容）
    if size > 0 {
        let off = if page < 1 { 0 } else { (page - 1) * size };
        sql.push_str(&format!(" LIMIT {size} OFFSET {off}"));
    }
    let rows = c.query(&sql, &[&ws_id, &show_deleted]).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            store_json(
                &row_to_store(r),
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
    let id: i64 = c
        .query_opt(
            "INSERT INTO sys_store (workspace_id, name, store_type, base_path, pg_dsn, pg_schema, remark) \
             VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id",
            &[
                &row.workspace_id,
                &row.name,
                &row.store_type,
                &opt_text(&row.base_path),
                &opt_text(&row.pg_dsn),
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

/// `PUT /map/catalog/stores/:id` — 编辑指针；目录/连接串变更不自动重扫，
/// 由前端提示「需重新发现」。`pgDsn` 留空表示沿用已登记的连接串
/// （对外只回显掩码，无法原样回填）。
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
                &opt_text(&row.pg_dsn),
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

/// `DELETE /map/catalog/stores/:id` — 软删除 Store：其在线服务全部停止并进垃圾箱
/// （`state='deleted'`），资源快照保留（随 Store 一起恢复），数据文件不动。
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

/// `POST /map/catalog/stores/:id/restore` — 垃圾箱恢复 Store 指针。
/// 只解自身的 `del_flag`；被级联打进垃圾箱的资源与已发布层保持原状，
/// 由各自的「恢复」按钮逐个处理（一次点击只改一类状态，失败面可控）。
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
// 连接测试与只读发现
// =====================================================================

/// `POST /map/catalog/stores/:id/connect` — 可达性测试（不落库、不改状态）。
///
/// * PostGIS → map-api `/admin/pg-test`；
/// * 目录型 → map-api `/admin/discover?depth=1`，能列出条目即为可达。
///
/// 上游失败不抛错，返回 `{ok:false, detail}`，让「测试连接」按钮拿到可读结论。
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

/// 新建数据源「测试连接」请求体：由前端表单字段拼装，后端生成 DSN 后转发引擎。
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

/// `POST /map/catalog/stores/connect` — 新建数据源时的连接测试（无需 store id）。
///
/// 前端传入 host/port/username/password/database，后端拼成 `postgres://...` DSN
/// 转发引擎 `/admin/pg-test`，返回 `{ok, detail}`。不落库、不记状态。
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
    // DSN 中的口令走请求体（不进 URL / 访问日志），与 pg-test 约定一致。
    let dsn = format!(
        "postgres://{}:{}@{}:{}/{}",
        body.username.trim(),
        body.password,
        host,
        body.port,
        database
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
    /// 目录型 Store 的递归层深（1..=4）；PostGIS 忽略。
    #[serde(default)]
    pub depth: Option<i64>,
    /// 仅 PostGIS：只发现该模式下的表（省略则用 Store 的 pg_schema）。
    #[serde(default)]
    pub schema: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportShpBody {
    /// 工作空间数据根目录内的 .shp 绝对路径（由前端目录树选出）。
    pub path: String,
    /// 导入后 PostGIS 中的表名（数据图层名）。
    pub table: String,
    /// 可选：覆盖 .prj 自动识别的 EPSG。
    #[serde(default)]
    pub srid: Option<i32>,
}

/// `POST /map/catalog/stores/:id/import-shp` — 把工作空间内的 Shapefile 导入该 PostGIS 库。
///
/// manager 只做路径/名称校验与 DSN 转发，实际导入由 map-api 调 `shp2pgsql` 完成。
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
            "shpPath": plain_abs_path(&shp),
            "table": table,
            "srid": body.srid,
        })),
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `POST /map/catalog/stores/:id/import-shp/stream` — 同一导入的 SSE 版本。
///
/// 大 Shapefile 的 shp2pgsql → COPY 可能持续几分钟，同步接口在前端看起来像卡死。
/// 这里把 map-api 的 SSE 原样透传给浏览器：`stage=start/copy/done/error`，
/// `copy` 事件带已写入行数与耗时，前端可持续刷新进度。
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
            "shpPath": plain_abs_path(&shp),
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

/// 错误也包装成 SSE 事件（HTTP 200），前端统一按 `stage=error` 处理，
/// 避免 4xx 让浏览器/RestTemplate 直接把流判为失败而丢掉错误原因。
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

/// 导入前的公共校验（同步接口与 SSE 接口共用）：Store 必须是 PostGIS、
/// 表名合法、.shp 必须落在所属工作空间的数据根目录内。
/// 返回 `(dsn, schema, table, shp 绝对路径)`。
async fn prepare_import(
    state: &AppState,
    id: i64,
    body: &ImportShpBody,
) -> AppResult<(String, String, String, std::path::PathBuf)> {
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

    // 文件必须落在该 Store 所属工作空间的数据根目录内（防目录穿越）。
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
    let root = canonical_dir(&data_root)
        .map_err(|_| AppError::Param(format!("数据根目录不可访问：{data_root}")))?;
    let raw = body.path.trim();
    if raw.is_empty() {
        return Err(AppError::Param("请选择要导入的 .shp 文件".to_string()));
    }
    if !raw.to_ascii_lowercase().ends_with(".shp") {
        return Err(AppError::Param("仅支持 .shp 矢量文件".to_string()));
    }
    let shp = std::fs::canonicalize(raw)
        .map_err(|_| AppError::Param(format!("文件不存在：{raw}")))?;
    if !shp.is_file() {
        return Err(AppError::Param(format!("不是文件：{raw}")));
    }
    if !shp.starts_with(&root) {
        return Err(AppError::Param(format!(
            "文件超出工作空间数据根目录：{}（根：{}）",
            shp.display(),
            root.display()
        )));
    }

    Ok((store.pg_dsn, store.pg_schema, table, shp))
}

/// 把 `canonicalize` 得到的 Windows verbatim 路径还原成普通绝对路径
/// （`\\?\E:\a.shp` → `E:\a.shp`，`\\?\UNC\srv\a.shp` → `\\srv\a.shp`）。
/// 非 Windows 或已是普通路径时原样返回。
fn plain_abs_path(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// PostGIS 标识符（导入表名）：小写字母/下划线开头，仅字母数字下划线，长度 ≤ 63。
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

/// `POST /map/catalog/stores/:id/discover` — 只读发现并刷新资源清单。
///
/// 引擎侧只读扫描（文件头 / 归档元数据 / `geometry_columns`），本接口把结果
/// 原样快照进 `sys_resource`（同名刷新元数据，不改 `del_flag`）。
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

/// 坐标系合理性校验：坐标范围是否落在 `src_crs` 的合理包围盒内。
/// 返回 `true` 表示「可疑」——坐标系名与坐标值明显不匹配，或坐标系缺失/未知。
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

/// 解析 `EPSG:<code>` 为整型；空 / LOCAL / UNKNOWN 返回 None（视为坐标系缺失）。
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

/// 已知坐标系的合理坐标范围候选列表 `((xmin,xmax),(ymin,ymax))`。
/// 同一坐标系可能有多种量纲语义（如 EPSG:3857 的 bbox 既可用经纬度描述，
/// 也可用米制 Web Mercator 描述），任一候选命中即视为合法。
fn expected_ranges(epsg: i32) -> Vec<((f64, f64), (f64, f64))> {
    match epsg {
        // 地理坐标（经纬度）
        4326 | 4490 => vec![((-180.0, 180.0), (-90.0, 90.0))],
        // Web Mercator：接受经纬度语义，也接受米制语义
        3857 => vec![
            ((-180.0, 180.0), (-90.0, 90.0)),
            ((-20037508.34, 20037508.34), (-20037508.34, 20037508.34)),
        ],
        // CGCS2000 高斯克吕格 3°带：EPSG = 4509 + 带号（34 带 = 4543，昆明）
        c if (4509 + 25..=4509 + 45).contains(&c) => {
            let zone = c - 4509; // 25..45
            let base = (zone as f64) * 1_000_000.0 + 500_000.0;
            vec![((base - 1_500_000.0, base + 1_500_000.0), (0.0, 5_000_000.0))]
        }
        _ => Vec::new(),
    }
}

/// 发现结果快照进 `sys_resource`：`on conflict do update` 只刷新元数据，
/// 保留用户的软删除标记（`del_flag` 不在更新列内）。
async fn upsert_resources(state: &AppState, store_id: i64, entries: &[Value]) -> AppResult<usize> {
    if entries.is_empty() {
        return Ok(0);
    }
    let c = conn(&state.pool).await?;
    // `::text::jsonb`：单写 `$9::jsonb` 会让 Postgres 把参数本身推断成 jsonb，
    // 而 tokio-postgres 未启用 serde_json feature，String 只能序列化为 text。
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
        // 坐标系缺失不再静默默认 EPSG:4326（那会掩盖「数据无坐标系」问题）：
        // 保留空值并标记可疑，发布时引导用户用 GIS 软件赋予或用坐标系树选择。
        let src_crs = str_field(e, "src_crs");
        let data_type = str_field(e, "data_type");
        // 坐标系合理性：坐标系名与坐标值范围不匹配 → 标记可疑（发布时拦截）。
        // 3dtiles / terrain 本身按 WGS 84/ECEF 规范渲染，不需要也不记录 EPSG；
        // pg_table（无 geometry）也没有坐标系语义。这三类不参与可疑校验。
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
/// 过滤优先级：storeId > workspaceId > 全量。三者均未指定（或 storeId=0）时
/// 列出全部资源，避免默认查 `store_id = 0` 永远返回空、被误判为接口故障。
pub async fn resource_list(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let store_id = query_i64(qs.as_deref(), "storeId").unwrap_or(0);
    let workspace_id = query_i64(qs.as_deref(), "workspaceId").unwrap_or(0);
    let include = query_flag(qs.as_deref(), "includeDeleted");
    let page = query_i64(qs.as_deref(), "page").unwrap_or(0);
    let size = query_i64(qs.as_deref(), "pageSize").unwrap_or(0);
    // 作用域：优先按 store；其次按 workspace；都不传则列出全部（`$1 >= 0` 恒真，
    // 仅用于占位以复用统一的参数绑定）。
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
    // 总数（不受分页影响）
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
    // 后端分页：仅当显式传 page/pageSize 时切片（pageSize<=0 视为不分页，向下兼容）
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

/// `DELETE /map/catalog/resources/:id` — 软删除（假删除仅标记）：
/// `del_flag=1` + 其已发布服务转 `state='deleted'`（r-mapserver 停止服务、保留
/// XML，进垃圾箱），不物理删文件、不调物理 DELETE。
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

/// `POST /map/catalog/resources/:id/restore` — 垃圾箱恢复：`del_flag=0`。
/// 服务侧恢复由 `/map/catalog/published/:id/state` 单独做（避免一次点击
/// 同时改两类状态却只成功一半时说不清）。
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

/// `POST /map/catalog/resources/:id/refresh` — 重新探查单个图层的元数据并刷新账本。
///
/// 数据图层是发现出来的派生记录，源文件/库表变化后靠这里单条重读：只调引擎
/// 只读探测（文件头 / 归档元数据 / `pg-tables`），不重扫整个数据源、不改发布态。
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

/// `GET /map/catalog/resources/:id/query?limit=` — PostGIS 普通表数据查询服务。
///
/// 仅对 `data_type=pg_table`（无 geometry 列的普通表）开放：加载资源所属 Store
/// 的连接串与表定位，转发引擎 `/admin/pg-query`，返回 `{columns, rows}`。
/// 空间图层走发布服务（MVT/WFS/OAF/WMS），不走此接口。
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

/// 把若干资源的已发布服务停止并打进垃圾箱（本地 `state` + 引擎登记簿同步）。
/// 返回处理的层数；单个层失败只记日志，不回滚软删除（软删除是本侧的权威意图）。
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

/// 引擎登记簿状态变更（`PUT /admin/services/:kind/:name/state`），按服务归属路由。
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
// 发布 / 修改发布信息
// =====================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishBody {
    /// 待发布资源；修改发布信息时以账本行关联的资源为准，此字段可省略。
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
    /// 瓦片网格模式：`ogc`（标准 LOD）/ `tdt`（天地图 LOD）/ `custom`（Conf.xml）。
    /// 空字符串 = 不指定，沿用引擎默认（WebMercatorQuad）。
    #[serde(default)]
    pub grid_mode: String,
    /// 网格快照（`custom` 必填，其余模式可由后端按预设生成）。结构见
    /// [`resolve_publish_grid`] 的返回说明。
    #[serde(default)]
    pub grid: Option<Value>,
}

/// 源 CRS：只接受 `EPSG:<code>`，与服务平面 `Layer.src_crs` 口径一致。
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

/// 四至：允许缺省（发布后可由引擎侧计算），给出则必须是 4 个有限值且 minX<maxX。
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

/// 从发现快照里取 bbox（引擎返回 null 时保持 None）。
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

/// 由资源 + Store 指针重建 r-mapserver 的服务文档（`POST /admin/services` 负载）。
///
/// 三种形状：3D Tiles → `<TilesetService>`（root/entry）；PostGIS →
/// `source.type=postgis` 的连接与表定位；其余 → `source.type=<kind>` + 文件路径。
/// 纯函数：不碰网络、不读库，便于单测覆盖到每个 kind。
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
        // Root 必须是**目录**（数据集根），不是 tileset.json 文件。
        // discover 可能返回目录路径（含 tileset.json 的父目录），也可能直接返回文件路径，
        // 这里统一归一：如果 source 指向 tileset.json 文件，则取其父目录作为 root。
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
        // source = schema.table（geometry_columns 发现时写入）。
        let (sch, tbl) = match res.source.split_once('.') {
            Some((s, t)) => (s.to_string(), t.to_string()),
            None => (store.pg_schema.clone(), res.source.clone()),
        };
        let geometry_column = {
            let g = str_field(&res.meta, "geometry_column");
            if g.is_empty() { "geom".to_string() } else { g }
        };
        let srid = res
            .meta
            .get("srid")
            .and_then(|v| v.as_i64())
            .or_else(|| src_crs.split(':').nth(1).and_then(|d| d.parse().ok()))
            .ok_or_else(|| {
                AppError::Msg(format!("资源 {} 缺少 SRID，请重新发现", res.name))
            })? as i32;
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
    // 瓦片网格：`grids` 引用网格 id；非预设网格（天地图 / Conf.xml）同时下发
    // `custom_grids`，引擎落盘后热重载即可按该网格出瓦片与预览。
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

/// 载入资源及其 Store（发布与修改共用；资源必须未被软删）。
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
    Ok((res, store))
}

/// ArcGIS 缓存的原生网格快照：发现时从 `Conf.xml` 读出 CRS / 瓦片尺寸 / 原点 /
/// LOD 阶梯，发布（或修改）时固化进 `sys_published_layer.grid`。运行时引擎仍以
/// 缓存自带的 `Conf.xml` 为准，这里只做账本记录与前端展示/核对。
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

/// 从 `EPSG:4490` 取数字码；非标准写法返回 `None`。
fn epsg_code(crs: &str) -> Option<u32> {
    crs.trim()
        .strip_prefix("EPSG:")
        .or_else(|| crs.trim().strip_prefix("epsg:"))
        .and_then(|s| s.trim().parse().ok())
}

/// OGC 0.28 mm/像素规则下的比例尺分母（仅供账本/前端展示，引擎以 resolution 为准）。
fn ogc_scale(resolution: f64, crs: &str) -> f64 {
    let mpu = if epsg_code(crs) == Some(3857) {
        1.0
    } else {
        111_319.490_793_273_57
    };
    resolution * mpu / 0.00028
}

/// 生成 `levels: [{level, scale, resolution}]` 展示阶梯。
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

/// 标准预设网格（WebMercatorQuad / WorldCRS84Quad / CGCS2000Quad）的账本快照。
/// 预设由引擎内置解析，`grid_def` 恒为 null。
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

/// 天地图 LOD（EPSG:4490 经纬度）：原点 [-180,90]，256px，1–18 级，
/// 分辨率 0.703125 / 2^(level-1)。属自定义网格（引擎 `custom_grids`）。
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

/// 自定义网格（ArcGIS `Conf.xml` 解析结果）→ 账本快照 + `grid_def`（供引擎
/// `custom_grids` 重建）。`src_crs` 与 XML 的 CRS 必须一致——不一致时报错，
/// 提示先转换坐标系，避免瓦片寻址与数据坐标错位。
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

/// 按发布表单解析并校验网格选择，返回要固化到 `sys_published_layer.grid` 的快照。
///
/// * ArcGIS 预切片缓存一律以 `Conf.xml` 原生网格为准（忽略表单）；
/// * `ogc`：3857→WebMercatorQuad、4326→WorldCRS84Quad、4490→CGCS2000Quad，
///   其他（投影）坐标系拒绝；
/// * `tdt`：仅 EPSG:4490；
/// * `custom`：取表单携带的 Conf.xml 解析结果，CRS 必须与 `src_crs` 一致。
/// 空模式返回 `None`，表示沿用引擎默认（WebMercatorQuad）。
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

/// 发布/改密的公共落库：upsert 已发布层 + 记录归属引擎。
/// 参数即 `sys_published_layer` 的列，逐个传比再包一层结构体更直白，因此不合并。
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
    // 归属引擎：登记走的是默认引擎，记下来供后续服务级操作按归属路由。
    let (base, _) = engine::default_engine(state).await;
    if let Some(eid) = engine::engine_id_by_base(state, base.trim_end_matches('/')).await {
        engine::record_service_engine(state, kind, layer_name, &eid).await;
    }
    Ok(json!({ "id": id, "layerName": layer_name, "kind": kind, "serviceTypes": service_types }))
}

/// `POST /map/catalog/publish` — 发现得到的 Resource → 地图服务。
///
/// 计划 §7.2：表单 → 依 `data_type` 取 `mapserver_kind` → 生成 `Layer`/`TilesetService`
/// 文档 → `POST /admin/services`（引擎侧落 XML + 热重载）→ upsert `sys_published_layer`。
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
    // 3dtiles / terrain 本身按 WGS 84/ECEF 规范渲染，不需要也不记录 EPSG，
    // 发布时跳过 CRS / 四至 / 网格 的全部校验。
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
            "资源「{}」未声明坐标系，无法发布。请先用 GIS 软件（如 QGIS）赋予正确坐标系后重新导入，\
             或在坐标系树中选择一个（WGS84 系的地理/投影坐标、CGCS2000 系的地理坐标与高斯投影）。",
            res.name
        )));
    }
    let src_crs = if is_3d {
        // 3dtiles / terrain：不校验 CRS，直接允许空串
        src_crs_in.trim().to_string()
    } else {
        validate_crs(&src_crs_in)?
    };
    let bbox = if is_3d {
        // 3dtiles / terrain：不校验四至，直接 null
        None
    } else {
        normalize_bbox(form.bbox.as_ref().or(res.bbox.as_ref()))?
    };
    // 坐标系合理性拦截：坐标系名与坐标值范围明显不匹配（或坐标系缺失）时停止发布。
    if !is_3d && crs_suspicious(&src_crs, bbox.as_ref()) {
        return Err(AppError::Msg(format!(
            "资源「{}」的坐标系（{}）与坐标值范围不匹配，疑似局部坐标或坐标系错误，已停止发布。\
             请先在 GIS 软件中核实坐标系、按真实经纬度配准后重新导入，\
             或在坐标系树中选择正确坐标系后再发布。",
            res.name, src_crs
        )));
    }
    let title = form.title.trim().to_string();
    // 网格：ArcGIS 以 Conf.xml 原生网格为准；其余按表单模式/自定义 XML 解析。
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

    // 同名服务已在引擎侧存在时拒绝（避免覆盖别人的既有配置）。
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

/// 服务名占用检查：账本里有未删除的同名行就拒绝（垃圾箱里的同名行允许复用，
/// 由 `record_published` 就地复活，与引擎侧「重新登记同名服务」保持一致）。
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

// ---- 已发布服务账本 ----

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
    /// 发布时固化的原生网格快照（ArcGIS 缓存：CRS / 瓦片尺寸 / 原点 / LOD）。
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

/// `GET /map/catalog/published?workspaceId=&includeDeleted=1` — 已发布服务清单
/// （工作区/Store/资源归属齐备，供分组视图与垃圾箱）。
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
    // 总数（不受分页影响）
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
    // 后端分页：仅当显式传 page/pageSize 时切片（pageSize<=0 视为不分页，向下兼容）
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

/// `PUT /map/catalog/published/:id` — 修改发布信息（计划 §7.4）。
///
/// 重建同一资源的服务文档并覆盖登记（`POST /admin/services` 对同名文件是幂等
/// 覆盖 + 热重载），因此改名走 `rename` 再覆盖，避免留下两份 XML。
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
    // ArcGIS 缓存同样以 Conf.xml 的 CRS 为准，修改时也不接受表单覆盖。
    let native_crs = res
        .meta
        .get("crs")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    // 仅 ArcGIS 缓存以 Conf.xml 的 CRS 为准（只读）；其他类型沿用发布向导的输入。
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
    // 网格：表单显式选择时按新值重建；未提交（仅改标题/备注等）沿用上次快照。
    // ArcGIS 始终以 Conf.xml 的原生网格为准。
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

    // 改名：先让引擎换名（同时保持配置文件内部标识一致），再按新名覆盖内容。
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
    // 改名后归属登记要跟着新名走。
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

/// 上游失败时补一句本地上下文，便于运维直接定位是哪个资源/文件出了问题。
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

/// `PUT /map/catalog/published/:id/state` — 生命周期状态（active|stopped|paused|deleted）。
///
/// 与 `/map/services/:kind/:name/state` 同源，但一并维护目录账本：置 `active`
/// 时顺手把被软删的资源恢复回目录（垃圾箱「恢复」即此动作）。
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

/// `GET /map/catalog/published/:id/endpoints` — 单个已发布服务的协议端点（预览、一键复制）。
///
/// 端点 URL 全部由引擎给出（[`engine::aggregate_services`] 聚合各引擎
/// `/admin/services`），r-manager 只按服务名挑出对应条目，不自行拼接，
/// 避免与「服务管理」页两处口径漂移。引擎目录里没有该服务（配置被外部删除、
/// 或引擎换了目录）时返回 `found=false` + 原因，而不是报错。
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
// 小工具：查询串解析与编码（与 engine.rs 同口径，手写不加依赖）
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

/// 空串按 NULL 存（目录型 Store 没有 DSN，反之亦然）。
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

/// 最小 percent-encode：只保留 unreserved 字符，其余按 UTF-8 逐字节转 `%XX`。
/// 目录路径里的 `\`、`:`、中文都必须编码后才能安全进查询串。
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

/// 最小 percent-decode（`+` 视空格，非法序列原样保留）。
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
        // PostGIS 不写 extent：引擎启动时按 ST_Extent 计算。
        assert!(body.get("extent").is_none());
        // 标题缺省回落资源名。
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
        let body = build_service_body(
            &res("3dtiles", r"D:\data\city", json!({})),
            &store("3dtiles_dir", r"D:\data", ""),
            &cap("3dtiles", &["3dtiles"], &["3dtiles"]),
            "city",
            "城市模型",
            "EPSG:4326",
            None,
            None,
        )
        .unwrap();
        assert_eq!(body["kind"], json!("3dtiles"));
        assert_eq!(body["root"], json!(r"D:\data\city"));
        // TilesetService 文档没有 src_crs / source 字段（引擎只读 Name/Title/Root/Entry）。
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
