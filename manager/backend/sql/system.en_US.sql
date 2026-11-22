-- =====================================================================
-- rust-gisserver - system.sql - System & Monitoring modules (DDL + seed data)
-- ---------------------------------------------------------------------
-- Purpose: rust-gisserver manager (Axum + PostgreSQL) baseline - create all tables and seed
--       System Administration / System Monitor menus, roles, users, dicts, configs.
-- Depends: none (runs standalone)
-- Usage: psql -h localhost -p 5434 -U postgres -d mapserver-manager -f system.sql
-- ---------------------------------------------------------------------
-- Contents:
--   §1 DDL: sys_dept / sys_user / sys_post / sys_role / sys_menu /
--              sys_user_role / sys_role_menu / sys_role_dept / sys_user_post /
--              sys_oper_log / sys_dict_type / sys_dict_data / sys_config /
--              sys_logininfor / sys_notice
--   §2 Patch: avatar column + log table PK sequences
--   §3 Seed data: menus / roles / users / depts / posts / dicts / configs / notices
--   §4 Patch: Online Users menu (missing from the original seed, added here)
--   §5 Normalize top-level directory sort order
-- ---------------------------------------------------------------------
-- Idempotent: CREATE TABLE IF NOT EXISTS; CREATE INDEX IF NOT EXISTS;
--         §0 migration guards - old DBs migrate once, new DBs skip;
--         §3 all seed data uses ON CONFLICT DO NOTHING;
--         §5 sort by menu_id - re-runs are side-effect free.
-- Note: Map / Data Services DDL and menus live in map.sql, not here.
-- =====================================================================

-- =====================================================================
-- §0 Normalize top-level menu IDs (legacy migration; must run before any menu INSERT)
-- ---------------------------------------------------------------------
-- Background: legacy top dirs - "Map Services" used 6, "Data Functions" used 123 (appended later),
--       top-dir IDs no longer match display order. Renumber as:
--         6 Data Services (Workspace / Data Source / Data Layer / Published Service)
--         7 Map Services (Service inventory / Logs / Styles / Engine ...)
--       "Data Services" before "Map Services": sources first, published services second.
-- Scope: sys_menu.menu_id, sys_menu.parent_id, sys_role_menu.menu_id - 3 places only;
--         no other table references menu_id (sys_menu has no FK constraints),
--         no side effects elsewhere. Frontend route name generated at runtime as
--         capitalize(path)+menu_id, not persisted - no cache invalidation needed.
-- Strategy: old IDs → temp high IDs → final IDs to avoid PK collisions;
--       wrapped in do $$ block - any exception rolls back the whole migration.
-- =====================================================================

do $$
begin
  -- Fresh DB (sys_menu not yet created): nothing to migrate
  if to_regclass('public.sys_menu') is null then
    return;
  end if;

  -- (1) Map Services 6 → temp 9106 → 7
  --    Check "is 6 named Map Services?" not "does 7 exist?":
  --    the latter silently skips migration if map.sql ran first, leaving no trace.
  if exists (select 1 from sys_menu where menu_id = 6 and menu_name = 'Map Services') then
    update sys_menu      set menu_id   = 9106 where menu_id   = 6 and menu_name = 'Map Services';
    update sys_menu      set parent_id = 9106 where parent_id = 6;
    update sys_role_menu set menu_id   = 9106 where menu_id   = 6;

    -- If target ID 7 is occupied by legacy data: evict occupant first, merge back later
    if exists (select 1 from sys_menu where menu_id = 7) then
      update sys_menu      set menu_id   = 9107 where menu_id = 7;
      update sys_menu      set parent_id = 9107 where parent_id = 7;
      update sys_role_menu set menu_id   = 9107 where menu_id = 7;
    end if;

    update sys_menu      set menu_id   = 7 where menu_id   = 9106;
    update sys_menu      set parent_id = 7 where parent_id = 9106;
    update sys_role_menu set menu_id   = 7 where menu_id   = 9106;

    -- Merge legacy occupant of 7: dedupe role grants, then move onto 7
    delete from sys_role_menu a using sys_role_menu b
      where a.menu_id = 9107 and b.menu_id = 7 and a.role_id = b.role_id;
    update sys_menu      set menu_id   = 7 where menu_id   = 9107;
    update sys_menu      set parent_id = 7 where parent_id = 9107;
    update sys_role_menu set menu_id   = 7 where menu_id   = 9107;
  end if;

  -- (2) Data Functions 123 → 6 (slot 6 is now free)
  if exists (select 1 from sys_menu where menu_id = 123 and menu_name in ('Data Functions', 'Data Services')) then
    update sys_menu      set menu_id   = 6 where menu_id = 123 and menu_name in ('Data Functions', 'Data Services');
    update sys_menu      set parent_id = 6 where parent_id = 123;
    update sys_role_menu set menu_id   = 6 where menu_id   = 123;
    update sys_menu set menu_name = 'Data Services' where menu_id = 6 and menu_name = 'Data Functions';
  end if;
end $$;


-- =====================================================================
-- §1 DDL scripts (system management schema; trimmed: social/oss/client/tenant/gen/test)
-- =====================================================================

create table if not exists sys_dept
(
    dept_id     int8,
    parent_id   int8        default 0,
    ancestors   varchar(500)default ''::varchar,
    dept_name   varchar(30) default ''::varchar,
    dept_category varchar(100) default null::varchar,
    order_num   int4        default 0,
    leader      int8        default null,
    phone       varchar(11) default null::varchar,
    email       varchar(50) default null::varchar,
    status      char        default '0'::bpchar,
    del_flag    char        default '0'::bpchar,
    create_dept int8,
    create_by   int8,
    create_time timestamp,
    update_by   int8,
    update_time timestamp,
    constraint "sys_dept_pk" primary key (dept_id)
);

comment on table sys_dept               is 'Dept table';
comment on column sys_dept.dept_id      is 'Dept ID';
comment on column sys_dept.parent_id    is 'Parent Dept ID';
comment on column sys_dept.ancestors    is 'Ancestor list';
comment on column sys_dept.dept_name    is 'Dept Name';
comment on column sys_dept.dept_category    is 'Dept Category Code';
comment on column sys_dept.order_num    is 'Display Order';
comment on column sys_dept.leader       is 'Owner';
comment on column sys_dept.phone        is 'Phone';
comment on column sys_dept.email        is 'Email';
comment on column sys_dept.status       is 'Dept status (0 normal, 1 disabled)';
comment on column sys_dept.del_flag     is 'Delete flag (0 present, 1 deleted)';
comment on column sys_dept.create_dept  is 'Creating Dept';
comment on column sys_dept.create_by    is 'Created By';
comment on column sys_dept.create_time  is 'Created At';
comment on column sys_dept.update_by    is 'Updated By';
comment on column sys_dept.update_time  is 'Updated At';

-- ----------------------------
-- Seed: sys_dept
-- ----------------------------

-- ----------------------------
-- 2. sys_user
-- ----------------------------
create table if not exists sys_user
(
    user_id     int8,
    dept_id     int8,
    user_name   varchar(30)  not null,
    nick_name   varchar(64)  not null,
    user_type   varchar(10)  default 'sys_user'::varchar,
    email       varchar(50)  default ''::varchar,
    phonenumber varchar(11)  default ''::varchar,
    sex         char         default '0'::bpchar,
    avatar      varchar(500) default null::varchar,
    password    varchar(100) default ''::varchar,
    status      char         default '0'::bpchar,
    del_flag    char         default '0'::bpchar,
    login_ip    varchar(128) default ''::varchar,
    login_date  timestamp,
    create_dept int8,
    create_by   int8,
    create_time timestamp,
    update_by   int8,
    update_time timestamp,
    remark      varchar(500) default null::varchar,
    constraint "sys_user_pk" primary key (user_id)
);

comment on table sys_user               is 'User Info table';
comment on column sys_user.user_id      is 'User ID';
comment on column sys_user.dept_id      is 'Dept ID';
comment on column sys_user.user_name    is 'Username';
comment on column sys_user.nick_name    is 'Nickname';
comment on column sys_user.user_type    is 'User type (sys_user = system user)';
comment on column sys_user.email        is 'Email';
comment on column sys_user.phonenumber  is 'Mobile';
comment on column sys_user.sex          is 'Gender (0 male, 1 female, 2 unknown)';
comment on column sys_user.avatar       is 'Avatar URL (relative path, e.g. /profile/avatar/2026/10/26/u1_xxx.png)';
comment on column sys_user.password     is 'Password';
comment on column sys_user.status       is 'Account status (0 normal, 1 disabled)';
comment on column sys_user.del_flag     is 'Delete flag (0 present, 1 deleted)';
comment on column sys_user.login_ip     is 'Last Login IP';
comment on column sys_user.login_date   is 'Last Login At';
comment on column sys_user.create_dept  is 'Creating Dept';
comment on column sys_user.create_by    is 'Created By';
comment on column sys_user.create_time  is 'Created At';
comment on column sys_user.update_by    is 'Updated By';
comment on column sys_user.update_time  is 'Updated At';
comment on column sys_user.remark       is 'Remark';

-- ----------------------------

-- Seed: sys_user
-- ----------------------------

-- ----------------------------
-- 3. sys_post
-- ----------------------------
create table if not exists sys_post
(
    post_id     int8,
    dept_id     int8,
    post_code   varchar(64) not null,
    post_category   varchar(100) default null,
    post_name   varchar(50) not null,
    post_sort   int4        not null,
    status      char        not null,
    create_dept int8,
    create_by   int8,
    create_time timestamp,
    update_by   int8,
    update_time timestamp,
    remark      varchar(500) default null::varchar,
    constraint "sys_post_pk" primary key (post_id)
);

comment on table sys_post               is 'Post Info table';
comment on column sys_post.post_id      is 'Post ID';
comment on column sys_post.dept_id      is 'Dept ID';
comment on column sys_post.post_code    is 'Post Code';
comment on column sys_post.post_category is 'Post Category Code';
comment on column sys_post.post_name    is 'Post Name';
comment on column sys_post.post_sort    is 'Display Order';
comment on column sys_post.status       is 'Status (0 normal, 1 disabled)';
comment on column sys_post.create_dept  is 'Creating Dept';
comment on column sys_post.create_by    is 'Created By';
comment on column sys_post.create_time  is 'Created At';
comment on column sys_post.update_by    is 'Updated By';
comment on column sys_post.update_time  is 'Updated At';
comment on column sys_post.remark       is 'Remark';

-- ----------------------------
-- Seed: sys_post
-- ----------------------------

-- ----------------------------
-- 4. sys_role
-- ----------------------------
create table if not exists sys_role
(
    role_id             int8,
    role_name           varchar(30)  not null,
    role_key            varchar(100) not null,
    role_sort           int4         not null,
    data_scope          char         default '1'::bpchar,
    menu_check_strictly bool         default true,
    dept_check_strictly bool         default true,
    status              char         not null,
    del_flag            char         default '0'::bpchar,
    create_dept         int8,
    create_by           int8,
    create_time         timestamp,
    update_by           int8,
    update_time         timestamp,
    remark              varchar(500) default null::varchar,
    constraint "sys_role_pk" primary key (role_id)
);

comment on table sys_role                       is 'Role Info table';
comment on column sys_role.role_id              is 'Role ID';
comment on column sys_role.role_name            is 'Role Name';
comment on column sys_role.role_key             is 'Role Permission String';
comment on column sys_role.role_sort            is 'Display Order';
comment on column sys_role.data_scope           is 'Data scope (1 all, 2 custom, 3 this dept, 4 this dept and below, 5 self only, 6 dept-and-below or self)';
comment on column sys_role.menu_check_strictly  is 'Linked visibility of menu tree items';
comment on column sys_role.dept_check_strictly  is 'Linked visibility of dept tree items';
comment on column sys_role.status               is 'Role status (0 normal, 1 disabled)';
comment on column sys_role.del_flag             is 'Delete flag (0 present, 1 deleted)';
comment on column sys_role.create_dept          is 'Creating Dept';
comment on column sys_role.create_by            is 'Created By';
comment on column sys_role.create_time          is 'Created At';
comment on column sys_role.update_by            is 'Updated By';
comment on column sys_role.update_time          is 'Updated At';
comment on column sys_role.remark               is 'Remark';

-- ----------------------------
-- Seed: sys_role
-- ----------------------------

-- ----------------------------
-- 5. sys_menu (menus/permissions)
-- ----------------------------
create table if not exists sys_menu
(
    menu_id     int8,
    menu_name   varchar(50) not null,
    parent_id   int8         default 0,
    order_num   int4         default 0,
    path        varchar(200) default ''::varchar,
    component   varchar(255) default null::varchar,
    query_param varchar(255) default null::varchar,
    is_frame    char         default '1'::bpchar,
    is_cache    char         default '0'::bpchar,
    menu_type   char         default ''::bpchar,
    visible     char         default '0'::bpchar,
    status      char         default '0'::bpchar,
    perms       varchar(100) default null::varchar,
    icon        varchar(100) default '#'::varchar,
    create_dept int8,
    create_by   int8,
    create_time timestamp,
    update_by   int8,
    update_time timestamp,
    remark      varchar(500) default ''::varchar,
    constraint "sys_menu_pk" primary key (menu_id)
);

comment on table sys_menu               is 'Menu-Permission table';
comment on column sys_menu.menu_id      is 'Menu ID';
comment on column sys_menu.menu_name    is 'Menu Name';
comment on column sys_menu.parent_id    is 'Parent Menu ID';
comment on column sys_menu.order_num    is 'Display Order';
comment on column sys_menu.path         is 'Route Path';
comment on column sys_menu.component    is 'Component Path';
comment on column sys_menu.query_param  is 'Route Params';
comment on column sys_menu.is_frame     is 'External link (0 yes, 1 no)';
comment on column sys_menu.is_cache     is 'Cached (0 yes, 1 no)';
comment on column sys_menu.menu_type    is 'Menu type (M directory, C menu, F button)';
comment on column sys_menu.visible      is 'Visibility (0 show, 1 hide)';
comment on column sys_menu.status       is 'Menu status (0 normal, 1 disabled)';
comment on column sys_menu.perms        is 'Permission Key';
comment on column sys_menu.icon         is 'Menu Icon';
comment on column sys_menu.create_dept  is 'Creating Dept';
comment on column sys_menu.create_by    is 'Created By';
comment on column sys_menu.create_time  is 'Created At';
comment on column sys_menu.update_by    is 'Updated By';
comment on column sys_menu.update_time  is 'Updated At';
comment on column sys_menu.remark       is 'Remark';

-- ----------------------------
-- Seed: sys_menu
-- ----------------------------
-- Level-1 menus
-- Level-2 menus

-- Spring Boot Admin monitor
-- OSS menu
-- Snail-Job server console

-- Level-3 menus
-- User Management buttons
-- Role Management buttons
-- Menu Management buttons
-- Dept Management buttons
-- Post Management buttons
-- Dictionary Management buttons
-- Config buttons
-- Notice buttons
-- Operation Log buttons
-- Login Log buttons
-- Online User buttons
-- Code Gen buttons
-- OSS-related buttons
-- Client Management buttons
-- Test Menu


-- ----------------------------
-- 6. sys_user_role (user N-1 role)
-- ----------------------------
create table if not exists sys_user_role
(
    user_id int8 not null,
    role_id int8 not null,
    constraint sys_user_role_pk primary key (user_id, role_id)
);

comment on table sys_user_role              is 'User-Role relation table';
comment on column sys_user_role.user_id     is 'User ID';
comment on column sys_user_role.role_id     is 'Role ID';

-- ----------------------------
-- Seed: sys_user_role
-- ----------------------------

-- ----------------------------
-- 7. sys_role_menu (role 1-N menu)
-- ----------------------------
create table if not exists sys_role_menu
(
    role_id int8 not null,
    menu_id int8 not null,
    constraint sys_role_menu_pk primary key (role_id, menu_id)
);

comment on table sys_role_menu              is 'Role-Menu relation table';
comment on column sys_role_menu.role_id     is 'Role ID';
comment on column sys_role_menu.menu_id     is 'Menu ID';

-- ----------------------------
-- Seed: sys_role_menu
-- ----------------------------

-- ----------------------------
-- 8. sys_role_dept (role 1-N dept)
-- ----------------------------
create table if not exists sys_role_dept
(
    role_id int8 not null,
    dept_id int8 not null,
    constraint sys_role_dept_pk primary key (role_id, dept_id)
);

comment on table sys_role_dept              is 'Role-Dept relation table';
comment on column sys_role_dept.role_id     is 'Role ID';
comment on column sys_role_dept.dept_id     is 'Dept ID';


-- ----------------------------
-- 9. sys_user_post (user 1-N post)
-- ----------------------------
create table if not exists sys_user_post
(
    user_id int8 not null,
    post_id int8 not null,
    constraint sys_user_post_pk primary key (user_id, post_id)
);

comment on table sys_user_post              is 'User-Post relation table';
comment on column sys_user_post.user_id     is 'User ID';
comment on column sys_user_post.post_id     is 'Post ID';

-- ----------------------------
-- Seed: sys_user_post
-- ----------------------------

-- ----------------------------
-- 10. sys_oper_log
-- ----------------------------
create table if not exists sys_oper_log
(
    oper_id        int8,
    title          varchar(50)   default ''::varchar,
    business_type  int4          default 0,
    method         varchar(100)  default ''::varchar,
    request_method varchar(10)   default ''::varchar,
    operator_type  int4          default 0,
    oper_name      varchar(50)   default ''::varchar,
    dept_name      varchar(50)   default ''::varchar,
    oper_url       varchar(255)  default ''::varchar,
    oper_ip        varchar(128)  default ''::varchar,
    oper_location  varchar(255)  default ''::varchar,
    oper_param     varchar(4000) default ''::varchar,
    json_result    varchar(4000) default ''::varchar,
    status         int4          default 0,
    error_msg      varchar(4000) default ''::varchar,
    oper_time      timestamp,
    cost_time      int8          default 0,
    constraint sys_oper_log_pk primary key (oper_id)
);

-- PK sequence: log table PKs have no business meaning; must use sequence or concurrent inserts collide
create sequence if not exists seq_sys_oper_log_oper_id start with 1000;
alter table sys_oper_log alter column oper_id set default nextval('seq_sys_oper_log_oper_id');
select setval('seq_sys_oper_log_oper_id', greatest((select coalesce(max(oper_id), 999) from sys_oper_log), 1000));

create index if not exists idx_sys_oper_log_bt ON sys_oper_log (business_type);
create index if not exists idx_sys_oper_log_s ON sys_oper_log (status);
create index if not exists idx_sys_oper_log_ot ON sys_oper_log (oper_time);

comment on table sys_oper_log                   is 'Operation log record';
comment on column sys_oper_log.oper_id          is 'Log ID';
comment on column sys_oper_log.title            is 'Module Title';
comment on column sys_oper_log.business_type    is 'Business type (0 other, 1 add, 2 edit, 3 delete)';
comment on column sys_oper_log.method           is 'Method name';
comment on column sys_oper_log.request_method   is 'Request Method';
comment on column sys_oper_log.operator_type    is 'Operation category (0 other, 1 admin user, 2 mobile user)';
comment on column sys_oper_log.oper_name        is 'Operator';
comment on column sys_oper_log.dept_name        is 'Dept Name';
comment on column sys_oper_log.oper_url         is 'Request URL';
comment on column sys_oper_log.oper_ip          is 'Host';
comment on column sys_oper_log.oper_location    is 'Operation location';
comment on column sys_oper_log.oper_param       is 'Request Params';
comment on column sys_oper_log.json_result      is 'Response Params';
comment on column sys_oper_log.status           is 'Operation status (0 normal, 1 error)';
comment on column sys_oper_log.error_msg        is 'Error Message';
comment on column sys_oper_log.oper_time        is 'Operation time';
comment on column sys_oper_log.cost_time        is 'Elapsed time';

-- ----------------------------
-- 11. sys_dict_type
-- ----------------------------
create table if not exists sys_dict_type
(
    dict_id     int8,
    dict_name   varchar(100) default ''::varchar,
    dict_type   varchar(100) default ''::varchar,
    create_dept int8,
    create_by   int8,
    create_time timestamp,
    update_by   int8,
    update_time timestamp,
    remark      varchar(500) default null::varchar,
    constraint sys_dict_type_pk primary key (dict_id)
);

create unique index if not exists sys_dict_type_index1 ON sys_dict_type (dict_type);

comment on table sys_dict_type                  is 'Dictionary Type table';
comment on column sys_dict_type.dict_id         is 'Dictionary ID';
comment on column sys_dict_type.dict_name       is 'Dictionary Name';
comment on column sys_dict_type.dict_type       is 'Dictionary Type';
comment on column sys_dict_type.create_dept     is 'Creating Dept';
comment on column sys_dict_type.create_by       is 'Created By';
comment on column sys_dict_type.create_time     is 'Created At';
comment on column sys_dict_type.update_by       is 'Updated By';
comment on column sys_dict_type.update_time     is 'Updated At';
comment on column sys_dict_type.remark          is 'Remark';


-- ----------------------------
-- 12. sys_dict_data
-- ----------------------------
create table if not exists sys_dict_data
(
    dict_code   int8,
    dict_sort   int4         default 0,
    dict_label  varchar(100) default ''::varchar,
    dict_value  varchar(100) default ''::varchar,
    dict_type   varchar(100) default ''::varchar,
    css_class   varchar(100) default null::varchar,
    list_class  varchar(100) default null::varchar,
    is_default  char         default 'N'::bpchar,
    create_dept int8,
    create_by   int8,
    create_time timestamp,
    update_by   int8,
    update_time timestamp,
    remark      varchar(500) default null::varchar,
    constraint sys_dict_data_pk primary key (dict_code)
);

comment on table sys_dict_data                  is 'Dictionary Data table';
comment on column sys_dict_data.dict_code       is 'Dictionary Code';
comment on column sys_dict_data.dict_sort       is 'Dictionary Order';
comment on column sys_dict_data.dict_label      is 'Dictionary Label';
comment on column sys_dict_data.dict_value      is 'Dictionary Value';
comment on column sys_dict_data.dict_type       is 'Dictionary Type';
comment on column sys_dict_data.css_class       is 'Style properties (other style extensions)';
comment on column sys_dict_data.list_class      is 'Table display style';
comment on column sys_dict_data.is_default      is 'Default (Y yes, N no)';
comment on column sys_dict_data.create_dept     is 'Creating Dept';
comment on column sys_dict_data.create_by       is 'Created By';
comment on column sys_dict_data.create_time     is 'Created At';
comment on column sys_dict_data.update_by       is 'Updated By';
comment on column sys_dict_data.update_time     is 'Updated At';
comment on column sys_dict_data.remark          is 'Remark';



-- ----------------------------
-- 13. sys_config
-- ----------------------------
create table if not exists sys_config
(
    config_id    int8,
    config_name  varchar(100) default ''::varchar,
    config_key   varchar(100) default ''::varchar,
    config_value varchar(500) default ''::varchar,
    config_type  char         default 'N'::bpchar,
    create_dept  int8,
    create_by    int8,
    create_time  timestamp,
    update_by    int8,
    update_time  timestamp,
    remark       varchar(500) default null::varchar,
    constraint sys_config_pk primary key (config_id)
);

comment on table sys_config                 is 'Config table';
comment on column sys_config.config_id      is 'Config ID';
comment on column sys_config.config_name    is 'Config Name';
comment on column sys_config.config_key     is 'Config Key';
comment on column sys_config.config_value   is 'Config Value';
comment on column sys_config.config_type    is 'Built-in (Y yes, N no)';
comment on column sys_config.create_dept    is 'Creating Dept';
comment on column sys_config.create_by      is 'Created By';
comment on column sys_config.create_time    is 'Created At';
comment on column sys_config.update_by      is 'Updated By';
comment on column sys_config.update_time    is 'Updated At';
comment on column sys_config.remark         is 'Remark';



-- ----------------------------
-- 14. sys_logininfor
-- ----------------------------
create table if not exists sys_logininfor
(
    info_id        int8,
    user_name      varchar(50)  default ''::varchar,
    client_key     varchar(32)  default ''::varchar,
    device_type    varchar(32)  default ''::varchar,
    ipaddr         varchar(128) default ''::varchar,
    login_location varchar(255) default ''::varchar,
    browser        varchar(50)  default ''::varchar,
    os             varchar(50)  default ''::varchar,
    status         char         default '0'::bpchar,
    msg            varchar(255) default ''::varchar,
    login_time     timestamp,
    constraint sys_logininfor_pk primary key (info_id)
);

-- PK sequence: same as oper_log - login log PKs from sequence
create sequence if not exists seq_sys_logininfor_info_id start with 1000;
alter table sys_logininfor alter column info_id set default nextval('seq_sys_logininfor_info_id');
select setval('seq_sys_logininfor_info_id', greatest((select coalesce(max(info_id), 999) from sys_logininfor), 1000));

create index if not exists idx_sys_logininfor_s ON sys_logininfor (status);
create index if not exists idx_sys_logininfor_lt ON sys_logininfor (login_time);

comment on table sys_logininfor                 is 'System Access Log';
comment on column sys_logininfor.info_id        is 'Access ID';
comment on column sys_logininfor.user_name      is 'Username';
comment on column sys_logininfor.client_key     is 'Client';
comment on column sys_logininfor.device_type    is 'Device type';
comment on column sys_logininfor.ipaddr         is 'Login IP';
comment on column sys_logininfor.login_location is 'Login location';
comment on column sys_logininfor.browser        is 'Browser type';
comment on column sys_logininfor.os             is 'Operating system';
comment on column sys_logininfor.status         is 'Login status (0 success, 1 failed)';
comment on column sys_logininfor.msg            is 'Hint message';
comment on column sys_logininfor.login_time     is 'Access Time';

-- ----------------------------
-- 17. sys_notice
-- ----------------------------
create table if not exists sys_notice
(
    notice_id      int8,
    notice_title   varchar(64)  not null,
    notice_type    char         not null,
    notice_content text,
    status         char         default '0'::bpchar,
    create_dept    int8,
    create_by      int8,
    create_time    timestamp,
    update_by      int8,
    update_time    timestamp,
    remark         varchar(255) default null::varchar,
    constraint sys_notice_pk primary key (notice_id)
);

comment on table sys_notice                 is 'Notice table';
comment on column sys_notice.notice_id      is 'Notice ID';
comment on column sys_notice.notice_title   is 'Notice Title';
comment on column sys_notice.notice_type    is 'Notice type (1 notification, 2 announcement)';
comment on column sys_notice.notice_content is 'Notice Content';
comment on column sys_notice.status         is 'Notice status (0 normal, 1 closed)';
comment on column sys_notice.create_dept    is 'Creating Dept';
comment on column sys_notice.create_by      is 'Created By';
comment on column sys_notice.create_time    is 'Created At';
comment on column sys_notice.update_by      is 'Updated By';
comment on column sys_notice.update_time    is 'Updated At';
comment on column sys_notice.remark         is 'Remark';

-- ----------------------------
-- Seed: sys_notice
-- ----------------------------


-- ----------------------------
-- 18. sys_gen_table
-- ----------------------------

-- =====================================================================
-- §2 Patch: avatar column + log table PK sequences
-- ---------------------------------------------------------------------
-- Earlier versions had sys_user.avatar as bigint (OSS file id); this project has no OSS,
-- changed to varchar(500) for a relative path (e.g. /profile/avatar/2026/10/26/u1_xxx.png).
-- Add sequences for oper_id / info_id to avoid PK collisions on concurrent inserts.
-- =====================================================================

BEGIN;

-- 1. avatar → varchar(500)
ALTER TABLE sys_user
    ALTER COLUMN avatar TYPE varchar(500) USING NULLIF(avatar::text, '');

-- 2. Avatar upload allows PNG/JPEG only, whitelist by extension (backend enforces the same)
COMMENT ON COLUMN sys_user.avatar IS 'Relative avatar path (/profile/avatar/...); empty means default avatar';

-- 3. oper_log.oper_id / logininfor.info_id have no default - app generates at insert;
--    add sequences here so concurrent inserts do not collide.
CREATE SEQUENCE IF NOT EXISTS seq_sys_oper_log_oper_id START WITH 1000;
ALTER TABLE sys_oper_log ALTER COLUMN oper_id SET DEFAULT nextval('seq_sys_oper_log_oper_id');

CREATE SEQUENCE IF NOT EXISTS seq_sys_logininfor_info_id START WITH 1000;
ALTER TABLE sys_logininfor ALTER COLUMN info_id SET DEFAULT nextval('seq_sys_logininfor_info_id');

-- 4. Align sequences with current max IDs
SELECT setval('seq_sys_oper_log_oper_id',
              GREATEST((SELECT COALESCE(max(oper_id), 999) FROM sys_oper_log), 1000));
SELECT setval('seq_sys_logininfor_info_id',
              GREATEST((SELECT COALESCE(max(info_id), 999) FROM sys_logininfor), 1000));

COMMIT;

-- =====================================================================
-- §3 Seed data (menus / roles / users / depts / posts / dicts / configs / notices)
-- ---------------------------------------------------------------------
-- Note: stripped menus aligned with frontend trim (tool/gen, system/oss, system/client,
--       system/tenant, monitor/{admin,cache,snailjob}, test/*) and their role_menu rows.
--       Map / Data Services menus & grants are in map.sql, not here.
--       All writes use ON CONFLICT DO NOTHING - re-runs are safe.
-- Default account: admin / admin123 (bcrypt - please change on first login)
-- =====================================================================

insert into sys_menu values('1', 'System Administration', '0', '1', 'system',           null, '', '1', '0', 'M', '0', '0', '', 'system',   103, 1, now(), null, null, 'System Administration') on conflict do nothing;
insert into sys_menu values('2', 'System Monitor', '0', '4', 'monitor',          null, '', '1', '0', 'M', '0', '0', '', 'monitor',  103, 1, now(), null, null, 'System Monitor') on conflict do nothing;
insert into sys_menu values('3', 'System Tools', '0', '5', 'tool',             null, '', '1', '0', 'M', '0', '0', '', 'tool',     103, 1, now(), null, null, 'System Tools directory') on conflict do nothing;
insert into sys_menu VALUES('5', 'Test Menu', '0', '7', 'demo',             null, '', '1', '0', 'M', '0', '0', null, 'star',       103, 1, now(), null, null, 'Test Menu') on conflict do nothing;
insert into sys_menu values('100', 'User Management', '1', '1', 'user',             'system/user/index',            '', '1', '0', 'C', '0', '0', 'system:user:list',            'user',          103, 1, now(), null, null, 'User Management') on conflict do nothing;
insert into sys_menu values('101', 'Role Management', '1', '2', 'role',             'system/role/index',            '', '1', '0', 'C', '0', '0', 'system:role:list',            'peoples',       103, 1, now(), null, null, 'Role Management') on conflict do nothing;
insert into sys_menu values('102', 'Menu Management', '1', '3', 'menu',             'system/menu/index',            '', '1', '0', 'C', '0', '0', 'system:menu:list',            'tree-table',    103, 1, now(), null, null, 'Menu Management') on conflict do nothing;
insert into sys_menu values('103', 'Dept Management', '1', '4', 'dept',             'system/dept/index',            '', '1', '0', 'C', '0', '0', 'system:dept:list',            'tree-dept',     103, 1, now(), null, null, 'Dept Management') on conflict do nothing;
insert into sys_menu values('104', 'Post Management', '1', '5', 'post',             'system/post/index',            '', '1', '0', 'C', '0', '0', 'system:post:list',            'post',          103, 1, now(), null, null, 'Post Management Menu') on conflict do nothing;
insert into sys_menu values('105', 'Dictionary Management', '1', '6', 'dict',             'system/dict/index',            '', '1', '0', 'C', '0', '0', 'system:dict:list',            'dict',          103, 1, now(), null, null, 'Dictionary Management') on conflict do nothing;
insert into sys_menu values('106', 'Config', '1', '7', 'config',           'system/config/index',          '', '1', '0', 'C', '0', '0', 'system:config:list',          'sliders',       103, 1, now(), null, null, 'Config Menu') on conflict do nothing;
insert into sys_menu values('107', 'Notices', '1', '8', 'notice',           'system/notice/index',          '', '1', '0', 'C', '0', '0', 'system:notice:list',          'message',       103, 1, now(), null, null, 'Notices') on conflict do nothing;
insert into sys_menu values('108', 'Log Management', '1', '9', 'log',              '',                             '', '1', '0', 'M', '0', '0', '',                            'log-sys',       103, 1, now(), null, null, 'Log Management') on conflict do nothing;
insert into sys_menu values('130', 'Assign Users', '1', '2', 'role-auth/user/:roleId', 'system/role/authUser', '', '1', '1', 'C', '1', '0', 'system:role:edit',      '#',               103, 1, now(), null, null, '/system/role') on conflict do nothing;
insert into sys_menu values('131', 'Assign Roles', '1', '1', 'user-auth/role/:userId', 'system/user/authRole', '', '1', '1', 'C', '1', '0', 'system:user:edit',      '#',               103, 1, now(), null, null, '/system/user') on conflict do nothing;
insert into sys_menu values('132', 'Dictionary Data', '1', '6', 'dict-data/index/:dictId', 'system/dict/data', '', '1', '1', 'C', '1', '0', 'system:dict:list',         '#',               103, 1, now(), null, null, '/system/dict') on conflict do nothing;
insert into sys_menu values('500', 'Operation Log', '108', '1', 'operlog',    'monitor/operlog/index',    '', '1', '0', 'C', '0', '0', 'monitor:operlog:list',    'log-sys-op',    103, 1, now(), null, null, 'Operation log Menu') on conflict do nothing;
insert into sys_menu values('501', 'Login Log', '108', '2', 'logininfor', 'monitor/logininfor/index', '', '1', '0', 'C', '0', '0', 'monitor:logininfor:list', 'log-sys-login', 103, 1, now(), null, null, 'Login Log Menu') on conflict do nothing;
insert into sys_menu values('109', 'Online Users', '2', '1', 'online',     'monitor/online/index',     '', '1', '0', 'C', '0', '0', 'monitor:online:list',     'online',        103, 1, now(), null, null, 'Online User Menu') on conflict do nothing;
insert into sys_menu values('1001', 'Query User', '100', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1002', 'Add User', '100', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1003', 'Edit User', '100', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1004', 'Delete User', '100', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1005', 'Export Users', '100', '5',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1006', 'Import Users', '100', '6',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:import',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1007', 'Reset Password', '100', '7',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:resetPwd',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1008', 'Query Role', '101', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1009', 'Add Role', '101', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1010', 'Edit Role', '101', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1011', 'Delete Role', '101', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1012', 'Export Roles', '101', '5',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1013', 'Query Menu', '102', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1014', 'Add Menu', '102', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1015', 'Edit Menu', '102', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1016', 'Delete Menu', '102', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1017', 'Query Dept', '103', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1018', 'Add Dept', '103', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1019', 'Edit Dept', '103', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1020', 'Delete Dept', '103', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1021', 'Query Post', '104', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1022', 'Add Post', '104', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1023', 'Edit Post', '104', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1024', 'Delete Post', '104', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1025', 'Export Posts', '104', '5',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1026', 'Query Dictionary', '105', '1', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1027', 'Add Dictionary', '105', '2', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1028', 'Edit Dictionary', '105', '3', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1029', 'Delete Dictionary', '105', '4', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1030', 'Export Dictionary', '105', '5', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1031', 'Query Config', '106', '1', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:query',        '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1032', 'Add Config', '106', '2', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:add',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1033', 'Edit Config', '106', '3', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:edit',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1034', 'Delete Config', '106', '4', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:remove',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1035', 'Export Config', '106', '5', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:export',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1036', 'Query Notice', '107', '1', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:query',        '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1037', 'Add Notice', '107', '2', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:add',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1038', 'Edit Notice', '107', '3', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:edit',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1039', 'Delete Notice', '107', '4', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:remove',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1040', 'Operation query', '500', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:operlog:query',      '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1041', 'Operation delete', '500', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:operlog:remove',     '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1042', 'Export Logs', '500', '4', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:operlog:export',     '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1043', 'Login query', '501', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:query',   '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1044', 'Login delete', '501', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:remove',  '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1045', 'Export Logs', '501', '3', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:export',  '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1050', 'Unlock Account', '501', '4', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:unlock',  '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1046', 'Online Query', '109', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:query',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1047', 'Batch Force Logout', '109', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:batchLogout', '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1048', 'Force Logout', '109', '3', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:forceLogout', '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_dept values(100, 0,   '0',          'Example Technology',   null,0, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(101, 100, '0,100',      'Shenzhen Headquarters', null,1, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(102, 100, '0,100',      'Changsha Branch', null,2, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(103, 101, '0,100,101',  'R&D Dept',   null,1, 1, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(104, 101, '0,100,101',  'Marketing Dept',   null,2, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(105, 101, '0,100,101',  'Test Dept',   null,3, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(106, 101, '0,100,101',  'Finance Dept',   null,4, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(107, 101, '0,100,101',  'Operations Dept',   null,5, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(108, 102, '0,100,102',  'Marketing Dept',   null,1, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(109, 102, '0,100,102',  'Finance Dept',   null,2, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_user values(1, 103, 'admin', 'admin', 'sys_user', 'admin@example.com', '15888888888', '1', null, '$2a$10$7JB720yubVSZvUI0rEqK/.VqGOZTH.ulu33dHOiBE8ByOhJIrdAu2', '0', '0', '127.0.0.1', now(), 103, 1, now(), null, null, 'Administrator') on conflict do nothing;
insert into sys_user VALUES(3, 108, 'test', 'This dept and below (password 666666)', 'sys_user', '', '', '0', null, '$2a$10$b8yUzN0C71sbz.PhNOCgJe.Tu1yWC3RNrTyjSQ8p1W0.aaUXUJ.Ne', '0', '0', '127.0.0.1', now(), 103, 1, now(), 3, now(), NULL) on conflict do nothing;
insert into sys_user VALUES(4, 102, 'test1', 'Self only (password 666666)', 'sys_user', '', '', '0', null, '$2a$10$b8yUzN0C71sbz.PhNOCgJe.Tu1yWC3RNrTyjSQ8p1W0.aaUXUJ.Ne', '0', '0', '127.0.0.1', now(), 103, 1, now(), 4, now(), NULL) on conflict do nothing;
insert into sys_post values(1, 103, 'ceo',  null, 'Chairman',    1, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_post values(2, 100, 'se',   null, 'Project Manager',  2, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_post values(3, 100, 'hr',   null, 'Human Resources',  3, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_post values(4, 100, 'user', null, 'Staff',  4, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_role values('1', 'Super Administrator',  'superadmin',  1, '1', 't', 't', '0', '0', 103, 1, now(), null, null, 'Super Administrator') on conflict do nothing;
insert into sys_role values('3', 'This dept and below', 'test1', 3, '4', 't', 't', '0', '0', 103, 1, now(), NULL, NULL, '') on conflict do nothing;
insert into sys_role values('4', 'Self only', 'test2', 4, '5', 't', 't', '0', '0', 103, 1, now(), NULL, NULL, '') on conflict do nothing;
insert into sys_user_role values ('1', '1') on conflict do nothing;
insert into sys_user_role values ('3', '3') on conflict do nothing;
insert into sys_user_role values ('4', '4') on conflict do nothing;
insert into sys_role_menu values ('3', '1') on conflict do nothing;
insert into sys_role_menu values ('3', '7') on conflict do nothing;
insert into sys_role_menu values ('3', '100') on conflict do nothing;
insert into sys_role_menu values ('3', '101') on conflict do nothing;
insert into sys_role_menu values ('3', '102') on conflict do nothing;
insert into sys_role_menu values ('3', '103') on conflict do nothing;
insert into sys_role_menu values ('3', '104') on conflict do nothing;
insert into sys_role_menu values ('3', '105') on conflict do nothing;
insert into sys_role_menu values ('3', '106') on conflict do nothing;
insert into sys_role_menu values ('3', '107') on conflict do nothing;
insert into sys_role_menu values ('3', '108') on conflict do nothing;
insert into sys_role_menu values ('3', '2') on conflict do nothing;
insert into sys_role_menu values ('3', '109') on conflict do nothing;
insert into sys_role_menu values ('3', '500') on conflict do nothing;
insert into sys_role_menu values ('3', '501') on conflict do nothing;
insert into sys_role_menu values ('3', '1001') on conflict do nothing;
insert into sys_role_menu values ('3', '1002') on conflict do nothing;
insert into sys_role_menu values ('3', '1003') on conflict do nothing;
insert into sys_role_menu values ('3', '1004') on conflict do nothing;
insert into sys_role_menu values ('3', '1005') on conflict do nothing;
insert into sys_role_menu values ('3', '1006') on conflict do nothing;
insert into sys_role_menu values ('3', '1007') on conflict do nothing;
insert into sys_role_menu values ('3', '1008') on conflict do nothing;
insert into sys_role_menu values ('3', '1009') on conflict do nothing;
insert into sys_role_menu values ('3', '1010') on conflict do nothing;
insert into sys_role_menu values ('3', '1011') on conflict do nothing;
insert into sys_role_menu values ('3', '1012') on conflict do nothing;
insert into sys_role_menu values ('3', '1013') on conflict do nothing;
insert into sys_role_menu values ('3', '1014') on conflict do nothing;
insert into sys_role_menu values ('3', '1015') on conflict do nothing;
insert into sys_role_menu values ('3', '1016') on conflict do nothing;
insert into sys_role_menu values ('3', '1017') on conflict do nothing;
insert into sys_role_menu values ('3', '1018') on conflict do nothing;
insert into sys_role_menu values ('3', '1019') on conflict do nothing;
insert into sys_role_menu values ('3', '1020') on conflict do nothing;
insert into sys_role_menu values ('3', '1021') on conflict do nothing;
insert into sys_role_menu values ('3', '1022') on conflict do nothing;
insert into sys_role_menu values ('3', '1023') on conflict do nothing;
insert into sys_role_menu values ('3', '1024') on conflict do nothing;
insert into sys_role_menu values ('3', '1025') on conflict do nothing;
insert into sys_role_menu values ('3', '1026') on conflict do nothing;
insert into sys_role_menu values ('3', '1027') on conflict do nothing;
insert into sys_role_menu values ('3', '1028') on conflict do nothing;
insert into sys_role_menu values ('3', '1029') on conflict do nothing;
insert into sys_role_menu values ('3', '1030') on conflict do nothing;
insert into sys_role_menu values ('3', '1031') on conflict do nothing;
insert into sys_role_menu values ('3', '1032') on conflict do nothing;
insert into sys_role_menu values ('3', '1033') on conflict do nothing;
insert into sys_role_menu values ('3', '1034') on conflict do nothing;
insert into sys_role_menu values ('3', '1035') on conflict do nothing;
insert into sys_role_menu values ('3', '1036') on conflict do nothing;
insert into sys_role_menu values ('3', '1037') on conflict do nothing;
insert into sys_role_menu values ('3', '1038') on conflict do nothing;
insert into sys_role_menu values ('3', '1039') on conflict do nothing;
insert into sys_role_menu values ('3', '1040') on conflict do nothing;
insert into sys_role_menu values ('3', '1041') on conflict do nothing;
insert into sys_role_menu values ('3', '1042') on conflict do nothing;
insert into sys_role_menu values ('3', '1043') on conflict do nothing;
insert into sys_role_menu values ('3', '1044') on conflict do nothing;
insert into sys_role_menu values ('3', '1045') on conflict do nothing;
insert into sys_role_menu values ('3', '1050') on conflict do nothing;
insert into sys_role_menu values ('3', '1046') on conflict do nothing;
insert into sys_role_menu values ('3', '1047') on conflict do nothing;
insert into sys_role_menu values ('3', '1048') on conflict do nothing;
insert into sys_role_menu values ('4', '5') on conflict do nothing;
insert into sys_user_post values ('1', '1') on conflict do nothing;
insert into sys_dict_type values(1, 'Gender', 'sys_user_sex',        103, 1, now(), null, null, 'Gender list') on conflict do nothing;
insert into sys_dict_type values(2, 'Menu Status', 'sys_show_hide',       103, 1, now(), null, null, 'Menu status list') on conflict do nothing;
insert into sys_dict_type values(3, 'System Switch', 'sys_normal_disable',  103, 1, now(), null, null, 'System Switch list') on conflict do nothing;
insert into sys_dict_type values(6, 'System Yes/No', 'sys_yes_no',          103, 1, now(), null, null, 'System Yes/No list') on conflict do nothing;
insert into sys_dict_type values(7, 'Notice Type', 'sys_notice_type',     103, 1, now(), null, null, 'Notice type list') on conflict do nothing;
insert into sys_dict_type values(8, 'Notice Status', 'sys_notice_status',   103, 1, now(), null, null, 'Notice status list') on conflict do nothing;
insert into sys_dict_type values(9, 'Operation type', 'sys_oper_type',       103, 1, now(), null, null, 'Operation type List') on conflict do nothing;
insert into sys_dict_type values(10, 'System Status', 'sys_common_status',  103, 1, now(), null, null, 'Login status list') on conflict do nothing;
insert into sys_dict_type values(11, 'Authorization type', 'sys_grant_type',     103, 1, now(), null, null, 'Authentication type') on conflict do nothing;
insert into sys_dict_type values(12, 'Device type', 'sys_device_type',    103, 1, now(), null, null, 'Client device type') on conflict do nothing;
insert into sys_dict_data values(1, 1,  'Male',       '0',       'sys_user_sex',        '',   '',        'Y', 103, 1, now(), null, null, 'Gender Male') on conflict do nothing;
insert into sys_dict_data values(2, 2,  'Female',       '1',       'sys_user_sex',        '',   '',        'N', 103, 1, now(), null, null, 'Gender Female') on conflict do nothing;
insert into sys_dict_data values(3, 3,  'Unknown',     '2',       'sys_user_sex',        '',   '',        'N', 103, 1, now(), null, null, 'Gender Unknown') on conflict do nothing;
insert into sys_dict_data values(4, 1,  'Show',     '0',       'sys_show_hide',       '',   'primary', 'Y', 103, 1, now(), null, null, 'Show Menu') on conflict do nothing;
insert into sys_dict_data values(5, 2,  'Hide',     '1',       'sys_show_hide',       '',   'danger',  'N', 103, 1, now(), null, null, 'Hide Menu') on conflict do nothing;
insert into sys_dict_data values(6, 1,  'Normal',     '0',       'sys_normal_disable',  '',   'primary', 'Y', 103, 1, now(), null, null, 'Normal Status') on conflict do nothing;
insert into sys_dict_data values(7, 2,  'Disabled',     '1',       'sys_normal_disable',  '',   'danger',  'N', 103, 1, now(), null, null, 'Disabled Status') on conflict do nothing;
insert into sys_dict_data values(12, 1,  'Yes',       'Y',       'sys_yes_no',          '',   'primary', 'Y', 103, 1, now(), null, null, 'System Default Yes') on conflict do nothing;
insert into sys_dict_data values(13, 2,  'No',       'N',       'sys_yes_no',          '',   'danger',  'N', 103, 1, now(), null, null, 'System Default No') on conflict do nothing;
insert into sys_dict_data values(14, 1,  'Notification',     '1',       'sys_notice_type',     '',   'warning', 'Y', 103, 1, now(), null, null, 'Notification') on conflict do nothing;
insert into sys_dict_data values(15, 2,  'Announcement',     '2',       'sys_notice_type',     '',   'success', 'N', 103, 1, now(), null, null, 'Announcement') on conflict do nothing;
insert into sys_dict_data values(16, 1,  'Normal',     '0',       'sys_notice_status',   '',   'primary', 'Y', 103, 1, now(), null, null, 'Normal Status') on conflict do nothing;
insert into sys_dict_data values(17, 2,  'Closed',     '1',       'sys_notice_status',   '',   'danger',  'N', 103, 1, now(), null, null, 'Closed Status') on conflict do nothing;
insert into sys_dict_data values(29, 99, 'Other',     '0',       'sys_oper_type',       '',   'info',    'N', 103, 1, now(), null, null, 'Other') on conflict do nothing;
insert into sys_dict_data values(18, 1,  'Add',     '1',       'sys_oper_type',       '',   'info',    'N', 103, 1, now(), null, null, 'Add') on conflict do nothing;
insert into sys_dict_data values(19, 2,  'Edit',     '2',       'sys_oper_type',       '',   'info',    'N', 103, 1, now(), null, null, 'Edit') on conflict do nothing;
insert into sys_dict_data values(20, 3,  'Delete',     '3',       'sys_oper_type',       '',   'danger',  'N', 103, 1, now(), null, null, 'Delete') on conflict do nothing;
insert into sys_dict_data values(21, 4,  'Grant',     '4',       'sys_oper_type',       '',   'primary', 'N', 103, 1, now(), null, null, 'Grant') on conflict do nothing;
insert into sys_dict_data values(22, 5,  'Export',     '5',       'sys_oper_type',       '',   'warning', 'N', 103, 1, now(), null, null, 'Export') on conflict do nothing;
insert into sys_dict_data values(23, 6,  'Import',     '6',       'sys_oper_type',       '',   'warning', 'N', 103, 1, now(), null, null, 'Import') on conflict do nothing;
insert into sys_dict_data values(24, 7,  'Force Logout',     '7',       'sys_oper_type',       '',   'danger',  'N', 103, 1, now(), null, null, 'Force Logout') on conflict do nothing;
insert into sys_dict_data values(25, 8,  'Code Generator', '8',       'sys_oper_type',       '',   'warning', 'N', 103, 1, now(), null, null, 'Generate') on conflict do nothing;
insert into sys_dict_data values(26, 9,  'Clear Data', '9',       'sys_oper_type',       '',   'danger',  'N', 103, 1, now(), null, null, 'Clear') on conflict do nothing;
insert into sys_dict_data values(27, 1,  'Success',     '0',       'sys_common_status',   '',   'primary', 'N', 103, 1, now(), null, null, 'Normal Status') on conflict do nothing;
insert into sys_dict_data values(28, 2,  'Failed',     '1',       'sys_common_status',   '',   'danger',  'N', 103, 1, now(), null, null, 'Disabled Status') on conflict do nothing;
insert into sys_dict_data values(30, 0,  'Password auth', 'password',   'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, 'Password auth') on conflict do nothing;
insert into sys_dict_data values(31, 0,  'SMS auth', 'sms',        'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, 'SMS auth') on conflict do nothing;
insert into sys_dict_data values(32, 0,  'Email auth', 'email',      'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, 'Email auth') on conflict do nothing;
insert into sys_dict_data values(33, 0,  'Mini Program auth', 'xcx',      'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, 'Mini Program auth') on conflict do nothing;
insert into sys_dict_data values(34, 0,  'Third-party login', 'social', 'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, 'Third-party login') on conflict do nothing;
insert into sys_dict_data values(35, 0,  'PC', 'pc',              'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, 'PC') on conflict do nothing;
insert into sys_dict_data values(36, 0,  'Android', 'android',       'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, 'Android') on conflict do nothing;
insert into sys_dict_data values(37, 0,  'iOS', 'ios',            'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, 'iOS') on conflict do nothing;
insert into sys_dict_data values(38, 0,  'Mini Program', 'xcx',         'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, 'Mini Program') on conflict do nothing;
insert into sys_config values(1, 'Main frame - default skin name',     'sys.index.skinName',            'skin-blue',     'Y', 103, 1, now(), null, null, 'blue skin-blue, green skin-green, purple skin-purple, red skin-red, yellow skin-yellow' ) on conflict do nothing;
insert into sys_config values(2, 'User Management - Initial Password',         'sys.user.initPassword',         '123456',        'Y', 103, 1, now(), null, null, 'Initial password 123456' ) on conflict do nothing;
insert into sys_config values(3, 'Main frame - sidebar theme',           'sys.index.sideTheme',           'theme-dark',    'Y', 103, 1, now(), null, null, 'Dark theme theme-dark, light theme theme-light' ) on conflict do nothing;
insert into sys_config values(5, 'Self-service - whether user registration is enabled',   'sys.account.registerUser',      'false',         'Y', 103, 1, now(), null, null, 'Whether user registration is enabled (true/false)') on conflict do nothing;
insert into sys_config values(11, 'OSS preview list resource switch',          'sys.oss.previewListResource',   'true',          'Y', 103, 1, now(), null, null, 'true: enabled, false: disabled') on conflict do nothing;
insert into sys_notice values('1', 'Notice: new version released on 2018-07-01', '2', 'New version content', '0', 103, 1, now(), null, null, 'Administrator') on conflict do nothing;
insert into sys_notice values('2', 'Maintenance notice: scheduled maintenance on 2018-07-01', '1', 'Maintenance content',   '0', 103, 1, now(), null, null, 'Administrator') on conflict do nothing;

-- =====================================================================
-- §4 Patch: Online Users menu
-- ---------------------------------------------------------------------
-- The original sys_menu seed has 109 (Online Users, parent=2 System Monitor) plus 1046/1047/1048
-- three button permissions - missing in this project seed, so monitor/online page had no entry.
-- =====================================================================

-- Menu: Online Users (under System Monitor directory)
insert into sys_menu values('109', 'Online Users', '2', '1', 'online', 'monitor/online/index', '', '1', '0', 'C', '0', '0', 'monitor:online:list', 'online', 103, 1, now(), null, null, 'Online User Menu')
on conflict (menu_id) do nothing;

-- Button permissions
insert into sys_menu values('1046', 'Online Query', '109', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:query', '#', 103, 1, now(), null, null, '')
on conflict (menu_id) do nothing;
insert into sys_menu values('1047', 'Batch Force Logout', '109', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:batchLogout', '#', 103, 1, now(), null, null, '')
on conflict (menu_id) do nothing;
insert into sys_menu values('1048', 'Force Logout', '109', '3', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:forceLogout', '#', 103, 1, now(), null, null, '')
on conflict (menu_id) do nothing;

-- Role grants: role 3 (test / test1) also needs parent dir System Monitor (2),
-- otherwise 109 has no parent in the menu tree and won't show up.
insert into sys_role_menu values ('3', '2') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '109') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '1046') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '1047') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '1048') on conflict (role_id, menu_id) do nothing;

-- =====================================================================
-- §5 Normalize top-level directory sort order
-- ---------------------------------------------------------------------
-- Target order: 1 System Admin / 6 Data Services / 7 Map Services / 2 System Monitor /
--           3 System Tools / 7 Test Menu
-- Lookup by menu_id (parent_id = 0 top-level rows); re-runs write same values back.
-- =====================================================================
update sys_menu set order_num = 1 where menu_id = '1' and parent_id = 0; -- System Admin
update sys_menu set order_num = 2 where menu_id = '6' and parent_id = 0; -- Data Services
update sys_menu set order_num = 3 where menu_id = '7' and parent_id = 0; -- Map Services
update sys_menu set order_num = 4 where menu_id = '2' and parent_id = 0; -- System Monitor
update sys_menu set order_num = 5 where menu_id = '3' and parent_id = 0; -- System Tools
update sys_menu set order_num = 7 where menu_id = '5' and parent_id = 0; -- Test Menu
