//! 用户管理。
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde_json::{json, Value};
use tokio_postgres::Row;

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::models::{UserVO, USER_COLS};
use crate::state::{execute, query, AppState};
use crate::util::{self, jarr_i64, jopt_i64, jopt_str, jstr, page_of, qopt, qopt_i64, Qb};

const BASE: &str = "FROM sys_user u \
    LEFT JOIN sys_dept d ON d.dept_id = u.dept_id";

/// 角色数据范围 → 允许的部门 id 列表；`None` 表示不限。
async fn dept_scope(state: &AppState, user: &CurrentUser) -> AppResult<Option<Vec<i64>>> {
    if user.admin {
        return Ok(None);
    }
    let sql = "SELECT data_scope::text AS data_scope FROM sys_role r \
               JOIN sys_user_role ur ON ur.role_id = r.role_id \
               WHERE ur.user_id = $1 AND r.status = '0' AND r.del_flag = '0'";
    let rows = query(&state.pool, sql, &[&user.user_id]).await?;
    let scopes: Vec<String> = rows.iter().map(|r| r.try_get("data_scope").unwrap_or_default()).collect();
    // 取最宽松的一个：2 自定义 / 3 本部门 / 4 本部门及以下 / 5 仅本人 / 1 全部
    let mut allowed: Option<Vec<i64>> = Some(Vec::new());
    for sc in &scopes {
        match sc.as_str() {
            "1" => return Ok(None),
            "2" => {
                let sql = "SELECT dept_id FROM sys_role_dept rd \
                           JOIN sys_user_role ur ON ur.role_id = rd.role_id WHERE ur.user_id = $1";
                let rows = query(&state.pool, sql, &[&user.user_id]).await?;
                for r in rows {
                    let id: i64 = r.try_get("dept_id").unwrap_or(0);
                    if let Some(v) = allowed.as_mut() {
                        if !v.contains(&id) {
                            v.push(id);
                        }
                    }
                }
            }
            "3" | "4" => {
                let d = user.dept_id;
                if let Some(v) = allowed.as_mut() {
                    if !v.contains(&d) {
                        v.push(d);
                    }
                }
                if sc == "4" {
                    // 本部门及以下：靠 ancestors 前缀匹配
                    let sql = "SELECT dept_id FROM sys_dept WHERE ancestors LIKE $1 OR dept_id = $2";
                    let pat = format!("%{}%", d);
                    let rows = query(&state.pool, sql, &[&pat, &d]).await?;
                    for r in rows {
                        let id: i64 = r.try_get("dept_id").unwrap_or(0);
                        if let Some(v) = allowed.as_mut() {
                            if !v.contains(&id) {
                                v.push(id);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(allowed)
}

/// GET /system/user/list
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let depts = dept_scope(&state, &user).await?;
    let mut qb = Qb::new();
    qb.raw("u.del_flag = '0'");
    qb.like("u.user_name", qopt(&q, "userName").as_deref());
    qb.like("u.nick_name", qopt(&q, "nickName").as_deref());
    qb.like("u.phonenumber", qopt(&q, "phonenumber").as_deref());
    if let Some(s) = qopt(&q, "status") {
        qb.when("u.status = {}", Some(s));
    }
    if let Some(id) = qopt_i64(&q, "deptId") {
        qb.when("u.dept_id = {}", Some(id));
        let ph = qb.p(format!("%{}%", id));
        qb.raw(&format!("d.ancestors LIKE {ph}"));
    }
    if let Some(ids) = depts {
        if ids.is_empty() {
            qb.raw("1 = 0");
        } else {
            let mut phs = Vec::new();
            for id in ids {
                phs.push(qb.p(id));
            }
            qb.raw(&format!("u.dept_id IN ({})", phs.join(",")));
        }
    }
    let where_sql = qb.where_sql();
    let (limit, offset) = page_of(&q);

    let count_sql = format!("SELECT count(*) {BASE}{where_sql}");
    let refs = qb.refs();
    let rows = query(&state.pool, &count_sql, &refs).await?;
    let total: i64 = rows.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);

    let lph = format!("${}", refs.len() + 1);
    let oph = format!("${}", refs.len() + 2);
    let mut all: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = refs.clone();
    let lim = limit;
    let off = offset;
    all.push(&lim);
    all.push(&off);
    let sql = format!(
        "SELECT {USER_COLS}, COALESCE(d.dept_name,'') AS dept_name {BASE}{where_sql} \
         ORDER BY u.user_id LIMIT {lph} OFFSET {oph}"
    );
    let rows = query(&state.pool, &sql, &all).await?;
    let vos: Vec<UserVO> = rows.iter().map(|r| row_to_vo(r)).collect();
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "total": total, "rows": vos })))
}

fn row_to_vo(r: &Row) -> UserVO {
    let mut v = crate::models::user_from_row(r, false);
    v.password = None;
    v
}

/// GET /system/user/{userId}
pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<i64>,
) -> AppResult<Json<Value>> {
    let sql = format!(
        "SELECT {USER_COLS}, COALESCE((SELECT d.dept_name FROM sys_dept d WHERE d.dept_id = u.dept_id),'') AS dept_name \
         FROM sys_user u WHERE u.user_id = $1"
    );
    let rows = query(&state.pool, &sql, &[&user_id]).await?;
    let r = rows.first().ok_or_else(|| AppError::bad("用户不存在"))?;
    let mut vo = row_to_vo(r);
    let roles: Vec<i64> = query(&state.pool, "SELECT role_id FROM sys_user_role WHERE user_id = $1", &[&user_id])
        .await?
        .iter()
        .map(|x| x.try_get("role_id").unwrap_or(0))
        .collect();
    let posts: Vec<i64> = query(&state.pool, "SELECT post_id FROM sys_user_post WHERE user_id = $1", &[&user_id])
        .await?
        .iter()
        .map(|x| x.try_get("post_id").unwrap_or(0))
        .collect();
    vo.role_ids = roles;
    vo.post_ids = posts;
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": vo, "roleIds": vo.role_ids, "postIds": vo.post_ids })))
}

/// POST /system/user
pub async fn add(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let name = jstr(&body, "userName");
    if name.is_empty() {
        return Err(AppError::Param("登录账号不能为空".into()));
    }
    let nick = if jstr(&body, "nickName").is_empty() { name.clone() } else { jstr(&body, "nickName") };
    let pwd = jstr(&body, "password");
    let hash = if pwd.is_empty() {
        bcrypt::hash("123456", bcrypt::DEFAULT_COST).map_err(|e| AppError::bad(e.to_string()))?
    } else {
        bcrypt::hash(&pwd, bcrypt::DEFAULT_COST).map_err(|e| AppError::bad(e.to_string()))?
    };
    let dept_id = jopt_i64(&body, "deptId").unwrap_or(0);
    let sql = "INSERT INTO sys_user (user_id, dept_id, user_name, nick_name, user_type, email, phonenumber, sex, password, status, del_flag, create_by, create_time) \
               VALUES (COALESCE((SELECT max(user_id) FROM sys_user),0) + $1, $2, $3, $4, 'sys_user', $5, $6, $7, $8, $9, '0', $10, NOW()) \
               RETURNING user_id";
    let pv = crate::pv![
        1i64,
        dept_id,
        name,
        nick,
        jstr(&body, "email"),
        jstr(&body, "phonenumber"),
        jstr(&body, "sex"),
        hash,
        jstr(&body, "status"),
        user.user_id,
    ];
    let rows = query(&state.pool, sql, &pv.refs()).await?;
    let new_id: i64 = rows.first().and_then(|r| r.try_get("user_id").ok()).unwrap_or(0);
    if new_id == 0 {
        return Err(AppError::bad("新增用户失败"));
    }
    set_roles(&state, new_id, jarr_i64(&body, "roleIds")).await?;
    set_posts(&state, new_id, jarr_i64(&body, "postIds")).await?;
    Ok(Json(json!({ "code": 200, "msg": "新增成功", "data": new_id })))
}

/// PUT /system/user
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "userId").ok_or(AppError::Param("userId 缺失".into()))?;
    let nick = if jstr(&body, "nickName").is_empty() { jstr(&body, "userName") } else { jstr(&body, "nickName") };
    let sql = "UPDATE sys_user SET dept_id = $1, nick_name = $2, email = $3, phonenumber = $4, sex = $5, status = $6, remark = $7, update_by = $8, update_time = NOW() WHERE user_id = $9";
    let pv = crate::pv![
        jopt_i64(&body, "deptId").unwrap_or(0),
        nick,
        jstr(&body, "email"),
        jstr(&body, "phonenumber"),
        jstr(&body, "sex"),
        jopt_str(&body, "status").unwrap_or_else(|| "0".into()),
        jstr(&body, "remark"),
        user.user_id,
        id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    set_roles(&state, id, jarr_i64(&body, "roleIds")).await?;
    set_posts(&state, id, jarr_i64(&body, "postIds")).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// DELETE /system/user/{ids}（逗号分隔）
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Path(ids): Path<String>,
) -> AppResult<Json<Value>> {
    let list: Vec<i64> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if list.is_empty() {
        return Err(AppError::Param("ids 缺失".into()));
    }
    if list.contains(&1) {
        return Err(AppError::bad("超级管理员账号不允许删除"));
    }
    let mut phs = Vec::new();
    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
    for id in &list {
        phs.push(format!("${}", params.len() + 1));
        params.push(id);
    }
    let sql = format!(
        "UPDATE sys_user SET del_flag = '2', update_time = NOW() WHERE user_id IN ({})",
        phs.join(",")
    );
    execute(&state.pool, &sql, &params).await?;
    Ok(Json(json!({ "code": 200, "msg": "删除成功" })))
}

/// PUT /system/user/changeStatus
pub async fn change_status(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "userId").ok_or(AppError::Param("userId 缺失".into()))?;
    let status = jopt_str(&body, "status").unwrap_or_else(|| "0".into());
    execute(&state.pool, "UPDATE sys_user SET status = $1 WHERE user_id = $2", &[&status, &id]).await?;
    Ok(Json(json!({ "code": 200, "msg": "状态修改成功" })))
}

/// PUT /system/user/resetPwd
pub async fn reset_pwd(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "userId").ok_or(AppError::Param("userId 缺失".into()))?;
    let pwd = jstr(&body, "password");
    let pwd = if pwd.is_empty() { "123456".to_string() } else { pwd };
    let hash = bcrypt::hash(&pwd, bcrypt::DEFAULT_COST).map_err(|e| AppError::bad(e.to_string()))?;
    execute(&state.pool, "UPDATE sys_user SET password = $1 WHERE user_id = $2", &[&hash, &id]).await?;
    Ok(Json(json!({ "code": 200, "msg": "密码重置成功" })))
}

/// GET /system/user/profile
pub async fn profile(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    let sql = format!(
        "SELECT {USER_COLS}, COALESCE((SELECT d.dept_name FROM sys_dept d WHERE d.dept_id = u.dept_id),'') AS dept_name \
         FROM sys_user u WHERE u.user_id = $1"
    );
    let rows = query(&state.pool, &sql, &[&user.user_id]).await?;
    let r = rows.first().ok_or(AppError::Unauthorized)?;
    let mut vo = row_to_vo(r);
    let roles: Vec<String> = query(&state.pool, "SELECT r.role_name FROM sys_user_role ur JOIN sys_role r ON r.role_id = ur.role_id WHERE ur.user_id = $1", &[&user.user_id])
        .await?
        .iter()
        .map(|x| x.try_get("role_name").unwrap_or_default())
        .collect();
    let posts: Vec<String> = query(&state.pool, "SELECT p.post_name FROM sys_user_post up JOIN sys_post p ON p.post_id = up.post_id WHERE up.user_id = $1", &[&user.user_id])
        .await?
        .iter()
        .map(|x| x.try_get("post_name").unwrap_or_default())
        .collect();
    vo.password = None;
    Ok(Json(json!({
        "code": 200, "msg": "操作成功",
        "data": vo,
        "roleGroup": roles.join(","),
        "postGroup": posts.join(",")
    })))
}

/// PUT /system/user/profile
pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let sql = "UPDATE sys_user SET nick_name = $1, phonenumber = $2, email = $3, sex = $4, update_time = NOW() WHERE user_id = $5";
    let pv = crate::pv![
        jstr(&body, "nickName"),
        jstr(&body, "phonenumber"),
        jstr(&body, "email"),
        jstr(&body, "sex"),
        user.user_id,
    ];
    execute(&state.pool, sql, &pv.refs()).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// PUT /system/user/profile/updatePwd
pub async fn update_pwd(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let old = jstr(&body, "oldPassword");
    let new = jstr(&body, "newPassword");
    let rows = query(&state.pool, "SELECT password FROM sys_user WHERE user_id = $1", &[&user.user_id]).await?;
    let hash: String = rows.first().and_then(|r| r.try_get("password").ok()).unwrap_or_default();
    if !bcrypt::verify(&old, &hash).unwrap_or(false) {
        return Err(AppError::bad("原密码不正确"));
    }
    let nh = bcrypt::hash(&new, bcrypt::DEFAULT_COST).map_err(|e| AppError::bad(e.to_string()))?;
    execute(&state.pool, "UPDATE sys_user SET password = $1 WHERE user_id = $2", &[&nh, &user.user_id]).await?;
    Ok(Json(json!({ "code": 200, "msg": "修改成功" })))
}

/// GET /system/user/deptTree —— 部门树（复用于岗位/人员的部门选择）
pub async fn dept_tree(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    crate::handlers::system::dept::tree(&state).await
}

/// GET /system/user/optionselect
pub async fn option_select(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let rows = query(&state.pool, "SELECT user_id, nick_name FROM sys_user WHERE status = '0' AND del_flag = '0' ORDER BY user_id", &[]).await?;
    let data: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "userId": r.try_get::<_, i64>("user_id").unwrap_or(0), "nickName": r.try_get::<_, String>("nick_name").unwrap_or_default() }))
        .collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "data": data })))
}

/// GET /system/user/authRole/{userId}
pub async fn auth_role_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let urows = query(&state.pool, &format!("SELECT {USER_COLS}, '' AS dept_name FROM sys_user u WHERE u.user_id = $1"), &[&id]).await?;
    let u = urows.first().ok_or_else(|| AppError::bad("用户不存在"))?;
    let mut vo = row_to_vo(u);
    vo.password = None;
    let roles = query(&state.pool, "SELECT r.role_id, r.role_name, r.role_key FROM sys_role r WHERE r.del_flag = '0' ORDER BY r.role_sort", &[]).await?;
    let list: Vec<Value> = roles
        .iter()
        .map(|r| json!({
            "roleId": r.try_get::<_, i64>("role_id").unwrap_or(0),
            "roleName": r.try_get::<_, String>("role_name").unwrap_or_default(),
            "roleKey": r.try_get::<_, String>("role_key").unwrap_or_default()
        }))
        .collect();
    let mine: Vec<i64> = query(&state.pool, "SELECT role_id FROM sys_user_role WHERE user_id = $1", &[&id])
        .await?
        .iter()
        .map(|r| r.try_get("role_id").unwrap_or(0))
        .collect();
    Ok(Json(json!({ "code": 200, "msg": "操作成功", "user": vo, "roles": list, "checkedKeys": mine })))
}

/// PUT /system/user/authRole
pub async fn auth_role(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let id = jopt_i64(&body, "userId").ok_or(AppError::Param("userId 缺失".into()))?;
    set_roles(&state, id, jarr_i64(&body, "roleIds")).await?;
    Ok(Json(json!({ "code": 200, "msg": "授权成功" })))
}

/// GET /system/user/list/dept/{deptId}
pub async fn list_by_dept(
    State(state): State<Arc<AppState>>,
    Path(dept_id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    let mut q2 = q.clone();
    q2.insert("deptId".into(), dept_id.to_string());
    let (limit, offset) = page_of(&q2);
    let sql = format!(
        "SELECT {USER_COLS}, '' AS dept_name FROM sys_user u \
         WHERE u.del_flag = '0' AND (u.dept_id = $1 OR u.dept_id IN (SELECT dept_id FROM sys_dept WHERE ancestors LIKE $2)) \
         ORDER BY u.user_id LIMIT $3 OFFSET $4"
    );
    let pat = format!("%{}%", dept_id);
    let rows = query(&state.pool, &sql, &[&dept_id, &pat, &limit, &offset]).await?;
    let vos: Vec<UserVO> = rows.iter().map(row_to_vo).collect();
    // 菜单/部门类列表返回 AjaxResult(data)，前端 res.data 消费
    Ok(Json(json!({ "code": 200, "msg": "查询成功", "data": vos })))
}

async fn set_roles(state: &AppState, user_id: i64, ids: Vec<i64>) -> AppResult<()> {
    execute(&state.pool, "DELETE FROM sys_user_role WHERE user_id = $1", &[&user_id]).await?;
    for rid in ids {
        execute(&state.pool, "INSERT INTO sys_user_role (user_id, role_id) VALUES ($1, $2)", &[&user_id, &rid]).await.ok();
    }
    state.perms.invalidate(user_id);
    Ok(())
}

async fn set_posts(state: &AppState, user_id: i64, ids: Vec<i64>) -> AppResult<()> {
    execute(&state.pool, "DELETE FROM sys_user_post WHERE user_id = $1", &[&user_id]).await?;
    for pid in ids {
        execute(&state.pool, "INSERT INTO sys_user_post (user_id, post_id) VALUES ($1, $2)", &[&user_id, &pid]).await.ok();
    }
    Ok(())
}

/// 供其它模块复用：把用户 id 列表转成 VO。
pub fn _to_vos(rows: &[Row]) -> Vec<UserVO> {
    rows.iter().map(row_to_vo).collect()
}

/// 工具：util 模块导出给其它 handler 使用
pub fn _util_ok() -> bool {
    util::qopt_i64(&HashMap::new(), "x").is_none()
}
