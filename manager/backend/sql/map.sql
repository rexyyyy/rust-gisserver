-- =====================================================================
-- rust-gisserver · map.sql —— 地图服务与数据服务板块（建表 + 菜单）
-- ---------------------------------------------------------------------
-- 用途：rust-gisserver 管理端的地图业务底座，包含两个顶级目录：
--       ① 数据服务（顶级目录 6）：工作空间 / 数据源 / 数据图层 / 已发布服务
--          —— Data 组语义：只管数据源组织与发布账本；
--       ② 地图服务（顶级目录 7）：服务清单 / 访问日志 / 样式库 / 服务垃圾箱 /
--          运营仪表盘 / 缓存管理 / 运行配置 / 3D Tiles / 栅格 / 矢量 / 瓦片 /
--          服务引擎管理 —— 已发布出去的服务的运维面。
--       「数据服务」排在「地图服务」之前：先管数据源，再管已发布服务。
-- 依赖：需先执行 system.sql（建好 sys_menu / sys_role_menu 等系统表）
-- 用法：psql -h localhost -p 5432 -U postgres -d mapserver-manager -f map.sql
-- ---------------------------------------------------------------------
-- 内容：
--   §0  顶级菜单 ID 归一（老库迁移；与 system.sql §0 同一块，可重复执行）
--   §1  地图服务板块菜单（顶级目录 7）
--   §2  服务引擎注册表 sys_engine + 引擎相关菜单
--   §3  服务与引擎的归属关系 sys_service_engine
--   §4  引擎监控事件 sys_engine_event + 服务分组 sys_service_meta
--   §5  组织层四表 sys_workspace / sys_store / sys_resource / sys_published_layer
--   §6   数据服务板块菜单（顶级目录 6）
--   §6.5 API Key 管理（sys_api_key + 菜单 128 / 按钮 1067）
-- ---------------------------------------------------------------------
-- 幂等性：建表 create if not exists；菜单与授权 insert 均带 on conflict do
--         nothing；§0 迁移块条件判定，重复执行无副作用。
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
-- §1 地图服务板块菜单（顶级目录 7）
-- ---------------------------------------------------------------------
-- 板块结构：
--   7    地图服务（目录 M）
--   110  服务总览（C，map/service/index，map:service:list）
--   111  访问日志（C，map/log/index，map:log:list）
--   112  样式库（C，map/style/index，map:style:list）
--   113  服务垃圾箱（C，map/trash/index，map:service:list）
--   114  运营仪表盘（C，map/dashboard/index，map:service:list）
--   115  缓存管理（C，map/cache/index，map:service:list）
--   116  3D Tiles管理（C，map/tiles/index，map:service:list）
--   117  运行配置（C，map/settings/index，map:service:list）
--   118  栅格服务 / 119 矢量服务 / 120 瓦片服务 / 121 服务引擎管理
--   1051..1059 按钮（F）
-- =====================================================================

insert into sys_menu values('7', '地图服务', '0', '3', 'map',      null,                    '', '1', '0', 'M', '0', '0', '',                   'map',     103, 1, now(), null, null, '地图服务管理目录')
  on conflict (menu_id) do nothing;

insert into sys_menu values('110', '服务管理', '7', '1', 'service',  'map/service/index',     '', '1', '0', 'C', '0', '0', 'map:service:list',   'service', 103, 1, now(), null, null, '地图服务清单与生命周期管理菜单')
  on conflict (menu_id) do nothing;

insert into sys_menu values('1051', '服务状态修改', '110', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:service:state',  '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1052', '服务重载', '110', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:service:reload', '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1053', '访问统计查询', '110', '3', '', '', '', '1', '0', 'F', '0', '0', 'map:usage:query',   '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;

-- 角色授权（角色 3「本部门及以下」；超管不走 role_menu）
insert into sys_role_menu values ('3', '7')    on conflict do nothing;
insert into sys_role_menu values ('3', '110')  on conflict do nothing;
insert into sys_role_menu values ('3', '1051') on conflict do nothing;
insert into sys_role_menu values ('3', '1052') on conflict do nothing;
insert into sys_role_menu values ('3', '1053') on conflict do nothing;

-- ② 新增：访问日志 / 样式库（2026-10-27 P0/P1 功能）
insert into sys_menu values('111', '访问日志', '7', '2', 'log',      'map/log/index',         '', '1', '0', 'C', '0', '0', 'map:log:list',       'log-map', 103, 1, now(), null, null, '服务访问日志与审计查询')
  on conflict (menu_id) do nothing;
insert into sys_menu values('112', '样式库', '7', '3', 'style',    'map/style/index',       '', '1', '0', 'C', '0', '0', 'map:style:list',     'brush',   103, 1, now(), null, null, 'SLD 样式库管理（QGIS 编辑引导）')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1054', '服务体检', '110', '4', '', '', '', '1', '0', 'F', '0', '0', 'map:service:check',  '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1056', '样式上传', '112', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:style:upload',   '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1057', '样式删除', '112', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:style:remove',   '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;

-- ② 新增菜单授权（角色 3）
insert into sys_role_menu values ('3', '111')  on conflict do nothing;
insert into sys_role_menu values ('3', '112')  on conflict do nothing;
insert into sys_role_menu values ('3', '1054') on conflict do nothing;
insert into sys_role_menu values ('3', '1056') on conflict do nothing;
insert into sys_role_menu values ('3', '1057') on conflict do nothing;

-- ③ 新增：服务垃圾箱（2026-10-28 服务详情+垃圾箱）
insert into sys_menu values('113', '服务垃圾箱', '7', '4', 'trash', 'map/trash/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'delete', 103, 1, now(), null, null, '已删除服务的回收箱：恢复后立即生效')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '113') on conflict do nothing;

-- ④ 新增：运营仪表盘（2026-10-28 首页仪表盘，复用 map:service:list 权限，无需改后端）
insert into sys_menu values('114', '运营仪表盘', '7', '0', 'dashboard', 'map/dashboard/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'dashboard', 103, 1, now(), null, null, '地图服务运营仪表盘：服务统计/组件探测/访问计数')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '114') on conflict do nothing;

-- ⑤ 新增：缓存管理（2026-10-28 P2 缓存管理 UI；查看/清理均复用 map:service:list 权限，无需改后端）
insert into sys_menu values('115', '缓存管理', '7', '6', 'cache', 'map/cache/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'cache', 103, 1, now(), null, null, 'map-api 瓦片/元数据缓存：实时计数与按范围清理')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '115') on conflict do nothing;

--   115   缓存管理（C，component: map/cache/index，perms: map:service:list）
-- ④ 3D Tiles 专门管理（2026-10-29）
insert into sys_menu values('116', '3D Tiles管理', '7', '5', 'tiles', 'map/tiles/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'box-3d', 103, 1, now(), null, null, '3D Tiles 服务的专门管理（列表/状态/一键 Cesium 预览）')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '116') on conflict do nothing;

-- ⑤ 配置/设置页（2026-10-29）
insert into sys_menu values('117', '运行配置', '7', '6', 'settings', 'map/settings/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'config', 103, 1, now(), null, null, '只读展示 manager 与 map-api 的运行配置')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '117') on conflict do nothing;

-- ⑥ 服务配置编辑（2026-10-29 P0 配置编辑对话框）
--    按钮权限挂在「服务管理」(110) 下，perms: map:service:edit
--    对应后端路由 PUT /map/services/:kind/:name（回写 XML 配置并热重载）
insert into sys_menu values('1058', '服务配置编辑', '110', '5', '', '', '', '1', '0', 'F', '0', '0', 'map:service:edit', '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '1058') on conflict do nothing;

-- =====================================================================
-- §2 服务引擎注册表 sys_engine + 引擎相关菜单
-- ---------------------------------------------------------------------
--   118 栅格服务（COG）/ 119 矢量服务（vector+postgis）/
--   120 瓦片服务（pmtiles+mbtiles）/ 123 ArcGIS瓦片图层（arcgis）/
--   121 服务引擎管理
--   1059 服务引擎维护（F，map:engine:edit）
-- =====================================================================

create table if not exists sys_engine (
  engine_id   varchar(64)  primary key,
  engine_name varchar(64)  not null,
  engine_type varchar(32)  not null default 'rust-gisserver',
  base_url    varchar(255) not null,
  admin_token varchar(128) default '',
  status      char(1)      not null default '0',
  is_default  char(1)      not null default '0',
  sort        int          not null default 0,
  remark      varchar(500) default '',
  create_time timestamp    default now(),
  update_time timestamp
);

comment on table sys_engine is '地图服务引擎注册表：管理代理转发默认引擎，健康探测按 base_url';
comment on column sys_engine.engine_type is '引擎类型（当前 rust-gisserver；预留扩展）';
comment on column sys_engine.is_default is '默认引擎：管理代理 /map/* 转发目标；无默认时回落 manager.xml mapsvc';

-- 种子：本地 mapserve（幂等；base_url 不覆盖已有环境配置）
insert into sys_engine (engine_id, engine_name, engine_type, base_url, status, is_default, sort, remark)
values ('mapserve', 'rust-gisserver mapserve', 'rust-gisserver', 'http://map-api:8080', '0', '1', 0,
        '本地 rust-gisserver 服务面（种子值，生产按环境改 base_url）')
on conflict (engine_id) do nothing;

-- ---------- B. 菜单 ----------
-- 110 改名：全类型清单 → 服务总览（幂等：重复执行仅重复赋值）
update sys_menu set menu_name = '服务总览', remark = '全部类型地图服务清单与生命周期管理'
 where menu_id = '110';

-- 118 栅格服务（COG）：复用 map/service/index 组件 + 包装页 presetKinds
insert into sys_menu values('118', '栅格服务', '7', '2', 'svc-cog', 'map/service/cog', '', '1', '0', 'C', '0', '0', 'map:service:list', 'picture', 103, 1, now(), null, null, 'COG 栅格影像服务')
  on conflict (menu_id) do nothing;

-- 119 矢量服务（vector + postgis）
insert into sys_menu values('119', '矢量服务', '7', '3', 'svc-vector', 'map/service/vector', '', '1', '0', 'C', '0', '0', 'map:service:list', 'vector', 103, 1, now(), null, null, '矢量文件与 PostGIS 服务')
  on conflict (menu_id) do nothing;

-- 120 瓦片服务（pmtiles + mbtiles）
insert into sys_menu values('120', '瓦片服务', '7', '4', 'svc-tile', 'map/service/tilesvc', '', '1', '0', 'C', '0', '0', 'map:service:list', 'tile', 103, 1, now(), null, null, 'PMTiles / MBTiles 瓦片包服务')
  on conflict (menu_id) do nothing;

-- 123 ArcGIS瓦片图层（arcgis 预切片缓存，网格/坐标系由 Conf.xml 自描述）
insert into sys_menu values('123', 'ArcGIS瓦片图层', '7', '5', 'svc-arcgis', 'map/service/arcgis', '', '1', '0', 'C', '0', '0', 'map:service:list', 'tile', 103, 1, now(), null, null, 'ArcGIS 预切片缓存服务（Conf.xml 自描述网格与坐标系）')
  on conflict (menu_id) do nothing;

-- 121 服务引擎管理
insert into sys_menu values('121', '服务引擎管理', '7', '9', 'engine', 'map/engine/index', '', '1', '0', 'C', '0', '0', 'map:engine:list', 'engine', 103, 1, now(), null, null, '多服务引擎注册表：登记/健康探测/默认引擎切换')
  on conflict (menu_id) do nothing;

-- 1059 服务引擎维护（F 按钮权限）
insert into sys_menu values('1059', '服务引擎维护', '121', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:engine:edit', '#', 103, 1, now(), null, null, '引擎新增/编辑/删除/设默认')
  on conflict (menu_id) do nothing;

-- ---------- C. 角色授权（角色 3）----------
insert into sys_role_menu values ('3', '118')  on conflict do nothing;
insert into sys_role_menu values ('3', '119')  on conflict do nothing;
insert into sys_role_menu values ('3', '120')  on conflict do nothing;
insert into sys_role_menu values ('3', '123')  on conflict do nothing;
insert into sys_role_menu values ('3', '121')  on conflict do nothing;
insert into sys_role_menu values ('3', '1059') on conflict do nothing;

-- =====================================================================
-- §3 服务与引擎的归属关系 sys_service_engine
-- ---------------------------------------------------------------------
--   * (kind, name) 唯一定位一个服务；engine_id 指向 sys_engine。
--   * 未登记归属的服务视为「默认引擎」（代理回落，兼容旧数据）。
--   * 服务级操作按归属路由；全局操作（reload/日志/缓存）仍走默认引擎。
-- =====================================================================

CREATE TABLE IF NOT EXISTS sys_service_engine (
    kind        VARCHAR(32)  NOT NULL,
    name        VARCHAR(128) NOT NULL,
    engine_id   VARCHAR(64)  NOT NULL,
    create_time TIMESTAMP    NOT NULL DEFAULT now(),
    update_time TIMESTAMP    DEFAULT NULL,
    PRIMARY KEY (kind, name)
);

COMMENT ON TABLE sys_service_engine IS '服务与引擎归属关系：未登记的服务按默认引擎处理';
COMMENT ON COLUMN sys_service_engine.kind IS '服务类型（cog/vector/pmtiles/mbtiles/postgis/3dtiles）';
COMMENT ON COLUMN sys_service_engine.name IS '服务名';
COMMENT ON COLUMN sys_service_engine.engine_id IS '归属引擎 ID（sys_engine.engine_id）';

-- =====================================================================
-- §4 引擎监控事件 sys_engine_event + 服务分组 sys_service_meta
-- ---------------------------------------------------------------------
-- 事件类型 event_type：
--   engine_online / engine_offline   引擎存活状态翻转（巡检探测）
--   service_added / service_removed  服务清单 diff：登记 / 消失
--   service_up / service_down        服务在服状态翻转（serving）
-- =====================================================================

-- ---------- A. 引擎事件日志（manager 后台巡检产生） ----------
-- 事件类型 event_type：
--   engine_online / engine_offline          引擎存活状态翻转（巡检探测）
--   service_added / service_removed         服务清单 diff：登记 / 消失
--   service_up / service_down               服务在服状态翻转（serving）
create table if not exists sys_engine_event (
  id          bigserial primary key,
  engine_id   varchar(64)  not null,
  event_type  varchar(32)  not null,
  summary     varchar(255) not null,
  detail      varchar(1000) default '',
  create_time timestamp    not null default now()
);

create index if not exists idx_engine_event_engine on sys_engine_event (engine_id, id desc);
create index if not exists idx_engine_event_type on sys_engine_event (event_type, id desc);

comment on table sys_engine_event is '引擎监控事件日志：manager 后台巡检（默认 30s）产生，引擎/服务状态变化留痕';

-- ---------- B. 服务分组（两级命名空间；空串 = 根目录） ----------
-- service_group 形如 '一级' 或 '一级/二级'；未登记 = 根目录。
create table if not exists sys_service_meta (
  kind          varchar(32)  not null,
  name          varchar(128) not null,
  service_group varchar(255) not null default '',
  create_time   timestamp    not null default now(),
  update_time   timestamp,
  primary key (kind, name)
);

comment on table sys_service_meta is '服务元数据：两级分组（命名空间），service_group 空 = 根目录';
comment on column sys_service_meta.service_group is '分组路径「一级/二级」，最多两级；空串 = 根目录';

-- =====================================================================
-- §5 组织层四表（Workspace / Store / Resource / 已发布服务）
-- ---------------------------------------------------------------------
-- 组织层：工作区 → 数据源 → 数据图层 → 已发布服务。
-- 纯元数据：不存数据、不存要素，只存指针与探测结果。
-- 菜单 122「数据目录总览」挂到「数据服务」(6) 下。
-- =====================================================================

-- ---------- A. 组织层四表 ----------

-- 工作区：命名空间容器（工作空间语义），默认种子 ws1。
create table if not exists sys_workspace (
  id          bigserial    primary key,
  name        varchar(64)  not null,
  title       varchar(128) not null default '',
  engine_id   varchar(64)  default '',
  data_root   varchar(512) default '',
  del_flag    smallint     not null default 0,
  remark      varchar(500) default '',
  create_time timestamp    not null default now(),
  update_time timestamp
);
-- 同名只允许一个「未删除」工作区（删除后可重建同名）。
create unique index if not exists uk_workspace_name on sys_workspace (name) where del_flag = 0;

comment on table  sys_workspace           is '工作区：地图服务命名空间容器，Store/已发布服务归属其下';
comment on column sys_workspace.name      is '工作区名（URL/树节点标识，ws 内唯一）';
comment on column sys_workspace.engine_id is '绑定的地图引擎 ID（sys_engine.engine_id，空=默认引擎）';
comment on column sys_workspace.data_root is '数据文件根路径（统一用 / 分隔，末尾带 /）';
comment on column sys_workspace.del_flag  is '0 正常 / 1 已删（软删除仅标记）';

insert into sys_workspace (name, title, remark)
values ('ws1', '默认工作区', '计划 §3 默认工作区种子：既有服务与未分workspace的数据都归它')
  on conflict do nothing;

-- 数据存储：纯指针（目录 或 PostGIS 连接），不导入任何数据。
create table if not exists sys_store (
  id           bigserial   primary key,
  workspace_id bigint      not null,
  name         varchar(128) not null,
  store_type   varchar(32)  not null,
  base_path    text,
  pg_dsn       text,
  pg_schema    varchar(64)  default 'public',
  del_flag     smallint     not null default 0,
  remark       varchar(500) default '',
  create_time  timestamp    not null default now(),
  update_time  timestamp
);
create unique index if not exists uk_store_ws_name on sys_store (workspace_id, name) where del_flag = 0;
create index if not exists idx_store_workspace on sys_store (workspace_id);

comment on table  sys_store            is '数据存储：文件目录或 PostGIS 连接的指针，发现时只读扫描，不落地数据';
comment on column sys_store.store_type is 'cog_dir|geojson_dir|shapefile_dir|pmtiles_dir|mbtiles_dir|3dtiles_dir|postgis';
comment on column sys_store.base_path  is '文件类 Store 的目录（map-api server.data_root 之内，路径守卫在服务端强制）';
comment on column sys_store.pg_dsn     is 'PostGIS 连接串：服务端原样保存并只用于转发 map-api；对外回显一律掩码口令';

-- 数据图层：只读发现得到的实体（Resource），携带探测到的 bbox / 源 CRS。
create table if not exists sys_resource (
  id          bigserial    primary key,
  store_id    bigint       not null,
  name        varchar(128) not null,
  data_type   varchar(32)  not null,
  source      text         not null,
  src_crs     varchar(32)  not null default 'EPSG:4326',
  bbox        double precision[],
  publishable smallint     not null default 1,
  reason      varchar(500) default '',
  meta        jsonb        not null default '{}'::jsonb,
  del_flag    smallint     not null default 0,
  create_time timestamp    not null default now(),
  update_time timestamp,
  crs_suspicious boolean    not null default false
);
-- 已执行过本脚本的库：补 meta / crs_suspicious 列（幂等）。
alter table sys_resource add column if not exists meta jsonb not null default '{}'::jsonb;
alter table sys_resource add column if not exists crs_suspicious boolean not null default false;
-- 资源名由「发现」按 store 内实体自动命名（非用户输入），因此唯一性必须全局：
-- 若用 where del_flag=0 的部分唯一索引，软删后再发现会插入同名的第二条，
-- 恢复那条时必然撞索引。故 (store_id, name) 一行一实体：发现只刷新元数据，不动 del_flag。
drop index if exists uk_resource_store_name;
create unique index if not exists uk_resource_store_name on sys_resource (store_id, name);
create index if not exists idx_resource_store on sys_resource (store_id);

comment on table  sys_resource             is '数据图层（Resource）：目录扫描 / geometry_columns 发现得到，纯元数据';
comment on column sys_resource.data_type   is 'cog|geojson|pmtiles|pmtiles_mvt|mbtiles|mbtiles_mvt|3dtiles|postgis_table|shapefile';
comment on column sys_resource.source      is '数据实体定位：文件绝对路径 或 schema.table';
comment on column sys_resource.bbox        is '只读探测得到的四至 [minx,miny,maxx,maxy]（WGS84 或源 CRS）';
comment on column sys_resource.publishable is '0 = 当前引擎不能发布（如 shapefile：无读取器，计划标『将来支持』）';
comment on column sys_resource.reason      is '不可发布 / 探测降级的人类可读原因';
comment on column sys_resource.meta        is '发现结果原样快照：PostGIS 的 geometry_column/srid、归档缩放区间、3D Tiles 入口等，发布时据此重建 source';
comment on column sys_resource.crs_suspicious is '坐标系可疑：声明 CRS 与探测到的 bbox 范围不符，发布前需人工确认';

-- 已发布服务：发布动作的产物账本；实体配置仍落在 rust-gisserver 的 XML 里。
create table if not exists sys_published_layer (
  id             bigserial    primary key,
  workspace_id   bigint       not null,
  resource_id    bigint,
  layer_name     varchar(128) not null,
  title          varchar(128) not null default '',
  src_crs        varchar(32)  not null default 'EPSG:4326',
  bbox           double precision[],
  service_types  varchar(128) not null default '',
  mapserver_kind varchar(32)  not null,
  state          varchar(16)  not null default 'active',
  del_flag       smallint     not null default 0,
  remark         varchar(500) default '',
  create_time    timestamp    not null default now(),
  update_time    timestamp,
  service_doc    jsonb,
  grid           jsonb
);
create unique index if not exists uk_published_layer_name on sys_published_layer (layer_name) where del_flag = 0;
create index if not exists idx_published_workspace on sys_published_layer (workspace_id);

-- 向后兼容：已部署库补列（幂等，可重复执行）
alter table sys_published_layer add column if not exists service_doc jsonb;
alter table sys_published_layer add column if not exists grid jsonb;

comment on table  sys_published_layer               is '已发布服务账本：与 rust-gisserver sys_service 登记簿同源，state 四态一致';
comment on column sys_published_layer.layer_name    is 'rust-gisserver 扁平唯一服务名（= Layer name / TilesetService Name）';
comment on column sys_published_layer.service_types is '发布勾选的协议，逗号分隔：wms,wmts,xyz,wfs,mvt,oaf,3dtiles';
comment on column sys_published_layer.mapserver_kind is 'cog|vector|pmtiles|mbtiles|postgis|3dtiles（决定写哪类 XML 文档）';
comment on column sys_published_layer.state         is 'active|stopped|paused|deleted（复用 rust-gisserver 登记簿四态）';
comment on column sys_published_layer.service_doc  is '完整服务定义（Layer JSON，含 custom_grids）：rust-gisserver backend=db 模式下的唯一真理源，manager 为唯一写入方';
comment on column sys_published_layer.grid         is '发布时固化的原生网格快照（ArcGIS：CRS/瓦片尺寸/原点/LOD 阶梯），运行时仍以缓存 Conf.xml 为准';

-- ---------- B. 菜单 ----------
-- 122 数据目录（C）：ws → store → resource 树 + 列表 + 发布向导。
insert into sys_menu values('122', '数据目录', '6', '1', 'catalog', 'map/catalog/index', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'tree-table', 103, 1, now(), null, null, '工作区/数据存储/数据图层组织与发布（只读发现，不导入数据）')
  on conflict (menu_id) do nothing;

-- 1060 目录维护（F）：工作区、Store 的增删改与 PostGIS 连接测试。
insert into sys_menu values('1060', '目录维护', '122', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:edit', '#', 103, 1, now(), null, null, '工作区/Store 新建、改名、删除与连接测试')
  on conflict (menu_id) do nothing;

-- 1061 数据发布（F）：发现 → 发布、修改发布信息、资源软删除与垃圾箱恢复。
insert into sys_menu values('1061', '数据发布', '122', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, '扫描发现数据图层、发布为地图服务、修改发布信息')
  on conflict (menu_id) do nothing;

-- ---------- C. 角色授权（角色 3「本部门及以下」；超管不走 role_menu）----------
insert into sys_role_menu values ('3', '122')  on conflict do nothing;
insert into sys_role_menu values ('3', '1060') on conflict do nothing;
insert into sys_role_menu values ('3', '1061') on conflict do nothing;

-- =====================================================================
-- §6 数据服务板块菜单（顶级目录 6）
-- ---------------------------------------------------------------------
-- Data 组拆分为 Workspaces / Stores / Layer previews 等独立入口，
-- 管理端照此拆：工作空间 / 数据源 / 数据图层 / 已发布服务各占一个菜单页。
--   124 工作空间 / 125 数据源 / 126 数据图层 / 127 已发布服务（C）
--   1062..1066 按钮（F）
--   122 数据目录总览（挂到 6 下，只留目录树与入口，不再承载 CRUD）
-- =====================================================================

-- ---------- A. 顶级目录 ----------
-- 顶级目录 order_num=2，「数据服务」排在「地图服务」(7) 之前：先管数据源，再管已发布服务。
insert into sys_menu values('6', '数据服务', '0', '2', 'data', null, '', '1', '0', 'M', '0', '0', '', 'data', 103, 1, now(), null, null, '工作区/数据源/数据图层组织与发布（只读发现，不导入数据）')
  on conflict (menu_id) do nothing;

-- ---------- B. 功能菜单（C）----------
-- 124 工作空间：命名空间容器（sys_workspace），对应「工作空间」。
insert into sys_menu values('124', '工作空间', '6', '1', 'workspace', 'map/catalog/workspace', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'workspace', 103, 1, now(), null, null, '工作空间清单与增删改（含其下数据源/在线服务计数）')
  on conflict (menu_id) do nothing;

-- 125 数据源：目录型或 PostGIS 型连接指针（sys_store），对应「数据源」。
insert into sys_menu values('125', '数据源', '6', '2', 'store', 'map/catalog/store', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'store', 103, 1, now(), null, null, '数据源（目录 / PostGIS 连接）新建、连接测试、只读发现与垃圾箱恢复')
  on conflict (menu_id) do nothing;

-- 126 数据图层：只读发现得到的资源（sys_resource）→ 发布为服务，对应「图层 / 要素类型」。
insert into sys_menu values('126', '数据图层', '6', '3', 'resource', 'map/catalog/resource', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'layers', 103, 1, now(), null, null, '发现出的数据图层清单与发布向导（服务协议勾选口径来自引擎能力矩阵）')
  on conflict (menu_id) do nothing;

-- 127 已发布服务：发布账本（sys_published_layer）生命周期与端点预览，对应「图层预览」。
insert into sys_menu values('127', '已发布服务', '6', '4', 'published', 'map/catalog/published', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'published', 103, 1, now(), null, null, '按 服务协议 × 数据类型 分组的在服清单：启停 / 暂停 / 删除与端点预览')
  on conflict (menu_id) do nothing;

-- ---------- C. 按钮（F）----------
insert into sys_menu values('1062', '工作空间维护', '124', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:edit', '#', 103, 1, now(), null, null, '工作空间新建、改名、删除')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1063', '数据源维护', '125', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:edit', '#', 103, 1, now(), null, null, '数据源新建、编辑、删除、连接测试与指针恢复')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1064', '只读发现', '126', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, '扫描数据源刷新数据图层清单（只读元数据，不落地数据）')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1065', '图层发布', '126', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, '发布为地图服务、修改发布信息、资源软删除与恢复')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1066', '服务生命周期', '127', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, '启动 / 停止 / 暂停 / 删除（进垃圾箱）与恢复')
  on conflict (menu_id) do nothing;

-- ---------- D. 旧的 122 数据目录改为总览页并挂到「数据服务」下 ----------
-- CRUD 已拆到 124..127：这里只留 ws → store 目录树与各功能页入口，避免两处维护同一份表单。
update sys_menu
   set parent_id = '6',
       order_num = 0,
       menu_name = '数据目录总览',
       remark    = '工作区→数据源目录树与计数概览；具体维护见工作空间 / 数据源 / 数据图层 / 已发布服务'
 where menu_id = '122';
-- 122 原有两个按钮（08 挂在 122 下）改挂到对应功能菜单，权限码不变（老库里角色已授过这两个 id）。
update sys_menu set parent_id = '124', order_num = 2 where menu_id = '1060';
update sys_menu set parent_id = '126', order_num = 3 where menu_id = '1061';


-- ---------- F. 角色授权（角色 3「本部门及以下」）----------
insert into sys_role_menu values ('3', '6')  on conflict do nothing;
insert into sys_role_menu values ('3', '124')  on conflict do nothing;
insert into sys_role_menu values ('3', '125')  on conflict do nothing;
insert into sys_role_menu values ('3', '126')  on conflict do nothing;
insert into sys_role_menu values ('3', '127')  on conflict do nothing;
insert into sys_role_menu values ('3', '1062') on conflict do nothing;
insert into sys_role_menu values ('3', '1063') on conflict do nothing;
insert into sys_role_menu values ('3', '1064') on conflict do nothing;
insert into sys_role_menu values ('3', '1065') on conflict do nothing;
insert into sys_role_menu values ('3', '1066') on conflict do nothing;

-- =====================================================================
-- §6.5 API Key 管理（rust-gisserver 访问凭证）
-- ---------------------------------------------------------------------
-- 一张表同时服务两种用途（purpose），与 rust-gisserver 现有两套令牌护栏对齐：
--   admin  manager → server 管理面（请求头 X-Admin-Token）
--   data   server → 客户端数据面（X-Data-Token / Authorization: Bearer / ?token=）
-- 明文存储便于管理端复制分发；库部署在内网，访问受登录态 + map:apikey:* 约束。
-- rust-gisserver 从同一共享库定时读取启用中的 key 作为「附加认可凭证」，
-- 主令牌（mapserver.xml 的 admin.token / server.data_token）仍是启用校验的开关。
-- =====================================================================

create table if not exists sys_api_key (
    key_id      varchar(64)  primary key,
    key_name    varchar(100) not null unique,
    api_key     varchar(200) not null unique,
    purpose     varchar(16)  not null default 'data',
    status      char(1)      not null default '0',
    expires_at  timestamp,
    remark      varchar(500) default '',
    create_by   varchar(64)  default '',
    create_time timestamp    not null default now(),
    update_time timestamp
);

comment on table  sys_api_key            is 'rust-gisserver 访问密钥：管理面(admin) / 客户端数据面(data)';
comment on column sys_api_key.purpose    is '用途：admin=manager 连接 server（X-Admin-Token）；data=客户端访问数据面';
comment on column sys_api_key.status     is '状态：0 启用 / 1 停用';
comment on column sys_api_key.expires_at is '过期时间（空 = 永不过期）；停用或过期的 key 不参与 server 校验';

create index if not exists idx_sys_api_key_purpose_status on sys_api_key (purpose, status);

-- ---------- A. 菜单（地图服务目录 7 下）----------
insert into sys_menu values('128', 'API Key 管理', '7', '10', 'apikeys', 'map/apikey/index', '', '1', '0', 'C', '0', '0',
  'map:apikey:list', 'key', 103, 1, now(), null, null,
  '为 rust-gisserver 生成访问密钥（名称 + Key），供 manager 连接 server 或下发给客户端')
  on conflict (menu_id) do nothing;

-- ---------- B. 按钮权限 ----------
insert into sys_menu values('1067', 'API Key 维护', '128', '1', '', '', '', '1', '0', 'F', '0', '0',
  'map:apikey:edit', '#', 103, 1, now(), null, null,
  '生成 / 重置 / 启停 / 编辑 / 删除 / 应用到默认引擎')
  on conflict (menu_id) do nothing;

-- ---------- C. 角色授权（角色 3「本部门及以下」；超管不走 role_menu）----------
insert into sys_role_menu values ('3', '128')  on conflict do nothing;
insert into sys_role_menu values ('3', '1067') on conflict do nothing;

-- =====================================================================
-- §7 顶级目录排序归一
-- ---------------------------------------------------------------------
-- 目标顺序：1 系统管理 / 6 数据服务 / 7 地图服务 / 2 系统监控 /
-- 与 system.sql §5 同一口径：map.sql 单独执行时也能把顶级目录排到位。
-- 按 menu_id 定位（parent_id = 0 的顶级行），重复执行只写回同一值。
-- =====================================================================
update sys_menu set order_num = 1 where menu_id = '1' and parent_id = 0; -- 系统管理
update sys_menu set order_num = 2 where menu_id = '6' and parent_id = 0; -- 数据服务
update sys_menu set order_num = 3 where menu_id = '7' and parent_id = 0; -- 地图服务
update sys_menu set order_num = 4 where menu_id = '2' and parent_id = 0; -- 系统监控
update sys_menu set order_num = 5 where menu_id = '3' and parent_id = 0; -- 系统工具
update sys_menu set order_num = 7 where menu_id = '5' and parent_id = 0; -- 测试菜单

-- ---------------------------------------------------------------------
-- 增量迁移：给 sys_workspace 加 data_root / engine_id 列（已存在则跳过）
-- =====================================================================
alter table sys_workspace add column if not exists data_root varchar(512) default '';
comment on column sys_workspace.data_root is '数据文件根路径（统一用 / 分隔，末尾带 /）';
alter table sys_workspace add column if not exists engine_id varchar(64) default '';
comment on column sys_workspace.engine_id is '绑定的地图引擎 ID（sys_engine.engine_id，空=默认引擎）';

-- =====================================================================
-- §8 SLD 样式库 sys_sld
-- ---------------------------------------------------------------------
-- 数据库存储为主，map-api styles 目录为文件系统副本（manager 写入时双写）。
-- 字段：sld_content 存完整 XML，style_name / layer_name / rule_count 为解析冗余，
-- 便于列表显示；sld_name 不含 .sld 后缀，sld_file 含后缀。
-- 菜单：已在 §1 预置 menu_id=112 样式库 + 按钮 1056 样式上传 / 1057 样式删除。
--
-- 三个解析冗余列的必填口径：SLD/SE 规范里 <UserStyle><Name> 与
-- <NamedLayer><Name> 都是可选元素（gisserver::renderer::sld::SldDocument 中
-- 均为 Option<String>）；map-api 上传校验只要求根为 StyledLayerDescriptor
-- 且至少一个 FeatureTypeStyle，不校验 Name；Rule 列表允许为空，故规则数可
-- 以为 0。所以三者业务上都不强制填写，一律用 '' / 0 表示「SLD 未写」。
-- 但存储层必须 NOT NULL：manager 的 style_list / style_content 用
-- r.get::<String>() / r.get::<i32>() 直接取值，遇到 NULL 会 panic。
-- =====================================================================

create table if not exists sys_sld (
  sld_id      bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  sld_name    varchar(100)  NOT NULL,
  sld_file    varchar(120)  NOT NULL,
  sld_content text          NOT NULL,
  style_name  varchar(100)  NOT NULL DEFAULT '',
  layer_name  varchar(100)  NOT NULL DEFAULT '',
  rule_count  integer       NOT NULL DEFAULT 0,
  remark      varchar(500)  DEFAULT '',
  create_by   bigint,
  create_time timestamp NOT NULL DEFAULT now(),
  update_by   bigint,
  update_time timestamp,
  CONSTRAINT uk_sys_sld_name UNIQUE (sld_name),
  CONSTRAINT uk_sys_sld_file UNIQUE (sld_file)
);

-- 兼容已建表的部署：create table if not exists 不会收紧已有列，先清理历史
-- NULL 再补约束。全幂等，可随全量脚本重复执行。
update sys_sld set style_name = '' where style_name is null;
update sys_sld set layer_name = '' where layer_name is null;
update sys_sld set rule_count = 0 where rule_count is null;
alter table sys_sld alter column style_name set not null;
alter table sys_sld alter column layer_name set not null;
alter table sys_sld alter column rule_count set not null;
alter table sys_sld alter column style_name set default '';
alter table sys_sld alter column layer_name set default '';
alter table sys_sld alter column rule_count set default 0;

create index if not exists idx_sys_sld_update_time on sys_sld (update_time desc);

comment on table  sys_sld             is 'SLD 样式库：数据库主存储 + map-api styles 目录双写';
comment on column sys_sld.sld_id     is '自增主键';
comment on column sys_sld.sld_name   is '样式名（不含 .sld 扩展名，全局唯一，前端/XML 引用用这个）';
comment on column sys_sld.sld_file   is '文件名（含 .sld 扩展名）';
comment on column sys_sld.sld_content is 'SLD XML 原文';
comment on column sys_sld.style_name is '解析冗余：UserStyle Name（SLD 未写则空串，业务非必填，存储 NOT NULL）';
comment on column sys_sld.layer_name is '解析冗余：NamedLayer Name（SLD 未写则空串，业务非必填，存储 NOT NULL）';
comment on column sys_sld.rule_count is '解析冗余：规则数（解析器统计，可为 0，非用户填写，存储 NOT NULL）';

-- =====================================================================
-- §9 MVT 前端样式 JSON（sys_published_layer）
-- ---------------------------------------------------------------------
-- MVT 是 protobuf 要素流，样式由前端 OpenLayers 渲染；manager 的
-- /map/services/{kind}/{name}/mvt-style 接口读写此列，map-api 不消费。
-- 空值走 viewer_mvt.html 内置默认样式。
-- =====================================================================
alter table sys_published_layer add column if not exists mvt_style_json jsonb;
comment on column sys_published_layer.mvt_style_json is 'MVT 矢量瓦片前端样式 JSON（OpenLayers Style JSON，manager 预览页读写）';
