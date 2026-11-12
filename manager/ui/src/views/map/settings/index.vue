<template>
  <div class="app-container">
    <el-row :gutter="16">
      <el-col :span="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>map-api 运行时状态</span>
              <el-tag :type="comp.map_api_up ? 'success' : 'danger'" size="small">
                {{ comp.map_api_up ? '在线' : '离线' }}
              </el-tag>
            </div>
          </template>
          <el-descriptions :column="1" border v-loading="loading">
            <el-descriptions-item label="PostgreSQL">
              <el-tag :type="comp.pg_enabled ? 'success' : 'danger'" size="small">
                {{ comp.pg_enabled ? '已启用' : '未启用' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="PROJ 投影">
              <el-tag :type="comp.proj_loaded ? 'success' : 'warning'" size="small">
                {{ comp.proj_loaded ? '已加载' : '未加载(跨 CRS 不可用)' }}
              </el-tag>
              <el-tooltip :content="comp.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
              </el-tooltip>
            </el-descriptions-item>
            <el-descriptions-item label="catalog 根目录">
              {{ comp.catalog_root || '—' }}
              <el-tag :type="comp.catalog_root_exists ? 'success' : 'danger'" size="small" style="margin-left: 8px">
                {{ comp.catalog_root_exists ? '存在' : '缺失' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="瓦片集目录">
              {{ comp.tilesets_dir || '—' }}
              <el-tag :type="comp.tilesets_dir_exists ? 'success' : 'danger'" size="small" style="margin-left: 8px">
                {{ comp.tilesets_dir_exists ? '存在' : '缺失' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="在服图层">{{ comp.served_layers ?? '—' }}</el-descriptions-item>
            <el-descriptions-item label="3D Tiles 服务">{{ comp.tilesets ?? '—' }}</el-descriptions-item>
            <el-descriptions-item label="缓存容量(MiB)">{{ comp.cache_mib ?? '—' }}</el-descriptions-item>
            <el-descriptions-item label="缓存 TTL(秒)">{{ comp.cache_ttl_secs ?? '—' }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <el-col :span="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>manager 运行配置</span>
              <el-tag type="info" size="small">只读</el-tag>
            </div>
          </template>
          <el-descriptions :column="1" border v-loading="loading">
            <el-descriptions-item label="监听地址">{{ rt.server_addr || '—' }}</el-descriptions-item>
            <el-descriptions-item label="API 前缀">{{ rt.base_api || '—' }}</el-descriptions-item>
            <el-descriptions-item label="CORS 白名单">
              {{ (rt.cors_origins && rt.cors_origins.length) ? rt.cors_origins.join(', ') : '—' }}
            </el-descriptions-item>
            <el-descriptions-item label="数据库连接(掩码)">
              {{ rt.database_url || '—' }}
            </el-descriptions-item>
            <el-descriptions-item label="连接池上限">{{ rt.pool_max ?? '—' }}</el-descriptions-item>
            <el-descriptions-item label="map-api 基地址">{{ rt.mapsvc_base_url || '—' }}</el-descriptions-item>
            <el-descriptions-item label="共享密钥">
              <el-tag :type="rt.mapsvc_token_configured ? 'success' : 'info'" size="small">
                {{ rt.mapsvc_token_configured ? '已配置' : '未配置(留空)' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="Token 有效期(分)">{{ rt.jwt_expire_minutes ?? '—' }}</el-descriptions-item>
            <el-descriptions-item label="登录验证码">
              <el-tag :type="rt.captcha_enabled ? 'success' : 'warning'" size="small">
                {{ rt.captcha_enabled ? '启用' : '关闭' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="头像存储目录">{{ rt.avatar_dir || '—' }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>
    </el-row>

    <el-alert
      type="info"
      :closable="false"
      style="margin-top: 16px"
      title="说明"
      description="当前页面为只读展示。manager 与 map-api 的连接地址、数据库连接、密钥等仍由后端配置文件（config/manager.xml、config/mapserver.xml）管理，暂未提供在线编辑。"
    />
  </div>
</template>

<script setup lang="ts" name="MapSettings">
import { onMounted, ref } from 'vue';
import { getComponents, getRuntime } from '@/api/map/service';
import { ComponentsResp, RuntimeResp } from '@/api/map/service/types';
import { CRS_SUPPORT_HINT, PROJ_MISSING_HINT } from '@/utils/crs';

const loading = ref(false);
const comp = ref<Partial<ComponentsResp>>({});
const rt = ref<Partial<RuntimeResp>>({});

const getList = async () => {
  loading.value = true;
  try {
    const [c, r] = await Promise.all([getComponents(), getRuntime()]);
    comp.value = (c as any).data || {};
    rt.value = (r as any).data || {};
  } finally {
    loading.value = false;
  }
};

onMounted(() => {
  getList();
});
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
