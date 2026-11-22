//! 监控：登录日志 / 操作日志 / 在线用户。
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get};
use axum::{Extension, Json, Router};
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::state::{execute, query, AppState};
use crate::util::{page_of, qopt, Qb};

/// 监控域路由（由 `handlers::router` 统一装配）。
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/monitor/online", get(online_info))
        .route("/monitor/online/list", get(online_list))
        .route("/monitor/online/myself/{tokenId}", delete(logout_self))
        .route("/monitor/online/myself", delete(myself))
        .route("/monitor/online/{tokenId}", delete(force_logout))
        .route("/monitor/logininfor/list", get(logininfor_list))
        .route("/monitor/logininfor/clean", delete(logininfor_clean))
        .route("/monitor/logininfor/unlock/{userName}", get(logininfor_unlock))
        .route("/monitor/logininfor/{ids}", delete(logininfor_delete))
        .route("/monitor/operlog/list", get(operlog_list))
        .route("/monitor/operlog/clean", delete(operlog_clean))
        .route("/monitor/operlog/{ids}", delete(operlog_delete))
}

/// GET /monitor/online/list —— 在线用户列表（需 `monitor:online:list`）。
pub async fn online_list(State(state): State<Arc<AppState>>) -> Json<Value> {
    let rows = state.online.list();
    Json(json!({ "code": 200, "msg": "查询成功", "total": rows.len() as i64, "rows": rows }))
}

/// GET /monitor/online —— 当前登录用户自己的在线设备（`getInfo`，无需按钮权限）。
pub async fn online_info(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
) -> Json<Value> {
    let rows = state.online.list_by_user(user.user_id);
    Json(json!({ "code": 200, "msg": "查询成功", "total": rows.len() as i64, "rows": rows }))
}

/// DELETE /monitor/online/{tokenId} —— 强制下线（需 `monitor:online:forceLogout`）。
pub async fn force_logout(
    State(state): State<Arc<AppState>>,
    Path(token_id): Path<String>,
) -> AppResult<Json<Value>> {
    state.online.remove(&token_id);
    Ok(Json(json!({ "code": 200, "msg": "强制下线成功" })))
}

/// DELETE /monitor/online/myself/{tokenId} —— 下线自己的某台设备（只能踢自己的）。
pub async fn logout_self(
    State(state): State<Arc<AppState>>,
    Path(token_id): Path<String>,
    Extension(user): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    if !state.online.owns(&token_id, user.user_id) {
        return Err(AppError::Forbidden("不能下线其他用户的会话".to_string()));
    }
    state.online.remove(&token_id);
    Ok(Json(json!({ "code": 200, "msg": "退出成功" })))
}

/// GET /monitor/logininfor/list
pub async fn logininfor_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("user_name", qopt(&q, "userName").as_deref());
    qb.like("ipaddr", qopt(&q, "ipaddr").as_deref());
    if let Some(s) = qopt(&q, "status") {
        qb.when("status = {}", Some(s));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_logininfor{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_logininfor{where_sql} ORDER BY info_id DESC LIMIT {lph} OFFSET {oph}",
        crate::models::LOGININFOR_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::LogininforVO> = rows.iter().map(crate::models::logininfor_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

/// DELETE /monitor/logininfor/{ids}
pub async fn logininfor_delete(
    State(state): State<Arc<AppState>>,
    Path(ids): Path<String>,
) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        execute(&state.pool, "DELETE FROM sys_logininfor WHERE info_id = $1", &[id]).await?;
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

/// DELETE /monitor/logininfor/clean
pub async fn logininfor_clean(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    execute(&state.pool, "TRUNCATE TABLE sys_logininfor", &[]).await?;
    Ok(Json(json!({ "code": 200, "msg": "清空成功" })))
}

/// GET /monitor/logininfor/unlock/{userName}
pub async fn logininfor_unlock(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    execute(&state.pool, "UPDATE sys_user SET status = '0' WHERE user_name = $1", &[&name]).await?;
    Ok(Json(json!({ "code": 200, "msg": "解锁成功" })))
}

/// GET /monitor/operlog/list
pub async fn operlog_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.like("title", qopt(&q, "title").as_deref());
    qb.like("oper_name", qopt(&q, "operName").as_deref());
    if let Some(s) = qopt(&q, "status") {
        qb.when("status = {}", Some(s));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_oper_log{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_oper_log{where_sql} ORDER BY oper_id DESC LIMIT {lph} OFFSET {oph}",
        crate::models::OPERLOG_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::OperlogVO> = rows.iter().map(crate::models::operlog_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

pub async fn operlog_delete(
    State(state): State<Arc<AppState>>,
    Path(ids): Path<String>,
) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    for id in &list {
        execute(&state.pool, "DELETE FROM sys_oper_log WHERE oper_id = $1", &[id]).await?;
    }
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

pub async fn operlog_clean(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    execute(&state.pool, "TRUNCATE TABLE sys_oper_log", &[]).await?;
    Ok(Json(json!({ "code": 200, "msg": "清空成功" })))
}

/// 当前登录者主动下线自己（供前端「退出」按钮之外的场景复用）
pub async fn myself(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    state.online.remove(&user.token_id);
    Ok(Json(json!({ "code": 200, "msg": "退出成功" })))
}
