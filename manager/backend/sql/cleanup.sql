-- =====================================================================
-- cleanup.sql —— 清理所有数据服务 / 地图服务测试数据
-- ---------------------------------------------------------------------
-- 用途：清除组织层四表（published/resource/store/workspace）的全部测试记录，
--       保留默认工作空间 ws1；清理引擎关联表与事件日志。
-- 用法：psql -h 127.0.0.1 -p 5434 -U postgres -d mapserver-manager -f cleanup.sql
-- 注意：不可逆操作，执行前请确认备份。
-- =====================================================================

-- ① 已发布服务账本
delete from sys_published_layer;

-- ② 数据图层（发现结果）
delete from sys_resource;

-- ③ 数据源（目录 / PostGIS 指针）
delete from sys_store;

-- ④ 服务-引擎归属关系
delete from sys_service_engine;

-- ⑤ 服务分组元数据
delete from sys_service_meta;

-- ⑥ 引擎监控事件日志
delete from sys_engine_event;

-- ⑦ 其他工作空间（保留 ws1）
delete from sys_workspace where name != 'ws1';

-- ⑧ 确保 ws1 存在（若被误删则重建）
insert into sys_workspace (name, title, remark)
values ('ws1', '默认工作空间', '系统默认工作空间')
on conflict (name) where del_flag = 0 do nothing;

-- =====================================================================
-- 执行后状态：
--   sys_workspace       : 仅 ws1
--   sys_store           : 空
--   sys_resource        : 空
--   sys_published_layer : 空
--   sys_service_engine  : 空
--   sys_service_meta    : 空
--   sys_engine_event    : 空
--   sys_engine          : 保留（map-api 种子）
--   sys_api_key         : 保留
-- =====================================================================
