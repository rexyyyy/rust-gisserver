export interface EngineVO {
  /** 引擎 ID（URL 段，字母/数字/-/_） */
  engineId: string;
  engineName: string;
  /** 引擎类型（当前 rust-gisserver；预留扩展） */
  engineType: string;
  /** 管理面 base_url */
  baseUrl: string;
  /** 服务端是否已配置 admin token（token 本身不回传） */
  tokenConfigured: boolean;
  /** 编辑时可回填：仅编辑场景由后端按需返回，列表接口恒为空串 */
  adminToken?: string;
  /** 状态：0 启用 | 1 停用 */
  status: string;
  /** 是否默认引擎：1 是 */
  isDefault: string;
  sort: number;
  remark: string;
}

export interface EngineListResp {
  engines: EngineVO[];
}

export interface EngineHealth {
  up: boolean;
  latencyMs: number;
  detail: string;
}

/** 引擎新增/编辑请求体 */
export interface EngineBody {
  engineId: string;
  engineName: string;
  engineType: string;
  baseUrl: string;
  adminToken: string;
  status: string;
  isDefault: string;
  sort: number;
  remark: string;
}

/** 服务归属引擎查询响应（GET /map/services/:kind/:name/engine） */
export interface ServiceEngineResp {
  kind: string;
  name: string;
  /** 已登记的归属引擎 ID；null = 未登记（实际走默认引擎） */
  engineId: string | null;
  engineName: string | null;
  effective: string | null;
}

/** 聚合响应中每引擎统计（GET /map/services 返回的 engines 字段） */
export interface EngineStat {
  engineId: string;
  engineName: string;
  up: boolean;
  count: number;
}

/** 引擎事件行（manager 后台巡检产生） */
export interface EngineEvent {
  id: number;
  engineId: string;
  /** engine_online | engine_offline | service_added | service_removed | service_state | service_up | service_down */
  eventType: string;
  summary: string;
  detail: string;
  createTime: string;
}

export interface EngineEventListResp {
  events: EngineEvent[];
  total: number;
}

/** map-api /admin/components 返回的服务器运行环境 */
export interface EngineComponents {
  map_api_up?: boolean;
  pg_enabled?: boolean;
  proj_loaded?: boolean;
  catalog_root?: string;
  catalog_root_exists?: boolean;
  tilesets_dir?: string;
  tilesets_dir_exists?: boolean;
  served_layers?: number;
  tilesets?: number;
  cache_mib?: number;
  cache_ttl_secs?: number;
  [key: string]: unknown;
}

/** 引擎详情响应（GET /map/engines/:engineId/detail） */
export interface EngineDetailResp {
  engine: EngineVO;
  health: EngineHealth;
  components: EngineComponents | null;
  serviceTotal: number;
  serviceServing: number;
  recentEvents: EngineEvent[];
}
