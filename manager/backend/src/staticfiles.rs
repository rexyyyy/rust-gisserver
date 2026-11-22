//! 前端静态资源托管（SPA 同源部署）。
//!
//! # 为什么需要它
//!
//! rust-gisserver 后端原本是**纯 API 服务**（不挂任何静态资源），前端 `ui/dist`
//! 需要由外部静态服务器（Vite dev server / Nginx / IIS）提供。这带来两个问题：
//!
//! 1. **跨域**：前端开发时走 Vite 代理，生产若前后端不同源就要处理 CORS；
//! 2. **部署件多**：现场要同时维护后端进程和静态服务器两套东西。
//!
//! 本模块让后端**同端口同源**直接提供前端页面：
//!
//! ```text
//!   http://host:6180/                 → ui/dist/index.html
//!   http://host:6180/assets/x.js      → ui/dist/assets/x.js
//!   http://host:6180/manager-api/**   → 业务 API（不受影响）
//! ```
//!
//! 前端 API 用的是**相对路径** `/manager-api`（见 `VITE_APP_BASE_API`），
//! 同源部署天然零 CORS，无需任何额外配置。
//!
//! # 关键设计
//!
//! * **SPA fallback**：前端用 `createWebHistory`（history 模式），
//!   路由如 `/map/service` 在磁盘上并不存在。直接刷新会 404，
//!   因此未命中静态文件的请求一律回落到 `index.html`，
//!   交给前端路由接管。这是 history 模式 SPA 的标准做法。
//! * **API 404 不被吞**：`handlers::router()` 给 API 子路由单独挂了 fallback，
//!   因为 axum 的 `nest` 会把子路由的 fallback 一并嵌到 `/manager-api/*` 下，
//!   未匹配的 API 路径会先命中它，不会掉到静态 fallback 变成 index.html。
//! * **缓存策略**：带内容 hash 的 `/assets/*` 可长期缓存（immutable），
//!   而 `index.html` 必须每次校验，否则前端发版后用户拿到旧壳子配新资源而白屏。
//! * **路径穿越防护**：`tower-http` 的 `ServeDir` 内部经 `open_file` 校验，
//!   含 `..` 的路径会被判为 `InvalidFilename`（见 tower-http 的
//!   `serve_dir/open_file.rs`），另有 `percent-encoding` 解码环节。
//!   这里额外做一次显式校验，不依赖底层库的实现细节。
//!
//!   实测（tower-http 0.6.11）：`%2e%2e` 这类会被 axum/matchit 在**路由匹配阶段**
//!   解码并规范化（`/%2e%2e/x` → `/x`），路径被收敛到 dist 目录内，安全；
//!   而 `%2f`（编码斜杠）不被解码为分隔符，原始形态保留到静态层，
//!   由本模块的显式校验拦下并返回 400。

use std::path::{Path, PathBuf};

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tower_http::services::{ServeDir, ServeFile};

/// 挂载静态资源时使用的内部前缀标记（仅日志用）。
const STATIC_MOUNT_HINT: &str = "静态资源";

/// 静态资源服务配置。
pub struct StaticOptions {
    /// 静态资源根目录（通常是 `ui/dist`）。
    pub dir: PathBuf,
}

/// 构造静态资源服务：`ServeDir` + SPA fallback。
///
/// 返回值可直接作为 axum 的 `fallback_service`：`ServeDir` 派生了 `Clone`，
/// 其 `Error` 类型是 `Infallible`（tower-http `serve_dir/mod.rs`），
/// 正好满足 axum 0.7 对 fallback service 的约束
/// （`Service<Request, Error = Infallible> + Clone + Send + 'static`）。
///
/// 未命中文件时回落到 `index.html`（history 模式 SPA 的必需行为）。
pub fn build_service(opts: &StaticOptions) -> ServeDir<ServeFile> {
    let dir = &opts.dir;
    let index = dir.join("index.html");

    // ServeDir 命中不到文件时 → 回落 index.html（SPA 路由接管）
    ServeDir::new(dir)
        // 目录请求补 index.html（如 / → index.html）
        .append_index_html_on_directories(true)
        // 未命中 → index.html；注意用 fallback 而非 not_found_service，
        // 前者保持 200 状态码（history 模式下这才是前端期望的语义）
        .fallback(ServeFile::new(index))
}

/// 显式拒绝含路径穿越特征的请求。
///
/// 注意：`Uri::path()` 返回的是**原始编码**路径，`%2e%2e` 这类编码绕过
/// 只有 percent-decoding 之后才看得见，因此这里先解码再判定。
///
/// `tower-http` 的 `ServeDir` 内部经 `open_file` 校验，含 `..` 的路径会被判为
/// `InvalidFilename`（见 tower-http `serve_dir/open_file.rs`）。这里是**纵深防御**：
/// 不依赖底层库的实现细节。
pub fn is_unsafe_path(path: &str) -> bool {
    // percent-decode 后再判定（解码失败时退回原始串）
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .into_owned();

    // 路径里出现 `..` 段
    decoded.split('/').any(|seg| seg == "..")
        // Windows 盘符 / UNC 前缀
        || decoded.contains('\\')
        || decoded.contains(':')
}

/// 构造缓存策略中间件所需的响应头覆盖规则。
///
/// * `/assets/*`（文件名带内容 hash）→ `public, max-age=31536000, immutable`
/// * `index.html` → `no-cache`（每次校验，保证发版后立刻生效）
/// * 其余 → 不干预
pub fn cache_control_for(path: &str) -> Option<&'static str> {
    if path.starts_with("/assets/") {
        Some("public, max-age=31536000, immutable")
    } else if path.ends_with("/index.html") || path == "/" {
        Some("no-cache")
    } else {
        None
    }
}

/// 给响应打上缓存头。
pub fn apply_cache_control(resp: Response, path: &str) -> Response {
    let Some(cc) = cache_control_for(path) else {
        return resp;
    };
    let mut resp = resp;
    if let Ok(v) = HeaderValue::from_str(cc) {
        resp.headers_mut().insert(header::CACHE_CONTROL, v);
    }
    resp
}

/// 静态资源缓存中间件：按请求路径补 `Cache-Control`，
/// 并拦截含路径穿越特征的请求。
///
/// 只包裹静态服务，不影响 API 与其它顶层路由的响应头。
pub async fn cache_middleware(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.uri().path().to_string();
    // 纵深防御：即便底层静态文件处理器已有防护，这里再拦一道，
    // 保证任何含 `..` / 反斜杠 / 盘符的路径都不会去碰文件系统。
    if is_unsafe_path(&path) {
        return (StatusCode::BAD_REQUEST, "非法路径").into_response();
    }
    let resp = next.run(req).await;
    apply_cache_control(resp, &path)
}

/// 解析配置里的静态目录：相对路径按**进程工作目录**解析。
///
/// 返回 `None` 表示不挂静态资源（纯 API 模式）。
pub fn resolve_dir(raw: Option<&str>) -> Option<PathBuf> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let p = Path::new(raw);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
    };
    // 规范化（解析 .. 与符号链接），失败时退回未规范化路径
    Some(std::fs::canonicalize(&abs).unwrap_or(abs))
}

/// 启动日志：把静态资源的挂载情况说清楚，避免现场「以为挂上了其实没挂」。
pub fn log_mount(dir: Option<&Path>) {
    match dir {
        Some(d) => {
            let index = d.join("index.html");
            if index.is_file() {
                tracing::info!("{STATIC_MOUNT_HINT}: 已启用，根目录 {}（SPA fallback → index.html）", d.display());
            } else {
                tracing::warn!(
                    "{STATIC_MOUNT_HINT}: 目录 {} 存在但缺少 index.html，前端路由刷新将 404；请确认这是 ui/dist",
                    d.display()
                );
            }
        }
        None => {
            tracing::info!("{STATIC_MOUNT_HINT}: 未启用（server.static_dir 未配置），本进程仅提供 API");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_control_assets_are_immutable() {
        assert_eq!(
            cache_control_for("/assets/index-abc123.js"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(
            cache_control_for("/assets/index-abc123.css"),
            Some("public, max-age=31536000, immutable")
        );
    }

    #[test]
    fn cache_control_index_must_revalidate() {
        // index.html 若被长期缓存，前端发版后用户会拿到旧壳子配新资源 → 白屏
        assert_eq!(cache_control_for("/index.html"), Some("no-cache"));
        assert_eq!(cache_control_for("/"), Some("no-cache"));
    }

    #[test]
    fn cache_control_other_paths_untouched() {
        assert_eq!(cache_control_for("/favicon.ico"), None);
        assert_eq!(cache_control_for("/profile/avatar/x.png"), None);
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(is_unsafe_path("/../secret.txt"));
        assert!(is_unsafe_path("/assets/../../etc/passwd"));
        assert!(is_unsafe_path("/a/../../b"));
        // 反斜杠与盘符
        assert!(is_unsafe_path("/a\\b"));
        assert!(is_unsafe_path("C:/Windows/system32"));
    }

    #[test]
    fn percent_encoded_traversal_is_rejected() {
        // Uri::path() 返回原始编码路径，只有解码后才看得见穿越意图
        assert!(is_unsafe_path("/%2e%2e/Cargo.toml"));
        assert!(is_unsafe_path("/%2E%2E%2Fsecret"));
        assert!(is_unsafe_path("/assets/%2e%2e/%2e%2e/Cargo.toml"));
        // 混合编码
        assert!(is_unsafe_path("/..%2fCargo.toml"));
        assert!(is_unsafe_path("/%2e./secret"));
    }

    #[test]
    fn safe_paths_are_accepted() {
        assert!(!is_unsafe_path("/assets/index-abc.js"));
        assert!(!is_unsafe_path("/map/service"));
        assert!(!is_unsafe_path("/"));
        // 文件名里的 ".." 出现在中间而非路径段时不算穿越
        assert!(!is_unsafe_path("/assets/a..b.js"));
    }

    #[test]
    fn resolve_dir_empty_means_disabled() {
        assert!(resolve_dir(None).is_none());
        assert!(resolve_dir(Some("")).is_none());
        assert!(resolve_dir(Some("   ")).is_none());
    }

    #[test]
    fn resolve_dir_returns_absolute_path() {
        let d = resolve_dir(Some(".")).expect("应解析出当前目录");
        assert!(d.is_absolute(), "相对路径必须解析为绝对路径，实际: {}", d.display());
    }

    #[test]
    fn build_service_is_cloneable() {
        // axum 的 fallback_service 要求 Clone + Send + 'static
        let dir = std::env::temp_dir();
        let svc = build_service(&StaticOptions { dir });
        let _cloned = svc.clone();
    }
}
