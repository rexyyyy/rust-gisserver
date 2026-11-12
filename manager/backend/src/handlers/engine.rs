//! 服务引擎管理：多引擎注册表（sys_engine）+ 健康探测 + 默认引擎解析。
//!
//! 结构上支持多个地图服务引擎（当前仅 r-mapserver `map-api` 一种）：
//! * 引擎登记在 `sys_engine` 表（engine_id 主键、base_url、可选 admin_token）；
//! * 「默认引擎」（is_default='1' 且 status='0'）供管理代理转发使用；
//!   无默认引擎时回落 `manager.xml` 的 `mapsvc.base_url`（兼容旧部署）；
//! * 健康探测打引擎的 `/admin/components`（map-api）或 `/health`（通用）。
use std::sync::Arc;

use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::resp::Resp;
use crate::state::{conn, AppState};

/// 引擎域路由（由 `handlers::router` 统一装配）。
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // 服务引擎管理（多引擎注册表；静态路径在前）
        .route("/map/engines", get(list).post(add))
        // 引擎事件日志（静态路径，先于 :engineId 通配）
        .route("/map/engines/events", get(events))
        .route("/map/engines/{engineId}/health", get(health))
        .route("/map/engines/{engineId}/detail", get(detail))
        .route("/map/engines/{engineId}/default", put(set_default))
        .route("/map/engines/{engineId}", put(update).delete(remove))
        // 服务归属引擎（5 段，与 /state、/check 同段数的字面量末段）
        .route(
            "/map/services/{kind}/{name}/engine",
            get(service_engine_get)
                .put(service_engine_assign)
                .delete(service_engine_clear),
        )
}

/// 引擎行（DB ↔ VO）。
#[derive(Debug, Clone)]
pub struct EngineRow {
    pub engine_id: String,
    pub engine_name: String,
    pub engine_type: String,
    pub base_url: String,
    pub admin_token: Option<String>,
    pub status: String,
    pub is_default: String,
    pub sort: i32,
    pub remark: Option<String>,
}

const ENGINE_COLS: &str =
    "engine_id, engine_name, engine_type, base_url, admin_token, status, is_default, sort, remark";

fn row_to_engine(row: &tokio_postgres::Row) -> EngineRow {
    EngineRow {
        engine_id: row.get("engine_id"),
        engine_name: row.get("engine_name"),
        engine_type: row.get("engine_type"),
        base_url: row.get("base_url"),
        admin_token: row.get("admin_token"),
        status: row.get("status"),
        is_default: row.get("is_default"),
        sort: row.get("sort"),
        remark: row.get("remark"),
    }
}

fn engine_json(e: &EngineRow, with_token: bool) -> Value {
    json!({
        "engineId": e.engine_id,
        "engineName": e.engine_name,
        "engineType": e.engine_type,
        "baseUrl": e.base_url,
        // 管理代理在服务端持有 token，不回传前端（仅返回是否已配置）。
        "tokenConfigured": e.admin_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false),
        "adminToken": if with_token { e.admin_token.clone().unwrap_or_default() } else { String::new() },
        "status": e.status,
        "isDefault": e.is_default,
        "sort": e.sort,
        "remark": e.remark.clone().unwrap_or_default(),
    })
}

/// 查全部引擎（按 sort、engine_id 排序）。
async fn list_rows(state: &AppState) -> AppResult<Vec<EngineRow>> {
    let sql = format!(
        "SELECT {ENGINE_COLS} FROM sys_engine ORDER BY sort, engine_id"
    );
    let c = conn(&state.pool).await?;
    let rows = c.query(&sql, &[]).await?;
    Ok(rows.iter().map(row_to_engine).collect())
}

/// 解析默认引擎的 (base_url, admin_token)：DB `is_default='1' AND status='0'`
/// 优先（按 sort 取最小），否则回落 manager.xml `mapsvc`。供管理代理调用。
pub async fn default_engine(state: &AppState) -> (String, Option<String>) {
    let sql = format!(
        "SELECT {ENGINE_COLS} FROM sys_engine \
         WHERE is_default = '1' AND status = '0' ORDER BY sort, engine_id LIMIT 1"
    );
    if let Ok(c) = conn(&state.pool).await {
        if let Ok(rows) = c.query(&sql, &[]).await {
            if let Some(row) = rows.first() {
                let e = row_to_engine(row);
                let token = e
                    .admin_token
                    .filter(|t| !t.is_empty())
                    .or_else(|| state.cfg.mapsvc.token.clone());
                return (e.base_url, token);
            }
        }
    }
    (state.cfg.mapsvc.base_url.clone(), state.cfg.mapsvc.token.clone())
}

/// `GET /map/engines` — 引擎清单。
pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let rows = list_rows(&state).await?;
    let engines: Vec<Value> = rows.iter().map(engine_json_vo).collect();
    Ok(Resp::ok(json!({ "engines": engines })))
}

fn engine_json_vo(e: &EngineRow) -> Value {
    engine_json(e, false)
}

/// `GET /map/engines/:engineId/health` — 探测引擎可达性（/admin/components，
/// 失败再试 /health），返回 { up, latencyMs, detail }。
pub async fn health(
    State(state): State<Arc<AppState>>,
    AxumPath(engine_id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let rows = list_rows(&state).await?;
    let e = rows
        .iter()
        .find(|r| r.engine_id == engine_id)
        .ok_or_else(|| AppError::Msg(format!("引擎不存在：{engine_id}")))?;
    Ok(Resp::ok(probe(&state, e).await))
}

/// 对单个引擎做健康探测。
pub async fn probe(state: &AppState, e: &EngineRow) -> Value {
    let base = e.base_url.trim_end_matches('/').to_string();
    let started = std::time::Instant::now();
    let mut req = state.map_http.get(format!("{base}/admin/components"));
    if let Some(tok) = e.admin_token.as_deref().filter(|t| !t.is_empty()) {
        req = req.header("X-Admin-Token", tok);
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => json!({
            "up": true,
            "latencyMs": started.elapsed().as_millis() as u64,
            "detail": "/admin/components OK",
        }),
        Ok(_) => {
            // 旧版引擎无 /admin/components 时退回 /health。
            match state.map_http.get(format!("{base}/health")).send().await {
                Ok(r2) if r2.status().is_success() => json!({
                    "up": true,
                    "latencyMs": started.elapsed().as_millis() as u64,
                    "detail": "/health OK（无 admin 面）",
                }),
                _ => json!({
                    "up": false,
                    "latencyMs": started.elapsed().as_millis() as u64,
                    "detail": format!("引擎不可达（{base}）"),
                }),
            }
        }
        Err(err) => json!({
            "up": false,
            "latencyMs": started.elapsed().as_millis() as u64,
            "detail": format!("引擎不可达（{base}）：{err}"),
        }),
    }
}

/// 引擎新增/编辑请求体（前端 camelCase 契约）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBody {
    // 编辑场景以 URL 路径为准，body 可省略
    #[serde(default)]
    pub engine_id: String,
    pub engine_name: String,
    #[serde(default)]
    pub engine_type: String,
    pub base_url: String,
    #[serde(default)]
    pub admin_token: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub is_default: String,
    #[serde(default)]
    pub sort: i32,
    #[serde(default)]
    pub remark: String,
}

fn validate_body(b: &EngineBody) -> Result<(), AppError> {
    let id = b.engine_id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(AppError::Msg(
            "engine_id 仅允许字母/数字/-/_ 且非空".into(),
        ));
    }
    if b.engine_name.trim().is_empty() {
        return Err(AppError::Msg("engine_name 不能为空".into()));
    }
    if !b.base_url.starts_with("http://") && !b.base_url.starts_with("https://") {
        return Err(AppError::Msg("base_url 必须以 http:// 或 https:// 开头".into()));
    }
    Ok(())
}

/// `POST /map/engines` — 新增引擎；is_default='1' 时清除其他默认。
pub async fn add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<EngineBody>,
) -> AppResult<Json<Resp<Value>>> {
    validate_body(&body)?;
    let mut b = body;
    b.engine_id = b.engine_id.trim().to_string();
    if b.engine_type.trim().is_empty() {
        b.engine_type = "r-mapserver".into();
    }
    if b.status.is_empty() {
        b.status = "0".into();
    }
    if b.is_default.is_empty() {
        b.is_default = "0".into();
    }
    let c = conn(&state.pool).await?;
    let exists: i64 = c
        .query_one("SELECT count(*) FROM sys_engine WHERE engine_id = $1", &[&b.engine_id])
        .await?
        .get(0);
    if exists > 0 {
        return Err(AppError::Msg(format!("引擎 ID 已存在：{}", b.engine_id)));
    }
    if b.is_default == "1" {
        c.execute("UPDATE sys_engine SET is_default = '0' WHERE is_default = '1'", &[])
            .await?;
    }
    c.execute(
        "INSERT INTO sys_engine (engine_id, engine_name, engine_type, base_url, admin_token, \
         status, is_default, sort, remark) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        &[
            &b.engine_id,
            &b.engine_name,
            &b.engine_type,
            &b.base_url,
            &b.admin_token,
            &b.status,
            &b.is_default,
            &b.sort,
            &b.remark,
        ],
    )
    .await?;
    Ok(Resp::ok(json!({ "engineId": b.engine_id })))
}

/// `PUT /map/engines/:engineId` — 更新引擎；is_default='1' 时清除其他默认。
pub async fn update(
    State(state): State<Arc<AppState>>,
    AxumPath(engine_id): AxumPath<String>,
    Json(body): Json<EngineBody>,
) -> AppResult<Json<Resp<Value>>> {
    let mut b = body;
    // URL 中的 engineId 为准（body 里的 engineId 可省略，编辑页不回传）。
    b.engine_id = engine_id;
    validate_body(&b)?;
    if b.status.is_empty() {
        b.status = "0".into();
    }
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_engine SET engine_name=$2, engine_type=$3, base_url=$4, admin_token=$5, \
             status=$6, is_default=$7, sort=$8, remark=$9, update_time=now() WHERE engine_id=$1",
            &[
                &b.engine_id,
                &b.engine_name,
                &b.engine_type,
                &b.base_url,
                &b.admin_token,
                &b.status,
                &b.is_default,
                &b.sort,
                &b.remark,
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("引擎不存在：{}", b.engine_id)));
    }
    if b.is_default == "1" {
        c.execute(
            "UPDATE sys_engine SET is_default = '0' WHERE is_default = '1' AND engine_id <> $1",
            &[&b.engine_id],
        )
        .await?;
    }
    Ok(Resp::ok(json!({ "engineId": b.engine_id })))
}

/// `DELETE /map/engines/:engineId` — 删除引擎（默认引擎删除后代理回落 manager.xml）。
pub async fn remove(
    State(state): State<Arc<AppState>>,
    AxumPath(engine_id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let n = c
        .execute("DELETE FROM sys_engine WHERE engine_id = $1", &[&engine_id])
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("引擎不存在：{engine_id}")));
    }
    Ok(Resp::ok(json!({ "engineId": engine_id })))
}

/// `PUT /map/engines/:engineId/default` — 设为默认引擎（并启用）。
pub async fn set_default(
    State(state): State<Arc<AppState>>,
    AxumPath(engine_id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let n = c
        .execute(
            "UPDATE sys_engine SET is_default = '1', status = '0', update_time = now() \
             WHERE engine_id = $1",
            &[&engine_id],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Msg(format!("引擎不存在：{engine_id}")));
    }
    c.execute(
        "UPDATE sys_engine SET is_default = '0' WHERE is_default = '1' AND engine_id <> $1",
        &[&engine_id],
    )
    .await?;
    Ok(Resp::ok(json!({ "engineId": engine_id })))
}

// =====================================================================
// 服务与引擎的归属关系（第二阶段）
// =====================================================================

/// 查服务当前归属的引擎行（sys_service_engine → sys_engine）。
/// 归属行存在但引擎已被删除时返回 None（按未归属处理）。
async fn service_engine_row(state: &AppState, kind: &str, name: &str) -> Option<EngineRow> {
    // 注意：engine_id 在 sys_engine 与 sys_service_engine 同名，
    // SELECT 列必须带 e. 前缀，否则 PG 报「字段关联不明确」（错误被 ok()? 吞掉会误判为未归属）。
    let sql = "SELECT e.engine_id, e.engine_name, e.engine_type, e.base_url, e.admin_token, \
               e.status, e.is_default, e.sort, e.remark \
               FROM sys_engine e \
               JOIN sys_service_engine se ON se.engine_id = e.engine_id \
               WHERE se.kind = $1 AND se.name = $2 LIMIT 1";
    let c = conn(&state.pool).await.ok()?;
    let rows = c.query(sql, &[&kind, &name]).await.ok()?;
    rows.first().map(row_to_engine)
}

/// 服务级路由解析：归属引擎（启用中）优先，未归属/引擎停用回落默认引擎。
pub async fn resolve_for_service(
    state: &AppState,
    kind: &str,
    name: &str,
) -> (String, Option<String>) {
    if let Some(e) = service_engine_row(state, kind, name).await {
        if e.status == "0" {
            let token = e
                .admin_token
                .filter(|t| !t.is_empty())
                .or_else(|| state.cfg.mapsvc.token.clone());
            return (e.base_url, token);
        }
    }
    default_engine(state).await
}

/// `GET /map/services/:kind/:name/engine` — 查服务归属。
pub async fn service_engine_get(
    State(state): State<Arc<AppState>>,
    AxumPath((kind, name)): AxumPath<(String, String)>,
) -> AppResult<Json<Resp<Value>>> {
    let e = service_engine_row(&state, &kind, &name).await;
    Ok(Resp::ok(json!({
        "kind": kind,
        "name": name,
        "engineId": e.as_ref().map(|r| r.engine_id.clone()),
        "engineName": e.as_ref().map(|r| r.engine_name.clone()),
        // 前端展示用：未登记归属显示「默认引擎」
        "effective": e.as_ref().map(|r| r.engine_id.clone()),
    })))
}

/// `PUT /map/services/:kind/:name/engine` — 调整服务归属（body: {engineId}）。
/// 引擎必须存在；置为空串/省略等同清除归属（回落默认引擎）。
pub async fn service_engine_assign(
    State(state): State<Arc<AppState>>,
    AxumPath((kind, name)): AxumPath<(String, String)>,
    body: Option<Json<Value>>,
) -> AppResult<Json<Resp<Value>>> {
    let engine_id = body
        .as_ref()
        .and_then(|Json(v)| v.get("engineId"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let c = conn(&state.pool).await?;
    if engine_id.is_empty() {
        // 清除归属
        c.execute(
            "DELETE FROM sys_service_engine WHERE kind = $1 AND name = $2",
            &[&kind, &name],
        )
        .await?;
        return Ok(Resp::ok(json!({ "kind": kind, "name": name, "engineId": null })));
    }
    let exists: i64 = c
        .query_one(
            "SELECT count(*) FROM sys_engine WHERE engine_id = $1",
            &[&engine_id],
        )
        .await?
        .get(0);
    if exists == 0 {
        return Err(AppError::Msg(format!("引擎不存在：{engine_id}")));
    }
    c.execute(
        "INSERT INTO sys_service_engine (kind, name, engine_id) VALUES ($1,$2,$3) \
         ON CONFLICT (kind, name) DO UPDATE SET engine_id = EXCLUDED.engine_id, \
         update_time = now()",
        &[&kind, &name, &engine_id],
    )
    .await?;
    Ok(Resp::ok(json!({ "kind": kind, "name": name, "engineId": engine_id })))
}

/// `DELETE /map/services/:kind/:name/engine` — 清除归属（回落默认引擎）。
pub async fn service_engine_clear(
    State(state): State<Arc<AppState>>,
    AxumPath((kind, name)): AxumPath<(String, String)>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    c.execute(
        "DELETE FROM sys_service_engine WHERE kind = $1 AND name = $2",
        &[&kind, &name],
    )
    .await?;
    Ok(Resp::ok(json!({ "kind": kind, "name": name, "engineId": null })))
}

/// 记录服务归属（登记服务成功后调用；失败不阻断主流程，仅日志）。
pub async fn record_service_engine(state: &AppState, kind: &str, name: &str, engine_id: &str) {
    if let Ok(c) = conn(&state.pool).await {
        let _ = c
            .execute(
                "INSERT INTO sys_service_engine (kind, name, engine_id) VALUES ($1,$2,$3) \
                 ON CONFLICT (kind, name) DO UPDATE SET engine_id = EXCLUDED.engine_id, \
                 update_time = now()",
                &[&kind, &name, &engine_id],
            )
            .await;
    }
}

/// 按 engineId 取启用的引擎行；找不到或停用返回 None。
pub async fn enabled_engine_by_id(state: &AppState, engine_id: &str) -> Option<EngineRow> {
    let rows = list_rows(state).await.ok()?;
    rows.into_iter()
        .find(|r| r.engine_id == engine_id && r.status == "0")
}

/// 按 base_url 反查引擎 ID（登记服务未显式指定引擎时记录归属用）。
/// 规整尾斜杠后比较；找不到返回 None。
pub async fn engine_id_by_base(state: &AppState, base: &str) -> Option<String> {
    let rows = list_rows(state).await.ok()?;
    let want = base.trim_end_matches('/');
    rows.into_iter()
        .find(|r| r.base_url.trim_end_matches('/') == want)
        .map(|r| r.engine_id)
}

/// 取所有启用（status='0'）的引擎行。供后台巡检与聚合使用。
pub async fn enabled_rows(state: &AppState) -> Vec<EngineRow> {
    match list_rows(state).await {
        Ok(rows) => rows.into_iter().filter(|r| r.status == "0").collect(),
        Err(_) => Vec::new(),
    }
}

// =====================================================================
// 引擎监控：事件日志 + 引擎详情（第三阶段）
// =====================================================================

/// 写引擎事件（尽力而为：失败仅记日志，不阻断巡检/请求）。
pub async fn log_event(state: &AppState, engine_id: &str, event_type: &str, summary: &str, detail: &str) {
    if let Ok(c) = conn(&state.pool).await {
        if let Err(e) = c
            .execute(
                "INSERT INTO sys_engine_event (engine_id, event_type, summary, detail) \
                 VALUES ($1,$2,$3,$4)",
                &[&engine_id, &event_type, &summary, &detail],
            )
            .await
        {
            tracing::warn!("写引擎事件失败（{engine_id}/{event_type}）：{e}");
        }
    }
}

/// `GET /map/engines/events?engineId=&eventType=&limit=` — 事件日志查询。
pub async fn events(State(state): State<Arc<AppState>>, RawQuery(qs): RawQuery) -> AppResult<Json<Resp<Value>>> {
    // 平铺 k=v&k=v 解析（手写，避免新增依赖）
    let pairs: Vec<(String, String)> = qs
        .as_deref()
        .unwrap_or("")
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (url_decode(k), url_decode(v))
        })
        .collect();
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    let engine_id = get("engineId").filter(|s| !s.is_empty());
    let event_type = get("eventType").filter(|s| !s.is_empty());
    let limit: usize = get("limit").and_then(|v| v.parse().ok()).unwrap_or(100).clamp(1, 500);

    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    if let Some(e) = engine_id {
        params.push(e);
        clauses.push(format!("engine_id = ${}", params.len()));
    }
    if let Some(t) = event_type {
        params.push(t);
        clauses.push(format!("event_type = ${}", params.len()));
    }
    // LIMIT 必须以整型绑定（String 会让 PG 推断为 text，序列化失败）
    let limit_i = limit as i64;
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT id, engine_id, event_type, summary, detail, \
         to_char(create_time, 'YYYY-MM-DD HH24:MI:SS') AS create_time \
         FROM sys_engine_event {where_sql} ORDER BY id DESC LIMIT ${}",
        params.len() + 1
    );
    let c = conn(&state.pool).await?;
    let mut refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        params.iter().map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
    refs.push(&limit_i);
    let rows = c.query(&sql, &refs).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, i64>("id"),
                "engineId": r.get::<_, String>("engine_id"),
                "eventType": r.get::<_, String>("event_type"),
                "summary": r.get::<_, String>("summary"),
                "detail": r.get::<_, String>("detail"),
                "createTime": r.get::<_, String>("create_time"),
            })
        })
        .collect();
    Ok(Resp::ok(json!({ "events": items, "total": items.len() })))
}

/// `GET /map/engines/:engineId/detail` — 引擎详情：基本信息 + 存活探测 +
/// 服务器运行环境（/admin/components）+ 服务统计 + 最近事件。
pub async fn detail(
    State(state): State<Arc<AppState>>,
    AxumPath(engine_id): AxumPath<String>,
) -> AppResult<Json<Resp<Value>>> {
    let rows = list_rows(&state).await?;
    let e = rows
        .iter()
        .find(|r| r.engine_id == engine_id)
        .ok_or_else(|| AppError::Msg(format!("引擎不存在：{engine_id}")))?;

    // 存活探测 + 服务清单（尽力而为，失败不阻断详情）
    let health = probe(&state, e).await;
    let base = e.base_url.trim_end_matches('/');
    let token = e.admin_token.as_deref().filter(|t| !t.is_empty());
    let components = fetch_components(&state, base, token).await.ok();
    let svc = fetch_services_of(&state, base, token).await.ok();
    let (svc_total, svc_serving) = svc
        .as_ref()
        .and_then(|v| v.get("services"))
        .and_then(|v| v.as_array())
        .map(|list| {
            let serving = list
                .iter()
                .filter(|s| s.get("serving").and_then(|v| v.as_bool()) == Some(true))
                .count();
            (list.len(), serving)
        })
        .unwrap_or((0, 0));

    // 最近 20 条事件
    let recent = recent_events(&state, &engine_id, 20).await;

    Ok(Resp::ok(json!({
        "engine": engine_json(e, false),
        "health": health,
        "components": components,
        "serviceTotal": svc_total,
        "serviceServing": svc_serving,
        "recentEvents": recent,
    })))
}

/// 拉引擎 `/admin/components`（服务器运行环境）。
async fn fetch_components(state: &AppState, base: &str, token: Option<&str>) -> AppResult<Value> {
    let mut req = state.map_http.get(format!("{base}/admin/components"));
    if let Some(tok) = token {
        req = req.header("X-Admin-Token", tok);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Msg(format!("map-api 不可达（{base}）：{e}")))?;
    let status = resp.status();
    let val: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Msg(format!("map-api 返回非 JSON（HTTP {status}）：{e}")))?;
    if !status.is_success() {
        return Err(AppError::Msg(format!("map-api HTTP {status}")));
    }
    Ok(val)
}

/// 最近 N 条事件（引擎详情用）。
async fn recent_events(state: &AppState, engine_id: &str, limit: usize) -> Vec<Value> {
    let sql = "SELECT event_type, summary, detail, \
               to_char(create_time, 'YYYY-MM-DD HH24:MI:SS') AS create_time \
               FROM sys_engine_event WHERE engine_id = $1 ORDER BY id DESC LIMIT $2";
    let Ok(c) = conn(&state.pool).await else { return Vec::new() };
    match c.query(sql, &[&engine_id, &(limit as i64)]).await {
        Ok(rows) => rows
            .iter()
            .map(|r| {
                json!({
                    "eventType": r.get::<_, String>("event_type"),
                    "summary": r.get::<_, String>("summary"),
                    "detail": r.get::<_, String>("detail"),
                    "createTime": r.get::<_, String>("create_time"),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// 手写 percent-decode（`+` 视为空格，%XX 十六进制解码；非法序列原样保留）。
fn url_decode(s: &str) -> String {
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

/// 跨引擎聚合服务清单：逐个启用引擎拉 `/admin/services`，合并并标注归属。
///
/// 返回 `{ services: [...每项附 engineId/engineName...], count, engines: [...每引擎 up/count...] }`。
/// 全部引擎都不可达时返回错误；部分可达时正常返回（不可达引擎在 engines 里标注）。
pub async fn aggregate_services(state: &AppState) -> AppResult<Value> {
    let rows = list_rows(state).await?;
    let enabled: Vec<&EngineRow> = rows.iter().filter(|r| r.status == "0").collect();
    // 服务分组映射（sys_service_meta；未登记 = 根目录 ""）
    let group_map = service_group_map(state).await;
    if enabled.is_empty() {
        // 无启用引擎：回落 manager.xml 配置（旧行为）
        let (base, token) = default_engine(state).await;
        return fetch_services_of(state, &base, token.as_deref()).await;
    }

    let mut merged: Vec<Value> = Vec::new();
    let mut engine_stats: Vec<Value> = Vec::new();
    let mut any_up = false;
    for e in &enabled {
        let (data, up) = match fetch_services_of(
            state,
            e.base_url.trim_end_matches('/'),
            e.admin_token.as_deref().filter(|t| !t.is_empty()),
        )
        .await
        {
            Ok(d) => (d, true),
            Err(_) => (json!(null), false),
        };
        let mut count = 0usize;
        if up {
            any_up = true;
            if let Some(list) = data.get("services").and_then(|v| v.as_array()) {
                count = list.len();
                let mut items = list.clone();
                for it in items.iter_mut() {
                    it["engineId"] = json!(e.engine_id);
                    it["engineName"] = json!(e.engine_name);
                    let kind = it.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let name = it.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let group = group_map
                        .get(&(kind.to_string(), name.to_string()))
                        .cloned()
                        .unwrap_or_default();
                    it["group"] = json!(group);
                }
                merged.append(&mut items);
            }
        }
        engine_stats.push(json!({
            "engineId": e.engine_id,
            "engineName": e.engine_name,
            "up": up,
            "count": count,
        }));
    }
    if !any_up {
        return Err(AppError::Msg("所有服务引擎均不可达".into()));
    }
    Ok(json!({
        "services": merged,
        "count": merged.len(),
        "engines": engine_stats,
    }))
}

/// 服务分组映射：(kind, name) -> group。聚合清单时附加 group 字段。
async fn service_group_map(state: &AppState) -> std::collections::HashMap<(String, String), String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(c) = conn(&state.pool).await {
        if let Ok(rows) = c
            .query("SELECT kind, name, service_group FROM sys_service_meta", &[])
            .await
        {
            for r in &rows {
                map.insert((r.get(0), r.get(1)), r.get(2));
            }
        }
    }
    map
}

/// 拉单个引擎的服务清单（map-api `/admin/services`）。
pub async fn fetch_services_of(
    state: &AppState,
    base: &str,
    token: Option<&str>,
) -> AppResult<Value> {
    let mut req = state.map_http.get(format!("{base}/admin/services"));
    if let Some(tok) = token {
        req = req.header("X-Admin-Token", tok);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Msg(format!("map-api 服务不可达（{base}）：{e}")))?;
    let status = resp.status();
    let val: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Msg(format!("map-api 返回非 JSON（HTTP {status}）：{e}")))?;
    if !status.is_success() {
        let msg = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("map-api 返回错误");
        return Err(AppError::Msg(format!("map-api HTTP {status}: {msg}")));
    }
    Ok(val)
}
