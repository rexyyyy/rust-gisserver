-- =====================================================================
-- rust-gisserver - map.sql - Map & Data Services modules (DDL + menus)
-- ---------------------------------------------------------------------
-- Purpose: rust-gisserver manager map business baseline with two top-level directories:
--       (1) Data Services (top dir 6): Workspace / Data Source / Data Layer / Published Service
--          Data Group semantics - source organization + publish ledger only;
--       (2) Map Services (top dir 7): Service inventory / Access log / Style library / Service trash /
--          Operations dashboard / Cache management / Runtime config / 3D Tiles / Raster / Vector / Tile /
--          Engine management - ops surface for published services.
--       "Data Services" before "Map Services": sources first, published services second.
-- Depends: system.sql must run first (creates sys_menu, sys_role_menu, etc.)
-- Usage: psql -h localhost -p 5432 -U postgres -d mapserver-manager -f map.sql
-- ---------------------------------------------------------------------
-- Contents:
--   §0  Normalize top-level menu IDs (legacy migration; same block as system.sql §0, re-runnable)
--   §1  Map Services module menus (top dir 7)
--   §2  Service engine registry sys_engine + engine menus
--   §3  Service-to-engine ownership sys_service_engine
--   §4  Engine monitor events sys_engine_event + service groups sys_service_meta
--   §5  Org layer tables sys_workspace / sys_store / sys_resource / sys_published_layer
--   §6   Data Services module menus (top dir 6)
--   §6.5 API Key management (sys_api_key + menu 128 / button 1067)
-- ---------------------------------------------------------------------
-- Idempotent: CREATE TABLE IF NOT EXISTS; menu & grant inserts use ON CONFLICT DO
--         NOTHING; §0 migration uses guards - re-runs are side-effect free.
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
-- §1 Map Services module menus (top dir 7)
-- ---------------------------------------------------------------------
-- Module structure:
--   7    Map Services (directory M)
--   110  Service Overview (C, map/service/index, map:service:list)
--   111  Access Log (C, map/log/index, map:log:list)
--   112  Style Library (C, map/style/index, map:style:list)
--   113  Service Trash (C, map/trash/index, map:service:list)
--   114  Operations Dashboard (C, map/dashboard/index, map:service:list)
--   115  Cache Management (C, map/cache/index, map:service:list)
--   116  3D Tiles Management (C, map/tiles/index, map:service:list)
--   117  Runtime Config (C, map/settings/index, map:service:list)
--   118  Raster Service / 119 Vector Service / 120 Tile Service / 121 Engine Management
--   1051..1059 buttons (F)
-- =====================================================================

insert into sys_menu values('7', 'Map Services', '0', '3', 'map',      null,                    '', '1', '0', 'M', '0', '0', '',                   'map',     103, 1, now(), null, null, 'Map Services')
  on conflict (menu_id) do nothing;

insert into sys_menu values('110', 'Service Management', '7', '1', 'service',  'map/service/index',     '', '1', '0', 'C', '0', '0', 'map:service:list',   'service', 103, 1, now(), null, null, 'Map service inventory and lifecycle menu')
  on conflict (menu_id) do nothing;

insert into sys_menu values('1051', 'Change Service Status', '110', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:service:state',  '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1052', 'Service Reload', '110', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:service:reload', '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1053', 'Access statistics query', '110', '3', '', '', '', '1', '0', 'F', '0', '0', 'map:usage:query',   '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;

-- Role grants (role 3 "This dept and below"; superadmin bypasses role_menu)
insert into sys_role_menu values ('3', '7')    on conflict do nothing;
insert into sys_role_menu values ('3', '110')  on conflict do nothing;
insert into sys_role_menu values ('3', '1051') on conflict do nothing;
insert into sys_role_menu values ('3', '1052') on conflict do nothing;
insert into sys_role_menu values ('3', '1053') on conflict do nothing;

-- (2) New: Access Log / Style Library (2026-10-27 P0/P1 features)
insert into sys_menu values('111', 'Access Log', '7', '2', 'log',      'map/log/index',         '', '1', '0', 'C', '0', '0', 'map:log:list',       'log-map', 103, 1, now(), null, null, 'Service access log and audit query')
  on conflict (menu_id) do nothing;
insert into sys_menu values('112', 'Style Library', '7', '3', 'style',    'map/style/index',       '', '1', '0', 'C', '0', '0', 'map:style:list',     'brush',   103, 1, now(), null, null, 'SLD style library management ( SLD style guide )')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1054', 'Service Health', '110', '4', '', '', '', '1', '0', 'F', '0', '0', 'map:service:check',  '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1056', 'Upload Style', '112', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:style:upload',   '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1057', 'Delete Style', '112', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:style:remove',   '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;

-- (2) New menu grants (role 3)
insert into sys_role_menu values ('3', '111')  on conflict do nothing;
insert into sys_role_menu values ('3', '112')  on conflict do nothing;
insert into sys_role_menu values ('3', '1054') on conflict do nothing;
insert into sys_role_menu values ('3', '1056') on conflict do nothing;
insert into sys_role_menu values ('3', '1057') on conflict do nothing;

-- (3) New: Service Trash (2026-10-28 service detail + trash)
insert into sys_menu values('113', 'Service Trash', '7', '4', 'trash', 'map/trash/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'delete', 103, 1, now(), null, null, 'removed Service Trash: Restore after takes effect immediately')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '113') on conflict do nothing;

-- (4) New: Operations Dashboard (2026-10-28 home dashboard, reuses map:service:list perms)
insert into sys_menu values('114', 'Operations Dashboard', '7', '0', 'dashboard', 'map/dashboard/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'dashboard', 103, 1, now(), null, null, 'Map service dashboard: statistics / component probes / access counts')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '114') on conflict do nothing;

-- (5) New: Cache Management (2026-10-28 P2 cache UI; view/purge reuse map:service:list)
insert into sys_menu values('115', 'Cache Management', '7', '6', 'cache', 'map/cache/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'cache', 103, 1, now(), null, null, 'map-api Tile/Metadata Cache: live counters and purge by range')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '115') on conflict do nothing;

--   115   Cache Management (C, component: map/cache/index, perms: map:service:list)
-- (4) Dedicated 3D Tiles management (2026-10-29)
insert into sys_menu values('116', '3D Tiles', '7', '5', 'tiles', 'map/tiles/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'box-3d', 103, 1, now(), null, null, '3D Tiles Service Management ( List/ Status/ One-click Cesium preview )')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '116') on conflict do nothing;

-- (5) Config / Settings page (2026-10-29)
insert into sys_menu values('117', 'Runtime Config', '7', '6', 'settings', 'map/settings/index', '', '1', '0', 'C', '0', '0', 'map:service:list', 'config', 103, 1, now(), null, null, 'Read-only view of manager / map-api runtime config')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '117') on conflict do nothing;

-- (6) Service config edit (2026-10-29 P0 edit dialog)
--    Button permission hangs under Service Management (110), perms: map:service:edit
--    Backend route: PUT /map/services/:kind/:name (rewrites XML config + hot reload)
insert into sys_menu values('1058', 'Service Config Edit', '110', '5', '', '', '', '1', '0', 'F', '0', '0', 'map:service:edit', '#', 103, 1, now(), null, null, '')
  on conflict (menu_id) do nothing;
insert into sys_role_menu values ('3', '1058') on conflict do nothing;

-- =====================================================================
-- §2 Service engine registry sys_engine + engine menus
-- ---------------------------------------------------------------------
--   118 Raster (COG) / 119 Vector (vector+postgis) /
--   120 Tile (pmtiles+mbtiles) / 123 ArcGIS Tile Layer (arcgis) /
--   121 Engine Management
--   1059 Engine Maintenance (F, map:engine:edit)
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

comment on table sys_engine is 'Engine registry: admin proxy forwards to the default engine; health probe uses base_url';
comment on column sys_engine.engine_type is 'Engine type (currently rust-gisserver; extensible)';
comment on column sys_engine.is_default is 'Default engine: admin proxy /map/* forward target; falls back to manager.xml mapsvc when unset';

-- Seed: local mapserve (idempotent; base_url does not overwrite existing env config)
insert into sys_engine (engine_id, engine_name, engine_type, base_url, status, is_default, sort, remark)
values ('mapserve', 'rust-gisserver mapserve', 'rust-gisserver', 'http://map-api:8080', '0', '1', 0,
        'Local rust-gisserver service plane (seed value; change base_url per environment)')
on conflict (engine_id) do nothing;

-- ---------- B. Menus ----------
-- 110 Rename: "All-type Inventory" → "Service Overview" (idempotent)
update sys_menu set menu_name = 'Service Overview', remark = 'Inventory and lifecycle management of all map service types'
 where menu_id = '110';

-- 118 Raster Service (COG): reuses map/service/index + wrapper presetKinds
insert into sys_menu values('118', 'Raster Service', '7', '2', 'svc-cog', 'map/service/cog', '', '1', '0', 'C', '0', '0', 'map:service:list', 'picture', 103, 1, now(), null, null, 'COG raster imagery service')
  on conflict (menu_id) do nothing;

-- 119 Vector Service (vector + postgis)
insert into sys_menu values('119', 'Vector Service', '7', '3', 'svc-vector', 'map/service/vector', '', '1', '0', 'C', '0', '0', 'map:service:list', 'vector', 103, 1, now(), null, null, 'Vector files and PostGIS services')
  on conflict (menu_id) do nothing;

-- 120 Tile Service (pmtiles + mbtiles)
insert into sys_menu values('120', 'Tile Service', '7', '4', 'svc-tile', 'map/service/tilesvc', '', '1', '0', 'C', '0', '0', 'map:service:list', 'tile', 103, 1, now(), null, null, 'PMTiles / MBTiles tile package service')
  on conflict (menu_id) do nothing;

-- 123 ArcGIS Tile Layer (arcgis pre-tiled cache; grid/CRS self-described by Conf.xml)
insert into sys_menu values('123', 'ArcGIS Tile Layer', '7', '5', 'svc-arcgis', 'map/service/arcgis', '', '1', '0', 'C', '0', '0', 'map:service:list', 'tile', 103, 1, now(), null, null, 'ArcGIS pre-tiled cache service ( Conf.xml self-describes the grid and CRS )')
  on conflict (menu_id) do nothing;

-- 121 Engine Management
insert into sys_menu values('121', 'Engine Management', '7', '9', 'engine', 'map/engine/index', '', '1', '0', 'C', '0', '0', 'map:engine:list', 'engine', 103, 1, now(), null, null, 'Engine registry: registration / health probe / default switch')
  on conflict (menu_id) do nothing;

-- 1059 Engine Maintenance (F button permission)
insert into sys_menu values('1059', 'Engine Maintenance', '121', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:engine:edit', '#', 103, 1, now(), null, null, 'Engine: add / edit / delete / set default')
  on conflict (menu_id) do nothing;

-- ---------- C. Role grants (role 3) ----------
insert into sys_role_menu values ('3', '118')  on conflict do nothing;
insert into sys_role_menu values ('3', '119')  on conflict do nothing;
insert into sys_role_menu values ('3', '120')  on conflict do nothing;
insert into sys_role_menu values ('3', '123')  on conflict do nothing;
insert into sys_role_menu values ('3', '121')  on conflict do nothing;
insert into sys_role_menu values ('3', '1059') on conflict do nothing;

-- =====================================================================
-- §3 Service-to-engine ownership sys_service_engine
-- ---------------------------------------------------------------------
--   * (kind, name) uniquely identifies a service; engine_id → sys_engine.
--   * Services without ownership → "default engine" (proxy fallback).
--   * Service-level ops route by ownership; global ops (reload/log/cache) use default engine.
-- =====================================================================

CREATE TABLE IF NOT EXISTS sys_service_engine (
    kind        VARCHAR(32)  NOT NULL,
    name        VARCHAR(128) NOT NULL,
    engine_id   VARCHAR(64)  NOT NULL,
    create_time TIMESTAMP    NOT NULL DEFAULT now(),
    update_time TIMESTAMP    DEFAULT NULL,
    PRIMARY KEY (kind, name)
);

COMMENT ON TABLE sys_service_engine IS 'Service-engine ownership: unregistered services use the default engine';
COMMENT ON COLUMN sys_service_engine.kind IS 'Service Type (cog/vector/pmtiles/mbtiles/postgis/3dtiles )';
COMMENT ON COLUMN sys_service_engine.name IS 'Service Name';
COMMENT ON COLUMN sys_service_engine.engine_id IS 'Owning engine ID (sys_engine.engine_id )';

-- =====================================================================
-- §4 Engine monitor events sys_engine_event + service groups sys_service_meta
-- ---------------------------------------------------------------------
-- Event types (event_type):
--   engine_online / engine_offline   engine liveness flip (health probe)
--   service_added / service_removed  service inventory diff: added / removed
--   service_up / service_down        service serving-status flip
-- =====================================================================

-- ---------- A. Engine event log (produced by manager background probe) ----------
-- Event types (event_type):
--   engine_online / engine_offline          engine liveness flip (health probe)
--   service_added / service_removed         service inventory diff: added / removed
--   service_up / service_down               service serving-status flip
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

comment on table sys_engine_event is 'Engine event log: produced by background polling (default 30s); records engine/service state changes';

-- ---------- B. Service groups (2-level namespace; empty = root) ----------
-- service_group looks like 'Level 1' or 'Level 1/Level 2'; unset = root.
create table if not exists sys_service_meta (
  kind          varchar(32)  not null,
  name          varchar(128) not null,
  service_group varchar(255) not null default '',
  create_time   timestamp    not null default now(),
  update_time   timestamp,
  primary key (kind, name)
);

comment on table sys_service_meta is 'Service metadata: two-level grouping (namespace); empty service_group means root';
comment on column sys_service_meta.service_group is 'Group Path" Level 1/ Level 2", at most two levels; empty string = root';

-- =====================================================================
-- §5 Organization layer (Workspace / Store / Resource / Published Service)
-- ---------------------------------------------------------------------
-- Org layer: Workspace → Data Source → Data Layer → Published Service.
-- Metadata only: no data/features - just pointers + probe results.
-- Menu 122 "Data Catalog Overview" hangs under "Data Services" (6).
-- =====================================================================

-- ---------- A. Org layer tables ----------

-- Workspace: namespace container (workspace semantics). Default seed: ws1.
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
-- Only one non-deleted workspace per name allowed (can recreate same name after delete).
create unique index if not exists uk_workspace_name on sys_workspace (name) where del_flag = 0;

comment on table  sys_workspace           is 'Workspace: namespace container for map services; stores and published services belong to it';
comment on column sys_workspace.name      is 'Workspace name (URL/tree id, unique within ws)';
comment on column sys_workspace.engine_id is 'Bound engine ID (sys_engine.engine_id; empty = default engine)';
comment on column sys_workspace.data_root is 'Data file root path (use / separators, trailing / required)';
comment on column sys_workspace.del_flag  is '0 normal / 1 deleted (soft delete flag only)';

insert into sys_workspace (name, title, remark)
values ('ws1', 'default workspace', 'System default workspace: existing services and unassigned data belong to it')
  on conflict do nothing;

-- Data store: pointer only (directory or PostGIS connection). No data imported.
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

comment on table  sys_store            is 'Data store: pointer to a file directory or PostGIS connection; read-only scan at discovery, no data persisted';
comment on column sys_store.store_type is 'cog_dir|geojson_dir|shapefile_dir|pmtiles_dir|mbtiles_dir|3dtiles_dir|postgis';
comment on column sys_store.base_path  is 'Directory of file-based stores (inside map-api server.data_root; path guard enforced server-side)';
comment on column sys_store.pg_dsn     is 'PostGIS connection string: stored verbatim on the server and used only to forward to map-api; passwords are always masked in responses';

-- Data layer: read-only discovered entity (Resource) with probed bbox / source CRS.
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
-- For DBs that already ran this script: add meta / crs_suspicious columns (idempotent).
alter table sys_resource add column if not exists meta jsonb not null default '{}'::jsonb;
alter table sys_resource add column if not exists crs_suspicious boolean not null default false;
-- Resource names are auto-named by discovery from store entities (not user input), so uniqueness must be global:
-- A partial unique index on del_flag=0 would let re-discovery insert a second row after soft-delete,
-- causing an index collision on restore. So (store_id, name) uniquely identifies: discovery only refreshes metadata, never touches del_flag.
drop index if exists uk_resource_store_name;
create unique index if not exists uk_resource_store_name on sys_resource (store_id, name);
create index if not exists idx_resource_store on sys_resource (store_id);

comment on table  sys_resource             is 'Data layer (Resource): discovered by directory scan / geometry_columns; metadata only';
comment on column sys_resource.data_type   is 'cog|geojson|pmtiles|pmtiles_mvt|mbtiles|mbtiles_mvt|3dtiles|postgis_table|shapefile';
comment on column sys_resource.source      is 'Data entity location: absolute file path or schema.table';
comment on column sys_resource.bbox        is 'Extent discovered read-only [minx,miny,maxx,maxy] (WGS84 or source CRS)';
comment on column sys_resource.publishable is '0 = current engine cannot publish (e.g. shapefile: no reader, planned support)';
comment on column sys_resource.reason      is 'Human-readable reason for not-publishable / probe downgrade';
comment on column sys_resource.meta        is 'Verbatim snapshot of discovery results: PostGIS geometry_column/srid, archive zoom range, 3D Tiles entry, etc., source is rebuilt from it at publish time';
comment on column sys_resource.crs_suspicious is 'Suspicious CRS: declared CRS does not match the discovered bbox; confirm manually before publishing';

-- Published service: ledger of publish actions; entity config lives in rust-gisserver XML.
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

-- Backward compat: add columns to deployed DBs (idempotent, re-runnable)
alter table sys_published_layer add column if not exists service_doc jsonb;
alter table sys_published_layer add column if not exists grid jsonb;

comment on table  sys_published_layer               is 'Published service ledger: same source as rust-gisserver sys_service registry, identical four states';
comment on column sys_published_layer.layer_name    is 'rust-gisserver flat unique service name (= Layer name / TilesetService Name )';
comment on column sys_published_layer.service_types is 'Comma-separated protocols selected at publish: wms,wmts,xyz,wfs,mvt,oaf,3dtiles';
comment on column sys_published_layer.mapserver_kind is 'cog|vector|pmtiles|mbtiles|postgis|3dtiles (selects which XML document to write)';
comment on column sys_published_layer.state         is 'active|stopped|paused|deleted (rust-gisserver registry states)';
comment on column sys_published_layer.service_doc  is 'Complete service definition ( Layer JSON with custom_grids ): single source of truth in rust-gisserver backend=db mode, manager is the only writer';
comment on column sys_published_layer.grid         is 'Native grid snapshot frozen at publish time ( ArcGIS: CRS / tile size / origin / LOD ladder ), runtime still uses the cache Conf.xml';

-- ---------- B. Menus ----------
-- 122 Data Catalog (C): ws → store → resource tree + list + publish wizard.
insert into sys_menu values('122', 'Data Catalog', '6', '1', 'catalog', 'map/catalog/index', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'tree-table', 103, 1, now(), null, null, 'Workspace / data store / data layer organization and publishing (read-only discovery, no data import)')
  on conflict (menu_id) do nothing;

-- 1060 Catalog Maintenance (F): Workspace/Store CRUD + PostGIS connection test.
insert into sys_menu values('1060', 'Catalog Maintenance', '122', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:edit', '#', 103, 1, now(), null, null, 'Workspace/Store: create, rename, delete and connection test')
  on conflict (menu_id) do nothing;

-- 1061 Data Publishing (F): discover → publish, edit publish info, soft-delete + trash restore.
insert into sys_menu values('1061', 'Data Publishing', '122', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, 'Discover data layers, publish map services, edit publish info')
  on conflict (menu_id) do nothing;

-- ---------- C. Role grants (role 3; superadmin bypasses role_menu) ----------
insert into sys_role_menu values ('3', '122')  on conflict do nothing;
insert into sys_role_menu values ('3', '1060') on conflict do nothing;
insert into sys_role_menu values ('3', '1061') on conflict do nothing;

-- =====================================================================
-- §6 Data Services module menus (top dir 6)
-- ---------------------------------------------------------------------
-- Data is split into Workspaces / Stores / Layer previews etc as independent entry points,
-- manager mirrors this: Workspace / Data Source / Data Layer / Published Service each get a page.
--   124 Workspace / 125 Data Source / 126 Data Layer / 127 Published Service (C)
--   1062..1066 buttons (F)
--   122 Data Catalog Overview (under 6, tree + links only, no CRUD)
-- =====================================================================

-- ---------- A. Top-level dir ----------
-- Top dir order_num=2, "Data Services" before "Map Services" (7): sources first, published second.
insert into sys_menu values('6', 'Data Services', '0', '2', 'data', null, '', '1', '0', 'M', '0', '0', '', 'data', 103, 1, now(), null, null, 'Workspace / data source / data layer organization and publishing (read-only discovery, no data import)')
  on conflict (menu_id) do nothing;

-- ---------- B. Function menus (C) ----------
-- 124 Workspace: namespace container (sys_workspace).
insert into sys_menu values('124', 'Workspace', '6', '1', 'workspace', 'map/catalog/workspace', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'workspace', 103, 1, now(), null, null, 'Workspace inventory with CRUD and data source / live service counts')
  on conflict (menu_id) do nothing;

-- 125 Data Source: directory or PostGIS connection pointer (sys_store).
insert into sys_menu values('125', 'Data Source', '6', '2', 'store', 'map/catalog/store', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'store', 103, 1, now(), null, null, 'Data source (directory / PostGIS): create, connection test, read-only discovery and trash restore')
  on conflict (menu_id) do nothing;

-- 126 Data Layer: discovered resource (sys_resource) → publish.
insert into sys_menu values('126', 'Data Layer', '6', '3', 'resource', 'map/catalog/resource', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'layers', 103, 1, now(), null, null, 'Discovered data layers and publish wizard ( protocol options come from the engine capability matrix )')
  on conflict (menu_id) do nothing;

-- 127 Published Service: publish ledger (sys_published_layer) lifecycle + endpoint preview.
insert into sys_menu values('127', 'Published Services', '6', '4', 'published', 'map/catalog/published', '', '1', '0', 'C', '0', '0', 'map:catalog:list', 'published', 103, 1, now(), null, null, 'Live services grouped by protocol and data type: start/stop/pause/delete and endpoint preview')
  on conflict (menu_id) do nothing;

-- ---------- C. Buttons (F) ----------
insert into sys_menu values('1062', 'Workspace Maintenance', '124', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:edit', '#', 103, 1, now(), null, null, 'Workspace: create, rename, delete')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1063', 'Data Source Maintenance', '125', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:edit', '#', 103, 1, now(), null, null, 'Data source: create, edit, delete, connection test and pointer restore')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1064', 'Read-only Discovery', '126', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, 'Refresh the data layer list from the source (read-only metadata)')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1065', 'Publish Layer', '126', '2', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, 'Publish map services, edit publish info, soft-delete and restore resources')
  on conflict (menu_id) do nothing;
insert into sys_menu values('1066', 'Service Lifecycle', '127', '1', '', '', '', '1', '0', 'F', '0', '0', 'map:catalog:publish', '#', 103, 1, now(), null, null, 'Start / Stop / Pause / Delete (to trash) and Restore')
  on conflict (menu_id) do nothing;

-- ---------- D. Former 122 Data Catalog → overview page under "Data Services" ----------
-- CRUD split out to 124..127: here only ws → store tree + page links, no duplicate forms.
update sys_menu
   set parent_id = '6',
       order_num = 0,
       menu_name = 'Data Catalog Overview',
       remark    = 'Workspace to data source tree and counts; see workspace / source / layer / published services pages'
 where menu_id = '122';
-- Two former 122 buttons (08 under 122) moved to feature menus; perm codes unchanged (old DB already granted these IDs).
update sys_menu set parent_id = '124', order_num = 2 where menu_id = '1060';
update sys_menu set parent_id = '126', order_num = 3 where menu_id = '1061';


-- ---------- F. Role grants (role 3 "This dept and below") ----------
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
-- §6.5 API Key management (rust-gisserver access credentials)
-- ---------------------------------------------------------------------
-- One table, two purposes, aligned with rust-gisserver's two token guards:
--   admin  manager → server admin surface (X-Admin-Token header)
--   data   server → client data surface (X-Data-Token / Authorization: Bearer / ?token=)
-- Plaintext for easy copy in manager; DB deployed on intranet, access guarded by login + map:apikey:*.
-- rust-gisserver periodically reads enabled keys from this shared DB as "additional accepted credentials",
-- Primary token (mapserver.xml admin.token / server.data_token) remains the master switch.
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

comment on table  sys_api_key            is 'rust-gisserver access key: admin plane (admin) / client data plane (data)';
comment on column sys_api_key.purpose    is 'Purpose: admin = manager to server (X-Admin-Token); data = client data plane';
comment on column sys_api_key.status     is 'Status: 0 enabled / 1 disabled';
comment on column sys_api_key.expires_at is 'Expiry (empty = never expires); disabled or expired keys are ignored by server validation';

create index if not exists idx_sys_api_key_purpose_status on sys_api_key (purpose, status);

-- ---------- A. Menus (under Map Services dir 7) ----------
insert into sys_menu values('128', 'API Keys', '7', '10', 'apikeys', 'map/apikey/index', '', '1', '0', 'C', '0', '0',
  'map:apikey:list', 'key', 103, 1, now(), null, null,
  'Generate access keys for rust-gisserver ( name + key ), for manager-to-server or client delivery')
  on conflict (menu_id) do nothing;

-- ---------- B. Button permissions ----------
insert into sys_menu values('1067', 'API Key Maintenance', '128', '1', '', '', '', '1', '0', 'F', '0', '0',
  'map:apikey:edit', '#', 103, 1, now(), null, null,
  'Generate / Reset / Start-Stop / Edit / Delete / Apply to default engine')
  on conflict (menu_id) do nothing;

-- ---------- C. Role grants (role 3; superadmin bypasses role_menu) ----------
insert into sys_role_menu values ('3', '128')  on conflict do nothing;
insert into sys_role_menu values ('3', '1067') on conflict do nothing;

-- =====================================================================
-- §7 Normalize top-level directory sort order
-- ---------------------------------------------------------------------
-- Target order: 1 System Admin / 6 Data Services / 7 Map Services / 2 System Monitor /
--           3 System Tools / 7 Test Menu
-- Same rule as system.sql §5: running map.sql alone also sorts top-level dirs correctly.
-- Lookup by menu_id (parent_id = 0 top-level rows); re-runs write same values back.
-- =====================================================================
update sys_menu set order_num = 1 where menu_id = '1' and parent_id = 0; -- System Admin
update sys_menu set order_num = 2 where menu_id = '6' and parent_id = 0; -- Data Services
update sys_menu set order_num = 3 where menu_id = '7' and parent_id = 0; -- Map Services
update sys_menu set order_num = 4 where menu_id = '2' and parent_id = 0; -- System Monitor
update sys_menu set order_num = 5 where menu_id = '3' and parent_id = 0; -- System Tools
update sys_menu set order_num = 7 where menu_id = '5' and parent_id = 0; -- Test Menu

-- ---------------------------------------------------------------------
-- Incremental migration: add data_root / engine_id columns to sys_workspace (skip if exist)
-- =====================================================================
alter table sys_workspace add column if not exists data_root varchar(512) default '';
comment on column sys_workspace.data_root is 'Data file root path (use / separators, trailing / required)';
alter table sys_workspace add column if not exists engine_id varchar(64) default '';
comment on column sys_workspace.engine_id is 'Bound engine ID (sys_engine.engine_id; empty = default engine)';

-- =====================================================================
-- §8 SLD style library sys_sld
-- ---------------------------------------------------------------------
-- DB primary; map-api styles dir is filesystem mirror (manager writes both).
-- Fields: sld_content = full XML; style_name / layer_name / rule_count are parsed redundancies,
-- for list display; sld_name has no .sld suffix, sld_file has the suffix.
-- Menus: already in §1 - menu_id=112 Style Library + buttons 1056 Upload / 1057 Delete.
--
-- Required-ness of three parsed columns: SLD/SE spec <UserStyle><Name> and
-- <NamedLayer><Name> are both optional (in gisserver::renderer::sld::SldDocument,
-- both are Option<String>); map-api upload only requires root StyledLayerDescriptor
-- and at least one FeatureTypeStyle; no Name validation; Rule list may be empty, so rule count
-- can be 0. So none are required - use '' / 0 for "SLD did not write this".
-- But storage layer must be NOT NULL: manager's style_list / style_content use
-- r.get::<String>() / r.get::<i32>() directly - NULL would panic.
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

-- Compat with existing deployments: CREATE TABLE IF NOT EXISTS does not tighten existing columns, so fix history
-- NULLs then add constraints. Fully idempotent, runs with full script.
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

comment on table  sys_sld             is 'SLD style library: Database primary storage + dual write to map-api styles directory';
comment on column sys_sld.sld_id     is 'Auto-increment ID';
comment on column sys_sld.sld_name   is 'Style name (without .sld extension, globally unique; referenced by frontend/XML)';
comment on column sys_sld.sld_file   is 'File name (including .sld extension)';
comment on column sys_sld.sld_content is 'SLD XML source';
comment on column sys_sld.style_name is 'Parsed redundancy: UserStyle Name (empty when SLD omits it; optional but NOT NULL)';
comment on column sys_sld.layer_name is 'Parsed redundancy: NamedLayer Name (empty when SLD omits it; optional but NOT NULL)';
comment on column sys_sld.rule_count is 'Parsed redundancy: rule count (parser computed, may be 0, not user input, NOT NULL)';

-- =====================================================================
-- §9 MVT frontend style JSON (sys_published_layer)
-- ---------------------------------------------------------------------
-- MVT is protobuf feature stream; styles rendered by frontend OpenLayers; manager's
-- /map/services/{kind}/{name}/mvt-style reads/writes this column; map-api does not consume it.
-- Empty value → viewer_mvt.html built-in default style.
-- =====================================================================
alter table sys_published_layer add column if not exists mvt_style_json jsonb;
comment on column sys_published_layer.mvt_style_json is 'MVT vector tile frontend style JSON ( OpenLayers Style JSON, read/write in the manager preview page )';
