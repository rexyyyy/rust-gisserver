//! 路由装配。
//!
//! 约定（与前端 api 契约一致）：
//! * 所有接口挂在 `server.base_api`（默认 `/manager-api`）下；
//! * 静态路径必须注册在动态路径之前（`/system/user/list` 早于 `/system/user/{id}`）；
//! * 各业务域路由由各自模块的 `routes()` 构造器注册（各司其职），本文件只做装配：
//!   合并 → 鉴权中间件 → `with_state` 折叠 → `nest` 到 base_api。
//! * 配置了 `server.static_dir` 时，同端口额外提供前端页面（见 [`crate::staticfiles`]）。
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use http_body::Body as HttpBody;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, RequestId, SetRequestIdLayer};
use tower_http::trace::{MakeSpan, TraceLayer};

use crate::auth::auth_mw;
use crate::state::AppState;

pub mod apikey;
pub mod auth;
pub mod catalog;
pub mod engine;
pub mod mapsvc;
pub mod monitor;
pub mod system;

/// 自定义 trace span：在 tower-http 默认记录的 method / uri / status / latency 之外，
/// 额外记录 `request_id`（由最外层 SetRequestIdLayer 写入请求扩展的 `RequestId`），
/// 使逐请求日志与回传前端的 `x-request-id` 响应头对齐，便于跨服务串联排查。
/// `status` / `latency` 字段保留为空，供 `DefaultOnResponse` 在响应阶段回填。
#[derive(Clone, Copy, Default)]
struct TraceSpanMaker;

impl<B> MakeSpan<B> for TraceSpanMaker
where
    B: HttpBody,
{
    fn make_span(&mut self, request: &axum::http::Request<B>) -> tracing::Span {
        let req_id = request
            .extensions()
            .get::<RequestId>()
            .and_then(|r| r.header_value().to_str().ok())
            .unwrap_or("")
            .to_string();
        tracing::info_span!(
            "request",
            method = %request.method(),
            uri = %request.uri().path(),
            version = ?request.version(),
            request_id = %req_id,
            status = tracing::field::Empty,
            latency = tracing::field::Empty,
        )
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let base = state.cfg.server.base_api.clone();
    let cors: CorsLayer = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 各业务域路由合并（保持原注册顺序：auth → system → engine → map → monitor）。
    // 首个 merge 由 auth::routes() 播种 Router<Arc<AppState>> 类型；不能用 Router::new()
    // （那是 Router<()>，与带 State 提取器的子路由类型不匹配）。折叠统一在下方 with_state。
    //
    // API 子路由必须自带 fallback（api_not_found）：axum 的 `nest` 会把子路由的
    // fallback 一并嵌到 `/manager-api/*` 下，因此未匹配的 API 路径会先命中它；
    // 若不挂，未匹配的 `/manager-api/xxx` 会掉到顶层静态 fallback，
    // 被 SPA 回落成 index.html（前端拿到 HTML 却期望 JSON，报错难排查）。
    let api = auth::routes()
        .merge(system::routes())
        .merge(engine::routes())
        .merge(mapsvc::routes())
        .merge(catalog::routes())
        .merge(apikey::routes())
        .merge(monitor::routes())
        .route("/health", get(health))
        .fallback(api_not_found)
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_mw))
        // Localize JSON messages for en_US. Outermost API layer so 401
        // bodies produced by auth_mw are translated too (see src/i18n.rs).
        .layer(axum::middleware::from_fn(crate::i18n::i18n_response))
        .with_state(state.clone());

    let mut app = Router::new()
        .nest(&base, api)
        // 头像静态回显：顶层路由，不进 base_api 也不挂鉴权
        // （浏览器 <img src> 不会带 Authorization 头，走鉴权会导致 401）
        .route(
            &format!("{}/{{*path}}", system::avatar::AVATAR_URL_PREFIX),
            get(system::avatar::serve),
        )
        .route("/health", get(health))
        .layer(cors)
        // 逐请求追踪：方法 / 状态码 / 耗时 + 请求 ID（与下方 x-request-id 串联）。
        .layer(TraceLayer::new_for_http().make_span_with(TraceSpanMaker))
        // 请求 ID：每个请求生成唯一 id 并经 `x-request-id` 响应头回传，作为最外层
        // 让追踪等内层中间件都能读到，供前端 / 网关 / 日志跨服务对齐同一请求。
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid::default()))
        // 顶层路由同样需要 state（头像静态回显要读 avatar_root）
        .with_state(state.clone());

    // 可选：同端口提供前端页面（SPA 同源部署）。
    // 静态服务作为**顶层 fallback**，因此不会影响已注册的 /manager-api/** 与 /profile/**。
    // 缓存中间件只包住静态服务（包成独立 Router 再作为 fallback），
    // 不污染 API 与其它顶层路由的响应头。
    if let Some(dir) = crate::staticfiles::resolve_dir(state.cfg.server.static_dir.as_deref()) {
        let static_svc = crate::staticfiles::build_service(&crate::staticfiles::StaticOptions { dir });
        let static_router = Router::new()
            .fallback_service(static_svc)
            .layer(axum::middleware::from_fn(crate::staticfiles::cache_middleware))
            // 前端 JS/CSS 等静态资源按需压缩（gzip/br，按 Accept-Encoding 协商），
            // API 响应不经过此 Router，保持原有行为不变。
            .layer(CompressionLayer::new());
        app = app.fallback_service(static_router);
    }

    app
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "code": 200, "msg": "ok" }))
}

/// API 路径未命中时的响应。
///
/// 保持前端契约形状（`{code,msg}`），避免前端拦截器因非 JSON 响应报错。
/// 注意鉴权中间件挂在这层之上：未登录访问未注册接口会先被 401 拦下，
/// 只有已登录请求才会走到这里看到 404。
async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "code": 404, "msg": "接口不存在" })))
}
