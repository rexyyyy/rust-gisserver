<template>
  <div class="app-container">
    <el-card shadow="never" :header="$t('map.auto.cacheMapApiCache')">
      <el-row :gutter="12" class="mb-3" align="middle">
        <el-col :span="8">
          <el-select v-model="purgeScope" :placeholder="$t('map.auto.text14')" size="default" style="width: 100%">
            <el-option v-for="o in scopeOptions" :key="o.value" :label="o.label" :value="o.value" />
          </el-select>
        </el-col>
        <el-col :span="10">
          <el-button type="warning" :loading="purgeLoading" v-hasPermi="['map:service:list']" @click="onPurge">
            {{ $t('map.auto.cache4') }}
          </el-button>
          <el-button type="primary" :loading="loading" @click="getStats">{{ $t('map.auto.refresh2') }}</el-button>
        </el-col>
        <el-col :span="6" class="text-right">
          <span class="text-sm">TTL：
            <el-tag size="small">{{ stats?.ttl_secs || 0 }} {{ $t('map.auto.second') }}</el-tag>
          </span>
        </el-col>
      </el-row>

      <el-alert
        v-if="purgeResult"
        class="mb-3"
        :type="purgeResult.purged_entries > 0 ? 'success' : 'info'"
        :closable="true"
        show-icon
        :title="`${t('map.auto.text15')}${purgeResult.scope}${t('map.auto.text16')}${purgeResult.purged_entries}${t('map.auto.cache')}${purgeResult.purges}${t('map.auto.text17')}`"
      />

      <el-table :data="rows" v-loading="loading" border size="small">
        <el-table-column prop="label" :label="$t('map.auto.cache2')" width="180" />
        <el-table-column prop="entries" :label="$t('map.auto.text18')" width="120" align="right" />
        <el-table-column :label="$t('map.auto.text19')" width="160" align="right">
          <template #default="scope">{{ formatBytes(scope.row.bytes) }}</template>
        </el-table-column>
        <el-table-column prop="desc" :label="$t('map.auto.text20')" min-width="280" show-overflow-tooltip />
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts" name="MapCache">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { ref, computed, onMounted } from 'vue';
import { cacheStats, cachePurge } from '@/api/map/service';
import { CacheStatsResp, CachePurgeResp } from '@/api/map/service/types';

const loading = ref(false);
const stats = ref<CacheStatsResp | null>(null);
const purgeScope = ref('all');
const purgeLoading = ref(false);
const purgeResult = ref<CachePurgeResp | null>(null);

const scopeOptions = [
  { value: 'all', label: t('map.auto.allcache') },
  { value: 'output', label: t('map.auto.l1L2') },
  { value: 'docs', label: t('map.auto.text21') },
  { value: 'catalog', label: t('map.auto.catalog') },
  { value: 'features', label: t('map.auto.text22') },
  { value: 'layers', label: t('map.auto.layer') }
];

// 缓存字段 → 中文标签 + 说明（对齐 map-api CacheStats 的字段名）
const META: Record<string, { label: string; desc: string }> = {
  output: { label: t('map.auto.text23'), desc: t('map.auto.l1L22') },
  layers: { label: t('map.auto.layer'), desc: t('map.auto.layerRowsHourTTL') },
  docs: { label: t('map.auto.text21'), desc: t('map.auto.capabilitiesTileJSONStyle') },
  catalog: { label: t('map.auto.catalog'), desc: t('map.auto.catalogOf') },
  features: { label: t('map.auto.text22'), desc: t('map.auto.geojson') },
  decoded_tiles: { label: t('map.auto.text24'), desc: t('map.auto.ofTileCache') },
  disk: { label: t('map.auto.l2'), desc: t('map.auto.foyerOutputOfTwoCacheDiskCacheHour') }
};

const rows = computed(() => {
  if (!stats.value) return [];
  const s = stats.value;
  return Object.keys(META)
    .filter((k) => (k === 'disk' ? s.disk != null : true))
    .map((k) => {
      const c = (k === 'disk' ? s.disk! : (s as Record<string, any>)[k]) as { entries: number; bytes: number };
      return {
        key: k,
        label: META[k].label,
        entries: c?.entries ?? 0,
        bytes: c?.bytes ?? 0,
        desc: META[k].desc
      };
    });
});

function formatBytes(n: number): string {
  if (!n) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 2)} ${u[i]}`;
}

async function getStats() {
  loading.value = true;
  try {
    const r = await cacheStats();
    stats.value = ((r as any).data ?? r) as CacheStatsResp;
  } finally {
    loading.value = false;
  }
}

async function onPurge() {
  purgeLoading.value = true;
  try {
    const r = await cachePurge(purgeScope.value);
    purgeResult.value = ((r as any).data ?? r) as CachePurgeResp;
    await getStats();
  } finally {
    purgeLoading.value = false;
  }
}

onMounted(() => getStats());
</script>
