/** 数据存储类型：引擎 store_types + 服务端补齐的 postgis */
export interface StoreTypeCap {
  store_type: string;
  extensions: string[];
}

/** 一种数据类型（data_type）的发布口径，来自 map-api /admin/capabilities */
export interface DataTypeCap {
  data_type: string;
  mapserver_kind: string;
  publishable: boolean;
  service_types: string[];
  default_service_types: string[];
  note: string;
}

/** GET /map/catalog/capabilities —— 发布向导的数据字典（单一口径来自引擎） */
export interface CatalogCaps {
  store_types: StoreTypeCap[];
  data_types: DataTypeCap[];
  service_types: string[];
}

export interface WorkspaceVO {
  id: number;
  name: string;
  title: string;
  engineId: string;
  dataRoot: string;
  /** 0 正常 / 1 已删（软删除仅标记） */
  delFlag: number;
  remark: string;
  storeCount: number;
  publishedCount: number;
}

export interface WorkspaceListResp {
  workspaces: WorkspaceVO[];
  total: number;
}

export interface WorkspaceBody {
  name: string;
  title: string;
  engineId: string;
  dataRoot: string;
  remark: string;
}

/** 工作空间数据根目录下的一个条目（GET /map/catalog/dirs） */
export interface DirEntry {
  name: string;
  is_dir: boolean;
  size: number;
  modified_ms: number;
}

/** GET /map/catalog/dirs —— 目录树一层 */
export interface DirListResp {
  path: string;
  parent: string;
  root: string;
  entries: DirEntry[];
}

export interface StoreVO {
  id: number;
  workspaceId: number;
  name: string;
  storeType: string;
  basePath: string;
  pgSchema: string;
  /** 服务端是否已登记连接串（口令不回传） */
  pgDsnConfigured: boolean;
  /** 掩码后的连接串，仅用于展示 */
  pgDsnMasked: string;
  isPostgis: boolean;
  remark: string;
  delFlag: number;
  resourceCount: number;
  publishableCount: number;
  /** 已发布（未移入回收站）的数据图层数量；>0 时数据源禁止修改/删除 */
  publishedCount: number;
}

export interface StoreListResp {
  stores: StoreVO[];
  total: number;
}

export interface StoreBody {
  workspaceId: number;
  name: string;
  storeType: string;
  basePath: string;
  /** 仅新增/改密时提交；回显永远是掩码 */
  pgDsn: string;
  pgSchema: string;
  remark: string;
}

/** POST /map/catalog/stores/:id/connect —— 不抛错，用 ok 表达可达性 */
export interface ConnectResp {
  ok: boolean;
  detail: string;
  path?: string;
  catalogRoot?: string;
  truncated?: boolean;
}

/** POST /map/catalog/stores/connect —— 新建数据源表单的连接测试参数 */
export interface PgConnectBody {
  host: string;
  port: number;
  username: string;
  password: string;
  database: string;
}

/** GET /map/catalog/resources/:id/query —— PostGIS 普通表数据查询结果 */
export interface PgQueryResp {
  schema: string;
  table: string;
  columns: string[];
  rows: Array<Array<string | null>>;
  row_count: number;
  limit: number;
}

export interface ResourceVO {
  id: number;
  storeId: number;
  name: string;
  dataType: string;
  /** 文件绝对路径 或 schema.table */
  source: string;
  srcCrs: string;
  /** 只读探测得到的 [minx,miny,maxx,maxy]，可空 */
  bbox: number[] | null;
  publishable: boolean;
  reason: string;
  /** 坐标系名与坐标值范围不匹配，或坐标系缺失 → 发布会被拦截 */
  crsSuspicious?: boolean;
  meta: Record<string, unknown>;
  /** 仅 includeDeleted=1 时返回 */
  delFlag?: number;
}

export interface ResourceListResp {
  resources: ResourceVO[];
  total: number;
}

/** ArcGIS 缓存的原生网格（来自 Conf.xml，发布时固化到账本） */
export interface NativeGridLevel {
  level: number;
  scale: number;
  resolution: number;
}

/** 引擎 `custom_grids` 的网格定义（发布时随服务文档下发） */
export interface GridDef {
  crs: string;
  origin: number[];
  extent: number[];
  tile_px: number;
  resolutions: number[];
  levels: number[];
  scales: number[];
}

export interface NativeGrid {
  crs: string;
  tile_cols: number | null;
  tile_rows: number | null;
  dpi: number | null;
  origin: number[] | null;
  min_zoom: number | null;
  max_zoom: number | null;
  levels: NativeGridLevel[];
  /** 网格模式：ogc（标准 LOD）| tdt（天地图）| custom（Conf.xml）| native（ArcGIS 原生） */
  mode?: string;
  /** 引擎侧网格 id（grids 引用名） */
  id?: string;
  /** 自定义网格来源 XML 路径 */
  source?: string | null;
  extent?: number[] | null;
  tile_px?: number | null;
  resolutions?: number[] | null;
  scales?: number[] | null;
  /** 非预设网格的重建定义（引擎 custom_grids） */
  grid_def?: GridDef | null;
}

/** POST /map/catalog/tile-grid —— 解析服务器上的 ArcGIS Conf.xml */
export interface TileGridParseResp {
  source: string;
  cache_dir?: string;
  crs: string;
  wkid?: number;
  origin: number[];
  extent: number[];
  tile_px: number;
  tile_cols?: number;
  tile_rows?: number;
  dpi?: number;
  levels: number[];
  resolutions: number[];
  scales: number[];
  matrix_w?: number[];
  matrix_h?: number[];
  min_zoom?: number;
  max_zoom?: number;
  bbox?: number[] | null;
  lods?: NativeGridLevel[];
}

export interface DiscoverResp {
  storeId: number;
  storeType: string;
  discovered: number;
  stored: number;
  resources: ResourceVO[];
}

/** POST /map/catalog/import-tpk —— 导入 ArcGIS 缓存包（.tpk / .tpkx） */
export interface ImportTpkBody {
  /** 工作空间 id */
  workspaceId: number;
  /** 服务器上 .tpk / .tpkx 包的绝对路径（须在工作空间数据根内） */
  path: string;
  /** 落点目录名，缺省取包名去扩展名 */
  name?: string;
}

/** 导入结果：解压落点 + 缓存定义记录（网格方案） */
export interface ImportTpkResp {
  /** 源包路径 */
  source: string;
  /** 解压落点目录（本身即一份标准 ArcGIS 缓存目录） */
  dir: string;
  /** 配置文件（conf.xml / root.json）路径 */
  config_file: string;
  /** 包类型：tpk | tpkx */
  package_kind: string;
  /** 解出的条目数 */
  entries: number;
  /** 定义记录编号 */
  definition_id: number;
  /** 是否复用了已有定义记录（内容完全一致，未新建） */
  definition_reused: boolean;
  /** 配置来源形态：conf.xml | root.json */
  config_kind: string;
  crs: string;
  wkid: number;
  origin: number[];
  tile_cols: number;
  tile_rows: number;
  dpi: number;
  packet_size: number;
  storage_format: string;
  /** 瓦片根目录名：_alllayers | tile */
  tile_root: string;
  min_zoom: number;
  max_zoom: number;
  lods: NativeGridLevel[];
  /** 数据范围（缓存 CRS 单位） */
  bounds: number[] | null;
  /** 缓存字节格式：v1 | v2 | wtc */
  cache_version: string;
  /** 实际存在的层级 */
  levels: number[];
  max_col: number;
  max_row: number;
}

/** POST /map/catalog/stores/{id}/import-shp —— 把工作空间内的 Shapefile 导入 PostGIS */
export interface ImportShpBody {
  /** 工作空间数据根目录内的 .shp 绝对路径 */
  path: string;
  /** 导入后的数据图层名（PostGIS 表名） */
  table: string;
  /** 可选：覆盖 .prj 自动识别的 EPSG */
  srid?: number;
}

export interface ImportShpResp {
  ok: boolean;
  schema: string;
  table: string;
  srid: number;
  sridDetected: boolean;
  geometryColumn: string | null;
  features: number;
  /** 实际写入的行数（COPY 阶段累计） */
  rows?: number;
  elapsedMs: number;
}

/** 导入 SHP 的 SSE 进度事件（stage=start/copy/done/error） */
export interface ImportShpProgress extends Partial<ImportShpResp> {
  stage: 'start' | 'copy' | 'done' | 'error';
  /** copy 阶段：已写入的行数 */
  rows?: number;
  /** error 阶段：失败原因 */
  error?: string;
}

export interface DiscoverBody {
  /** 目录型 Store 的递归层深 1..=4 */
  depth?: number;
  /** 仅 PostGIS：限定 schema，省略则用 Store 的 pgSchema */
  schema?: string;
}

export interface PublishedVO {
  id: number;
  workspaceId: number;
  resourceId: number | null;
  layerName: string;
  title: string;
  srcCrs: string;
  bbox: number[] | null;
  serviceTypes: string[];
  mapserverKind: string;
  /** active | stopped | paused | deleted */
  state: string;
  remark: string;
  workspaceName: string | null;
  storeName: string | null;
  resourceName: string | null;
  dataType: string | null;
  source: string | null;
  /** ArcGIS 发布时固化的原生网格快照 */
  grid?: NativeGrid | null;
  /** 几何类型（从 sys_resource.meta.geometry_type 透出）：MULTIPOLYGON / LINESTRING / POINT / GEOMETRY */
  geometryType?: string;
}

export interface PublishedListResp {
  published: PublishedVO[];
  total: number;
}

/** POST /map/catalog/publish 与 PUT /map/catalog/published/:id 的请求体 */
export interface PublishBody {
  resourceId: number;
  layerName: string;
  title: string;
  srcCrs: string;
  bbox: number[] | null;
  serviceTypes: string[];
  remark: string;
  /** 瓦片网格模式：ogc | tdt | custom；留空沿用引擎默认（WebMercatorQuad） */
  gridMode?: string;
  /** 自定义/天地图网格快照，或标准预设的账本快照 */
  grid?: NativeGrid | null;
}

/** 协议端点（URL 由引擎给出，管理端只做一键复制与新窗口预览） */
export interface ServiceEndpoint {
  name: string;
  url: string;
}

/** GET /map/catalog/published/:id/endpoints —— 预览/服务地址 */
export interface PublishedEndpoints {
  layerName: string;
  state: string;
  /** 引擎目录里是否还存在该服务的配置 */
  found: boolean;
  /** 是否在服（state=active 且配置在位） */
  serving: boolean;
  kind?: string;
  engineName?: string | null;
  /** 未在服原因（引擎侧） */
  note?: string | null;
  /** 引擎侧没有该配置时的处置提示 */
  hint?: string;
  endpoints: ServiceEndpoint[];
}

/** 坐标系树节点：WGS84 系 / CGCS2000 系（叶子带 EPSG 编码，中间层为分组） */
export interface CrsTreeNode {
  name: string;
  /** EPSG 编码，形如 EPSG:4326；中间分组节点为空 */
  code?: string;
  children?: CrsTreeNode[];
}

/** GET /map/catalog/crs-tree —— 发布向导的坐标系选择树 */
export interface CrsTreeResp {
  tree: CrsTreeNode[];
}
