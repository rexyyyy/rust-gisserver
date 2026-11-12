//! 地图服务管理代理：把前端对 map-api 管理面的访问统一转发到
//! r-mapserver `map-api` 服务的 `/admin/*` 与 `/api/usage`。
//!
//! 设计要点：
//! * r-manager 后端是 map-api admin 面的**唯一**调用方（内网部署）；
//!   可选 `mapsvc.token` 与 map-api `admin.token` 一致（X-Admin-Token 头）。
//! * 上游返回 JSON 原样塞进 `data` 字段（统一 `{code,msg,data}` 契约）；
//!   上游非 2xx 时转为 `AppError::Msg`（HTTP 200 + code 500，前端拦截器弹 msg）。
//! * map-api 停机时给出明确中文提示，不裸抛连接错误。
//! * 二进制端点（样式图例 PNG）走 `upstream_bytes` 直通，不包 JSON 契约。
use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::handlers::catalog::{query_i64, qs_decode};

/// 从原始查询串里取一个字符串参数（大小写不敏感；无值返回 None）。
///
/// 键与值都按 form 规则 percent-decode（`%2C`→`,`、`%E7%93%A6`→「瓦」），与
/// `catalog::query_pairs` 同口径：前端 axios 会把逗号/中文编码后发出，不解码则
/// `kind=pmtiles,mbtiles` 这类多值过滤与中文名称检索一律匹配不到。
fn qs_param(qs: Option<&str>, key: &str) -> Option<String> {
    let qs = qs?;
    let key = key.to_ascii_lowercase();
    for kv in qs.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if qs_decode(k.trim()).to_ascii_lowercase() == key {
            return Some(qs_decode(v.trim()));
        }
    }
    None
}
use crate::resp::Resp;
use crate::state::{conn, AppState};

/// 地图服务域路由（由 `handlers::router` 统一装配）。
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // 地图服务管理（代理 map-api 管理面）
        .route("/map/services", get(services))
        .route(
            "/map/services/{kind}/{name}",
            get(service_config).put(service_update),
        )
        .route("/map/services/{kind}/{name}/state", put(set_state))
        // 服务分组（两级命名空间）
        .route("/map/service-groups", get(service_groups))
        .route(
            "/map/services/{kind}/{name}/group",
            get(service_group_get).put(service_group_put),
        )
        .route("/map/reload", post(reload))
        .route("/map/usage", get(usage))
        .route("/map/logs", get(logs))
        .route("/map/services/{kind}/{name}/check", post(check_service))
        // 新增服务向导 / 运营仪表盘
        .route("/map/browse", get(browse))
        .route("/map/services", post(create_service))
        .route("/map/pg-test", post(pg_test))
        .route("/map/components", get(components))
        .route("/map/runtime", get(runtime))
        .route("/map/cache", get(cache_stats))
        .route("/map/cache/purge", post(cache_purge))
        // 样式库（SLD）：静态路径在前
        .route("/map/styles", get(style_list))
        .route("/map/styles/{name}/content", get(style_content))
        .route("/map/styles/{name}/legend.png", get(style_legend))
        .route("/map/styles/{name}", post(style_upload).delete(style_remove))
        .route("/map/health", get(health))
}

/// 向 map-api 发请求并解析 JSON（走默认引擎）。供 `/map/*` 与数据目录代理共用。
pub(crate) async fn upstream_json(
    state: &AppState,
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    body: Option<Value>,
) -> AppResult<Value> {
    let engine = super::engine::default_engine(state).await;
    upstream_json_engine(state, &engine, method, path, query, body).await
}

/// 向指定引擎 `(base_url, admin_token)` 发请求并解析 JSON。
///
/// 多引擎第二阶段：服务级操作（配置读写/启停/体检）按服务归属路由到
/// `engine::resolve_for_service`，全局操作仍走默认引擎。目录面的发布与状态变更
/// 同样按归属路由，因此对 crate 内可见。
pub(crate) async fn upstream_json_engine(
    state: &AppState,
    engine: &(String, Option<String>),
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    body: Option<Value>,
) -> AppResult<Value> {
    let (base_raw, token) = engine;
    let base = base_raw.trim_end_matches('/');
    let mut url = format!("{base}{path}");
    if let Some(q) = query {
        if !q.is_empty() {
            url.push('?');
            url.push_str(q);
        }
    }
    let mut req = state.map_http.request(method, &url);
    if let Some(tok) = token {
        req = req.header("X-Admin-Token", tok);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| {
        AppError::Msg(format!("map-api 服务不可达（{base}）：{e}"))
    })?;
    let status = resp.status();
    let val: Value = resp.json().await.map_err(|e| {
        AppError::Msg(format!("map-api 返回非 JSON（HTTP {status}）：{e}"))
    })?;
    if !status.is_success() {
        let msg = val
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("map-api 返回错误");
        return Err(AppError::Msg(format!("map-api HTTP {status}: {msg}")));
    }
    Ok(val)
}

/// 向 map-api 取二进制负载（PNG 图例等），返回 `(bytes, content_type)`。
async fn upstream_bytes(
    state: &AppState,
    path: &str,
    query: Option<&str>,
) -> AppResult<(Vec<u8>, String)> {
    let (base_raw, token) = super::engine::default_engine(&state).await;
    let base = base_raw.trim_end_matches('/');
    let mut url = format!("{base}{path}");
    if let Some(q) = query {
        if !q.is_empty() {
            url.push('?');
            url.push_str(q);
        }
    }
    let mut req = state.map_http.get(&url);
    if let Some(tok) = &token {
        req = req.header("X-Admin-Token", tok);
    }
    let resp = req.send().await.map_err(|e| {
        AppError::Msg(format!("map-api 服务不可达（{base}）：{e}"))
    })?;
    let status = resp.status();
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = resp.bytes().await.map_err(|e| {
        AppError::Msg(format!("map-api 返回读取失败（HTTP {status}）：{e}"))
    })?;
    if !status.is_success() {
        let msg = String::from_utf8_lossy(&bytes);
        return Err(AppError::Msg(format!("map-api HTTP {status}: {msg}")));
    }
    Ok((bytes.to_vec(), ct))
}

/// `GET /map/services?page=&pageSize=&kind=&state=&name=` — 服务清单（跨引擎聚合后
/// 在 manager 端按 kind/state/name 过滤，再按 page/pageSize 切片，满足前端“后端分页”契约）。
/// `kind` 支持逗号多值（如 `pmtiles,mbtiles`，类型拆分菜单页的 presetKinds 透传）。
pub async fn services(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let page = query_i64(qs.as_deref(), "page").unwrap_or(0);
    let size = query_i64(qs.as_deref(), "pageSize").unwrap_or(0);
    let name = qs_param(qs.as_deref(), "name").filter(|s| !s.is_empty());
    let kind = qs_param(qs.as_deref(), "kind").filter(|s| !s.is_empty());
    let kinds: Option<Vec<String>> =
        kind.map(|k| k.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
    let qstate = qs_param(qs.as_deref(), "state").filter(|s| !s.is_empty());
    let data = super::engine::aggregate_services(&state).await?;
    let mut services = data
        .get("services")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 服务端过滤（与前端快速过滤同口径：kind/state 精确、name 包含）
    services.retain(|s| {
        if let Some(ks) = &kinds {
            let hit = s
                .get("kind")
                .and_then(|v| v.as_str())
                .is_some_and(|actual| ks.iter().any(|k| k == actual));
            if !hit {
                return false;
            }
        }
        if let Some(st) = &qstate {
            if s.get("state").and_then(|v| v.as_str()) != Some(st.as_str()) {
                return false;
            }
        }
        if let Some(n) = &name {
            let hay = format!(
                "{}{}",
                s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                s.get("title").and_then(|v| v.as_str()).unwrap_or("")
            )
            .to_lowercase();
            if !hay.contains(&n.to_lowercase()) {
                return false;
            }
        }
        true
    });
    let total = services.len() as i64;
    if size > 0 {
        let start = if page < 1 { 0 } else { ((page - 1) * size) as usize };
        let end = (start + size as usize).min(services.len());
        services = services.get(start..end).unwrap_or(&[]).to_vec();
    }
    let engines = data.get("engines").cloned().unwrap_or(json!([]));
    Ok(Resp::ok(json!({
        "services": services,
        "total": total,
        "engines": engines,
    })))
}

/// `GET /map/services/:kind/:name` — 取服务当前配置原文（XML 文本）。
/// 按服务归属路由：有归属引擎走归属，否则走默认引擎。
pub async fn service_config(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
) -> AppResult<Json<Resp<Value>>> {
    let engine = super::engine::resolve_for_service(&state, &kind, &name).await;
    let data = upstream_json_engine(
        &state,
        &engine,
        reqwest::Method::GET,
        &format!("/admin/services/{kind}/{name}"),
        None,
        None,
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `PUT /map/services/:kind/:name` — 回写编辑后的配置（body: {"xml": "..."}）。
/// 按服务归属路由。
pub async fn service_update(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Resp<Value>>> {
    let engine = super::engine::resolve_for_service(&state, &kind, &name).await;
    let data = upstream_json_engine(
        &state,
        &engine,
        reqwest::Method::PUT,
        &format!("/admin/services/{kind}/{name}"),
        None,
        Some(body),
    )
    .await?;
    Ok(Resp::ok(data))
}

#[derive(Deserialize)]
pub struct StateBody {
    /// active | stopped | paused | deleted
    pub state: String,
}

/// `PUT /map/services/:kind/:name/state` — 变更服务生命周期状态，即时生效。
/// 按服务归属路由。
pub async fn set_state(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
    Json(body): Json<StateBody>,
) -> AppResult<Json<Resp<Value>>> {
    if !matches!(body.state.as_str(), "active" | "stopped" | "paused" | "deleted") {
        return Err(AppError::Param(
            "state 仅支持 active|stopped|paused|deleted".into(),
        ));
    }
    let engine = super::engine::resolve_for_service(&state, &kind, &name).await;
    let data = upstream_json_engine(
        &state,
        &engine,
        reqwest::Method::PUT,
        &format!("/admin/services/{kind}/{name}/state"),
        None,
        Some(serde_json::json!({ "state": body.state })),
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `POST /map/services/:kind/:name/check` — 数据源体检（文件/可读性/PG 连接等）。
/// 按服务归属路由。
pub async fn check_service(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
) -> AppResult<Json<Resp<Value>>> {
    let engine = super::engine::resolve_for_service(&state, &kind, &name).await;
    let data = upstream_json_engine(
        &state,
        &engine,
        reqwest::Method::POST,
        &format!("/admin/services/{kind}/{name}/check"),
        None,
        None,
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `POST /map/reload` — 热重载：重读 mapserver.xml + 重扫目录 + 重建服务集。
pub async fn reload(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(&state, reqwest::Method::POST, "/admin/reload", None, None).await?;
    Ok(Resp::ok(data))
}

/// `GET /map/usage?period=day|week|month|year&group_by=name|type&kind=...`
/// 查询串原样透传（map-api 按 WMS KVP 约定大小写不敏感）。
pub async fn usage(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(
        &state,
        reqwest::Method::GET,
        "/api/usage",
        qs.as_deref(),
        None,
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `GET /map/logs?page=&pageSize=&status=2xx..5xx&kind=&name=&q=` — 访问日志审计查询。
/// map-api 的 access-logs 仅支持 limit（无 offset），故 manager 侧强制拉满（limit=2000）
/// 后在 manager 端按 page/pageSize 切片，返回 `{rows,total,...}` 满足前端“后端分页”契约。
pub async fn logs(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let page = query_i64(qs.as_deref(), "page").unwrap_or(0);
    let size = query_i64(qs.as_deref(), "pageSize").unwrap_or(0);
    // 强制拉满上限，确保 manager 端可对任意页切片（map-api 自身 limit 上限为 2000）
    let upstream_qs = match qs.as_deref() {
        Some(s) if !s.is_empty() => format!("{s}&limit=2000"),
        _ => "limit=2000".to_string(),
    };
    let data = upstream_json(
        &state,
        reqwest::Method::GET,
        "/admin/access-logs",
        Some(&upstream_qs),
        None,
    )
    .await?;
    let mut rows = data
        .get("rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let total = data
        .get("total")
        .and_then(|v| v.as_i64())
        .unwrap_or(rows.len() as i64);
    if size > 0 {
        let start = if page < 1 { 0 } else { ((page - 1) * size) as usize };
        let end = (start + size as usize).min(rows.len());
        rows = rows.get(start..end).unwrap_or(&[]).to_vec();
    }
    Ok(Resp::ok(json!({
        "rows": rows,
        "total": total,
        "status_counts": data.get("status_counts").cloned().unwrap_or(json!({})),
        "errors": data.get("errors").cloned().unwrap_or(json!({})),
    })))
}

// ---- 样式库（SLD）----

/// `GET /map/styles` — 样式清单（解析摘要 + 被引用图层）。
pub async fn style_list(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(&state, reqwest::Method::GET, "/admin/styles", None, None).await?;
    Ok(Resp::ok(data))
}

#[derive(Deserialize)]
pub struct StyleUploadBody {
    /// SLD 文档原文（XML）。
    pub content: String,
}

/// `POST /map/styles/:name` — 上传/替换一个 SLD（map-api 端先解析校验再落盘）。
pub async fn style_upload(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<StyleUploadBody>,
) -> AppResult<Json<Resp<Value>>> {
    let (base_raw, token) = super::engine::default_engine(&state).await;
    let base = base_raw.trim_end_matches('/');
    let mut req = state
        .map_http
        .post(format!("{base}/admin/styles/{name}"))
        .header("Content-Type", "application/xml")
        .body(body.content);
    if let Some(tok) = &token {
        req = req.header("X-Admin-Token", tok);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Msg(format!("map-api 服务不可达：{e}")))?;
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
    Ok(Resp::ok(val))
}

/// `DELETE /map/styles/:name` — 删除样式。
pub async fn style_remove(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(
        &state,
        reqwest::Method::DELETE,
        &format!("/admin/styles/{name}"),
        None,
        None,
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `GET /map/styles/:name/content` — 样式原文。
pub async fn style_content(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(
        &state,
        reqwest::Method::GET,
        &format!("/admin/styles/{name}/content"),
        None,
        None,
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `GET /map/styles/:name/legend.png?w=&h=` — 样式图例预览（PNG 直通）。
pub async fn style_legend(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    RawQuery(qs): RawQuery,
) -> AppResult<Response> {
    let (bytes, ct) = upstream_bytes(
        &state,
        &format!("/admin/styles/{name}/legend.png"),
        qs.as_deref(),
    )
    .await?;
    Ok((StatusCode::OK, [( "content-type", ct.as_str() )], bytes).into_response())
}

/// `GET /map/health` — map-api 存活探测（供页面顶部的组件状态灯）。
pub async fn health(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let (base_raw, _) = super::engine::default_engine(&state).await;
    let base = base_raw.trim_end_matches('/');
    let ok = state
        .map_http
        .get(format!("{base}/health"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    Ok(Resp::ok(serde_json::json!({
        "base_url": base,
        "up": ok,
    })))
}

/// `GET /map/browse?path=` — 目录浏览（代理 map-api `/admin/browse`）。
pub async fn browse(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(&state, reqwest::Method::GET, "/admin/browse", qs.as_deref(), None).await?;
    Ok(Resp::ok(data))
}

/// `POST /map/services` — 登记新服务（代理 map-api `/admin/services`）。
///
/// 多引擎第二阶段：body 可携带 `engineId` 指定目标引擎（省略走默认引擎）；
/// `engineId` 不会转发给 map-api，登记成功后自动记录服务归属。
pub async fn create_service(
    State(state): State<Arc<AppState>>,
    Json(mut body): Json<Value>,
) -> AppResult<Json<Resp<Value>>> {
    // 摘出 engineId / serviceGroup（不透传上游），解析目标引擎；kind/name 从请求体提取。
    let engine_id = body
        .get("engineId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let service_group = body
        .get("serviceGroup")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    {
        let obj = body.as_object_mut().unwrap();
        obj.remove("engineId");
        obj.remove("serviceGroup");
    }
    let engine = match engine_id.as_str() {
        "" => super::engine::default_engine(&state).await,
        id => {
            let e = super::engine::enabled_engine_by_id(&state, id)
                .await
                .ok_or_else(|| AppError::Msg(format!("引擎不存在或已停用：{id}")))?;
            (
                e.base_url,
                e.admin_token.filter(|t| !t.is_empty()).or(state.cfg.mapsvc.token.clone()),
            )
        }
    };
    let data = upstream_json_engine(
        &state,
        &engine,
        reqwest::Method::POST,
        "/admin/services",
        None,
        Some(body),
    )
    .await?;
    // 登记成功后记录归属（engineId 缺省时用引擎实际 base_url 反查 id）。
    if !kind.is_empty() && !name.is_empty() {
        let rid = if engine_id.is_empty() {
            super::engine::engine_id_by_base(&state, engine.0.trim_end_matches('/')).await
        } else {
            Some(engine_id.clone())
        };
        if let Some(rid) = rid {
            super::engine::record_service_engine(&state, &kind, &name, &rid).await;
        }
        if !service_group.is_empty() {
            record_service_group(&state, &kind, &name, &service_group).await;
        }
    }
    Ok(Resp::ok(data))
}

/// `POST /map/pg-test` — PostGIS 连接测试（代理 map-api `/admin/pg-test`）。
pub async fn pg_test(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/admin/pg-test",
        None,
        Some(body),
    )
    .await?;
    Ok(Resp::ok(data))
}

/// `GET /map/components` — 组件探测（代理 map-api `/admin/components`）。
pub async fn components(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(&state, reqwest::Method::GET, "/admin/components", qs.as_deref(), None).await?;
    Ok(Resp::ok(data))
}

/// `GET /map/runtime` — 只读展示 manager 自身运行配置（敏感字段掩码）。
///
/// 不暴露 secret：数据库口令、mapsvc 共享密钥仅以「是否配置」呈现。
pub async fn runtime(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let cfg = &state.cfg;
    let data = json!({
        "server_addr": cfg.server.addr,
        "base_api": cfg.server.base_api,
        "cors_origins": cfg.server.cors_origins,
        "database_url": mask_conn(&cfg.database.url),
        "pool_max": cfg.database.pool_max,
        "mapsvc_base_url": cfg.mapsvc.base_url,
        "mapsvc_token_configured": cfg.mapsvc.token.is_some(),
        "jwt_expire_minutes": cfg.security.token_expire_minutes,
        "captcha_enabled": cfg.security.captcha_enabled,
        "avatar_dir": cfg.storage.avatar_dir,
    });
    Ok(Resp::ok(data))
}

/// 把连接串里的口令掩码：`postgresql://user:pass@host/db` → `postgresql://user:***@host/db`。
pub(crate) fn mask_conn(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        if let Some(at) = rest.rfind('@') {
            let creds = &rest[..at];
            if let Some(colon) = creds.rfind(':') {
                let user_part = &creds[..colon];
                return format!("{}://{}:***@{}", &url[..scheme_end], user_part, &rest[at + 1..]);
            }
        }
    }
    url.to_string()
}

/// `GET /map/cache` — 缓存实时计数器（代理 map-api `/api/cache`）。
pub async fn cache_stats(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(&state, reqwest::Method::GET, "/api/cache", None, None).await?;
    Ok(Resp::ok(data))
}

/// `POST /map/cache/purge?scope=all|output|docs|catalog|features|layers`
/// 清缓存（代理 map-api `/api/cache/purge`）。scope 透传，非法值由上游拒绝。
pub async fn cache_purge(
    State(state): State<Arc<AppState>>,
    RawQuery(qs): RawQuery,
) -> AppResult<Json<Resp<Value>>> {
    let data = upstream_json(
        &state,
        reqwest::Method::POST,
        "/api/cache/purge",
        qs.as_deref(),
        None,
    )
    .await?;
    Ok(Resp::ok(data))
}

// =====================================================================
// 服务分组（两级命名空间；sys_service_meta，空串 = 根目录）
// =====================================================================

/// 校验分组路径：最多两级（`一级` 或 `一级/二级`），各级非空。
fn validate_group(g: &str) -> Result<(), AppError> {
    if g.is_empty() {
        return Ok(());
    }
    let parts: Vec<&str> = g.split('/').collect();
    if parts.len() > 2
        || parts.iter().any(|p| p.trim().is_empty())
        || parts.iter().any(|p| p.len() > 120)
    {
        return Err(AppError::Param(
            "分组最多两级，形如「一级/二级」，且各级非空（≤120 字符）".into(),
        ));
    }
    Ok(())
}

/// `GET /map/service-groups` — 全部分组（去重、非空），前端构两级树。
pub async fn service_groups(State(state): State<Arc<AppState>>) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let rows = c
        .query("SELECT DISTINCT service_group FROM sys_service_meta WHERE service_group <> '' ORDER BY service_group", &[])
        .await?;
    let groups: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    Ok(Resp::ok(json!({ "groups": groups })))
}

/// `GET /map/services/:kind/:name/group` — 查服务分组（空 = 根目录）。
pub async fn service_group_get(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
) -> AppResult<Json<Resp<Value>>> {
    let c = conn(&state.pool).await?;
    let rows = c
        .query(
            "SELECT service_group FROM sys_service_meta WHERE kind = $1 AND name = $2",
            &[&kind, &name],
        )
        .await?;
    let group = rows.first().map(|r| r.get::<_, String>(0)).unwrap_or_default();
    Ok(Resp::ok(json!({ "kind": kind, "name": name, "group": group })))
}

/// `PUT /map/services/:kind/:name/group` — 调整分组（body: {"group": "一级/二级"}）。
/// 空串 = 回到根目录（删除登记行）。
pub async fn service_group_put(
    State(state): State<Arc<AppState>>,
    Path((kind, name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Resp<Value>>> {
    let group = body
        .get("group")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    validate_group(&group)?;
    let c = conn(&state.pool).await?;
    if group.is_empty() {
        c.execute(
            "DELETE FROM sys_service_meta WHERE kind = $1 AND name = $2",
            &[&kind, &name],
        )
        .await?;
    } else {
        c.execute(
            "INSERT INTO sys_service_meta (kind, name, service_group) VALUES ($1,$2,$3) \
             ON CONFLICT (kind, name) DO UPDATE SET service_group = EXCLUDED.service_group, \
             update_time = now()",
            &[&kind, &name, &group],
        )
        .await?;
    }
    Ok(Resp::ok(json!({ "kind": kind, "name": name, "group": group })))
}

/// 登记服务成功后记录分组（尽力而为，失败不阻断）。
async fn record_service_group(state: &AppState, kind: &str, name: &str, group: &str) {
    if let Ok(c) = conn(&state.pool).await {
        let _ = c
            .execute(
                "INSERT INTO sys_service_meta (kind, name, service_group) VALUES ($1,$2,$3) \
                 ON CONFLICT (kind, name) DO UPDATE SET service_group = EXCLUDED.service_group, \
                 update_time = now()",
                &[&kind, &name, &group],
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::qs_param;

    #[test]
    fn qs_param_decodes_encoded_values() {
        // 前端 axios 会把逗号/中文 percent-encode 后发出：不解码则多 kind 过滤
        // 与中文名称检索恒匹配不到（瓦片服务/矢量服务页列表为空的根因）。
        assert_eq!(qs_param(Some("kind=pmtiles%2Cmbtiles"), "kind").as_deref(), Some("pmtiles,mbtiles"));
        assert_eq!(qs_param(Some("name=%E7%93%A6%E7%89%87"), "name").as_deref(), Some("瓦片"));
        assert_eq!(qs_param(Some("name=a+b"), "name").as_deref(), Some("a b"));
        // 未编码与大小写键名照旧可用。
        assert_eq!(qs_param(Some("KIND=arcgis&page=1"), "kind").as_deref(), Some("arcgis"));
        assert_eq!(qs_param(Some("page=1"), "kind"), None);
    }
}
