-- ============================================================================
-- 英语环境首次初始化（PostGIS 首次建库时由镜像自动执行）
-- -----------------------------------------------------------------------------
-- 依序执行两份英文基线（与中文基线结构逐字同构，仅字符串字面量为英文，
-- 标签类列宽已统一 64，英文标签放得下）：
--   system.en_US.sql —— 系统管理与监控板块建表 + 英文种子
--   map.en_US.sql    —— 地图/数据服务板块建表 + 英文菜单（依赖前者先跑）
--
-- 注意：manager 后端 i18n 中间件只做 zh→en 动态翻译（中文是源语言）。
-- 库种成英文后，接口 msg 仍由中间件正常译出；但中文请求的 DB 文本
-- （菜单/字典等）会显示英文——本部署形态面向纯英语环境。若要切回中文，
-- 到 ../deploy_zh-cn 直接 up（两项目卷互相独立），或在库上执行
-- ../backend/sql/switch_to_zh_CN.sql。
-- ============================================================================
\set ON_ERROR_STOP on
\i /sql/system.en_US.sql
\i /sql/map.en_US.sql

-- Container form: point the default workspace ws1's data root at the gisdata
-- mount. compose mounts GISDATA_DIR (default ./gisdata) at /app/data in both
-- map-api (read-only) and manager (writable); map-api's path guard uses the
-- same server.data_root=/app/data.
update sys_workspace set data_root = '/app/data/' where name = 'ws1';
