//! 用户头像：本地文件存储 + 静态回显。
//!
//! 背景：早期结构 `sys_user.avatar` 为 bigint（OSS 文件 id），本项目不引入 OSS，
//! 改为 varchar(500) 存放相对路径（如 `/profile/avatar/2026/10/26/u1_a1b2c3.png`），
//! 与前端 `user.avatar` 为「可直接访问的 URL 字符串」的期望一致。
//!
//! 设计要点：
//! * 上传走 multipart/form-data，字段名 `avatarfile`（与前端 `uploadAvatar` 一致）；
//! * 只接受 PNG/JPEG，按「文件头魔数」二次校验，不信任客户端的 content-type；
//! * 落盘文件名用 `用户id_随机串`，避免覆盖与路径穿越；
//! * 静态回显走 `/profile/avatar/*path`，由 axum 直接读文件返回，不入库。
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Multipart, Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 对外访问前缀（挂在 base_api 之外的顶层路径，前端可直接引用）。
pub const AVATAR_URL_PREFIX: &str = "/profile/avatar";

/// 单个头像最大 5MB。
const MAX_SIZE: usize = 5 * 1024 * 1024;

/// POST /system/user/profile/avatar
///
/// 请求体：`multipart/form-data`，字段名 `avatarfile`。
/// 响应：`{ code: 200, msg: "操作成功", data: { imgUrl: "/profile/avatar/..." } }`
pub async fn upload(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> AppResult<axum::Json<Value>> {
    let mut file_bytes: Option<(String, Vec<u8>)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad(format!("读取上传内容失败: {e}")))?
    {
        // 只取约定的字段名，避免误收其它表单字段
        if field.name() != Some("avatarfile") {
            continue;
        }
        let file_name = field.file_name().unwrap_or("").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::bad(format!("读取文件内容失败: {e}")))?;
        file_bytes = Some((file_name, bytes.to_vec()));
        break;
    }

    let (file_name, bytes) = file_bytes.ok_or_else(|| AppError::bad("未找到上传文件"))?;
    if bytes.is_empty() {
        return Err(AppError::bad("上传文件为空"));
    }
    if bytes.len() > MAX_SIZE {
        return Err(AppError::bad("图片大小不能超过 5MB"));
    }

    // 以文件头魔数判定类型，不信任客户端给的扩展名/content-type
    let ext = sniff_image(&bytes).ok_or_else(|| AppError::bad("仅支持 PNG/JPEG 格式的图片"))?;

    // 用 uuid 命名，杜绝路径穿越与同名覆盖
    let stem = uuid::Uuid::new_v4().simple().to_string();
    let rel = format!("{}/{}/{}/u{}_{}.{}", now_year(), now_month(), now_day(), user.user_id, stem, ext);

    let target = state.avatar_root.join(&rel);
    let target_clone = target.clone();
    let bytes_clone = bytes.clone();
    let avatar_root = state.avatar_root.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(target_clone.parent().unwrap_or(&avatar_root))
            .map_err(|e| AppError::bad(format!("创建头像目录失败: {e}")))?;
        std::fs::write(&target_clone, &bytes_clone)
            .map_err(|e| AppError::bad(format!("保存头像失败: {e}")))?;
        Ok::<_, AppError>(())
    })
    .await
    .map_err(|e| AppError::bad(format!("spawn_blocking join 错误: {e}")))?
    .map_err(|e| AppError::bad(format!("保存头像失败: {e}")))?;

    let url = format!("{AVATAR_URL_PREFIX}/{rel}");

    // 回写当前用户的头像字段
    let sql = "UPDATE sys_user SET avatar = $1, update_by = $2, update_time = NOW() WHERE user_id = $3";
    let pv = crate::pv![url.clone(), user.user_id, user.user_id];
    crate::state::execute(&state.pool, sql, &pv.refs()).await?;

    tracing::info!("用户 {} 上传头像: {} ({})", user.user_id, url, file_name);

    Ok(axum::Json(json!({
        "code": 200,
        "msg": "操作成功",
        "data": { "imgUrl": url }
    })))
}

/// GET /profile/avatar/*path —— 回显已保存的头像文件。
///
/// 放在顶层路由（不进 base_api、不挂鉴权中间件），因为 `<img src>` 不会带
/// Authorization 头，走鉴权会导致图片 401。
pub async fn serve(
    State(state): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    // 路径穿越防护：拒绝 `..`、绝对路径、反斜杠
    if path.contains("..") || path.contains('\\') || path.starts_with('/') {
        return (StatusCode::BAD_REQUEST, "非法路径").into_response();
    }

    let root = state.avatar_root.clone();
    let full = root.join(&path);
    // 归一化后必须仍在根目录内（spawn_blocking 避免阻塞事件循环）
    let full_clone = full.clone();
    let resolved = match tokio::task::spawn_blocking(move || {
        std::fs::canonicalize(&full_clone).unwrap_or_else(|_| full_clone.clone())
    })
    .await
    {
        Ok(r) => r,
        Err(_) => full.clone(),
    };
    if !resolved.starts_with(&root) {
        return (StatusCode::BAD_REQUEST, "非法路径").into_response();
    }

    match tokio::fs::read(&resolved).await {
        Ok(data) => {
            let mime = match resolved.extension().and_then(|e| e.to_str()) {
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                _ => "application/octet-stream",
            };
            let mut resp = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, "public, max-age=86400");
            if let Ok(md) = tokio::fs::metadata(&resolved).await {
                resp = resp.header(header::CONTENT_LENGTH, md.len());
            }
            resp.body(Body::from(data)).unwrap_or_else(|_| {
                (StatusCode::INTERNAL_SERVER_ERROR, "响应构造失败").into_response()
            })
        }
        Err(_) => (StatusCode::NOT_FOUND, "文件不存在").into_response(),
    }
}

/// 依据文件头魔数判断图片类型，返回小写扩展名。
fn sniff_image(b: &[u8]) -> Option<&'static str> {
    if b.len() >= 8 && &b[0..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return Some("png");
    }
    // JPEG：FF D8 FF
    if b.len() >= 3 && &b[0..3] == [0xff, 0xd8, 0xff] {
        return Some("jpg");
    }
    None
}

fn now_year() -> String {
    chrono::Local::now().format("%Y").to_string()
}
fn now_month() -> String {
    chrono::Local::now().format("%m").to_string()
}
fn now_day() -> String {
    chrono::Local::now().format("%d").to_string()
}
