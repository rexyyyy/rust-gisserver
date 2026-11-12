//! JWT 签发/校验 + 鉴权中间件 + `CurrentUser` 提取。
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// 用户 id
    pub sub: i64,
    /// 登录账号
    pub name: String,
    /// 会话 id（用于在线用户与强踢）
    pub jti: String,
    pub exp: i64,
    pub iat: i64,
}

pub fn issue(state: &AppState, user_id: i64, user_name: &str, jti: &str) -> AppResult<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let exp = now + state.cfg.security.token_expire_minutes * 60;
    let claims = Claims { sub: user_id, name: user_name.to_string(), jti: jti.to_string(), exp, iat: now };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.cfg.security.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::bad(format!("签发 token 失败: {e}")))
}

pub fn verify(state: &AppState, token: &str) -> Option<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.cfg.security.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map(|d| d.claims)
    .ok()
}

/// 注入到请求扩展中的当前登录用户。
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user_id: i64,
    pub user_name: String,
    pub nick_name: String,
    pub dept_id: i64,
    pub admin: bool,
    pub token_id: String,
    pub roles: Vec<String>,
    pub perms: Vec<String>,
}

impl CurrentUser {
    /// 无请求上下文时的占位（定时任务/内部调用用）。
    pub fn system() -> Self {
        Self {
            user_id: 1,
            user_name: "admin".into(),
            nick_name: "admin".into(),
            dept_id: 0,
            admin: true,
            token_id: String::new(),
            roles: vec!["admin".into()],
            perms: Vec::new(),
        }
    }
}

/// 鉴权中间件：白名单与静态资源放行，其余校验 token → 用户状态 → 权限码。
pub async fn auth_mw(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let base = state.cfg.server.base_api.clone();
    let path = req.uri().path().to_string();
    let rel = path
        .strip_prefix(&base)
        .unwrap_or(&path)
        .trim_end_matches('/')
        .to_string();
    let rel = if rel.is_empty() { "/".to_string() } else { rel };
    let method = req.method().as_str().to_uppercase();

    // 白名单
    if state.cfg.is_white(&rel) || rel.starts_with("/auth/login") {
        return Ok(next.run(req).await);
    }

    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").map(|s| s.to_string()))
        .ok_or(AppError::Unauthorized)?;

    let claims = verify(&state, &token).ok_or(AppError::Unauthorized)?;
    if state.online.is_kicked(&claims.jti) {
        return Err(AppError::Unauthorized);
    }

    // 用户状态（离职/停用/删除即时生效）
    let sql = "SELECT user_id, user_name, nick_name, dept_id, status::text AS status \
               FROM sys_user WHERE user_id = $1 AND del_flag = '0'";
    let rows = crate::state::query(&state.pool, sql, &[&claims.sub]).await?;
    let row = rows.first().ok_or(AppError::Unauthorized)?;
    let status: String = row.try_get("status").unwrap_or_default();
    if status != "0" {
        return Err(AppError::Unauthorized);
    }

    let (roles, perms) = crate::perm::load(&state, claims.sub).await?;
    let admin = claims.sub == 1 || roles.iter().any(|r| r == "admin");
    // 上游 RuoYi 语义：超级管理员持有全部权限码（前端 v-hasPermi 依赖
    // "*:*:*" 判定按钮可见性，不给它会把 admin 的所有操作按钮都隐藏）。
    let perms = if admin { vec!["*:*:*".to_string()] } else { perms };

    crate::perm::allows(&method, &rel, admin, &perms)?;

    let user = CurrentUser {
        user_id: claims.sub,
        user_name: row.try_get("user_name").unwrap_or_default(),
        nick_name: row.try_get("nick_name").unwrap_or_default(),
        dept_id: row.try_get("dept_id").unwrap_or(0),
        admin,
        token_id: claims.jti.clone(),
        roles,
        perms,
    };
    state.online.touch(&claims.jti);
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}

/// 记录登录日志（成功/失败都记）。
pub async fn log_login(
    state: &AppState,
    user_name: &str,
    status: &str,
    msg: &str,
    ip: &str,
    browser: &str,
    os: &str,
) {
    let sql = "INSERT INTO sys_logininfor (info_id, user_name, ipaddr, browser, os, status, msg) \
               VALUES (COALESCE((SELECT max(info_id) FROM sys_logininfor),0) + 1, $1, $2, $3, $4, $5, $6)";
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        vec![&user_name, &ip, &browser, &os, &status, &msg];
    if let Err(e) = crate::state::execute(&state.pool, sql, &params).await {
        tracing::warn!("写登录日志失败: {e}");
    }
}

/// 记录操作日志。
pub async fn log_oper(
    state: &AppState,
    title: &str,
    method: &str,
    oper_name: &str,
    oper_url: &str,
    oper_param: &str,
    status: &str,
    error_msg: &str,
    cost_ms: i64,
) {
    let sql = "INSERT INTO sys_oper_log (oper_id, title, business_type, method, request_method, \
               oper_name, oper_url, oper_param, status, error_msg, oper_time, cost_time) \
               VALUES (COALESCE((SELECT max(oper_id) FROM sys_oper_log),0) + 1, $1, 2, $2, $3, $4, $5, $6, $7, $8, NOW(), $9)";
    let status_code: i32 = if status == "1" { 1 } else { 0 };
    let pv = crate::pv![
        title.to_string(),
        method.to_string(),
        String::new(),
        oper_name.to_string(),
        oper_url.to_string(),
        oper_param.to_string(),
        status_code,
        error_msg.to_string(),
        cost_ms,
    ];
    if let Err(e) = crate::state::execute(&state.pool, sql, &pv.refs()).await {
        tracing::warn!("写操作日志失败: {e}");
    }
}
