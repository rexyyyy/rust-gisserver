-- =====================================================================
-- rust-gisserver · system.sql —— 系统管理与监控板块（建表 + 初始化数据）
-- ---------------------------------------------------------------------
-- 用途：rust-gisserver 管理端（Axum + PostgreSQL）的系统底座，一次性建表并写入
--       系统管理 / 系统监控两个板块的菜单、角色、用户、字典与参数。
-- 依赖：无（可独立执行）
-- 用法：psql -h localhost -p 5434 -U postgres -d mapserver-manager -f system.sql
-- ---------------------------------------------------------------------
-- 内容：
--   §1 建表：sys_dept / sys_user / sys_post / sys_role / sys_menu /
--              sys_user_role / sys_role_menu / sys_role_dept / sys_user_post /
--              sys_oper_log / sys_dict_type / sys_dict_data / sys_config /
--              sys_logininfor / sys_notice
--   §2 头像字段与日志表主键序列补丁
--   §3 初始化数据：系统菜单 / 角色 / 用户 / 部门 / 岗位 / 字典 / 参数 / 公告
--   §4 在线用户菜单补丁（历史种子缺失，本项目补齐）
--   §5 顶级目录排序归一
-- ---------------------------------------------------------------------
-- 幂等性：建表 create if not exists；索引 create index if not exists；
--         §0 迁移块走条件判定，老库迁移一次即跳过；
--         §3 初始化数据全部带 on conflict do nothing；
--         §5 排序按 menu_id 对齐，重复执行无副作用。
-- 注意：地图服务与数据服务两个板块的建表与菜单在 map.sql，不在本文件。
-- =====================================================================

-- =====================================================================
-- §0 顶级菜单 ID 归一（老库迁移，必须先于任何菜单写入执行）
-- ---------------------------------------------------------------------
-- 背景：历史顶级目录「地图服务」占 6、「数据功能」占 123（后期追加的号），
--       顶级目录 ID 与展示顺序脱节。本项目重排为：
--         6 数据服务（工作空间 / 数据源 / 数据图层 / 已发布服务）
--         7 地图服务（服务清单 / 日志 / 样式 / 引擎 …）
--       「数据服务」排在「地图服务」之前：先管数据源，再管已发布服务。
-- 影响面：sys_menu.menu_id、sys_menu.parent_id、sys_role_menu.menu_id 三处；
--         全库仅这三处引用 menu_id（sys_menu 除主键外无外键约束），
--         改号不会波及其他表。前端路由 name 由后端运行时按
--         capitalize(path)+menu_id 生成，不落库持久化，改号无需清缓存。
-- 步骤：旧号先搬到临时高位号，再落到最终号，避免主键互相撞车；
--       全程包在 do $$ 块里，异常即整体回滚。
-- =====================================================================

do $$
begin
  -- 新库（sys_menu 尚未建）：无需迁移
  if to_regclass('public.sys_menu') is null then
    return;
  end if;

  -- ① 地图服务 6 → 临时 9106 → 7
  --    判定用「6 号是否叫地图服务」而非「7 号是否存在」：
  --    后者会让「先跑 map.sql」的顺序静默跳过迁移，且不留任何痕迹。
  if exists (select 1 from sys_menu where menu_id = 6 and menu_name = '地图服务') then
    update sys_menu      set menu_id   = 9106 where menu_id   = 6 and menu_name = '地图服务';
    update sys_menu      set parent_id = 9106 where parent_id = 6;
    update sys_role_menu set menu_id   = 9106 where menu_id   = 6;

    -- 目标号 7 若被旧数据占用（历史遗留），先让位：把占用者临时挪走再并回
    if exists (select 1 from sys_menu where menu_id = 7) then
      update sys_menu      set menu_id   = 9107 where menu_id = 7;
      update sys_menu      set parent_id = 9107 where parent_id = 7;
      update sys_role_menu set menu_id   = 9107 where menu_id = 7;
    end if;

    update sys_menu      set menu_id   = 7 where menu_id   = 9106;
    update sys_menu      set parent_id = 7 where parent_id = 9106;
    update sys_role_menu set menu_id   = 7 where menu_id   = 9106;

    -- ⑦ 号旧占用者并入：同角色授权先去重，再落回 7
    delete from sys_role_menu a using sys_role_menu b
      where a.menu_id = 9107 and b.menu_id = 7 and a.role_id = b.role_id;
    update sys_menu      set menu_id   = 7 where menu_id   = 9107;
    update sys_menu      set parent_id = 7 where parent_id = 9107;
    update sys_role_menu set menu_id   = 7 where menu_id   = 9107;
  end if;

  -- ② 数据功能 123 → 6（此时 6 已空出）
  if exists (select 1 from sys_menu where menu_id = 123 and menu_name in ('数据功能', '数据服务')) then
    update sys_menu      set menu_id   = 6 where menu_id = 123 and menu_name in ('数据功能', '数据服务');
    update sys_menu      set parent_id = 6 where parent_id = 123;
    update sys_role_menu set menu_id   = 6 where menu_id   = 123;
    update sys_menu set menu_name = '数据服务' where menu_id = 6 and menu_name = '数据功能';
  end if;
end $$;


-- =====================================================================
-- §1 建表脚本（系统管理表结构，裁剪 social/oss/client/tenant/gen/test）
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

comment on table sys_dept               is '部门表';
comment on column sys_dept.dept_id      is '部门ID';
comment on column sys_dept.parent_id    is '父部门ID';
comment on column sys_dept.ancestors    is '祖级列表';
comment on column sys_dept.dept_name    is '部门名称';
comment on column sys_dept.dept_category    is '部门类别编码';
comment on column sys_dept.order_num    is '显示顺序';
comment on column sys_dept.leader       is '负责人';
comment on column sys_dept.phone        is '联系电话';
comment on column sys_dept.email        is '邮箱';
comment on column sys_dept.status       is '部门状态（0正常 1停用）';
comment on column sys_dept.del_flag     is '删除标志（0代表存在 1代表删除）';
comment on column sys_dept.create_dept  is '创建部门';
comment on column sys_dept.create_by    is '创建者';
comment on column sys_dept.create_time  is '创建时间';
comment on column sys_dept.update_by    is '更新者';
comment on column sys_dept.update_time  is '更新时间';

-- ----------------------------
-- 初始化-部门表数据
-- ----------------------------

-- ----------------------------
-- 2、用户信息表
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

comment on table sys_user               is '用户信息表';
comment on column sys_user.user_id      is '用户ID';
comment on column sys_user.dept_id      is '部门ID';
comment on column sys_user.user_name    is '用户账号';
comment on column sys_user.nick_name    is '用户昵称';
comment on column sys_user.user_type    is '用户类型（sys_user系统用户）';
comment on column sys_user.email        is '用户邮箱';
comment on column sys_user.phonenumber  is '手机号码';
comment on column sys_user.sex          is '用户性别（0男 1女 2未知）';
comment on column sys_user.avatar       is '头像地址（相对路径，如 /profile/avatar/2026/10/26/u1_xxx.png）';
comment on column sys_user.password     is '密码';
comment on column sys_user.status       is '账号状态（0正常 1停用）';
comment on column sys_user.del_flag     is '删除标志（0代表存在 1代表删除）';
comment on column sys_user.login_ip     is '最后登陆IP';
comment on column sys_user.login_date   is '最后登陆时间';
comment on column sys_user.create_dept  is '创建部门';
comment on column sys_user.create_by    is '创建者';
comment on column sys_user.create_time  is '创建时间';
comment on column sys_user.update_by    is '更新者';
comment on column sys_user.update_time  is '更新时间';
comment on column sys_user.remark       is '备注';

-- ----------------------------

-- 初始化-用户信息表数据
-- ----------------------------

-- ----------------------------
-- 3、岗位信息表
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

comment on table sys_post               is '岗位信息表';
comment on column sys_post.post_id      is '岗位ID';
comment on column sys_post.dept_id      is '部门id';
comment on column sys_post.post_code    is '岗位编码';
comment on column sys_post.post_category is '岗位类别编码';
comment on column sys_post.post_name    is '岗位名称';
comment on column sys_post.post_sort    is '显示顺序';
comment on column sys_post.status       is '状态（0正常 1停用）';
comment on column sys_post.create_dept  is '创建部门';
comment on column sys_post.create_by    is '创建者';
comment on column sys_post.create_time  is '创建时间';
comment on column sys_post.update_by    is '更新者';
comment on column sys_post.update_time  is '更新时间';
comment on column sys_post.remark       is '备注';

-- ----------------------------
-- 初始化-岗位信息表数据
-- ----------------------------

-- ----------------------------
-- 4、角色信息表
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

comment on table sys_role                       is '角色信息表';
comment on column sys_role.role_id              is '角色ID';
comment on column sys_role.role_name            is '角色名称';
comment on column sys_role.role_key             is '角色权限字符串';
comment on column sys_role.role_sort            is '显示顺序';
comment on column sys_role.data_scope           is '数据范围（1：全部数据权限 2：自定数据权限 3：本部门数据权限 4：本部门及以下数据权限 5：仅本人数据权限 6：部门及以下或本人数据权限）';
comment on column sys_role.menu_check_strictly  is '菜单树选择项是否关联显示';
comment on column sys_role.dept_check_strictly  is '部门树选择项是否关联显示';
comment on column sys_role.status               is '角色状态（0正常 1停用）';
comment on column sys_role.del_flag             is '删除标志（0代表存在 1代表删除）';
comment on column sys_role.create_dept          is '创建部门';
comment on column sys_role.create_by            is '创建者';
comment on column sys_role.create_time          is '创建时间';
comment on column sys_role.update_by            is '更新者';
comment on column sys_role.update_time          is '更新时间';
comment on column sys_role.remark               is '备注';

-- ----------------------------
-- 初始化-角色信息表数据
-- ----------------------------

-- ----------------------------
-- 5、菜单权限表
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

comment on table sys_menu               is '菜单权限表';
comment on column sys_menu.menu_id      is '菜单ID';
comment on column sys_menu.menu_name    is '菜单名称';
comment on column sys_menu.parent_id    is '父菜单ID';
comment on column sys_menu.order_num    is '显示顺序';
comment on column sys_menu.path         is '路由地址';
comment on column sys_menu.component    is '组件路径';
comment on column sys_menu.query_param  is '路由参数';
comment on column sys_menu.is_frame     is '是否为外链（0是 1否）';
comment on column sys_menu.is_cache     is '是否缓存（0缓存 1不缓存）';
comment on column sys_menu.menu_type    is '菜单类型（M目录 C菜单 F按钮）';
comment on column sys_menu.visible      is '显示状态（0显示 1隐藏）';
comment on column sys_menu.status       is '菜单状态（0正常 1停用）';
comment on column sys_menu.perms        is '权限标识';
comment on column sys_menu.icon         is '菜单图标';
comment on column sys_menu.create_dept  is '创建部门';
comment on column sys_menu.create_by    is '创建者';
comment on column sys_menu.create_time  is '创建时间';
comment on column sys_menu.update_by    is '更新者';
comment on column sys_menu.update_time  is '更新时间';
comment on column sys_menu.remark       is '备注';

-- ----------------------------
-- 初始化-菜单信息表数据
-- ----------------------------
-- 一级菜单
-- 二级菜单

-- springboot-admin监控
-- oss菜单
-- snail-job server控制台

-- 三级菜单
-- 用户管理按钮
-- 角色管理按钮
-- 菜单管理按钮
-- 部门管理按钮
-- 岗位管理按钮
-- 字典管理按钮
-- 参数设置按钮
-- 通知公告按钮
-- 操作日志按钮
-- 登录日志按钮
-- 在线用户按钮
-- 代码生成按钮
-- oss相关按钮
-- 客户端管理按钮
-- 测试菜单


-- ----------------------------
-- 6、用户和角色关联表  用户N-1角色
-- ----------------------------
create table if not exists sys_user_role
(
    user_id int8 not null,
    role_id int8 not null,
    constraint sys_user_role_pk primary key (user_id, role_id)
);

comment on table sys_user_role              is '用户和角色关联表';
comment on column sys_user_role.user_id     is '用户ID';
comment on column sys_user_role.role_id     is '角色ID';

-- ----------------------------
-- 初始化-用户和角色关联表数据
-- ----------------------------

-- ----------------------------
-- 7、角色和菜单关联表  角色1-N菜单
-- ----------------------------
create table if not exists sys_role_menu
(
    role_id int8 not null,
    menu_id int8 not null,
    constraint sys_role_menu_pk primary key (role_id, menu_id)
);

comment on table sys_role_menu              is '角色和菜单关联表';
comment on column sys_role_menu.role_id     is '角色ID';
comment on column sys_role_menu.menu_id     is '菜单ID';

-- ----------------------------
-- 初始化-角色和菜单关联表数据
-- ----------------------------

-- ----------------------------
-- 8、角色和部门关联表  角色1-N部门
-- ----------------------------
create table if not exists sys_role_dept
(
    role_id int8 not null,
    dept_id int8 not null,
    constraint sys_role_dept_pk primary key (role_id, dept_id)
);

comment on table sys_role_dept              is '角色和部门关联表';
comment on column sys_role_dept.role_id     is '角色ID';
comment on column sys_role_dept.dept_id     is '部门ID';


-- ----------------------------
-- 9、用户与岗位关联表  用户1-N岗位
-- ----------------------------
create table if not exists sys_user_post
(
    user_id int8 not null,
    post_id int8 not null,
    constraint sys_user_post_pk primary key (user_id, post_id)
);

comment on table sys_user_post              is '用户与岗位关联表';
comment on column sys_user_post.user_id     is '用户ID';
comment on column sys_user_post.post_id     is '岗位ID';

-- ----------------------------
-- 初始化-用户与岗位关联表数据
-- ----------------------------

-- ----------------------------
-- 10、操作日志记录
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

-- 主键序列：日志表主键无业务含义，必须由序列生成，否则并发插入会撞主键
create sequence if not exists seq_sys_oper_log_oper_id start with 1000;
alter table sys_oper_log alter column oper_id set default nextval('seq_sys_oper_log_oper_id');
select setval('seq_sys_oper_log_oper_id', greatest((select coalesce(max(oper_id), 999) from sys_oper_log), 1000));

create index if not exists idx_sys_oper_log_bt ON sys_oper_log (business_type);
create index if not exists idx_sys_oper_log_s ON sys_oper_log (status);
create index if not exists idx_sys_oper_log_ot ON sys_oper_log (oper_time);

comment on table sys_oper_log                   is '操作日志记录';
comment on column sys_oper_log.oper_id          is '日志主键';
comment on column sys_oper_log.title            is '模块标题';
comment on column sys_oper_log.business_type    is '业务类型（0其它 1新增 2修改 3删除）';
comment on column sys_oper_log.method           is '方法名称';
comment on column sys_oper_log.request_method   is '请求方式';
comment on column sys_oper_log.operator_type    is '操作类别（0其它 1后台用户 2手机端用户）';
comment on column sys_oper_log.oper_name        is '操作人员';
comment on column sys_oper_log.dept_name        is '部门名称';
comment on column sys_oper_log.oper_url         is '请求URL';
comment on column sys_oper_log.oper_ip          is '主机地址';
comment on column sys_oper_log.oper_location    is '操作地点';
comment on column sys_oper_log.oper_param       is '请求参数';
comment on column sys_oper_log.json_result      is '返回参数';
comment on column sys_oper_log.status           is '操作状态（0正常 1异常）';
comment on column sys_oper_log.error_msg        is '错误消息';
comment on column sys_oper_log.oper_time        is '操作时间';
comment on column sys_oper_log.cost_time        is '消耗时间';

-- ----------------------------
-- 11、字典类型表
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

comment on table sys_dict_type                  is '字典类型表';
comment on column sys_dict_type.dict_id         is '字典主键';
comment on column sys_dict_type.dict_name       is '字典名称';
comment on column sys_dict_type.dict_type       is '字典类型';
comment on column sys_dict_type.create_dept     is '创建部门';
comment on column sys_dict_type.create_by       is '创建者';
comment on column sys_dict_type.create_time     is '创建时间';
comment on column sys_dict_type.update_by       is '更新者';
comment on column sys_dict_type.update_time     is '更新时间';
comment on column sys_dict_type.remark          is '备注';


-- ----------------------------
-- 12、字典数据表
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

comment on table sys_dict_data                  is '字典数据表';
comment on column sys_dict_data.dict_code       is '字典编码';
comment on column sys_dict_data.dict_sort       is '字典排序';
comment on column sys_dict_data.dict_label      is '字典标签';
comment on column sys_dict_data.dict_value      is '字典键值';
comment on column sys_dict_data.dict_type       is '字典类型';
comment on column sys_dict_data.css_class       is '样式属性（其他样式扩展）';
comment on column sys_dict_data.list_class      is '表格回显样式';
comment on column sys_dict_data.is_default      is '是否默认（Y是 N否）';
comment on column sys_dict_data.create_dept     is '创建部门';
comment on column sys_dict_data.create_by       is '创建者';
comment on column sys_dict_data.create_time     is '创建时间';
comment on column sys_dict_data.update_by       is '更新者';
comment on column sys_dict_data.update_time     is '更新时间';
comment on column sys_dict_data.remark          is '备注';



-- ----------------------------
-- 13、参数配置表
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

comment on table sys_config                 is '参数配置表';
comment on column sys_config.config_id      is '参数主键';
comment on column sys_config.config_name    is '参数名称';
comment on column sys_config.config_key     is '参数键名';
comment on column sys_config.config_value   is '参数键值';
comment on column sys_config.config_type    is '系统内置（Y是 N否）';
comment on column sys_config.create_dept    is '创建部门';
comment on column sys_config.create_by      is '创建者';
comment on column sys_config.create_time    is '创建时间';
comment on column sys_config.update_by      is '更新者';
comment on column sys_config.update_time    is '更新时间';
comment on column sys_config.remark         is '备注';



-- ----------------------------
-- 14、系统访问记录
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

-- 主键序列：同操作日志，访问记录主键由序列生成
create sequence if not exists seq_sys_logininfor_info_id start with 1000;
alter table sys_logininfor alter column info_id set default nextval('seq_sys_logininfor_info_id');
select setval('seq_sys_logininfor_info_id', greatest((select coalesce(max(info_id), 999) from sys_logininfor), 1000));

create index if not exists idx_sys_logininfor_s ON sys_logininfor (status);
create index if not exists idx_sys_logininfor_lt ON sys_logininfor (login_time);

comment on table sys_logininfor                 is '系统访问记录';
comment on column sys_logininfor.info_id        is '访问ID';
comment on column sys_logininfor.user_name      is '用户账号';
comment on column sys_logininfor.client_key     is '客户端';
comment on column sys_logininfor.device_type    is '设备类型';
comment on column sys_logininfor.ipaddr         is '登录IP地址';
comment on column sys_logininfor.login_location is '登录地点';
comment on column sys_logininfor.browser        is '浏览器类型';
comment on column sys_logininfor.os             is '操作系统';
comment on column sys_logininfor.status         is '登录状态（0成功 1失败）';
comment on column sys_logininfor.msg            is '提示消息';
comment on column sys_logininfor.login_time     is '访问时间';

-- ----------------------------
-- 17、通知公告表
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

comment on table sys_notice                 is '通知公告表';
comment on column sys_notice.notice_id      is '公告ID';
comment on column sys_notice.notice_title   is '公告标题';
comment on column sys_notice.notice_type    is '公告类型（1通知 2公告）';
comment on column sys_notice.notice_content is '公告内容';
comment on column sys_notice.status         is '公告状态（0正常 1关闭）';
comment on column sys_notice.create_dept    is '创建部门';
comment on column sys_notice.create_by      is '创建者';
comment on column sys_notice.create_time    is '创建时间';
comment on column sys_notice.update_by      is '更新者';
comment on column sys_notice.update_time    is '更新时间';
comment on column sys_notice.remark         is '备注';

-- ----------------------------
-- 初始化-公告信息表数据
-- ----------------------------


-- ----------------------------
-- 18、代码生成业务表
-- ----------------------------

-- =====================================================================
-- §2 头像字段与日志表主键序列补丁
-- ---------------------------------------------------------------------
-- 5.X 原结构 sys_user.avatar 为 bigint（OSS 文件 id），本项目不引入 OSS，
-- 改为 varchar(500) 存放相对路径（如 /profile/avatar/2026/10/26/u1_xxx.png）。
-- oper_id / info_id 补序列，避免并发插入撞主键。
-- =====================================================================

BEGIN;

-- 1. avatar 改为 varchar(500)
ALTER TABLE sys_user
    ALTER COLUMN avatar TYPE varchar(500) USING NULLIF(avatar::text, '');

-- 2. 头像上传只允许 PNG/JPEG，按扩展名白名单落地（后端同样校验）
COMMENT ON COLUMN sys_user.avatar IS '用户头像相对路径（/profile/avatar/...），为空表示使用默认头像';

-- 3. 操作日志表的 oper_id / 登录日志表的 info_id 无默认值，插入时由应用侧生成；
--    这里补上序列，避免并发插入撞主键。
CREATE SEQUENCE IF NOT EXISTS seq_sys_oper_log_oper_id START WITH 1000;
ALTER TABLE sys_oper_log ALTER COLUMN oper_id SET DEFAULT nextval('seq_sys_oper_log_oper_id');

CREATE SEQUENCE IF NOT EXISTS seq_sys_logininfor_info_id START WITH 1000;
ALTER TABLE sys_logininfor ALTER COLUMN info_id SET DEFAULT nextval('seq_sys_logininfor_info_id');

-- 4. 主键序列与当前最大 id 对齐
SELECT setval('seq_sys_oper_log_oper_id',
              GREATEST((SELECT COALESCE(max(oper_id), 999) FROM sys_oper_log), 1000));
SELECT setval('seq_sys_logininfor_info_id',
              GREATEST((SELECT COALESCE(max(info_id), 999) FROM sys_logininfor), 1000));

COMMIT;

-- =====================================================================
-- §3 初始化数据（系统菜单 / 角色 / 用户 / 部门 / 岗位 / 字典 / 参数 / 公告）
-- ---------------------------------------------------------------------
-- 说明：已剔除与前端裁剪同步下线的菜单（tool/gen、system/oss、system/client、
--       system/tenant、monitor/{admin,cache,snailjob}、test/*）及其 role_menu 记录。
--       地图服务 / 数据服务两个板块的菜单与授权在 map.sql，不在本段。
--       全部写入带 on conflict do nothing，重复执行不报错。
-- 默认账号：admin / admin123（bcrypt，首次登录后请修改）
-- =====================================================================

insert into sys_menu values('1', '系统管理', '0', '1', 'system',           null, '', '1', '0', 'M', '0', '0', '', 'system',   103, 1, now(), null, null, '系统管理目录') on conflict do nothing;
insert into sys_menu values('2', '系统监控', '0', '4', 'monitor',          null, '', '1', '0', 'M', '0', '0', '', 'monitor',  103, 1, now(), null, null, '系统监控目录') on conflict do nothing;
insert into sys_menu values('3', '系统工具', '0', '5', 'tool',             null, '', '1', '0', 'M', '0', '0', '', 'tool',     103, 1, now(), null, null, '系统工具目录') on conflict do nothing;
insert into sys_menu VALUES('5', '测试菜单', '0', '7', 'demo',             null, '', '1', '0', 'M', '0', '0', null, 'star',       103, 1, now(), null, null, '测试菜单') on conflict do nothing;
insert into sys_menu values('100', '用户管理', '1', '1', 'user',             'system/user/index',            '', '1', '0', 'C', '0', '0', 'system:user:list',            'user',          103, 1, now(), null, null, '用户管理菜单') on conflict do nothing;
insert into sys_menu values('101', '角色管理', '1', '2', 'role',             'system/role/index',            '', '1', '0', 'C', '0', '0', 'system:role:list',            'peoples',       103, 1, now(), null, null, '角色管理菜单') on conflict do nothing;
insert into sys_menu values('102', '菜单管理', '1', '3', 'menu',             'system/menu/index',            '', '1', '0', 'C', '0', '0', 'system:menu:list',            'tree-table',    103, 1, now(), null, null, '菜单管理菜单') on conflict do nothing;
insert into sys_menu values('103', '部门管理', '1', '4', 'dept',             'system/dept/index',            '', '1', '0', 'C', '0', '0', 'system:dept:list',            'tree-dept',     103, 1, now(), null, null, '部门管理菜单') on conflict do nothing;
insert into sys_menu values('104', '岗位管理', '1', '5', 'post',             'system/post/index',            '', '1', '0', 'C', '0', '0', 'system:post:list',            'post',          103, 1, now(), null, null, '岗位管理菜单') on conflict do nothing;
insert into sys_menu values('105', '字典管理', '1', '6', 'dict',             'system/dict/index',            '', '1', '0', 'C', '0', '0', 'system:dict:list',            'dict',          103, 1, now(), null, null, '字典管理菜单') on conflict do nothing;
insert into sys_menu values('106', '参数设置', '1', '7', 'config',           'system/config/index',          '', '1', '0', 'C', '0', '0', 'system:config:list',          'sliders',       103, 1, now(), null, null, '参数设置菜单') on conflict do nothing;
insert into sys_menu values('107', '通知公告', '1', '8', 'notice',           'system/notice/index',          '', '1', '0', 'C', '0', '0', 'system:notice:list',          'message',       103, 1, now(), null, null, '通知公告菜单') on conflict do nothing;
insert into sys_menu values('108', '日志管理', '1', '9', 'log',              '',                             '', '1', '0', 'M', '0', '0', '',                            'log-sys',       103, 1, now(), null, null, '日志管理菜单') on conflict do nothing;
insert into sys_menu values('130', '分配用户', '1', '2', 'role-auth/user/:roleId', 'system/role/authUser', '', '1', '1', 'C', '1', '0', 'system:role:edit',      '#',               103, 1, now(), null, null, '/system/role') on conflict do nothing;
insert into sys_menu values('131', '分配角色', '1', '1', 'user-auth/role/:userId', 'system/user/authRole', '', '1', '1', 'C', '1', '0', 'system:user:edit',      '#',               103, 1, now(), null, null, '/system/user') on conflict do nothing;
insert into sys_menu values('132', '字典数据', '1', '6', 'dict-data/index/:dictId', 'system/dict/data', '', '1', '1', 'C', '1', '0', 'system:dict:list',         '#',               103, 1, now(), null, null, '/system/dict') on conflict do nothing;
insert into sys_menu values('500', '操作日志', '108', '1', 'operlog',    'monitor/operlog/index',    '', '1', '0', 'C', '0', '0', 'monitor:operlog:list',    'log-sys-op',    103, 1, now(), null, null, '操作日志菜单') on conflict do nothing;
insert into sys_menu values('501', '登录日志', '108', '2', 'logininfor', 'monitor/logininfor/index', '', '1', '0', 'C', '0', '0', 'monitor:logininfor:list', 'log-sys-login', 103, 1, now(), null, null, '登录日志菜单') on conflict do nothing;
insert into sys_menu values('109', '在线用户', '2', '1', 'online',     'monitor/online/index',     '', '1', '0', 'C', '0', '0', 'monitor:online:list',     'online',        103, 1, now(), null, null, '在线用户菜单') on conflict do nothing;
insert into sys_menu values('1001', '用户查询', '100', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1002', '用户新增', '100', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1003', '用户修改', '100', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1004', '用户删除', '100', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1005', '用户导出', '100', '5',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1006', '用户导入', '100', '6',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:import',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1007', '重置密码', '100', '7',  '', '', '', '1', '0', 'F', '0', '0', 'system:user:resetPwd',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1008', '角色查询', '101', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1009', '角色新增', '101', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1010', '角色修改', '101', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1011', '角色删除', '101', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1012', '角色导出', '101', '5',  '', '', '', '1', '0', 'F', '0', '0', 'system:role:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1013', '菜单查询', '102', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1014', '菜单新增', '102', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1015', '菜单修改', '102', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1016', '菜单删除', '102', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:menu:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1017', '部门查询', '103', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1018', '部门新增', '103', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1019', '部门修改', '103', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1020', '部门删除', '103', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:dept:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1021', '岗位查询', '104', '1',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1022', '岗位新增', '104', '2',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1023', '岗位修改', '104', '3',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1024', '岗位删除', '104', '4',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1025', '岗位导出', '104', '5',  '', '', '', '1', '0', 'F', '0', '0', 'system:post:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1026', '字典查询', '105', '1', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:query',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1027', '字典新增', '105', '2', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:add',            '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1028', '字典修改', '105', '3', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:edit',           '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1029', '字典删除', '105', '4', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:remove',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1030', '字典导出', '105', '5', '#', '', '', '1', '0', 'F', '0', '0', 'system:dict:export',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1031', '参数查询', '106', '1', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:query',        '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1032', '参数新增', '106', '2', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:add',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1033', '参数修改', '106', '3', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:edit',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1034', '参数删除', '106', '4', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:remove',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1035', '参数导出', '106', '5', '#', '', '', '1', '0', 'F', '0', '0', 'system:config:export',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1036', '公告查询', '107', '1', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:query',        '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1037', '公告新增', '107', '2', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:add',          '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1038', '公告修改', '107', '3', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:edit',         '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1039', '公告删除', '107', '4', '#', '', '', '1', '0', 'F', '0', '0', 'system:notice:remove',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1040', '操作查询', '500', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:operlog:query',      '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1041', '操作删除', '500', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:operlog:remove',     '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1042', '日志导出', '500', '4', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:operlog:export',     '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1043', '登录查询', '501', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:query',   '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1044', '登录删除', '501', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:remove',  '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1045', '日志导出', '501', '3', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:export',  '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1050', '账户解锁', '501', '4', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:logininfor:unlock',  '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1046', '在线查询', '109', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:query',       '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1047', '批量强退', '109', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:batchLogout', '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_menu values('1048', '单条强退', '109', '3', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:forceLogout', '#', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_dept values(100, 0,   '0',          'XXX科技',   null,0, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(101, 100, '0,100',      '深圳总公司', null,1, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(102, 100, '0,100',      '长沙分公司', null,2, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(103, 101, '0,100,101',  '研发部门',   null,1, 1, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(104, 101, '0,100,101',  '市场部门',   null,2, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(105, 101, '0,100,101',  '测试部门',   null,3, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(106, 101, '0,100,101',  '财务部门',   null,4, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(107, 101, '0,100,101',  '运维部门',   null,5, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(108, 102, '0,100,102',  '市场部门',   null,1, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_dept values(109, 102, '0,100,102',  '财务部门',   null,2, null, '15888888888', 'xxx@qq.com', '0', '0', 103, 1, now(), null, null) on conflict do nothing;
insert into sys_user values(1, 103, 'admin', 'admin', 'sys_user', 'admin@example.com', '15888888888', '1', null, '$2a$10$7JB720yubVSZvUI0rEqK/.VqGOZTH.ulu33dHOiBE8ByOhJIrdAu2', '0', '0', '127.0.0.1', now(), 103, 1, now(), null, null, '管理员') on conflict do nothing;
insert into sys_user VALUES(3, 108, 'test', '本部门及以下 密码666666', 'sys_user', '', '', '0', null, '$2a$10$b8yUzN0C71sbz.PhNOCgJe.Tu1yWC3RNrTyjSQ8p1W0.aaUXUJ.Ne', '0', '0', '127.0.0.1', now(), 103, 1, now(), 3, now(), NULL) on conflict do nothing;
insert into sys_user VALUES(4, 102, 'test1', '仅本人 密码666666', 'sys_user', '', '', '0', null, '$2a$10$b8yUzN0C71sbz.PhNOCgJe.Tu1yWC3RNrTyjSQ8p1W0.aaUXUJ.Ne', '0', '0', '127.0.0.1', now(), 103, 1, now(), 4, now(), NULL) on conflict do nothing;
insert into sys_post values(1, 103, 'ceo',  null, '董事长',    1, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_post values(2, 100, 'se',   null, '项目经理',  2, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_post values(3, 100, 'hr',   null, '人力资源',  3, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_post values(4, 100, 'user', null, '普通员工',  4, '0', 103, 1, now(), null, null, '') on conflict do nothing;
insert into sys_role values('1', '超级管理员',  'superadmin',  1, '1', 't', 't', '0', '0', 103, 1, now(), null, null, '超级管理员') on conflict do nothing;
insert into sys_role values('3', '本部门及以下', 'test1', 3, '4', 't', 't', '0', '0', 103, 1, now(), NULL, NULL, '') on conflict do nothing;
insert into sys_role values('4', '仅本人', 'test2', 4, '5', 't', 't', '0', '0', 103, 1, now(), NULL, NULL, '') on conflict do nothing;
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
insert into sys_dict_type values(1, '用户性别', 'sys_user_sex',        103, 1, now(), null, null, '用户性别列表') on conflict do nothing;
insert into sys_dict_type values(2, '菜单状态', 'sys_show_hide',       103, 1, now(), null, null, '菜单状态列表') on conflict do nothing;
insert into sys_dict_type values(3, '系统开关', 'sys_normal_disable',  103, 1, now(), null, null, '系统开关列表') on conflict do nothing;
insert into sys_dict_type values(6, '系统是否', 'sys_yes_no',          103, 1, now(), null, null, '系统是否列表') on conflict do nothing;
insert into sys_dict_type values(7, '通知类型', 'sys_notice_type',     103, 1, now(), null, null, '通知类型列表') on conflict do nothing;
insert into sys_dict_type values(8, '通知状态', 'sys_notice_status',   103, 1, now(), null, null, '通知状态列表') on conflict do nothing;
insert into sys_dict_type values(9, '操作类型', 'sys_oper_type',       103, 1, now(), null, null, '操作类型列表') on conflict do nothing;
insert into sys_dict_type values(10, '系统状态', 'sys_common_status',  103, 1, now(), null, null, '登录状态列表') on conflict do nothing;
insert into sys_dict_type values(11, '授权类型', 'sys_grant_type',     103, 1, now(), null, null, '认证授权类型') on conflict do nothing;
insert into sys_dict_type values(12, '设备类型', 'sys_device_type',    103, 1, now(), null, null, '客户端设备类型') on conflict do nothing;
insert into sys_dict_data values(1, 1,  '男',       '0',       'sys_user_sex',        '',   '',        'Y', 103, 1, now(), null, null, '性别男') on conflict do nothing;
insert into sys_dict_data values(2, 2,  '女',       '1',       'sys_user_sex',        '',   '',        'N', 103, 1, now(), null, null, '性别女') on conflict do nothing;
insert into sys_dict_data values(3, 3,  '未知',     '2',       'sys_user_sex',        '',   '',        'N', 103, 1, now(), null, null, '性别未知') on conflict do nothing;
insert into sys_dict_data values(4, 1,  '显示',     '0',       'sys_show_hide',       '',   'primary', 'Y', 103, 1, now(), null, null, '显示菜单') on conflict do nothing;
insert into sys_dict_data values(5, 2,  '隐藏',     '1',       'sys_show_hide',       '',   'danger',  'N', 103, 1, now(), null, null, '隐藏菜单') on conflict do nothing;
insert into sys_dict_data values(6, 1,  '正常',     '0',       'sys_normal_disable',  '',   'primary', 'Y', 103, 1, now(), null, null, '正常状态') on conflict do nothing;
insert into sys_dict_data values(7, 2,  '停用',     '1',       'sys_normal_disable',  '',   'danger',  'N', 103, 1, now(), null, null, '停用状态') on conflict do nothing;
insert into sys_dict_data values(12, 1,  '是',       'Y',       'sys_yes_no',          '',   'primary', 'Y', 103, 1, now(), null, null, '系统默认是') on conflict do nothing;
insert into sys_dict_data values(13, 2,  '否',       'N',       'sys_yes_no',          '',   'danger',  'N', 103, 1, now(), null, null, '系统默认否') on conflict do nothing;
insert into sys_dict_data values(14, 1,  '通知',     '1',       'sys_notice_type',     '',   'warning', 'Y', 103, 1, now(), null, null, '通知') on conflict do nothing;
insert into sys_dict_data values(15, 2,  '公告',     '2',       'sys_notice_type',     '',   'success', 'N', 103, 1, now(), null, null, '公告') on conflict do nothing;
insert into sys_dict_data values(16, 1,  '正常',     '0',       'sys_notice_status',   '',   'primary', 'Y', 103, 1, now(), null, null, '正常状态') on conflict do nothing;
insert into sys_dict_data values(17, 2,  '关闭',     '1',       'sys_notice_status',   '',   'danger',  'N', 103, 1, now(), null, null, '关闭状态') on conflict do nothing;
insert into sys_dict_data values(29, 99, '其他',     '0',       'sys_oper_type',       '',   'info',    'N', 103, 1, now(), null, null, '其他操作') on conflict do nothing;
insert into sys_dict_data values(18, 1,  '新增',     '1',       'sys_oper_type',       '',   'info',    'N', 103, 1, now(), null, null, '新增操作') on conflict do nothing;
insert into sys_dict_data values(19, 2,  '修改',     '2',       'sys_oper_type',       '',   'info',    'N', 103, 1, now(), null, null, '修改操作') on conflict do nothing;
insert into sys_dict_data values(20, 3,  '删除',     '3',       'sys_oper_type',       '',   'danger',  'N', 103, 1, now(), null, null, '删除操作') on conflict do nothing;
insert into sys_dict_data values(21, 4,  '授权',     '4',       'sys_oper_type',       '',   'primary', 'N', 103, 1, now(), null, null, '授权操作') on conflict do nothing;
insert into sys_dict_data values(22, 5,  '导出',     '5',       'sys_oper_type',       '',   'warning', 'N', 103, 1, now(), null, null, '导出操作') on conflict do nothing;
insert into sys_dict_data values(23, 6,  '导入',     '6',       'sys_oper_type',       '',   'warning', 'N', 103, 1, now(), null, null, '导入操作') on conflict do nothing;
insert into sys_dict_data values(24, 7,  '强退',     '7',       'sys_oper_type',       '',   'danger',  'N', 103, 1, now(), null, null, '强退操作') on conflict do nothing;
insert into sys_dict_data values(25, 8,  '生成代码', '8',       'sys_oper_type',       '',   'warning', 'N', 103, 1, now(), null, null, '生成操作') on conflict do nothing;
insert into sys_dict_data values(26, 9,  '清空数据', '9',       'sys_oper_type',       '',   'danger',  'N', 103, 1, now(), null, null, '清空操作') on conflict do nothing;
insert into sys_dict_data values(27, 1,  '成功',     '0',       'sys_common_status',   '',   'primary', 'N', 103, 1, now(), null, null, '正常状态') on conflict do nothing;
insert into sys_dict_data values(28, 2,  '失败',     '1',       'sys_common_status',   '',   'danger',  'N', 103, 1, now(), null, null, '停用状态') on conflict do nothing;
insert into sys_dict_data values(30, 0,  '密码认证', 'password',   'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, '密码认证') on conflict do nothing;
insert into sys_dict_data values(31, 0,  '短信认证', 'sms',        'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, '短信认证') on conflict do nothing;
insert into sys_dict_data values(32, 0,  '邮件认证', 'email',      'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, '邮件认证') on conflict do nothing;
insert into sys_dict_data values(33, 0,  '小程序认证', 'xcx',      'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, '小程序认证') on conflict do nothing;
insert into sys_dict_data values(34, 0,  '三方登录认证', 'social', 'sys_grant_type',   '',   'default', 'N', 103, 1, now(), null, null, '三方登录认证') on conflict do nothing;
insert into sys_dict_data values(35, 0,  'PC', 'pc',              'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, 'PC') on conflict do nothing;
insert into sys_dict_data values(36, 0,  '安卓', 'android',       'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, '安卓') on conflict do nothing;
insert into sys_dict_data values(37, 0,  'iOS', 'ios',            'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, 'iOS') on conflict do nothing;
insert into sys_dict_data values(38, 0,  '小程序', 'xcx',         'sys_device_type',   '',   'default', 'N', 103, 1, now(), null, null, '小程序') on conflict do nothing;
insert into sys_config values(1, '主框架页-默认皮肤样式名称',     'sys.index.skinName',            'skin-blue',     'Y', 103, 1, now(), null, null, '蓝色 skin-blue、绿色 skin-green、紫色 skin-purple、红色 skin-red、黄色 skin-yellow' ) on conflict do nothing;
insert into sys_config values(2, '用户管理-账号初始密码',         'sys.user.initPassword',         '123456',        'Y', 103, 1, now(), null, null, '初始化密码 123456' ) on conflict do nothing;
insert into sys_config values(3, '主框架页-侧边栏主题',           'sys.index.sideTheme',           'theme-dark',    'Y', 103, 1, now(), null, null, '深色主题theme-dark，浅色主题theme-light' ) on conflict do nothing;
insert into sys_config values(5, '账号自助-是否开启用户注册功能',   'sys.account.registerUser',      'false',         'Y', 103, 1, now(), null, null, '是否开启注册用户功能（true开启，false关闭）') on conflict do nothing;
insert into sys_config values(11, 'OSS预览列表资源开关',          'sys.oss.previewListResource',   'true',          'Y', 103, 1, now(), null, null, 'true:开启, false:关闭') on conflict do nothing;
insert into sys_notice values('1', '温馨提醒：2018-07-01 新版本发布啦', '2', '新版本内容', '0', 103, 1, now(), null, null, '管理员') on conflict do nothing;
insert into sys_notice values('2', '维护通知：2018-07-01 系统凌晨维护', '1', '维护内容',   '0', 103, 1, now(), null, null, '管理员') on conflict do nothing;

-- =====================================================================
-- §4 在线用户菜单补丁
-- ---------------------------------------------------------------------
-- 历史种子中 sys_menu 有 109（在线用户，parent=2 系统监控）与 1046/1047/1048
-- 三个按钮权限，本项目初始种子缺失，导致前端 monitor/online 页面无入口。
-- =====================================================================

-- 菜单：在线用户（挂在「系统监控」目录下）
insert into sys_menu values('109', '在线用户', '2', '1', 'online', 'monitor/online/index', '', '1', '0', 'C', '0', '0', 'monitor:online:list', 'online', 103, 1, now(), null, null, '在线用户菜单')
on conflict (menu_id) do nothing;

-- 按钮权限
insert into sys_menu values('1046', '在线查询', '109', '1', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:query', '#', 103, 1, now(), null, null, '')
on conflict (menu_id) do nothing;
insert into sys_menu values('1047', '批量强退', '109', '2', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:batchLogout', '#', 103, 1, now(), null, null, '')
on conflict (menu_id) do nothing;
insert into sys_menu values('1048', '单条强退', '109', '3', '#', '', '', '1', '0', 'F', '0', '0', 'monitor:online:forceLogout', '#', 103, 1, now(), null, null, '')
on conflict (menu_id) do nothing;

-- 角色授权：role 3（test / test1）需要同时拿到父目录「系统监控」(2)，
-- 否则 109 在菜单树上挂不上、前端不显示。
insert into sys_role_menu values ('3', '2') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '109') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '1046') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '1047') on conflict (role_id, menu_id) do nothing;
insert into sys_role_menu values ('3', '1048') on conflict (role_id, menu_id) do nothing;

-- =====================================================================
-- §5 顶级目录排序归一
-- ---------------------------------------------------------------------
-- 目标顺序：1 系统管理 / 6 数据服务 / 7 地图服务 / 2 系统监控 /
-- 按 menu_id 定位（parent_id = 0 的顶级行），重复执行只写回同一值。
-- =====================================================================
update sys_menu set order_num = 1 where menu_id = '1' and parent_id = 0; -- 系统管理
update sys_menu set order_num = 2 where menu_id = '6' and parent_id = 0; -- 数据服务
update sys_menu set order_num = 3 where menu_id = '7' and parent_id = 0; -- 地图服务
update sys_menu set order_num = 4 where menu_id = '2' and parent_id = 0; -- 系统监控
update sys_menu set order_num = 5 where menu_id = '3' and parent_id = 0; -- 系统工具
update sys_menu set order_num = 7 where menu_id = '5' and parent_id = 0; -- 测试菜单
