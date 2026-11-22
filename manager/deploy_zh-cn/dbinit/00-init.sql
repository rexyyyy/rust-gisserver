-- ============================================================================
-- 中文环境首次初始化（PostGIS 首次建库时由镜像自动执行）
-- -----------------------------------------------------------------------------
-- 依序执行两份中文基线（manager 后端 i18n 中间件按请求头 X-Language 把
-- 中文 msg 动态译成英文，DB 必须种中文源文本）：
--   system.sql  —— 系统管理与监控板块建表 + 种子
--   map.sql     —— 地图/数据服务板块建表 + 菜单（依赖 system.sql 先跑）
-- 不进自动初始化：*.en_US.sql（英文环境见 ../deploy_en）、
-- switch_to_*.sql（存量库语言切换补丁）、cleanup.sql / clear_seed.sql（破坏性）。
-- ============================================================================

-- PostGIS 扩展必须在 system.sql 之前启用（system.sql 里有 SRID 外键依赖 spatial_ref_sys）
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;

\set ON_ERROR_STOP on
\i /sql/system.sql
\i /sql/map.sql

-- 容器形态：默认工作区 ws1 的数据根指向 gisdata 挂载点。
-- compose 把 GISDATA_DIR（默认 ./gisdata）只挂到 map-api 的 /app/data（只读）；
-- manager 不挂数据卷，所有数据操作经 map-api 接口执行，路径守卫的
-- server.data_root 也是 /app/data。
update sys_workspace set data_root = '/app/data/' where name = 'ws1';
