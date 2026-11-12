//! r-mapserver 系统管理后端（axum + PostgreSQL 单库）。
mod auth;
mod captcha;
mod config;
mod engine_monitor;
mod error;
mod handlers;
mod models;
mod perm;
mod resp;
mod state;
mod staticfiles;
mod util;
mod xmlcfg;

use std::sync::Arc;

use tokio::signal;

use crate::state::{build_pool, AppState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = config::Config::load_default()?;
    let pool = build_pool(&cfg)?;
    // 起服务前先探一次库，避免带着坏连接串启动
    {
        let c = pool.get().await?;
        let row = c.query_one("SELECT version()", &[]).await?;
        let v: String = row.try_get(0)?;
        tracing::info!("已连接 PostgreSQL: {}", v.split(',').next().unwrap_or(&v));
    }

    let state = Arc::new(AppState {
        pool,
        cfg: Arc::new(cfg.clone()),
        online: Default::default(),
        perms: Default::default(),
        captcha: Default::default(),
        avatar_root: resolve_avatar_root(&cfg),
        map_http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
    });

    let addr = cfg.server.addr.clone();
    tracing::info!("系统管理后端启动: http://{}{}", addr, cfg.server.base_api);
    // 前端静态资源（同端口部署时提供页面；未配置则仅提供 API）
    staticfiles::log_mount(staticfiles::resolve_dir(cfg.server.static_dir.as_deref()).as_deref());
    // 引擎后台巡检（30s）：引擎存活/服务状态变化写 sys_engine_event
    engine_monitor::spawn(state.clone());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // 优雅停机：收到 SIGINT(Ctrl-C) / SIGTERM 后停止接收新连接，
    // 等待在途请求完成再退出，避免发布/重启掐断前端已发出的请求。
    axum::serve(listener, handlers::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// 停机信号：Ctrl-C 跨平台；SIGTERM 仅 Unix（Windows 下永不触发）。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            let _ = sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("收到 Ctrl-C，开始优雅停机"),
        _ = terminate => tracing::info!("收到 SIGTERM，开始优雅停机"),
    }
}

/// 解析头像存储根目录并确保其存在，返回**规范化后的绝对路径**。
///
/// 相对路径按进程工作目录解析；绝对路径原样使用（便于服务化部署时挂载数据盘）。
///
/// 必须做规范化：Windows 下 `canonicalize` 会返回 `\\?\` 前缀的 verbatim 路径，
/// 而静态回显的安全校验用 `starts_with` 做组件级比较，根目录若未规范化则永远
/// 比对不上（表现为上传成功但回显 400）。
fn resolve_avatar_root(cfg: &config::Config) -> std::path::PathBuf {
    let raw = std::path::Path::new(&cfg.storage.avatar_dir);
    let abs = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(raw)
    };
    if let Err(e) = std::fs::create_dir_all(&abs) {
        tracing::error!("创建头像目录失败: {} ({})", abs.display(), e);
        return abs;
    }
    // 规范化（解析 `..`、软链接、统一分隔符），失败时退回未规范化路径
    let root = std::fs::canonicalize(&abs).unwrap_or(abs);
    tracing::info!("头像存储目录: {}", root.display());
    root
}
