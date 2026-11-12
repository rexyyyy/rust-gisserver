import request from '@/utils/request';
import {
  AccessLogResp,
  CachePurgeResp,
  CacheStatsResp,
  CheckResp,
  MapServiceListResp,
  MapServiceVO,
  MapSvcHealth,
  ReloadReport,
  ServiceConfigResp,
  ServiceConfigUpdateResp,
  StyleListResp,
  UsageResp
} from './types';
import { AxiosPromise } from 'axios';

// 服务清单（目录扫描 + 登记状态 + 在服标志 + 协议端点）
// 支持后端分页：page/pageSize + 服务端过滤 kind/state/name（manager 侧聚合后切片）
export function listServices(params?: {
  page?: number;
  pageSize?: number;
  name?: string;
  kind?: string;
  state?: string;
}): AxiosPromise<MapServiceListResp> {
  return request({
    url: '/map/services',
    method: 'get',
    params
  });
}

// 变更服务状态：active(恢复/启用) | stopped(停止) | paused(暂停) | deleted(删除进垃圾箱)
export function changeServiceState(kind: string, name: string, state: string) {
  return request({
    url: `/map/services/${kind}/${name}/state`,
    method: 'put',
    data: { state }
  });
}

// 数据源体检（文件/可读性/PG 连接/SRS/样式）
export function checkService(kind: string, name: string): AxiosPromise<CheckResp> {
  return request({
    url: `/map/services/${kind}/${name}/check`,
    method: 'post'
  });
}

// 热重载：重读配置 + 重扫目录 + 重建服务集
export function reloadServices(): AxiosPromise<ReloadReport> {
  return request({
    url: '/map/reload',
    method: 'post'
  });
}

// 访问计数：period = day | week | month | year；group_by = name | type
export function usage(params: { period: string; group_by?: string }): AxiosPromise<UsageResp> {
  return request({
    url: '/map/usage',
    method: 'get',
    params
  });
}

// 访问日志审计查询：status 传 2xx/3xx/4xx/5xx
// 后端分页：page/pageSize（manager 侧强制 upstream limit=2000 拉满后切片，total 为全局总数）
export function listAccessLogs(params: {
  page?: number;
  pageSize?: number;
  limit?: number;
  status?: string;
  kind?: string;
  name?: string;
  q?: string;
}): AxiosPromise<AccessLogResp> {
  return request({
    url: '/map/logs',
    method: 'get',
    params
  });
}

// 样式库清单
export function listStyles(): AxiosPromise<StyleListResp> {
  return request({
    url: '/map/styles',
    method: 'get'
  });
}

// 上传/替换 SLD（服务端先解析校验再落盘）
export function uploadStyle(name: string, content: string) {
  return request({
    url: `/map/styles/${name}`,
    method: 'post',
    data: { content }
  });
}

// 删除样式
export function deleteStyle(name: string) {
  return request({
    url: `/map/styles/${name}`,
    method: 'delete'
  });
}

// 样式原文
export function styleContent(name: string): AxiosPromise<{ name: string; content: string }> {
  return request({
    url: `/map/styles/${name}/content`,
    method: 'get'
  });
}

// 样式图例预览（PNG 二进制，转 blob）
export function styleLegend(name: string, params?: { width?: number; height?: number }): AxiosPromise<Blob> {
  return request({
    url: `/map/styles/${name}/legend.png`,
    method: 'get',
    params,
    responseType: 'blob'
  });
}

// map-api 存活探测
export function mapHealth(): AxiosPromise<MapSvcHealth> {
  return request({
    url: '/map/health',
    method: 'get'
  });
}

// 目录浏览（新增服务向导选 data_root）
export function browseDir(params: { path?: string }): AxiosPromise<BrowseResp> {
  return request({
    url: '/map/browse',
    method: 'get',
    params
  });
}

// 取服务配置文件原文（XML，含注释）
export function getServiceConfig(kind: string, name: string): AxiosPromise<ServiceConfigResp> {
  return request({
    url: `/map/services/${kind}/${name}`,
    method: 'get'
  });
}

// 回写服务配置（XML）并热重载；写盘前自动备份 .xml.bak，重载失败回滚
export function updateServiceConfig(
  kind: string,
  name: string,
  xml: string
): AxiosPromise<ServiceConfigUpdateResp> {
  return request({
    url: `/map/services/${kind}/${name}`,
    method: 'put',
    data: { xml }
  });
}

// 登记新服务（写 catalog YAML + 热重载）
export function createService(data: Record<string, any>): AxiosPromise<CreateServiceResp> {
  return request({
    url: '/map/services',
    method: 'post',
    data
  });
}

// PostGIS 连接测试
export function testPg(data: { dsn: string }): AxiosPromise<PgTestResp> {
  return request({
    url: '/map/pg-test',
    method: 'post',
    data
  });
}

// 组件探测（运营仪表盘）
export function getComponents(): AxiosPromise<ComponentsResp> {
  return request({
    url: '/map/components',
    method: 'get'
  });
}

// 运行配置（配置/设置页，只读）
export function getRuntime(): AxiosPromise<RuntimeResp> {
  return request({
    url: '/map/runtime',
    method: 'get'
  });
}

// 缓存实时计数器
export function cacheStats(): AxiosPromise<CacheStatsResp> {
  return request({
    url: '/map/cache',
    method: 'get'
  });
}

// 清缓存：scope = all | output | docs | catalog | features | layers
export function cachePurge(scope: string): AxiosPromise<CachePurgeResp> {
  return request({
    url: '/map/cache/purge',
    method: 'post',
    params: { scope }
  });
}

export type { MapServiceVO };

// ================= 服务分组（两级命名空间） =================
// 全部分组（去重非空，形如「一级」或「一级/二级」）
export function listServiceGroups(): AxiosPromise<{ groups: string[] }> {
  return request({
    url: '/map/service-groups',
    method: 'get'
  });
}

// 查服务分组（空 = 根目录）
export function getServiceGroup(kind: string, name: string): AxiosPromise<{ kind: string; name: string; group: string }> {
  return request({
    url: `/map/services/${kind}/${name}/group`,
    method: 'get'
  });
}

// 调整服务分组（group 空串 = 回到根目录）
export function assignServiceGroup(kind: string, name: string, group: string) {
  return request({
    url: `/map/services/${kind}/${name}/group`,
    method: 'put',
    data: { group }
  });
}
