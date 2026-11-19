//! 配置加载。默认读取 `config/manager.xml`，可用 `--config <path>` 覆盖。
//!
//! 配置文件为 **XML**：经 [`crate::xmlcfg`] 桥接成 JSON 值后，再交给本文件的
//! serde 结构体反序列化（结构体零改动）。
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("读取配置文件失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("解析配置失败: {0}")]
    Xml(String),
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    pub server: ServerCfg,
    pub database: DatabaseCfg,
    pub security: SecurityCfg,
    /// 本地文件存储配置（头像等）。
    #[serde(default)]
    pub storage: StorageCfg,
    /// map-api（rust-gisserver 服务面）管理代理配置。
    #[serde(default)]
    pub mapsvc: MapSvcCfg,
}

/// map-api 管理面连接配置：rust-gisserver 后端是唯一调用方，
/// 通过它代理前端对 map-api `/admin/*` 与 `/api/usage` 的访问。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MapSvcCfg {
    /// map-api 基地址（不含尾斜杠）。
    #[serde(default = "default_mapsvc_base")]
    pub base_url: String,
    /// 与 map-api `admin.token` 一致的可选共享密钥（X-Admin-Token）。
    #[serde(default)]
    pub token: Option<String>,
}

impl Default for MapSvcCfg {
    fn default() -> Self {
        Self { base_url: default_mapsvc_base(), token: None }
    }
}

fn default_mapsvc_base() -> String {
    "http://127.0.0.1:8095".to_string()
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServerCfg {
    pub addr: String,
    #[serde(default = "default_base_api")]
    pub base_api: String,
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// 前端静态资源目录（`ui/dist`）。配置后由后端同端口提供前端页面，
    /// 实现「单进程单端口」的一体化部署，免去独立 Nginx/IIS。
    ///
    /// 相对路径按**后端进程工作目录**解析（与 `storage.avatar_dir` 一致）。
    /// 留空（默认）则只提供 API，不挂静态资源——此时前端由 Vite dev server
    /// 或独立静态服务器提供，两种部署方式可共存。
    #[serde(default)]
    pub static_dir: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DatabaseCfg {
    pub url: String,
    #[serde(default = "default_pool_max")]
    pub pool_max: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SecurityCfg {
    pub jwt_secret: String,
    #[serde(default = "default_expire")]
    pub token_expire_minutes: i64,
    #[serde(default = "default_true")]
    pub captcha_enabled: bool,
    #[serde(default)]
    pub white_list: Vec<String>,
    /// DSN 落库加密算法：`des`（默认）或 `sm4`。
    #[serde(default = "default_dsn_cipher")]
    pub dsn_cipher: String,
    /// DSN 加密密钥；留空时由 `jwt_secret` 派生（生产建议显式配置）。
    #[serde(default)]
    pub dsn_key: Option<String>,
}

fn default_dsn_cipher() -> String {
    "des".to_string()
}

/// 本地文件存储配置（头像等）。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StorageCfg {
    /// 头像文件根目录。相对路径按「后端进程工作目录」解析；
    /// 生产部署建议给绝对路径（如 `/var/lib/rust-gisserver/avatar`）。
    #[serde(default = "default_avatar_dir")]
    pub avatar_dir: String,
}

impl Default for StorageCfg {
    fn default() -> Self {
        Self { avatar_dir: default_avatar_dir() }
    }
}

fn default_avatar_dir() -> String {
    "storage/avatar".to_string()
}

fn default_base_api() -> String {
    "/manager-api".to_string()
}
fn default_pool_max() -> usize {
    16
}
fn default_expire() -> i64 {
    1440
}
fn default_true() -> bool {
    true
}

impl Config {
    pub fn load_file(path: &str) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(Path::new(path))?;
        let value = crate::xmlcfg::parse_to_value(&text).map_err(ConfigError::Xml)?;
        serde_json::from_value(value).map_err(|e| ConfigError::Xml(e.to_string()))
    }

    pub fn load_default() -> Result<Self, ConfigError> {
        let arg_path = std::env::args()
            .position(|a| a == "--config")
            .and_then(|i| std::env::args().nth(i + 1));
        let path = arg_path.unwrap_or_else(|| "config/manager.xml".to_string());
        Self::load_file(&path)
    }

    /// 白名单 URL 在此判定（入参是不含 base_api 的路径，如 `/auth/login`）。
    pub fn is_white(&self, path: &str) -> bool {
        self.security
            .white_list
            .iter()
            .any(|p| path == p || path.starts_with(&format!("{p}/")))
    }
}
