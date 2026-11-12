//! 登录 / 登出 / 验证码 / 当前用户信息 / 动态路由。
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::{json, Value};

use crate::auth::{self, CurrentUser};
use crate::error::{AppError, AppResult};
use crate::models::{UserVO, USER_COLS};
use crate::resp::{PageResp, Resp};
use crate::state::{query, AppState};
use crate::util::{jstr, page_of};
use std::collections::HashMap;

/// 认证域路由（由 `handlers::router` 统一装配）。
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/code", get(code))
        .route("/captchaImage", get(captcha_alias))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/system/user/getInfo", get(get_info))
        .route("/system/menu/getRouters", get(get_routers))
}

/// GET /auth/code —— 验证码（自绘 SVG data URL）。
pub async fn code(State(state): State<Arc<AppState>>) -> Json<Value> {
    let enabled = state.cfg.security.captcha_enabled;
    if !enabled {
        return Json(json!({
            "code": 200, "msg": "操作成功",
            "data": { "captchaEnabled": false, "img": "", "uuid": "" }
        }));
    }
    let (uuid, img) = state.captcha.issue();
    Json(json!({
        "code": 200, "msg": "操作成功",
        "data": { "captchaEnabled": true, "img": img, "uuid": uuid }
    }))
}

/// POST /auth/login
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let user_name = jstr(&body, "username");
    let password = jstr(&body, "password");
    let code = jstr(&body, "code");
    let uuid = jstr(&body, "uuid");
    let ip = "127.0.0.1".to_string();
    let ua = String::new();

    if user_name.is_empty() || password.is_empty() {
        return Err(AppError::bad("用户名或密码不能为空"));
    }
    if state.cfg.security.captcha_enabled && !state.captcha.check(&uuid, &code) {
        auth::log_login(&state, &user_name, "1", "验证码错误", &ip, "Chrome", "Windows").await;
        return Err(AppError::bad("验证码错误"));
    }

    let sql = format!("SELECT {USER_COLS}, u.password FROM sys_user u WHERE u.user_name = $1 AND u.del_flag = '0'");
    let rows = query(&state.pool, &sql, &[&user_name]).await?;
    let row = rows.first().ok_or_else(|| AppError::bad("用户不存在或已被删除"))?;

    let hash: String = row.try_get("password").unwrap_or_default();
    let ok = bcrypt::verify(&password, &hash).unwrap_or(false);
    if !ok {
        auth::log_login(&state, &user_name, "1", "密码错误", &ip, "Chrome", "Windows").await;
        return Err(AppError::bad("用户名或密码错误"));
    }
    let status: String = row.try_get("status").unwrap_or_default();
    if status != "0" {
        auth::log_login(&state, &user_name, "1", "账号已停用", &ip, "Chrome", "Windows").await;
        return Err(AppError::bad("账号已停用，请联系管理员"));
    }

    let mut user = crate::models::user_from_row(row, false);
    user.password = None;

    let jti = uuid::Uuid::new_v4().simple().to_string();
    let token = auth::issue(&state, user.user_id, &user.user_name, &jti)?;

    // 在线会话
    use crate::state::OnlineSession;
    state.online.put(OnlineSession {
        token_id: jti.clone(),
        user_id: user.user_id,
        user_name: user.user_name.clone(),
        dept_name: user.dept_name.clone(),
        ipaddr: ip.clone(),
        browser: "Chrome".into(),
        os: "Windows".into(),
        login_time: chrono::Utc::now().naive_utc(),
        last_access: std::time::Instant::now(),
    });
    let _ = ua;

    // 更新登录信息
    let upd = "UPDATE sys_user SET login_ip = $1, login_date = NOW() WHERE user_id = $2";
    let _ = crate::state::execute(&state.pool, upd, &[&ip, &user.user_id]).await;
    auth::log_login(&state, &user_name, "0", "登录成功", &ip, "Chrome", "Windows").await;

    Ok(Json(json!({ "code": 200, "msg": "登录成功", "data": { "access_token": token } })))
}

/// POST /auth/logout
pub async fn logout(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
) -> Json<Value> {
    state.online.remove(&user.token_id);
    Json(json!({ "code": 200, "msg": "退出成功" }))
}

/// GET /system/user/getInfo
pub async fn get_info(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    let sql = format!(
        "SELECT {USER_COLS}, (SELECT d.dept_name FROM sys_dept d WHERE d.dept_id = u.dept_id) AS dept_name \
         FROM sys_user u WHERE u.user_id = $1"
    );
    let rows = query(&state.pool, &sql, &[&user.user_id]).await?;
    let row = rows.first().ok_or(AppError::Unauthorized)?;
    let mut vo: UserVO = crate::models::user_from_row(row, false);
    vo.password = None;
    vo.admin = user.admin;
    Ok(Json(json!({
        "code": 200, "msg": "操作成功",
        "data": { "user": vo, "roles": user.roles, "permissions": user.perms }
    })))
}

/// GET /system/menu/getRouters
pub async fn get_routers(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    let rows = menu_rows(&state, &user).await?;
    let flat: Vec<crate::models::MenuVO> = rows.iter().map(crate::models::menu_from_row).collect();
    let tree = crate::perm::build_tree(flat);
    Ok(Json(json!({
        "code": 200, "msg": "操作成功",
        "data": crate::perm::to_routers(tree)
    })))
}

pub(super) async fn menu_rows(
    state: &AppState,
    user: &CurrentUser,
) -> AppResult<Vec<tokio_postgres::Row>> {
    let cols = crate::models::MENU_COLS;
    if user.admin {
        let sql = format!(
            "SELECT {cols} FROM sys_menu WHERE menu_type IN ('M','C') AND status = '0' ORDER BY parent_id, order_num"
        );
        return query(&state.pool, &sql, &[]).await;
    }
    // 非管理员：仅取角色授权菜单，M/C 都取（否则目录骨架缺失，子菜单无处挂载）。
    // 用 IN 子查询，外层只保留 sys_menu 单表，避免多表 JOIN 时 menu_id/parent_id/order_num 列歧义。
    let sql = format!(
        "SELECT {cols} FROM sys_menu
          WHERE status = '0'
            AND menu_id IN (
              SELECT rm.menu_id
                FROM sys_role_menu rm
                JOIN sys_role r ON r.role_id = rm.role_id AND r.status = '0' AND r.del_flag = '0'
                JOIN sys_user_role ur ON ur.role_id = r.role_id AND ur.user_id = $1
            )
          ORDER BY parent_id, order_num"
    );
    query(&state.pool, &sql, &[&user.user_id]).await
}

/// GET /system/user/profile
pub async fn profile_page(State(_s): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "code": 200, "msg": "操作成功" }))
}

/// 兼容旧前端顶层路径 /captchaImage。
pub async fn captcha_alias(State(state): State<Arc<AppState>>) -> Json<Value> {
    code(State(state)).await
}

/// 首页统计（占位，便于前端 index 页不报错）。
pub async fn dashboard(
    State(state): State<Arc<AppState>>,
    _q: Query<HashMap<String, String>>,
) -> Json<Value> {
    let online = state.online.list().len() as i64;
    Json(json!({ "code": 200, "msg": "操作成功", "data": { "online": online } }))
}

/// 供 monitor 复用：分页参数占位函数（保持导出面一致）。
pub fn _page(q: &HashMap<String, String>) -> (i64, i64) {
    page_of(q)
}

/// 统一成功响应（部分无数据接口复用）。
pub fn ok_void() -> Json<Value> {
    Json(json!({ "code": 200, "msg": "操作成功" }))
}

/// 列表响应包装（rows/total 顶层）。
pub fn page_resp<T: serde::Serialize>(rows: Vec<T>, total: i64) -> Json<PageResp<T>> {
    PageResp::new(rows, total)
}

/// 单对象响应包装。
pub fn data_resp<T: serde::Serialize>(data: T) -> Json<Resp<T>> {
    Resp::ok(data)
}
