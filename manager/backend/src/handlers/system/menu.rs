//! 菜单管理。
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::models::{MenuVO, MENU_COLS};
use crate::state::{execute, query, AppState};
use crate::util::{jopt_i32, jopt_i64, jopt_str, jstr, qopt};

fn row_menu(r: &tokio_postgres::Row) -> MenuVO {
    crate::models::menu_from_row(r)
}

/// GET /system/menu/list
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();
    if let Some(name) = qopt(&q, "menuName") {
        params.push(Box::new(format!("%{name}%")));
        conds.push(format!("menu_name LIKE ${}", params.len()));
    }
    if let Some(s) = qopt(&q, "status") {
        params.push(Box::new(s));
        conds.push(format!("status = ${}", params.len()));
    }
    let where_sql = if conds.is_empty() { String::new() } else { format!(" WHERE {}", conds.join(" AND ")) };
    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        params.iter().map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
    let sql = format!("SELECT {MENU_COLS} FROM sys_menu{where_sql} ORDER BY parent_id, order_num");
    let rows = query(&state.pool, &sql, &refs).await?;
    let vos: Vec<MenuVO> = rows.iter().map(row_menu).collect();
    // 上游 RuoYi 语义：菜单/部门类列表返回 AjaxResult(data)，前端 res.data 消费
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "data": vos })))
}

/// GET /system/menu/treeselect
pub async fn treeselect(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {MENU_COLS} FROM sys_menu WHERE menu_type IN ('M','C') AND status = '0' ORDER BY parent_id, order_num");
    let rows = query(&state.pool, &sql, &[]).await?;
    let vos: Vec<MenuVO> = rows.iter().map(row_menu).collect();
    let tree = crate::perm::build_tree(vos);
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": tree })))
}

/// GET /system/menu/roleMenuTreeselect/{roleId}
pub async fn role_menu_treeselect(
    State(state): State<Arc<AppState>>,
    Path(role_id): Path<i64>,
) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {MENU_COLS} FROM sys_menu WHERE menu_type IN ('M','C') AND status = '0' ORDER BY parent_id, order_num");
    let rows = query(&state.pool, &sql, &[]).await?;
    let vos: Vec<MenuVO> = rows.iter().map(row_menu).collect();
    let checked: Vec<i64> = query(&state.pool, "SELECT menu_id FROM sys_role_menu WHERE role_id = $1", &[&role_id])
        .await?
        .iter()
        .map(|r| r.try_get("menu_id").unwrap_or(0))
        .collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "menus": crate::perm::build_tree(vos), "checkedKeys": checked })))
}

/// GET /system/menu/{menuId}
pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let sql = format!("SELECT {MENU_COLS} FROM sys_menu WHERE menu_id = $1");
    let rows = query(&state.pool, &sql, &[&id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("菜单不存在"))?;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": row_menu(r) })))
}

/// POST /system/menu
pub async fn add(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let sql = "INSERT INTO sys_menu (menu_id, menu_name, parent_id, order_num, path, component, query_param, is_frame, is_cache, menu_type, visible, status, perms, icon, create_time, remark) \
               VALUES (COALESCE((SELECT max(menu_id) FROM sys_menu WHERE menu_id < 2000),0) + $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, NOW(), $15) RETURNING menu_id";
    let pv = crate::pv![
        1i64,
        jstr(&body, "menuName"),
        jopt_i64(&body, "parentId").unwrap_or(0),
        jopt_i32(&body, "orderNum").unwrap_or(1),
        jstr(&body, "path"),
        jopt_str(&body, "component").unwrap_or_default(),
        jstr(&body, "queryParam"),
        jopt_str(&body, "isFrame").unwrap_or_else(|| "1".into()),
        jopt_str(&body, "isCache").unwrap_or_else(|| "0".into()),
        jopt_str(&body, "menuType").unwrap_or_else(|| "C".into()),
        jopt_str(&body, "visible").unwrap_or_else(|| "0".into()),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "perms"),
        jstr(&body, "icon"),
        jstr(&body, "remark"),
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let id: i64 = rows.first().and_then(|r| r.try_get("menu_id").ok()).unwrap_or(0);
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": id })))
}

/// PUT /system/menu
pub async fn update(State(state): State<Arc<AppState>>, Json(body): Json<Value>) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "menuId").ok_or(AppError::Param("menuId 缺失".into()))?;
    let sql = "UPDATE sys_menu SET menu_name = $1, parent_id = $2, order_num = $3, path = $4, component = $5, query_param = $6, is_frame = $7, is_cache = $8, menu_type = $9, visible = $10, status = $11, perms = $12, icon = $13, remark = $14, update_time = NOW() WHERE menu_id = $15";
    let pv = crate::pv![
        jstr(&body, "menuName"),
        jopt_i64(&body, "parentId").unwrap_or(0),
        jopt_i32(&body, "orderNum").unwrap_or(1),
        jstr(&body, "path"),
        jopt_str(&body, "component").unwrap_or_default(),
        jstr(&body, "queryParam"),
        jopt_str(&body, "isFrame").unwrap_or_else(|| "1".into()),
        jopt_str(&body, "isCache").unwrap_or_else(|| "0".into()),
        jopt_str(&body, "menuType").unwrap_or_else(|| "C".into()),
        jopt_str(&body, "visible").unwrap_or_else(|| "0".into()),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "perms"),
        jstr(&body, "icon"),
        jstr(&body, "remark"),
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// DELETE /system/menu/{menuId}
pub async fn delete(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let children: i64 = query(&state.pool, "SELECT count(*) FROM sys_menu WHERE parent_id = $1", &[&id])
        .await?
        .first()
        .and_then(|r| r.try_get(0).ok())
        .unwrap_or(0);
    if children > 0 {
        return Err(AppError::bad("存在子菜单，不允许删除"));
    }
    execute(&state.pool, "DELETE FROM sys_role_menu WHERE menu_id = $1", &[&id]).await?;
    execute(&state.pool, "DELETE FROM sys_menu WHERE menu_id = $1", &[&id]).await?;
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

/// DELETE /system/menu/cascade/{menuId}（级联删除子菜单与授权记录）
pub async fn cascade_delete(
    State(state): State<Arc<AppState>>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let sql = "WITH RECURSIVE sub AS (\
                 SELECT menu_id FROM sys_menu WHERE menu_id = $1 \
                 UNION ALL SELECT m.menu_id FROM sys_menu m JOIN sub s ON m.parent_id = s.menu_id) \
               DELETE FROM sys_role_menu WHERE menu_id IN (SELECT menu_id FROM sub)";
    execute(&state.pool, sql, &[&id]).await?;
    let sql2 = "WITH RECURSIVE sub AS (\
                 SELECT menu_id FROM sys_menu WHERE menu_id = $1 \
                 UNION ALL SELECT m.menu_id FROM sys_menu m JOIN sub s ON m.parent_id = s.menu_id) \
                DELETE FROM sys_menu WHERE menu_id IN (SELECT menu_id FROM sub)";
    execute(&state.pool, sql2, &[&id]).await?;
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}
