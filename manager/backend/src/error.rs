//! 统一错误类型：所有错误最终都渲染成 `{code, msg}` 的 JSON（HTTP 状态由 `status()` 决定）。
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Bad(&'static str),
    #[error("{0}")]
    Msg(String),
    #[error("未认证或 Token 已过期")]
    Unauthorized,
    #[error("没有权限：{0}")]
    Forbidden(String),
    #[error("数据库错误: {0}")]
    Db(String),
    #[error("参数错误: {0}")]
    Param(String),
    #[error(transparent)]
    Pool(#[from] deadpool_postgres::PoolError),
    #[error(transparent)]
    Postgres(#[from] tokio_postgres::Error),
}

impl AppError {
    /// HTTP 状态码。业务错误一律 200 + code 字段（与前端拦截器约定一致），
    /// 只有鉴权失败按前端拦截器要求返回真实 401。
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::OK,
        }
    }

    pub fn code(&self) -> i32 {
        match self {
            AppError::Unauthorized => 401,
            AppError::Forbidden(_) => 403,
            _ => 500,
        }
    }

    pub fn bad(msg: impl Into<String>) -> Self {
        AppError::Msg(msg.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = json!({ "code": self.code(), "msg": self.to_string() });
        tracing::warn!(target: "manager", "请求失败: {}", self);
        (status, Json(body)).into_response()
    }
}

/// `Result<T>` 别名：右值统一为 AppError。
pub type AppResult<T> = Result<T, AppError>;

/// 把 Option 转成 Result（查不到数据时使用）。
pub fn require<T>(opt: Option<T>, msg: &str) -> AppResult<T> {
    opt.ok_or_else(|| AppError::bad(msg))
}
