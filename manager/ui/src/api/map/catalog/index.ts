import request from '@/utils/request';
import { getToken } from '@/utils/auth';
import { AxiosPromise } from 'axios';
import {
  CatalogCaps,
  ConnectResp,
  CrsTreeResp,
  DirListResp,
  DiscoverBody,
  DiscoverResp,
  ImportShpBody,
  ImportShpProgress,
  ImportShpResp,
  PgConnectBody,
  PgQueryResp,
  PublishBody,
  PublishedEndpoints,
  PublishedListResp,
  PublishedVO,
  ResourceListResp,
  StoreBody,
  StoreListResp,
  TileGridParseResp,
  WorkspaceBody,
  WorkspaceListResp
} from './types';

// 能力矩阵（数据类型 → 服务类型勾选口径；单一来源为引擎）
export function getCapabilities(): AxiosPromise<CatalogCaps> {
  return request({
    url: '/map/catalog/capabilities',
    method: 'get'
  });
}

// 坐标系选择树（WGS84 系的地理/投影、CGCS2000 系的地理/高斯投影）
export function getCrsTree(): AxiosPromise<CrsTreeResp> {
  return request({
    url: '/map/catalog/crs-tree',
    method: 'get'
  });
}

// ---------- 工作区 ----------
// 工作空间数据根目录浏览（新建数据源左侧目录树；按工作区范围读取一层）
export function listWorkspaceDirs(params: { workspaceId: number; path?: string }): AxiosPromise<DirListResp> {
  return request({
    url: '/map/catalog/dirs',
    method: 'get',
    params
  });
}

// 解析服务器上的 ArcGIS Conf.xml（发布向导「选择瓦片配置 XML」）
export function parseTileGrid(data: { workspaceId: number; path: string }): AxiosPromise<TileGridParseResp> {
  return request({
    url: '/map/catalog/tile-grid',
    method: 'post',
    data
  });
}

export function listWorkspaces(params?: { includeDeleted?: number }): AxiosPromise<WorkspaceListResp> {
  return request({
    url: '/map/catalog/workspaces',
    method: 'get',
    params
  });
}

export function addWorkspace(data: WorkspaceBody) {
  return request({
    url: '/map/catalog/workspaces',
    method: 'post',
    data
  });
}

export function updateWorkspace(id: number, data: WorkspaceBody) {
  return request({
    url: `/map/catalog/workspaces/${id}`,
    method: 'put',
    data
  });
}

export function deleteWorkspace(id: number) {
  return request({
    url: `/map/catalog/workspaces/${id}`,
    method: 'delete'
  });
}

// ---------- 数据存储 ----------
export function listStores(params?: { workspaceId?: number; includeDeleted?: number }): AxiosPromise<StoreListResp> {
  return request({
    url: '/map/catalog/stores',
    method: 'get',
    params
  });
}

export function addStore(data: StoreBody) {
  return request({
    url: '/map/catalog/stores',
    method: 'post',
    data
  });
}

export function updateStore(id: number, data: StoreBody) {
  return request({
    url: `/map/catalog/stores/${id}`,
    method: 'put',
    data
  });
}

export function deleteStore(id: number) {
  return request({
    url: `/map/catalog/stores/${id}`,
    method: 'delete'
  });
}

// 垃圾箱恢复 Store 指针（其级联进垃圾箱的资源需逐个恢复）
export function restoreStore(id: number) {
  return request({
    url: `/map/catalog/stores/${id}/restore`,
    method: 'post'
  });
}

// 连接测试（目录可达 / PostGIS 可连），失败也以 ok:false 返回
export function testStoreConnection(id: number): AxiosPromise<ConnectResp> {
  return request({
    url: `/map/catalog/stores/${id}/connect`,
    method: 'post'
  });
}

// 新建数据源时的连接测试（无需已落库的 store id），由表单字段拼装 DSN
export function testStoreConnectionPreflight(data: PgConnectBody): AxiosPromise<ConnectResp> {
  return request({
    url: '/map/catalog/stores/connect',
    method: 'post',
    data
  });
}

// 只读发现：引擎扫描元数据并刷新资源清单（不导入、不生成瓦片）
export function discoverStore(id: number, data?: DiscoverBody): AxiosPromise<DiscoverResp> {
  return request({
    url: `/map/catalog/stores/${id}/discover`,
    method: 'post',
    data: data || {}
  });
}

// 导入 SHP：把工作空间内的 Shapefile 交给 map-api 落进该 PostGIS 库
export function importStoreShp(id: number, data: ImportShpBody): AxiosPromise<ImportShpResp> {
  return request({
    url: `/map/catalog/stores/${id}/import-shp`,
    method: 'post',
    data
  });
}

/**
 * 导入 SHP（SSE 流式版）：大文件导入期间持续收到 `stage=start/copy/done/error`
 * 事件，避免前端长时间干等。响应体为 `text/event-stream`，故走 fetch 而非 axios。
 *
 * @param onEvent 每收到一条事件回调一次
 * @param signal  可选中止（AbortController）
 */
export async function importStoreShpStream(
  id: number,
  data: ImportShpBody,
  onEvent: (e: ImportShpProgress) => void,
  signal?: AbortSignal
): Promise<void> {
  const base = (import.meta.env.VITE_APP_BASE_API || '').replace(/\/$/, '');
  const resp = await fetch(`${base}/map/catalog/stores/${id}/import-shp/stream`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json;charset=utf-8',
      Authorization: 'Bearer ' + getToken()
    },
    body: JSON.stringify(data),
    signal
  });
  if (!resp.ok || !resp.body) {
    throw new Error(`导入接口异常（HTTP ${resp.status}）`);
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  // SSE 事件以空行分隔；逐个解析 data: 行（心跳注释行直接忽略）
  const flush = (chunk: string) => {
    const payload = chunk
      .split(/\r?\n/)
      .filter((l) => l.startsWith('data:'))
      .map((l) => l.slice(5).trim())
      .join('');
    if (!payload) return;
    try {
      onEvent(JSON.parse(payload) as ImportShpProgress);
    } catch {
      // 非 JSON（心跳/异常输出）不参与进度展示
    }
  };
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let idx = buf.search(/\r?\n\r?\n/);
    while (idx >= 0) {
      const chunk = buf.slice(0, idx);
      const sepLen = buf.slice(idx).startsWith('\r\n') ? 4 : 2;
      buf = buf.slice(idx + sepLen);
      flush(chunk);
      idx = buf.search(/\r?\n\r?\n/);
    }
  }
  if (buf.trim()) flush(buf);
}

// ---------- 数据图层（Resource）----------
export function listResources(params: { storeId: number; includeDeleted?: number }): AxiosPromise<ResourceListResp> {
  return request({
    url: '/map/catalog/resources',
    method: 'get',
    params
  });
}

// 软删除（仅标记）：其在线服务同步转 deleted 进垃圾箱
export function deleteResource(id: number) {
  return request({
    url: `/map/catalog/resources/${id}`,
    method: 'delete'
  });
}

export function restoreResource(id: number) {
  return request({
    url: `/map/catalog/resources/${id}/restore`,
    method: 'post'
  });
}

// 单条图层重新探查元数据（不重扫整个数据源、不改发布态）
export function refreshResource(id: number) {
  return request({
    url: `/map/catalog/resources/${id}/refresh`,
    method: 'post'
  });
}

// PostGIS 普通表数据查询服务（不发布图层，直接返回表数据）
export function queryResource(id: number, limit = 100): AxiosPromise<PgQueryResp> {
  return request({
    url: `/map/catalog/resources/${id}/query`,
    method: 'get',
    params: { limit }
  });
}

// ---------- 发布 / 已发布服务 ----------
export function publishService(data: PublishBody) {
  return request({
    url: '/map/catalog/publish',
    method: 'post',
    data
  });
}

export function listPublished(params?: { workspaceId?: number; includeDeleted?: number }): AxiosPromise<PublishedListResp> {
  return request({
    url: '/map/catalog/published',
    method: 'get',
    params
  });
}

// 修改发布信息（重建服务文档并热重载；layerName 变更时引擎侧改名）
export function updatePublished(id: number, data: PublishBody) {
  return request({
    url: `/map/catalog/published/${id}`,
    method: 'put',
    data
  });
}

// 生命周期：active | stopped | paused | deleted
export function setPublishedState(id: number, state: string) {
  return request({
    url: `/map/catalog/published/${id}/state`,
    method: 'put',
    data: { state }
  });
}

// 协议端点与内置预览地址（URL 全部由引擎给出，管理端不自行拼接）
export function listPublishedEndpoints(id: number): AxiosPromise<PublishedEndpoints> {
  return request({
    url: `/map/catalog/published/${id}/endpoints`,
    method: 'get'
  });
}
