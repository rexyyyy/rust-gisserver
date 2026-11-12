//! 共享库 API Key 缓存：rust-gisserver 生成的访问凭证在这里被 rust-gisserver 认可。
//!
//! 设计（与 manager 端 `sys_api_key` 表对应）：
//! * manager 与 map-api 连同一个 PostgreSQL 库（`admin.dsn`）；
//! * 本模块每 15 秒读取一次启用中、未过期的 key，按 `purpose` 分成
//!   `admin` / `data` 两组，写入内存缓存；校验在内存完成，零逐请求查库；
//! * **主令牌仍是「是否启用校验」的开关**：`admin.token` / `server.data_token`
//!   未配置时数据面与管理面照旧不设防（零配置向后兼容）；配置后，主令牌
//!   **或**库中同用途的任一启用 key 均被认可；
//! * 表不存在 / DSN 不可达 / 未配 DSN 时静默跳过（只打 debug 日志），
//!   不影响瓦片服务：旧部署与单文件目录后端保持原行为；
//! * 用途严格隔离：admin key 不能访问数据面，data key 不能访问管理面。
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use pg_source::PgPool;

/// 刷新间隔：manager 生成/停用 key 后最多 15 秒在 server 侧生效。
const REFRESH_SECS: u64 = 15;

#[derive(Debug, Default)]
pub struct ApiKeySet {
    pub admin: HashSet<String>,
    pub data: HashSet<String>,
}

/// 可克隆的共享句柄（`AppState` 与后台刷新任务各持一份）。
#[derive(Clone, Debug, Default)]
pub struct ApiKeyCache {
    inner: Arc<RwLock<ApiKeySet>>,
}

impl ApiKeyCache {
    /// 该用途下 `key` 是否为库中启用且未过期的凭证。
    pub fn is_valid(&self, purpose: &str, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        match purpose {
            "admin" => guard.admin.contains(key),
            "data" => guard.data.contains(key),
            _ => false,
        }
    }

    /// 当前缓存的 (admin, data) 数量，供日志与运维观察。
    pub fn counts(&self) -> (usize, usize) {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        (guard.admin.len(), guard.data.len())
    }

    fn replace(&self, set: ApiKeySet) {
        *self.inner.write().unwrap_or_else(|e| e.into_inner()) = set;
    }
}

/// 启动后台刷新任务（需要在 tokio runtime 内调用）。
///
/// 连接失败时释放池并在下一轮重建，避免坏连接被永久复用；表不存在属于
/// 预期情况（旧库尚未跑 `map.sql`），只记 debug，不刷屏。
pub fn spawn_refresh(dsn: Option<String>, cache: ApiKeyCache) {
    let Some(dsn) = dsn.filter(|s| !s.trim().is_empty()) else {
        tracing::info!("apikey sync disabled (no admin.dsn configured)");
        return;
    };
    tracing::info!(every_secs = REFRESH_SECS, "apikey sync enabled (shared PostgreSQL)");
    tokio::spawn(async move {
        let mut pool: Option<Arc<PgPool>> = None;
        let mut tick = tokio::time::interval(Duration::from_secs(REFRESH_SECS));
        loop {
            tick.tick().await; // first tick completes immediately
            if pool.is_none() {
                match PgPool::new_plain(&dsn, 2).await {
                    Ok(p) => pool = Some(Arc::new(p)),
                    Err(e) => {
                        tracing::debug!("apikey sync: registry DB unavailable: {e}");
                        continue;
                    }
                }
            }
            let Some(p) = pool.as_ref() else { continue };
            match load(p).await {
                Ok(set) => {
                    tracing::debug!(admin = set.admin.len(), data = set.data.len(), "apikey cache refreshed");
                    cache.replace(set);
                }
                Err(e) => {
                    // 表不存在 / 连接断开：下轮重连再试。
                    tracing::debug!("apikey sync: load failed (will retry): {e}");
                    pool = None;
                }
            }
        }
    });
}

async fn load(pool: &PgPool) -> Result<ApiKeySet, String> {
    let conn = pool.get().await.map_err(|e| e.to_string())?;
    let rows = conn
        .query(
            "SELECT purpose, api_key FROM sys_api_key \
             WHERE status = '0' AND (expires_at IS NULL OR expires_at > now())",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut set = ApiKeySet::default();
    for r in rows {
        let purpose: String = r.get(0);
        let key: String = r.get(1);
        match purpose.as_str() {
            "admin" => {
                set.admin.insert(key);
            }
            "data" => {
                set.data.insert(key);
            }
            _ => {}
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with(admin: &[&str], data: &[&str]) -> ApiKeyCache {
        let c = ApiKeyCache::default();
        c.replace(ApiKeySet {
            admin: admin.iter().map(|s| s.to_string()).collect(),
            data: data.iter().map(|s| s.to_string()).collect(),
        });
        c
    }

    #[test]
    fn purposes_are_isolated() {
        let c = cache_with(&["rms_admin_A"], &["rms_data_B"]);
        assert!(c.is_valid("admin", "rms_admin_A"));
        assert!(c.is_valid("data", "rms_data_B"));
        assert!(!c.is_valid("admin", "rms_data_B"));
        assert!(!c.is_valid("data", "rms_admin_A"));
        assert!(!c.is_valid("other", "rms_admin_A"));
        assert!(!c.is_valid("admin", ""));
        assert_eq!(c.counts(), (1, 1));
    }

    #[test]
    fn replace_retires_revoked_keys_immediately() {
        let c = cache_with(&["k1"], &["k2"]);
        assert!(c.is_valid("data", "k2"));
        c.replace(ApiKeySet::default());
        assert!(!c.is_valid("data", "k2"));
        assert_eq!(c.counts(), (0, 0));
    }
}
