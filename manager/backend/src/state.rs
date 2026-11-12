//! 共享状态：连接池、配置、在线会话表、权限缓存。
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::config::Config;
use deadpool_postgres::{ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::{NoTls, Row};

/// 一次登录后登记到内存中的会话（用于「在线用户」与强制下线）。
#[derive(Debug, Clone)]
pub struct OnlineSession {
    pub token_id: String,
    pub user_id: i64,
    pub user_name: String,
    pub dept_name: String,
    pub ipaddr: String,
    pub browser: String,
    pub os: String,
    pub login_time: chrono::NaiveDateTime,
    pub last_access: Instant,
}

#[derive(Debug, Default)]
pub struct OnlineRegistry {
    inner: Mutex<HashMap<String, OnlineSession>>,
    kicked: Mutex<Vec<String>>,
}

impl OnlineRegistry {
    pub fn put(&self, s: OnlineSession) {
        let mut m = self.inner.lock().unwrap();
        m.insert(s.token_id.clone(), s);
    }
    pub fn touch(&self, token_id: &str) {
        if let Some(s) = self.inner.lock().unwrap().get_mut(token_id) {
            s.last_access = Instant::now();
        }
    }
    pub fn remove(&self, token_id: &str) {
        self.inner.lock().unwrap().remove(token_id);
        self.kicked.lock().unwrap().push(token_id.to_string());
    }
    /// 被强踢的 token 需要让鉴权中间件识别为失效。
    pub fn is_kicked(&self, token_id: &str) -> bool {
        self.kicked.lock().unwrap().contains(&token_id.to_string())
    }
    pub fn list(&self) -> Vec<crate::models::OnlineVO> {
        self.inner
            .lock()
            .unwrap()
            .values()
            .map(|s| crate::models::OnlineVO {
                token_id: s.token_id.clone(),
                user_name: s.user_name.clone(),
                dept_name: s.dept_name.clone(),
                ipaddr: s.ipaddr.clone(),
                browser: s.browser.clone(),
                os: s.os.clone(),
                login_time: crate::models::fmt_dt(Some(s.login_time)),
                nick_name: s.user_name.clone(),
            })
            .collect()
    }
    /// 列出某个用户的在线会话（上游 `GET /monitor/online` 返回当前登录用户的设备）。
    pub fn list_by_user(&self, user_id: i64) -> Vec<crate::models::OnlineVO> {
        self.inner
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.user_id == user_id)
            .map(|s| crate::models::OnlineVO {
                token_id: s.token_id.clone(),
                user_name: s.user_name.clone(),
                dept_name: s.dept_name.clone(),
                ipaddr: s.ipaddr.clone(),
                browser: s.browser.clone(),
                os: s.os.clone(),
                login_time: crate::models::fmt_dt(Some(s.login_time)),
                nick_name: s.user_name.clone(),
            })
            .collect()
    }
    /// 判断某 token 是否属于该用户（防止越权踢掉别人的会话）。
    pub fn owns(&self, token_id: &str, user_id: i64) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(token_id)
            .map(|s| s.user_id == user_id)
            .unwrap_or(false)
    }
    pub fn remove_by_user(&self, user_name: &str) {
        let ids: Vec<String> = self
            .inner
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.user_name == user_name)
            .map(|s| s.token_id.clone())
            .collect();
        for id in ids {
            self.remove(&id);
        }
    }
}

/// 用户权限缓存：60 秒刷新一次，避免逐请求查库。
#[derive(Debug, Default)]
pub struct PermCache {
    inner: RwLock<HashMap<i64, (Instant, Vec<String>, Vec<String>)>>,
}

impl PermCache {
    const TTL: Duration = Duration::from_secs(60);

    pub fn get(&self, user_id: i64) -> Option<(Vec<String>, Vec<String>)> {
        let m = self.inner.read().unwrap();
        let (ts, roles, perms) = m.get(&user_id)?;
        if ts.elapsed() > Self::TTL {
            return None;
        }
        Some((roles.clone(), perms.clone()))
    }
    pub fn put(&self, user_id: i64, roles: Vec<String>, perms: Vec<String>) {
        self.inner
            .write()
            .unwrap()
            .insert(user_id, (Instant::now(), roles, perms));
    }
    pub fn invalidate(&self, user_id: i64) {
        self.inner.write().unwrap().remove(&user_id);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub cfg: Arc<Config>,
    pub online: Arc<OnlineRegistry>,
    pub perms: Arc<PermCache>,
    pub captcha: Arc<crate::captcha::CaptchaStore>,
    /// 头像存储根目录（启动时已解析为绝对路径），供上传与静态回显共用。
    pub avatar_root: std::path::PathBuf,
    /// map-api 管理面 HTTP 客户端（reqwest Client 内部连接池，克隆廉价）。
    pub map_http: reqwest::Client,
}

pub fn build_pool(cfg: &Config) -> Result<Pool, tokio_postgres::Error> {
    let url = cfg.database.url.as_str();
    let parsed: tokio_postgres::Config = url.parse()?;
    let manager = deadpool_postgres::Manager::from_config(
        parsed,
        NoTls,
        ManagerConfig { recycling_method: RecyclingMethod::Fast },
    );
    let pool = Pool::builder(manager)
        .max_size(cfg.database.pool_max)
        .runtime(Runtime::Tokio1)
        .build()
        .expect("构建 postgres 连接池失败");
    Ok(pool)
}

/// 取连接。
pub async fn conn<'a>(
    pool: &'a Pool,
) -> Result<deadpool_postgres::Object, crate::error::AppError> {
    Ok(pool.get().await?)
}

/// 执行查询并返回全部行。参数列表为空时使用不带参数的 `query`。
pub async fn query(
    pool: &Pool,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<Vec<Row>, crate::error::AppError> {
    let c = conn(pool).await?;
    let rows = if params.is_empty() {
        c.query(sql, &[]).await?
    } else {
        c.query(sql, params).await?
    };
    Ok(rows)
}

/// 执行增删改，返回影响行数。
pub async fn execute(
    pool: &Pool,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<u64, crate::error::AppError> {
    let c = conn(pool).await?;
    let n = if params.is_empty() {
        c.execute(sql, &[]).await?
    } else {
        c.execute(sql, params).await?
    };
    Ok(n)
}
