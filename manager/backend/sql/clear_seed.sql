-- 清除种子表（菜单/用户/角色/部门/字典/引擎/目录/服务）
-- 保留运行时表：sys_oper_log, sys_logininfor, sys_service_engine

TRUNCATE TABLE
  sys_user_role,
  sys_role_menu,
  sys_role_dept,
  sys_user_post,
  sys_dict_data,
  sys_dict_type,
  sys_config,
  sys_notice,
  sys_sld,
  sys_api_key,
  sys_published_layer,
  sys_resource,
  sys_store,
  sys_workspace,
  sys_engine_event,
  sys_service_meta,
  sys_engine,
  sys_menu,
  sys_role,
  sys_post,
  sys_user,
  sys_dept
RESTART IDENTITY CASCADE;
