/** API Key 用途：admin=manager 连接 server 管理面；data=客户端访问数据面 */
export type ApiKeyPurpose = 'admin' | 'data';

/** 后端按用途生成的使用说明（可直接复制到客户端 / 网关） */
export interface ApiKeyUsage {
  header: string;
  sample: string;
  bearer?: string;
  query?: string;
  note: string;
}

export interface ApiKeyVO {
  keyId: string;
  /** 名称（唯一，便于识别持有人/用途） */
  keyName: string;
  /** 明文 Key（管理端可复制；页面默认打码显示） */
  apiKey: string;
  purpose: ApiKeyPurpose;
  purposeName: string;
  /** 0 启用 / 1 停用 */
  status: string;
  /** YYYY-MM-DD HH:mm:ss，空串 = 永不过期 */
  expiresAt: string;
  expired: boolean;
  /** 当前是否真正有效（启用且未过期） */
  effective: boolean;
  remark: string;
  createBy: string;
  createTime: string;
  updateTime: string;
  usage: ApiKeyUsage;
}

export interface ApiKeyListResp {
  keys: ApiKeyVO[];
  total: number;
}

export interface ApiKeyBody {
  keyName: string;
  purpose: ApiKeyPurpose;
  status: string;
  expiresAt?: string;
  remark: string;
}

/** 应用结果：admin 写入默认引擎；data 仅返回下发用法 */
export interface ApiKeyApplyResp {
  applied: boolean;
  engineId?: string;
  keyName: string;
  usage: ApiKeyUsage;
}
