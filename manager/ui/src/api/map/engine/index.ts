import request from '@/utils/request';
import { EngineBody, EngineHealth, EngineListResp, ServiceEngineResp, EngineEventListResp, EngineDetailResp } from './types';
import { AxiosPromise } from 'axios';

// 引擎清单
export function listEngines(): AxiosPromise<EngineListResp> {
  return request({
    url: '/map/engines',
    method: 'get'
  });
}

// 新增引擎
export function addEngine(data: EngineBody) {
  return request({
    url: '/map/engines',
    method: 'post',
    data
  });
}

// 更新引擎（engineId 以 URL 为准）
export function updateEngine(engineId: string, data: EngineBody) {
  return request({
    url: `/map/engines/${engineId}`,
    method: 'put',
    data
  });
}

// 删除引擎（默认引擎删除后代理回落 manager.xml mapsvc）
export function deleteEngine(engineId: string) {
  return request({
    url: `/map/engines/${engineId}`,
    method: 'delete'
  });
}

// 设为默认引擎（并启用）
export function setDefaultEngine(engineId: string) {
  return request({
    url: `/map/engines/${engineId}/default`,
    method: 'put'
  });
}

// 健康探测（/admin/components，退回 /health）
export function engineHealth(engineId: string): AxiosPromise<EngineHealth> {
  return request({
    url: `/map/engines/${engineId}/health`,
    method: 'get'
  });
}

// 查询服务归属引擎（未登记归属时 engineId 为 null，实际走默认引擎）
export function getServiceEngine(kind: string, name: string): AxiosPromise<ServiceEngineResp> {
  return request({
    url: `/map/services/${kind}/${name}/engine`,
    method: 'get'
  });
}

// 调整服务归属引擎（engineId 传空串等同清除归属）
export function assignServiceEngine(kind: string, name: string, engineId: string) {
  return request({
    url: `/map/services/${kind}/${name}/engine`,
    method: 'put',
    data: { engineId }
  });
}

// 清除服务归属（回落默认引擎）
export function clearServiceEngine(kind: string, name: string) {
  return request({
    url: `/map/services/${kind}/${name}/engine`,
    method: 'delete'
  });
}

// 引擎事件日志（manager 后台巡检产生）
export function listEngineEvents(params: { engineId?: string; eventType?: string; limit?: number }): AxiosPromise<EngineEventListResp> {
  return request({
    url: '/map/engines/events',
    method: 'get',
    params
  });
}

// 引擎详情：基本信息 + 存活探测 + 服务器运行环境 + 服务统计 + 最近事件
export function engineDetail(engineId: string): AxiosPromise<EngineDetailResp> {
  return request({
    url: `/map/engines/${engineId}/detail`,
    method: 'get'
  });
}
