-- ====================================================================
--  rust-gisserver 数据库初始化脚本（第 1 步）
--  目标：创建 mapserver-manager 数据库 + 启用 PostGIS 扩展
--  用法：psql -U postgres -p <端口> -f init_db.sql
-- ====================================================================

-- 1. 创建数据库（如果不存在）
-- 注意：CREATE DATABASE 不能在事务块内执行，psql 默认 autocommit 所以没事
SELECT 'CREATE DATABASE "mapserver-manager" ENCODING ''UTF8'' LC_COLLATE ''en_US.UTF-8'' LC_CTYPE ''en_US.UTF-8'' TEMPLATE template0'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'mapserver-manager')
\gexec

-- 2. 切到新库
\connect "mapserver-manager"

-- 3. 启用 PostGIS 扩展（在新库里执行）
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS postgis_topology;
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;
CREATE EXTENSION IF NOT EXISTS postgis_tiger_geocoder;

-- 4. 验证
SELECT
  current_database() AS "当前数据库",
  extname || ' ' || extversion AS "已启用扩展"
FROM pg_extension
ORDER BY extname;

-- ====================================================================
--  接下来：依次执行 system.sql 和 map.sql
-- ====================================================================
-- psql -U postgres -p <端口> -d "mapserver-manager" -f system.sql
-- psql -U postgres -p <端口> -d "mapserver-manager" -f map.sql
