//! 部门管理。
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::models::{DeptVO, TreeSelectVO, DEPT_COLS};
use crate::state::{execute, query, AppState};
use crate::util::{Qb, jopt_i32, jopt_i64, jopt_str, jstr, qopt};

/// GET /system/dept/list —— 返回平铺列表（前端 handleTree 自行组装）
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut qb = Qb::new();
    qb.raw("del_flag = '0'");
    qb.like("dept_name", qopt(&q, "deptName").as_deref());
    if let Some(s) = qopt(&q, "status") {
        qb.when("status = {}", Some(s));
    }
    let where_sql = qb.where_sql();
    let refs = qb.refs();
    let sql = format!("SELECT {DEPT_COLS} FROM sys_dept{where_sql} ORDER BY order_num, dept_id");
    let rows = query(&state.pool, &sql, &refs).await?;
    let vos: Vec<DeptVO> = rows.iter().map(crate::models::dept_from_row).collect();
    // 菜单/部门类列表返回 AjaxResult(data)，前端 res.data 消费
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "data": vos })))
}

/// 部门树（TreeSelect 结构），供用户/岗位/角色的部门选择复用。
pub async fn tree(state: &AppState) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {DEPT_COLS} FROM sys_dept WHERE del_flag = '0' ORDER BY order_num, dept_id");
    let rows = query(&state.pool, &sql, &[]).await?;
    let items: Vec<DeptVO> = rows.iter().map(crate::models::dept_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": to_tree(items) })))
}

/// GET /system/post/deptTree、/system/role/deptTree/{roleId} 等复用的部门树。
pub async fn dept_tree(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    tree(&state).await
}

fn to_tree(items: Vec<DeptVO>) -> Vec<TreeSelectVO> {
    fn walk(parent: i64, all: &[DeptVO]) -> Vec<TreeSelectVO> {
        all.iter()
            .filter(|d| d.parent_id == parent)
            .map(|d| TreeSelectVO {
                id: d.dept_id,
                label: d.dept_name.clone(),
                children: walk(d.dept_id, all),
            })
            .collect()
    }
    walk(0, &items)
}

/// GET /system/dept/{deptId}
pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {DEPT_COLS} FROM sys_dept WHERE dept_id = $1");
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("部门不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": crate::models::dept_from_row(r) })))
}

/// POST /system/dept
pub async fn add(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let parent_id = jopt_i64(&body, "parentId").unwrap_or(0);
    let ancestors = ancestors_of(&state, parent_id).await?;
    let sql = "INSERT INTO sys_dept (dept_id, parent_id, ancestors, dept_name, dept_category, order_num, leader, phone, email, status, del_flag, create_time) \
               VALUES (COALESCE((SELECT max(dept_id) FROM sys_dept),0) + $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, '0', NOW()) RETURNING dept_id";
    let pv = crate::pv![
        1i64,
        parent_id,
        ancestors,
        jstr(&body, "deptName"),
        jstr(&body, "deptCategory"),
        jopt_i32(&body, "orderNum").unwrap_or(1),
        jopt_i64(&body, "leader").unwrap_or(0),
        jstr(&body, "phone"),
        jstr(&body, "email"),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("dept_id").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

/// PUT /system/dept
pub async fn update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "deptId").ok_or(AppError::Param("deptId 缺失".into()))?;
    let parent_id = jopt_i64(&body, "parentId").unwrap_or(0);
    let ancestors = ancestors_of(&state, parent_id).await?;
    let sql = "UPDATE sys_dept SET parent_id = $1, ancestors = $2, dept_name = $3, dept_category = $4, order_num = $5, leader = $6, phone = $7, email = $8, status = $9, update_time = NOW() WHERE dept_id = $10";
    let pv = crate::pv![
        parent_id,
        ancestors,
        jstr(&body, "deptName"),
        jstr(&body, "deptCategory"),
        jopt_i32(&body, "orderNum").unwrap_or(1),
        jopt_i64(&body, "leader").unwrap_or(0),
        jstr(&body, "phone"),
        jstr(&body, "email"),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// DELETE /system/dept/{deptId}
pub async fn delete(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let children: i64 = query(&state.pool, "SELECT count(*) FROM sys_dept WHERE parent_id = $1 AND del_flag = '0'", &[&id])
        .await?
        .first()
        .and_then(|r| r.try_get(0).ok())
        .unwrap_or(0);
    if children > 0 {
        return Err(AppError::bad("存在下级部门，不允许删除"));
    }
    let users: i64 = query(&state.pool, "SELECT count(*) FROM sys_user WHERE dept_id = $1 AND del_flag = '0'", &[&id])
        .await?
        .first()
        .and_then(|r| r.try_get(0).ok())
        .unwrap_or(0);
    if users > 0 {
        return Err(AppError::bad("部门下存在用户，不允许删除"));
    }
    execute(&state.pool, "UPDATE sys_dept SET del_flag = '2' WHERE dept_id = $1", &[&id]).await?;
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

/// GET /system/dept/list/exclude/{deptId}
pub async fn exclude(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {DEPT_COLS} FROM sys_dept WHERE del_flag = '0' AND dept_id <> $1 ORDER BY order_num");
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let vos: Vec<DeptVO> = rows.iter().map(crate::models::dept_from_row).collect();
    // 菜单/部门类列表返回 AjaxResult(data)，前端 res.data 消费
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "data": vos })))
}

/// GET /system/dept/optionselect
pub async fn option_select(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {DEPT_COLS} FROM sys_dept WHERE del_flag = '0' ORDER BY order_num");
    let rows = query(&state.pool, &sql, &[]).await?;
    let vos: Vec<DeptVO> = rows.iter().map(crate::models::dept_from_row).collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": to_tree(vos) })))
}

async fn ancestors_of(state: &AppState, parent_id: i64) -> AppResult<String> {
    if parent_id == 0 {
        return Ok("0".to_string());
    }
    let rows = query(&state.pool, "SELECT ancestors FROM sys_dept WHERE dept_id = $1", &[&parent_id]).await?;
    let p: String = rows.first().and_then(|r| r.try_get("ancestors").ok()).unwrap_or_else(|| "0".into());
    Ok(format!("{},{}", p, parent_id))
}
