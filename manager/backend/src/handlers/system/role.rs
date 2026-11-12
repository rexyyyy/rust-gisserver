//! 角色管理（含菜单授权、数据范围、分配用户）。
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::models::{RoleVO, ROLE_COLS};
use crate::state::{execute, query, AppState};
use crate::util::{Qb, jarr_i64, jopt_i32, jopt_i64, jopt_str, jstr, page_of, qopt, self};

/// GET /system/role/list
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.raw("del_flag = '0'");
    qb.like("role_name", qopt(&q, "roleName").as_deref());
    qb.like("role_key", qopt(&q, "roleKey").as_deref());
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

    let count_sql = format!("SELECT count(*) FROM sys_role{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs)
        .await?
        .first()
        .and_then(|r| r.try_get(0).ok())
        .unwrap_or(0);
    let sql = format!(
        "SELECT {ROLE_COLS} FROM sys_role{where_sql} ORDER BY role_sort, role_id LIMIT {lph} OFFSET {oph}"
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<RoleVO> = rows.iter().map(crate::models::role_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

/// GET /system/role/{roleId}
pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {ROLE_COLS} FROM sys_role WHERE role_id = $1");
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("角色不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::role_from_row(r) })))
}

/// POST /system/role
pub async fn add(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let sql = "INSERT INTO sys_role (role_id, role_name, role_key, role_sort, data_scope, menu_check_strictly, dept_check_strictly, status, del_flag, create_by, create_time) \
               VALUES (COALESCE((SELECT max(role_id) FROM sys_role),0) + $1, $2, $3, $4, $5, $6, $7, $8, '0', $9, NOW()) RETURNING role_id";
    let menu_strict = crate::util::jopt_bool(&body, "menuCheckStrictly").unwrap_or(true);
    let dept_strict = crate::util::jopt_bool(&body, "deptCheckStrictly").unwrap_or(true);
    let pv = crate::pv![
        1i64,
        jstr(&body, "roleName"),
        jstr(&body, "roleKey"),
        jopt_i32(&body, "roleSort").unwrap_or(1),
        jopt_str(&body, "dataScope").unwrap_or_else(|| "1".into()),
        menu_strict,
        dept_strict,
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        user.user_id,
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("role_id").ok()).unwrap_or(0);
    set_menus(&state, id, jarr_i64(&body, "menuIds")).await?;
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

/// PUT /system/role
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "roleId").ok_or(AppError::Param("roleId 缺失".into()))?;
    let menu_strict = crate::util::jopt_bool(&body, "menuCheckStrictly").unwrap_or(true);
    let dept_strict = crate::util::jopt_bool(&body, "deptCheckStrictly").unwrap_or(true);
    let sql = "UPDATE sys_role SET role_name = $1, role_key = $2, role_sort = $3, data_scope = $4, menu_check_strictly = $5, dept_check_strictly = $6, status = $7, remark = $8, update_by = $9, update_time = NOW() WHERE role_id = $10";
    let pv = crate::pv![
        jstr(&body, "roleName"),
        jstr(&body, "roleKey"),
        jopt_i32(&body, "roleSort").unwrap_or(1),
        jopt_str(&body, "dataScope").unwrap_or_else(|| "1".into()),
        menu_strict,
        dept_strict,
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "remark"),
        user.user_id,
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    set_menus(&state, id, jarr_i64(&body, "menuIds")).await?;
    invalidate_all(&state).await;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// DELETE /system/role/{ids}
pub async fn delete(State(state): State<Arc<AppState>>, Path(ids): Path<String>) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    if list.contains(&1) {
        return Err(AppError::bad("超级管理员角色不允许删除"));
    }
    for id in &list {
        let used: i64 = query(&state.pool, "SELECT count(*) FROM sys_user_role WHERE role_id = $1", &[id])
            .await?
            .first()
            .and_then(|r| r.try_get(0).ok())
            .unwrap_or(0);
        if used > 0 {
            return Err(AppError::bad("角色已分配用户，不允许删除"));
        }
        execute(&state.pool, "UPDATE sys_role SET del_flag = '2' WHERE role_id = $1", &[id]).await?;
        execute(&state.pool, "DELETE FROM sys_role_menu WHERE role_id = $1", &[id]).await?;
        execute(&state.pool, "DELETE FROM sys_role_dept WHERE role_id = $1", &[id]).await?;
    }
    invalidate_all(&state).await;
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

/// PUT /system/role/changeStatus
pub async fn change_status(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "roleId").ok_or(AppError::Param("roleId 缺失".into()))?;
    let status = jopt_str(&body, "status").unwrap_or_else(|| "0".into());
    execute(&state.pool, "UPDATE sys_role SET status = $1 WHERE role_id = $2", &[&status, &id]).await?;
    invalidate_all(&state).await;
    Ok(Json(json!({ "code": 200, "msg": "状态修改成功" })))
}

/// PUT /system/role/dataScope
pub async fn data_scope(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "roleId").ok_or(AppError::Param("roleId 缺失".into()))?;
    let scope = jopt_str(&body, "dataScope").unwrap_or_else(|| "1".into());
    execute(&state.pool, "UPDATE sys_role SET data_scope = $1 WHERE role_id = $2", &[&scope, &id]).await?;
    execute(&state.pool, "DELETE FROM sys_role_dept WHERE role_id = $1", &[&id]).await?;
    for dept in jarr_i64(&body, "deptIds") {
        execute(&state.pool, "INSERT INTO sys_role_dept (role_id, dept_id) VALUES ($1, $2)", &[&id, &dept]).await.ok();
    }
    invalidate_all(&state).await;
    Ok(Json(json!({ "code": 200, "msg": "数据权限设置成功" })))
}

/// GET /system/role/optionselect
pub async fn option_select(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {ROLE_COLS} FROM sys_role WHERE del_flag = '0' AND status = '0' ORDER BY role_sort");
    let rows = query(&state.pool, &sql, &[]).await?;
    let vos: Vec<RoleVO> = rows.iter().map(crate::models::role_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": vos })))
}

/// GET /system/role/deptTree/{roleId}
pub async fn dept_tree(State(state): State<Arc<AppState>>, Path(role_id): Path<i64>) -> AppResult<Json<Value>> {
    let mut resp = crate::handlers::system::dept::tree(&state).await?;
    let checked: Vec<i64> = query(&state.pool, "SELECT dept_id FROM sys_role_dept WHERE role_id = $1", &[&role_id])
        .await?
        .iter()
        .map(|r| r.try_get("dept_id").unwrap_or(0))
        .collect();
    if let Value::Object(map) = &mut resp.0 {
        map.insert("checkedKeys".into(), json!(checked));
    }
    Ok(resp)
}

// ---------- 分配用户 ----------

/// GET /system/role/authUser/allocatedList
pub async fn allocated(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let role_id = util::qopt_i64(&q, "roleId").unwrap_or(0);
    let mut qb = Qb::new();
    qb.raw("u.del_flag = '0'");
    qb.raw("EXISTS (SELECT 1 FROM sys_user_role ur WHERE ur.user_id = u.user_id AND ur.role_id = $1)");
    qb.p(role_id);
    qb.like("u.user_name", qopt(&q, "userName").as_deref());
    qb.like("u.phonenumber", qopt(&q, "phonenumber").as_deref());
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_user u{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_user u{where_sql} ORDER BY u.user_id LIMIT {lph} OFFSET {oph}",
        crate::models::USER_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::UserVO> = rows.iter().map(|r| crate::models::user_from_row(r, false)).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

/// GET /system/role/authUser/unallocatedList
pub async fn unallocated(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let role_id = util::qopt_i64(&q, "roleId").unwrap_or(0);
    let mut qb = Qb::new();
    qb.raw("u.del_flag = '0'");
    qb.raw("NOT EXISTS (SELECT 1 FROM sys_user_role ur WHERE ur.user_id = u.user_id AND ur.role_id = $1)");
    qb.p(role_id);
    qb.raw("u.user_id <> 1");
    qb.like("u.user_name", qopt(&q, "userName").as_deref());
    qb.like("u.phonenumber", qopt(&q, "phonenumber").as_deref());
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let (limit, offset) = page_of(&q);
    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    all.push(&limit);
    all.push(&offset);
    let count_sql = format!("SELECT count(*) FROM sys_user u{where_sql}");
    let total: i64 = query(&state.pool, &count_sql, &refs).await?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
    let sql = format!(
        "SELECT {} FROM sys_user u{where_sql} ORDER BY u.user_id LIMIT {lph} OFFSET {oph}",
        crate::models::USER_COLS
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<crate::models::UserVO> = rows.iter().map(|r| crate::models::user_from_row(r, false)).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

/// PUT /system/role/authUser/selectAll
pub async fn select_all(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let role_id = jopt_i64(&body, "roleId").ok_or(AppError::Param("roleId 缺失".into()))?;
    for uid in jarr_i64(&body, "userIds") {
        let sql = "INSERT INTO sys_user_role (user_id, role_id) SELECT $1, $2 \
                   WHERE NOT EXISTS (SELECT 1 FROM sys_user_role WHERE user_id = $1 AND role_id = $2)";
        execute(&state.pool, sql, &[&uid, &role_id]).await.ok();
        state.perms.invalidate(uid);
    }
    Ok(Json(json!({ "code": 200, "msg": "授权成功" })))
}

/// PUT /system/role/authUser/cancel
pub async fn cancel(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let role_id = jopt_i64(&body, "roleId").ok_or(AppError::Param("roleId 缺失".into()))?;
    let user_id = jopt_i64(&body, "userId").ok_or(AppError::Param("userId 缺失".into()))?;
    execute(&state.pool, "DELETE FROM sys_user_role WHERE user_id = $1 AND role_id = $2", &[&user_id, &role_id]).await?;
    state.perms.invalidate(user_id);
    Ok(Json(json!({ "code": 200, "msg": "取消授权成功" })))
}

/// PUT /system/role/authUser/cancelAll
pub async fn cancel_all(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let role_id = jopt_i64(&body, "roleId").ok_or(AppError::Param("roleId 缺失".into()))?;
    let ids = jarr_i64(&body, "userIds");
    if ids.is_empty() {
        execute(&state.pool, "DELETE FROM sys_user_role WHERE role_id = $1", &[&role_id]).await?;
    } else {
        for uid in ids {
            execute(&state.pool, "DELETE FROM sys_user_role WHERE user_id = $1 AND role_id = $2", &[&uid, &role_id]).await?;
            state.perms.invalidate(uid);
        }
    }
    Ok(Json(json!({ "code": 200, "msg": "批量取消授权成功" })))
}

async fn set_menus(state: &AppState, role_id: i64, ids: Vec<i64>) -> AppResult<()> {
    execute(&state.pool, "DELETE FROM sys_role_menu WHERE role_id = $1", &[&role_id]).await?;
    for mid in ids {
        execute(&state.pool, "INSERT INTO sys_role_menu (role_id, menu_id) VALUES ($1, $2)", &[&role_id, &mid]).await.ok();
    }
    Ok(())
}

/// 角色/菜单变更后，清空所有用户的权限缓存（表小，直接全清）。
async fn invalidate_all(state: &AppState) {
    let rows = query(&state.pool, "SELECT DISTINCT user_id FROM sys_user_role", &[]).await.unwrap_or_default();
    for r in rows {
        let uid: i64 = r.try_get("user_id").unwrap_or(0);
        state.perms.invalidate(uid);
    }
}
