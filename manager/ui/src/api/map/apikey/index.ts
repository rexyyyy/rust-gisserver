import request from '@/utils/request';
import { AxiosPromise } from 'axios';
import { ApiKeyApplyResp, ApiKeyBody, ApiKeyListResp, ApiKeyVO } from './types';

// API Key 清单（含明文，供管理端复制分发）
export function listApiKeys(): AxiosPromise<ApiKeyListResp> {
  return request({ url: '/map/apikeys', method: 'get' });
}

// 生成新 Key（返回完整明文，仅生成时醒目展示一次）
export function createApiKey(data: ApiKeyBody): AxiosPromise<ApiKeyVO> {
  return request({ url: '/map/apikeys', method: 'post', data });
}

// 编辑名称 / 用途 / 状态 / 有效期 / 备注（不改 Key 本身）
export function updateApiKey(keyId: string, data: ApiKeyBody): AxiosPromise<ApiKeyVO> {
  return request({ url: `/map/apikeys/${keyId}`, method: 'put', data });
}

// 删除 Key
export function deleteApiKey(keyId: string): AxiosPromise<any> {
  return request({ url: `/map/apikeys/${keyId}`, method: 'delete' });
}

// 重置 Key（旧 Key 立即失效）
export function rotateApiKey(keyId: string): AxiosPromise<ApiKeyVO> {
  return request({ url: `/map/apikeys/${keyId}/rotate`, method: 'post' });
}

// 应用：admin 写入默认引擎 admin_token；data 返回客户端下发用法
export function applyApiKey(keyId: string): AxiosPromise<ApiKeyApplyResp> {
  return request({ url: `/map/apikeys/${keyId}/apply`, method: 'post' });
}
