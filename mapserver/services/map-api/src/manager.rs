//! 运行模式判定：启动时探测归属管理端（manager）。
//!
//! map-api 是地图引擎，不托管 UI。它在配置里声明「受哪个 manager 管理」，
//! 启动时对该 manager 做一次短超时 HTTP 探活：
//!
//! * 探活成功 → `Managed`：由该 manager 提供界面与预览入口，引擎只出数据；
//! * 探活失败 / 未配置 → `Standalone`：独立引擎，不挂任何预览 / 页面路由，
//!   仅提供 OGC 数据面、运维面与管理面。
//!
//! 探测只发一个 `GET /health`，不走鉴权、不需要额外依赖：manager 的顶层
//! `/health` 是免鉴权探活端点，与 `/manager-api/*` 的鉴权中间件分开。
//! 判定结果随 `/health` 暴露，供巡检与部署脚本读取。

use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::ManagerOpts;

/// 启动自检后的运行模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// 已连通配置的管理端，界面由 manager 托管。
    Managed,
    /// 未找到管理端，纯引擎运行。
    Standalone,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::Managed => "managed",
            RunMode::Standalone => "standalone",
        }
    }
}

/// 归属关系，写入 `AppState` 并随 `/health` 暴露。
#[derive(Clone, Debug, Serialize)]
pub struct ManagerLink {
    pub mode: RunMode,
    /// 受管时的管理端基址；独立模式为 `None`。
    pub url: Option<String>,
}

impl ManagerLink {
    pub fn standalone() -> Self {
        ManagerLink { mode: RunMode::Standalone, url: None }
    }

    pub fn is_managed(&self) -> bool {
        self.mode == RunMode::Managed
    }
}

/// 拆 `http://host[:port][/prefix]` → (`host:port`, `prefix`)。
/// 只接内网 HTTP；`https://` 需 TLS 栈，当前不支持，交由调用方按独立模式处理。
fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, prefix) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return None;
    }
    let authority = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    Some((authority, prefix.to_string()))
}

/// 对 manager 发一次 `GET <prefix>/health`，只要状态行是 2xx 即视为可达。
///
/// 手写最小 HTTP/1.1 请求，避免为一次启动探活引入 reqwest / TLS 依赖；
/// 只读取响应头（不关心 body），`Connection: close` 让对端主动收尾。
async fn probe(url: &str, timeout_ms: u64) -> bool {
    let Some((authority, prefix)) = split_url(url) else {
        return false;
    };
    let path = format!("{}/health", prefix.trim_end_matches('/'));
    let attempt = async {
        let mut stream = TcpStream::connect(&authority).await.ok()?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut buf = [0u8; 512];
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        let head = String::from_utf8_lossy(&buf[..n]);
        let code = head.lines().next().unwrap_or("").split_whitespace().nth(1)?;
        Some(code.starts_with('2'))
    };
    tokio::time::timeout(Duration::from_millis(timeout_ms), attempt)
        .await
        .unwrap_or(None)
        .unwrap_or(false)
}

/// 按配置判定运行模式，并写一条明确的启动日志。
pub async fn resolve(opts: &ManagerOpts) -> ManagerLink {
    let Some(raw) = opts.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        tracing::info!("未声明归属管理端：独立引擎模式（仅提供数据服务）");
        return ManagerLink::standalone();
    };
    if !raw.starts_with("http://") {
        tracing::warn!(url = %raw, "管理端地址须为 http://host:port，按独立引擎模式启动");
        return ManagerLink::standalone();
    }
    if probe(raw, opts.probe_timeout_ms).await {
        let url = raw.trim_end_matches('/').to_string();
        tracing::info!(manager = %url, "受管模式：管理端已连通，界面与预览由其托管");
        ManagerLink {
            mode: RunMode::Managed,
            url: Some(url),
        }
    } else {
        tracing::warn!(manager = %raw, "未找到管理端：独立引擎模式（仅提供数据服务）");
        ManagerLink::standalone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_handles_host_port_and_prefix() {
        assert_eq!(
            split_url("http://127.0.0.1:6180"),
            Some(("127.0.0.1:6180".into(), "".into()))
        );
        assert_eq!(
            split_url("http://gis.example.com/console"),
            Some(("gis.example.com:80".into(), "/console".into()))
        );
        assert_eq!(split_url("https://127.0.0.1:6180"), None);
        assert_eq!(split_url("http://"), None);
    }

    #[tokio::test]
    async fn unconfigured_is_standalone() {
        let link = resolve(&ManagerOpts::default()).await;
        assert_eq!(link.mode, RunMode::Standalone);
        assert!(link.url.is_none());
    }

    #[tokio::test]
    async fn unreachable_manager_is_standalone() {
        let link = resolve(&ManagerOpts {
            url: Some("http://127.0.0.1:1".into()),
            probe_timeout_ms: 200,
        })
        .await;
        assert_eq!(link.mode, RunMode::Standalone);
        assert!(!link.is_managed());
    }

    /// 真起一个临时 HTTP 端点模拟 manager，验证探活通过时进入受管模式。
    #[tokio::test]
    async fn reachable_manager_is_managed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .await;
        });
        let url = format!("http://{addr}");
        let link = resolve(&ManagerOpts {
            url: Some(url.clone()),
            probe_timeout_ms: 1000,
        })
        .await;
        assert_eq!(link.mode, RunMode::Managed);
        assert!(link.is_managed());
        assert_eq!(link.url.as_deref(), Some(url.as_str()));
    }
}
