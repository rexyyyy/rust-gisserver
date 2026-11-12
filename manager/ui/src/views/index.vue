<template>
  <div class="app-container home-dashboard">
    <!-- 标题行 -->
    <div class="dash-header">
      <span class="dash-title">地图服务总览</span>
      <span class="dash-sub">
        <template v-if="registryMode">登记簿 {{ registryMode === 'pg' ? 'PostgreSQL' : '文件 sidecar' }} ·</template>
        map-api
        <el-tag :type="comp.map_api_up ? 'success' : 'danger'" size="small" effect="dark">
          {{ comp.map_api_up ? '在线' : '离线' }}
        </el-tag>
      </span>
      <el-button text type="primary" icon="Refresh" @click="loadAll">刷新</el-button>
    </div>

    <!-- ① 核心指标卡：点击跳转对应菜单 -->
    <el-row :gutter="12">
      <el-col :xs="12" :sm="8" :lg="4" v-for="c in statCards" :key="c.title">
        <el-card shadow="hover" class="stat-card" @click="go(c.path)">
          <div class="stat-label">
            {{ c.title }}
            <el-icon class="stat-arrow"><ArrowRight /></el-icon>
          </div>
          <div class="stat-value" :style="{ color: c.color || 'inherit' }">
            {{ c.value }}<span v-if="c.sub" class="stat-sub">{{ c.sub }}</span>
          </div>
          <div class="stat-foot">{{ c.foot }}</div>
        </el-card>
      </el-col>
    </el-row>

    <!-- ② 服务统计 / 访问计数 -->
    <el-row :gutter="12" class="mt-3">
      <el-col :xs="24" :lg="12">
        <el-card shadow="never" header="服务统计（类型 × 状态）">
          <el-table :data="kindRows" size="small" border max-height="240">
            <el-table-column prop="kind" label="类型" width="110">
              <template #default="s">{{ kindLabel(s.row.kind) }}</template>
            </el-table-column>
            <el-table-column prop="count" label="数量" width="70" align="center" />
            <el-table-column label="状态分布" min-width="150">
              <template #default="s">
                <el-tag
                  v-for="(n, st) in s.row.states"
                  :key="st"
                  size="small"
                  class="mr-1"
                  :type="st === 'active' ? 'success' : st === 'deleted' ? 'danger' : 'warning'"
                >{{ stateLabel(st as string) }}:{{ n }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="12">
        <el-card shadow="never">
          <template #header>
            <div class="card-header">
              <span>访问计数（Top 5）</span>
              <el-select v-model="period" size="small" style="width: 130px" @change="loadUsage">
                <el-option label="近 24 小时" value="day" />
                <el-option label="近 7 天" value="week" />
                <el-option label="近 30 天" value="month" />
                <el-option label="近一年" value="year" />
              </el-select>
            </div>
          </template>
          <div class="text-2xl font-bold mb-2">
            {{ totalHits }} <span class="text-xs font-normal text-gray-400">次请求</span>
          </div>
          <el-table :data="topRows" size="small" border max-height="190">
            <el-table-column prop="name" label="服务" min-width="120" show-overflow-tooltip />
            <el-table-column label="类型" width="90">
              <template #default="s">{{ kindLabel(s.row.kind) }}</template>
            </el-table-column>
            <el-table-column prop="hits" label="命中" width="80" align="center" sortable />
          </el-table>
        </el-card>
      </el-col>
    </el-row>

    <!-- ③ 访问日志 / 样式库 · 组件探测 · 缓存 / 垃圾箱 -->
    <el-row :gutter="12" class="mt-3">
      <el-col :xs="24" :lg="8">
        <el-card shadow="never" header="访问日志 / 样式库">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item label="累计请求">
              <el-link type="primary" :underline="false" @click="go('/map/log')">{{ logStats.total }}</el-link>
            </el-descriptions-item>
            <el-descriptions-item label="客户端 4xx">
              <span :style="logStats.client_4xx ? 'color:#e6a23c' : ''">{{ logStats.client_4xx }}</span>
            </el-descriptions-item>
            <el-descriptions-item label="服务端 5xx">
              <span :style="logStats.server_5xx ? 'color:#f56c6c' : ''">{{ logStats.server_5xx }}</span>
            </el-descriptions-item>
            <el-descriptions-item label="样式库">
              <el-link type="primary" :underline="false" @click="go('/map/style')">{{ styleCount }} 个</el-link>
            </el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="8">
        <el-card shadow="never" header="组件探测（map-api 运行环境）">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item label="PostGIS">
              <el-tag :type="comp.pg_enabled ? 'success' : 'info'" size="small">{{ comp.pg_enabled ? '已启用' : '未启用' }}</el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="PROJ 重投影">
              <el-tag :type="comp.proj_loaded ? 'success' : 'info'" size="small">{{ comp.proj_loaded ? '已加载' : '未加载' }}</el-tag>
              <el-tooltip :content="comp.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
              </el-tooltip>
            </el-descriptions-item>
            <el-descriptions-item label="在服图层">{{ comp.served_layers }}</el-descriptions-item>
            <el-descriptions-item label="缓存上限">{{ comp.cache_mib }} MiB / TTL {{ comp.cache_ttl_secs }} 秒</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="8">
        <el-card shadow="never" header="缓存 / 垃圾箱">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item label="缓存条目">
              <el-link type="primary" :underline="false" @click="go('/map/cache')">{{ cacheEntries }}</el-link>
            </el-descriptions-item>
            <el-descriptions-item label="缓存占用">{{ cacheBytesText }}</el-descriptions-item>
            <el-descriptions-item label="服务垃圾箱">
              <el-link type="primary" :underline="false" @click="go('/map/trash')">{{ deletedCount }} 个待恢复</el-link>
            </el-descriptions-item>
            <el-descriptions-item label="参数设置">
              <el-link type="primary" :underline="false" @click="go('/map/settings')">运行配置</el-link>
            </el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>
    </el-row>

    <!-- ④ 服务引擎（独占一行） -->
    <el-row :gutter="12" class="mt-3">
      <el-col :xs="24" :lg="24">
        <el-card shadow="never">
          <template #header>
            <div class="card-header">
              <span>服务引擎（在线 / 服务数）</span>
              <el-link type="primary" :underline="false" @click="go('/map/engine')">引擎管理</el-link>
            </div>
          </template>
          <el-table :data="engineRows" size="small" border max-height="220">
            <el-table-column prop="engineName" label="引擎" min-width="130" show-overflow-tooltip />
            <el-table-column label="在线" width="80" align="center">
              <template #default="s">
                <el-tag :type="s.row.up ? 'success' : 'danger'" size="small" effect="dark">
                  {{ s.row.up ? '在线' : '离线' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="count" label="服务数" width="80" align="center" />
            <el-table-column label="角色" width="90" align="center">
              <template #default="s">
                <el-tag v-if="s.row.isDefault" size="small" type="warning" effect="plain">默认</el-tag>
                <span v-else>—</span>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup name="Index" lang="ts">
import { ref, computed, onMounted } from 'vue';
import { useRouter } from 'vue-router';
// ArrowRight 由全局注册的 Element Plus Icons 提供
import { CRS_SUPPORT_HINT, PROJ_MISSING_HINT } from '@/utils/crs';
import {
  listServices,
  usage,
  getComponents,
  listAccessLogs,
  listStyles,
  cacheStats
} from '@/api/map/service';
import { listEngines } from '@/api/map/engine';
import type {
  MapServiceVO,
  ComponentsResp,
  UsageRow,
  CacheStatsResp
} from '@/api/map/service/types';
import type { EngineVO } from '@/api/map/engine/types';

const router = useRouter();

// ---------- 服务清单（聚合响应：服务 + 引擎统计） ----------
const services = ref<MapServiceVO[]>([]);
const engineStats = ref<{ engineId: string; engineName: string; up: boolean; count: number }[]>([]);
const registryMode = ref('');

const activeCount = computed(() => services.value.filter((s) => s.state === 'active').length);
const servingCount = computed(() => services.value.filter((s) => s.serving).length);
const deletedCount = computed(() => services.value.filter((s) => s.state === 'deleted').length);
const tiles3d = computed(() => services.value.filter((s) => s.kind === '3dtiles'));
const tilesServing = computed(() => tiles3d.value.filter((s) => s.serving).length);

// ---------- 组件探测 ----------
const comp = ref<ComponentsResp>({
  map_api_up: false,
  pg_enabled: false,
  proj_loaded: false,
  catalog_root: '',
  catalog_root_exists: false,
  tilesets_dir: '',
  tilesets_dir_exists: false,
  served_layers: 0,
  tilesets: 0,
  cache_mib: 0,
  cache_ttl_secs: 0
});

// ---------- 访问计数 ----------
const period = ref('day');
const totalHits = ref(0);
const topRows = ref<UsageRow[]>([]);

// ---------- 访问日志 / 样式 / 缓存 ----------
const logStats = ref({ total: 0, client_4xx: 0, server_5xx: 0 });
const styleCount = ref(0);
const cache = ref<CacheStatsResp | null>(null);

const cacheEntries = computed(() => {
  const s: any = cache.value;
  if (!s) return 0;
  return ['output', 'layers', 'docs', 'catalog', 'features', 'decoded_tiles'].reduce(
    (a, k) => a + (s[k]?.entries || 0),
    0
  );
});
const cacheBytes = computed(() => {
  const s: any = cache.value;
  if (!s) return 0;
  let bytes = s.output?.bytes || 0;
  if (s.disk?.bytes) bytes += s.disk.bytes;
  return bytes;
});
const cacheBytesText = computed(() => {
  const n = cacheBytes.value;
  if (!n) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 2)} ${u[i]}`;
});

// ---------- 引擎行：聚合统计 + 注册表（默认标记） ----------
const engineList = ref<EngineVO[]>([]);
const engineRows = computed(() => {
  const defId = engineList.value.find((e) => e.isDefault === '1')?.engineId;
  const rows = engineStats.value.map((s) => ({ ...s, isDefault: s.engineId === defId }));
  // 注册表中登记但聚合未返回的引擎（如离线且无服务快照）
  for (const e of engineList.value) {
    if (!rows.find((r) => r.engineId === e.engineId)) {
      rows.push({ engineId: e.engineId, engineName: e.engineName, up: false, count: 0, isDefault: e.isDefault === '1' });
    }
  }
  return rows;
});

// ---------- 指标卡 ----------
const statCards = computed(() => [
  {
    title: '登记服务',
    value: services.value.filter((s) => s.state !== 'deleted').length,
    foot: `在服 ${servingCount.value} · 在用 ${activeCount.value}`,
    path: '/map/service',
    color: '#0092C8'
  },
  {
    title: '访问请求（近24h）',
    value: totalHits.value,
    foot: '点击进入运营仪表盘',
    path: '/map/dashboard'
  },
  {
    title: '3D Tiles',
    value: tilesServing.value,
    sub: ` / ${tiles3d.value.length}`,
    foot: '在服 / 总数',
    path: '/map/tiles'
  },
  {
    title: '服务引擎',
    value: engineStats.value.filter((e) => e.up).length,
    sub: ` / ${engineRows.value.length}`,
    foot: '在线 / 已登记',
    path: '/map/engine'
  },
  {
    title: '缓存占用',
    value: cacheBytesText.value,
    foot: `${cacheEntries.value} 条 · 点击管理`,
    path: '/map/cache'
  },
  {
    title: '服务垃圾箱',
    value: deletedCount.value,
    foot: deletedCount.value ? '有待恢复服务' : '暂无待恢复服务',
    path: '/map/trash',
    color: deletedCount.value ? '#e6a23c' : undefined
  }
]);

// ---------- 分类/状态标签 ----------
const kindLabel = (k: string) => {
  const map: Record<string, string> = {
    xyz: 'XYZ瓦片',
    tms: 'TMS瓦片',
    cog: 'COG栅格',
    vector: '矢量',
    postgis: 'PostGIS',
    mbtiles: 'MBTiles',
    pmtiles: 'PMTiles',
    arcgis: 'ArcGIS瓦片',
    '3dtiles': '3D Tiles'
  };
  return map[k] || k;
};
const stateLabel = (s: string) => {
  const map: Record<string, string> = { active: '在用', stopped: '停止', paused: '暂停', deleted: '删除' };
  return map[s] || s;
};

// ---------- 类型 × 状态行 ----------
const kindRows = computed(() => {
  const byKind = new Map<string, { count: number; states: Record<string, number> }>();
  for (const s of services.value) {
    const e = byKind.get(s.kind) || { count: 0, states: {} };
    e.count += 1;
    e.states[s.state] = (e.states[s.state] || 0) + 1;
    byKind.set(s.kind, e);
  }
  return Array.from(byKind.entries()).map(([kind, v]) => ({ kind, ...v }));
});

// ---------- 数据加载（各项独立容错，失败显示为空不阻塞首页） ----------
async function loadServices() {
  try {
    const { data } = await listServices();
    const info: any = data.data || data;
    services.value = info.services || [];
    engineStats.value = info.engines || [];
    registryMode.value = info.registry_mode || '';
  } catch {
    services.value = [];
    engineStats.value = [];
  }
}

async function loadComponents() {
  try {
    const { data } = await getComponents();
    comp.value = (data.data || data) as ComponentsResp;
  } catch {
    /* map-api 离线时保持默认 */
  }
}

async function loadUsage() {
  try {
    const { data } = await usage({ period: period.value, group_by: 'name' });
    const u: any = data.data || data;
    totalHits.value = (u.rows || []).reduce((a: number, r: UsageRow) => a + (r.hits || 0), 0);
    topRows.value = [...(u.rows || [])]
      .sort((a: UsageRow, b: UsageRow) => (b.hits || 0) - (a.hits || 0))
      .slice(0, 5);
  } catch {
    totalHits.value = 0;
    topRows.value = [];
  }
}

async function loadLogs() {
  try {
    const { data } = await listAccessLogs({ limit: 1 });
    const r: any = data.data || data;
    logStats.value = {
      total: r.total || 0,
      client_4xx: r.errors?.client_4xx || 0,
      server_5xx: r.errors?.server_5xx || 0
    };
  } catch {
    logStats.value = { total: 0, client_4xx: 0, server_5xx: 0 };
  }
}

async function loadStyles() {
  try {
    const { data } = await listStyles();
    const r: any = data.data || data;
    styleCount.value = r.count ?? (r.styles || []).length;
  } catch {
    styleCount.value = 0;
  }
}

async function loadCache() {
  try {
    const { data } = await cacheStats();
    cache.value = (data.data || data) as CacheStatsResp;
  } catch {
    cache.value = null;
  }
}

async function loadEngines() {
  try {
    const { data } = await listEngines();
    const r: any = data.data || data;
    engineList.value = r.engines || [];
  } catch {
    engineList.value = [];
  }
}

const go = (path: string) => {
  router.push(path);
};

const loadAll = () => {
  loadServices();
  loadComponents();
  loadUsage();
  loadLogs();
  loadStyles();
  loadCache();
  loadEngines();
};

onMounted(loadAll);
</script>

<style lang="scss" scoped>
.home-dashboard {
  .dash-header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;

    .dash-title {
      font-size: 18px;
      font-weight: 600;
    }

    .dash-sub {
      color: var(--el-text-color-secondary);
      font-size: 13px;
      display: inline-flex;
      align-items: center;
      gap: 4px;
    }
  }

  .stat-card {
    cursor: pointer;
    margin-bottom: 12px;

    .stat-label {
      display: flex;
      align-items: center;
      justify-content: space-between;
      color: var(--el-text-color-secondary);
      font-size: 13px;
      margin-bottom: 6px;
    }

    .stat-arrow {
      color: var(--el-text-color-placeholder);
    }

    .stat-value {
      font-size: 24px;
      font-weight: 600;
      line-height: 1.2;
    }

    .stat-sub {
      font-size: 14px;
      color: var(--el-text-color-secondary);
      font-weight: 400;
    }

    .stat-foot {
      margin-top: 6px;
      color: var(--el-text-color-placeholder);
      font-size: 12px;
    }

    &:hover .stat-arrow {
      color: var(--el-color-primary);
    }
  }

  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .mr-1 {
    margin-right: 4px;
  }

  .mt-3 {
    margin-top: 12px;
  }

  .mb-2 {
    margin-bottom: 8px;
  }

  .text-2xl {
    font-size: 24px;
  }

  .text-xs {
    font-size: 12px;
  }

  .font-bold {
    font-weight: 600;
  }

  .text-gray-400 {
    color: var(--el-text-color-placeholder);
  }
}
</style>
