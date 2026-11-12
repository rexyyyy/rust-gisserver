//! map-api 管理面：服务登记簿（sys_service / 文件 sidecar）、请求计量
//! （/api/usage）、访问日志、样式库、数据源体检、只读数据发现与管理端点（/admin/*）。
//! 供 manager 系统通过 HTTP 调用，见 docs/manager-scope-analysis-2026-10-26.md。

pub mod access_log;
pub mod catalog_admin;
pub mod check;
pub mod discover;
pub mod http;
pub mod pg_import;
pub mod registry;
pub mod styles;
pub mod tile_grid;
pub mod usage;
