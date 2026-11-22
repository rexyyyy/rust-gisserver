<template>
  <div class="app-container">
    <el-row :gutter="16">
      <el-col :span="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ $t('map.auto.mapApiRowsHourStatus') }}</span>
              <el-tag :type="comp.map_api_up ? 'success' : 'danger'" size="small">
                {{ comp.map_api_up ? $t('map.auto.text102') : $t('map.auto.text103') }}
              </el-tag>
            </div>
          </template>
          <el-descriptions :column="1" border v-loading="loading">
            <el-descriptions-item label="PostgreSQL">
              <el-tag :type="comp.pg_enabled ? 'success' : 'danger'" size="small">
                {{ comp.pg_enabled ? $t('map.auto.enable2') : $t('map.auto.enable3') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.proj3')">
              <el-tag :type="comp.proj_loaded ? 'success' : 'warning'" size="small">
                {{ comp.proj_loaded ? $t('map.auto.text104') : $t('map.auto.crs10') }}
              </el-tag>
              <el-tooltip :content="comp.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
              </el-tooltip>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.catalogCatalog')">
              {{ comp.catalog_root || '—' }}
              <el-tag :type="comp.catalog_root_exists ? 'success' : 'danger'" size="small" style="margin-left: 8px">
                {{ comp.catalog_root_exists ? $t('map.auto.text119') : $t('map.auto.text120') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.catalog8')">
              {{ comp.tilesets_dir || '—' }}
              <el-tag :type="comp.tilesets_dir_exists ? 'success' : 'danger'" size="small" style="margin-left: 8px">
                {{ comp.tilesets_dir_exists ? $t('map.auto.text119') : $t('map.auto.text120') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.inServiceLayer')">{{ comp.served_layers ?? '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.n3dTiles')">{{ comp.tilesets ?? '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.cacheMiB2')">{{ comp.cache_mib ?? '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.cacheTTLSecond')">{{ comp.cache_ttl_secs ?? '—' }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <el-col :span="12">
        <el-card shadow="hover">
          <template #header>
            <div class="card-header">
              <span>{{ $t('map.auto.managerRows') }}</span>
              <el-tag type="info" size="small">{{ $t('map.auto.text141') }}</el-tag>
            </div>
          </template>
          <el-descriptions :column="1" border v-loading="loading">
            <el-descriptions-item :label="$t('map.auto.address')">{{ rt.server_addr || '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.api')">{{ rt.base_api || '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.cors')">
              {{ (rt.cors_origins && rt.cors_origins.length) ? rt.cors_origins.join(', ') : '—' }}
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.text142')">
              {{ rt.database_url || '—' }}
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.text143')">{{ rt.pool_max ?? '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.mapApiAddress')">{{ rt.mapsvc_base_url || '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.text144')">
              <el-tag :type="rt.mapsvc_token_configured ? 'success' : 'info'" size="small">
                {{ rt.mapsvc_token_configured ? $t('map.auto.text145') : $t('map.auto.text146') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.tokenMinute')">{{ rt.jwt_expire_minutes ?? '—' }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.text147')">
              <el-tag :type="rt.captcha_enabled ? 'success' : 'warning'" size="small">
                {{ rt.captcha_enabled ? $t('map.auto.enable') : $t('map.auto.close') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.catalog9')">{{ rt.avatar_dir || '—' }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>
    </el-row>

    <el-alert
      type="info"
      :closable="false"
      style="margin-top: 16px"
      :title="$t('map.auto.text20')"
      :description="$t('map.auto.managerMapApiOfAddressConfigManagerXmlConf')"
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
