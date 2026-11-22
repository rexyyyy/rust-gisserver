//! 统一响应封装，字段与前端请求拦截器对齐：
//! `code`（200 为成功）、`msg`，列表数据额外在顶层给出 `rows` / `total`。
use axum::Json;
use serde::Serialize;

pub const OK: i32 = 200;

#[derive(Debug, Serialize)]
pub struct Resp<T> {
    pub code: i32,
    pub msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T: Serialize> Resp<T> {
    pub fn ok(data: T) -> Json<Resp<T>> {
        Json(Resp { code: OK, msg: "操作成功".into(), data: Some(data) })
    }
    pub fn ok_msg(data: T, msg: &str) -> Json<Resp<T>> {
        Json(Resp { code: OK, msg: msg.into(), data: Some(data) })
    }
    pub fn none() -> Json<Resp<()>> {
        Json(Resp { code: OK, msg: "操作成功".into(), data: None })
    }
    pub fn msg(msg: &str) -> Json<Resp<()>> {
        Json(Resp { code: OK, msg: msg.into(), data: None })
    }
}

#[derive(Debug, Serialize)]
pub struct PageResp<T> {
    pub code: i32,
    pub msg: String,
    pub total: i64,
    pub rows: Vec<T>,
}

impl<T: Serialize> PageResp<T> {
    pub fn new(rows: Vec<T>, total: i64) -> Json<PageResp<T>> {
        Json(PageResp { code: OK, msg: "查询成功".into(), total, rows })
    }
}
