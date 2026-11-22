export interface MapServiceVO {
  /** 服务名（图层名 / 3dtiles 服务名） */
  name: string;
  /** 服务类：cog/vector/postgis/pmtiles/mbtiles/arcgis/3dtiles，与 map-api catalog 一致 */
  kind: string;
  title: string;
  /** 数据来源（文件相对路径 / postgis 表等） */
  source: string;
  /** 登记状态：active | stopped | paused | deleted */
  state: string;
  /** 是否在服（运行时可访问） */
  serving: boolean;
  /** 在用但未在服时的原因说明（如数据目录缺失） */
  note?: string | null;
  /** 协议端点（测试浏览一键复制） */
  endpoints: ServiceEndpoint[];
  /** 归属引擎（跨引擎聚合时由 manager 标注；单引擎旧响应无此字段） */
  engineId?: string;
  engineName?: string;
  /** 服务分组（「一级」或「一级/二级」；空/缺省 = 根目录） */
  group?: string;
  /** 默认瓦片网格类型（仅出图类服务有；3dtiles/terrain 等为 null） */
  grid?: ServiceGrid | null;
}

/** 默认瓦片网格：id=网格名，crs=坐标系（标准 EPSG 与否），standard=是否 OGC 预设网格 */
export interface ServiceGrid {
  /** 网格标识（WebMercatorQuad / WorldCRS84Quad / arcgis_<图层> / 自定义 id） */
  id: string;
  /** 发布坐标系（如 EPSG:4490） */
  crs: string;
  /** true=标准/OGC 预设网格；false=自定义网格（与 crs 是否标准 EPSG 正交） */
  standard: boolean;
}

export interface ServiceEndpoint {
  name: string;
  url: string;
}

export interface MapServiceListResp {
  registry_mode: string;
  count: number;
  services: MapServiceVO[];
  /** 后端分页总条数（manager 侧按 kind/state/name 过滤 + 切片后的总数） */
  total?: number;
  /** 跨引擎聚合统计（仅多引擎聚合响应携带） */
  engines?: import('@/api/map/engine/types').EngineStat[];
}

export interface ReloadReport {
  registry_mode?: string;
  served?: string[];
  tilesets?: string[];
  skipped?: string[];
  warnings?: string[];
}

export interface UsageRow {
  kind: string;
  name: string;
  hits: number;
}

export interface UsageResp {
  period: string;
  hours: number;
  group_by: string;
  /** pg | memory */
  source: string;
  rows: UsageRow[];
}

export interface MapSvcHealth {
  base_url: string;
  up: boolean;
}

/** 访问日志行 */
export interface AccessLogRow {
  ts_ms: number;
  method: string;
  path: string;
  query: string;
  kind: string;
  name: string;
  status: number;
  duration_ms: number;
  client: string;
}

export interface AccessLogResp {
  /** 重启以来累计请求数 */
  total: number;
  status_counts: Record<string, number>;
  errors: { client_4xx: number; server_5xx: number };
  rows: AccessLogRow[];
}

/** 数据源体检行 */
export interface CheckRow {
  item: string;
  ok: boolean;
  /** critical 失败影响总体结论；info 仅提示 */
  level: 'critical' | 'info';
  detail: string;
}

export interface CheckResp {
  name: string;
  kind: string;
  ok: boolean;
  checks: CheckRow[];
}

/** 样式库条目 */
export interface StyleVO {
  name: string;
  size: number;
  modified: number;
  style_name: string | null;
  layer_name: string | null;
  rules: number;
  parse_ok: boolean;
  used_by: string[];
}

export interface StyleListResp {
  dir: string;
  count: number;
  styles: StyleVO[];
}

/** 目录浏览条目 */
export interface BrowseEntry {
  name: string;
  is_dir: boolean;
  size: number;
  modified_ms: number;
}

/** 目录浏览响应 */
export interface BrowseResp {
  path: string;
  parent: string;
  root: string;
  entries: BrowseEntry[];
}

/** 组件探测响应（运营仪表盘组件卡） */
export interface ComponentsResp {
  map_api_up: boolean;
  pg_enabled: boolean;
  proj_loaded: boolean;
  catalog_root: string;
  catalog_root_exists: boolean;
  tilesets_dir: string;
  tilesets_dir_exists: boolean;
  served_layers: number;
  tilesets: number;
  cache_mib: number;
  cache_ttl_secs: number;
}

/** PostGIS 连接测试响应 */
export interface PgTestResp {
  ok: boolean;
  detail: string;
}

/** 登记新服务响应 */
export interface CreateServiceResp {
  msg?: string;
  name?: string;
  kind?: string;
  served?: boolean;
  skip_reason?: string;
  report?: Record<string, any>;
}

export type MapServiceQuery = {
  serviceName?: string;
  kind?: string;
};

/** 单个缓存计数 */
export interface CacheCounters {
  entries: number;
  bytes: number;
}

/** 缓存统计响应（GET /map/cache） */
export interface CacheStatsResp {
  ttl_secs: number;
  purges: number;
  output: CacheCounters;
  layers: CacheCounters;
  docs: CacheCounters;
  catalog: CacheCounters;
  features: CacheCounters;
  decoded_tiles: CacheCounters;
  disk?: CacheCounters | null;
}

/** 清缓存响应（POST /map/cache/purge） */
export interface CachePurgeResp {
  scope: string;
  purged_entries: number;
  purges: number;
}

/** manager 运行配置（GET /map/runtime，敏感字段已掩码） */
export interface RuntimeResp {
  server_addr: string;
  base_api: string;
  cors_origins: string[];
  database_url: string;
  pool_max: number;
  mapsvc_base_url: string;
  mapsvc_token_configured: boolean;
  jwt_expire_minutes: number;
  captcha_enabled: boolean;
  avatar_dir: string;
}

/** 服务配置文件原文（GET /map/services/:kind/:name） */
export interface ServiceConfigResp {
  /** 磁盘上的配置文件路径 */
  path: string;
  /** XML 原文（含注释） */
  xml: string;
}

/** 配置回写结果（PUT /map/services/:kind/:name） */
export interface ServiceConfigUpdateResp {
  msg?: string;
  /** 回写并热重载后该服务是否在服 */
  served?: boolean;
  /** 未在服时的原因（语法/数据源问题） */
  skip_reason?: string | null;
  /** 热重载报告 */
  report?: Record<string, any>;
}
