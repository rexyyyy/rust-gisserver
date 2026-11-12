<template>
  <div class="app-container">
    <el-card shadow="never" header="缓存管理（map-api 瓦片 / 元数据缓存）">
      <el-row :gutter="12" class="mb-3" align="middle">
        <el-col :span="8">
          <el-select v-model="purgeScope" placeholder="选择清理范围" size="default" style="width: 100%">
            <el-option v-for="o in scopeOptions" :key="o.value" :label="o.label" :value="o.value" />
          </el-select>
        </el-col>
        <el-col :span="10">
          <el-button type="warning" :loading="purgeLoading" v-hasPermi="['map:service:list']" @click="onPurge">
            清除缓存
          </el-button>
          <el-button type="primary" :loading="loading" @click="getStats">刷新统计</el-button>
        </el-col>
        <el-col :span="6" class="text-right">
          <span class="text-sm">TTL：
            <el-tag size="small">{{ stats?.ttl_secs || 0 }} 秒</el-tag>
          </span>
        </el-col>
      </el-row>

      <el-alert
        v-if="purgeResult"
        class="mb-3"
        :type="purgeResult.purged_entries > 0 ? 'success' : 'info'"
        :closable="true"
        show-icon
        :title="`已清理范围「${purgeResult.scope}」：丢弃 ${purgeResult.purged_entries} 条缓存项（进程累计清理 ${purgeResult.purges} 次）`"
      />

      <el-table :data="rows" v-loading="loading" border size="small">
        <el-table-column prop="label" label="缓存区" width="180" />
        <el-table-column prop="entries" label="条目数" width="120" align="right" />
        <el-table-column label="占用字节" width="160" align="right">
          <template #default="scope">{{ formatBytes(scope.row.bytes) }}</template>
        </el-table-column>
        <el-table-column prop="desc" label="说明" min-width="280" show-overflow-tooltip />
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts" name="MapCache">
import { ref, computed, onMounted } from 'vue';
import { cacheStats, cachePurge } from '@/api/map/service';
import { CacheStatsResp, CachePurgeResp } from '@/api/map/service/types';

const loading = ref(false);
const stats = ref<CacheStatsResp | null>(null);
const purgeScope = ref('all');
const purgeLoading = ref(false);
const purgeResult = ref<CachePurgeResp | null>(null);

const scopeOptions = [
  { value: 'all', label: '全部缓存' },
  { value: 'output', label: '瓦片输出（L1/L2）' },
  { value: 'docs', label: '元数据 / 文档' },
  { value: 'catalog', label: '目录 / 集合' },
  { value: 'features', label: '要素' },
  { value: 'layers', label: '图层索引' }
];

// 缓存字段 → 中文标签 + 说明（对齐 map-api CacheStats 的字段名）
const META: Record<string, { label: string; desc: string }> = {
  output: { label: '瓦片输出', desc: '最终出图瓦片（内存 L1 + 可选磁盘 L2），体积最大' },
  layers: { label: '图层索引', desc: '图层名 → 运行时对象索引（无 TTL）' },
  docs: { label: '元数据 / 文档', desc: 'Capabilities / TileJSON / 样式等渲染文档' },
  catalog: { label: '目录 / 集合', desc: '由 catalog 派生出的集合元数据' },
  features: { label: '要素', desc: '单要素 GeoJSON（要素服务）' },
  decoded_tiles: { label: '解码瓦片', desc: '解码后的瓦片位图（TileCache）' },
  disk: { label: '磁盘 L2 层', desc: 'foyer output 的磁盘二级缓存（仅配置了 disk_cache 时存在）' }
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
