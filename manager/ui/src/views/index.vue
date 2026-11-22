<template>
  <div class="app-container home-dashboard">
    <!-- 标题行 -->
    <div class="dash-header">
      <span class="dash-title">{{ $t('misc.auto.text21') }}</span>
      <span class="dash-sub">
        <template v-if="registryMode">{{ $t('misc.auto.text22') }}{{ registryMode === 'pg' ? 'PostgreSQL' : $t('misc.auto.sidecar') }} ·</template>
        map-api
        <el-tag :type="comp.map_api_up ? 'success' : 'danger'" size="small" effect="dark">
          {{ comp.map_api_up ? $t('misc.auto.text23') : $t('misc.auto.text24') }}
        </el-tag>
      </span>
      <el-button text type="primary" icon="Refresh" @click="loadAll">{{ $t('misc.auto.refresh') }}</el-button>
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
        <el-card shadow="never" :header="$t('misc.auto.typeStatus')">
          <el-table :data="kindRows" size="small" border max-height="240">
            <el-table-column prop="kind" :label="$t('misc.auto.type')" width="110">
              <template #default="s">{{ kindLabel(s.row.kind) }}</template>
            </el-table-column>
            <el-table-column prop="count" :label="$t('misc.auto.text25')" width="70" align="center" />
            <el-table-column :label="$t('misc.auto.statusminute')" min-width="150">
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
              <span>{{ $t('misc.auto.top5') }}</span>
              <el-select v-model="period" size="small" style="width: 130px" @change="loadUsage">
                <el-option :label="$t('misc.auto.n24Hour')" value="day" />
                <el-option :label="$t('misc.auto.n7')" value="week" />
                <el-option :label="$t('misc.auto.n30')" value="month" />
                <el-option :label="$t('misc.auto.oneyear')" value="year" />
              </el-select>
            </div>
          </template>
          <div class="text-2xl font-bold mb-2">
            {{ totalHits }} <span class="text-xs font-normal text-gray-400">{{ $t('misc.auto.text26') }}</span>
          </div>
          <el-table :data="topRows" size="small" border max-height="190">
            <el-table-column prop="name" :label="$t('misc.auto.text27')" min-width="120" show-overflow-tooltip />
            <el-table-column :label="$t('misc.auto.type')" width="90">
              <template #default="s">{{ kindLabel(s.row.kind) }}</template>
            </el-table-column>
            <el-table-column prop="hits" :label="$t('misc.auto.text28')" width="80" align="center" sortable />
          </el-table>
        </el-card>
      </el-col>
    </el-row>

    <!-- ③ 访问日志 / 样式库 · 组件探测 · 缓存 / 垃圾箱 -->
    <el-row :gutter="12" class="mt-3">
      <el-col :xs="24" :lg="8">
        <el-card shadow="never" :header="$t('misc.auto.dayStyle')">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item :label="$t('misc.auto.text29')">
              <el-link type="primary" :underline="false" @click="go('/map/log')">{{ logStats.total }}</el-link>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.client4xx')">
              <span :style="logStats.client_4xx ? 'color:#e6a23c' : ''">{{ logStats.client_4xx }}</span>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.n5xx')">
              <span :style="logStats.server_5xx ? 'color:#f56c6c' : ''">{{ logStats.server_5xx }}</span>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.style')">
              <el-link type="primary" :underline="false" @click="go('/map/style')">{{ styleCount }} {{ $t('misc.auto.text30') }}</el-link>
            </el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="8">
        <el-card shadow="never" :header="$t('misc.auto.mapApiRows')">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item label="PostGIS">
              <el-tag :type="comp.pg_enabled ? 'success' : 'info'" size="small">{{ comp.pg_enabled ? $t('misc.auto.enable') : $t('misc.auto.enable2') }}</el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.proj')">
              <el-tag :type="comp.proj_loaded ? 'success' : 'info'" size="small">{{ comp.proj_loaded ? $t('misc.auto.text31') : $t('misc.auto.text32') }}</el-tag>
              <el-tooltip :content="comp.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
              </el-tooltip>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.inServiceLayer')">{{ comp.served_layers }}</el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.cache')">{{ comp.cache_mib }} MiB / TTL {{ comp.cache_ttl_secs }} {{ $t('misc.auto.second') }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <el-col :xs="24" :lg="8">
        <el-card shadow="never" :header="$t('misc.auto.cache2')">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item :label="$t('misc.auto.cache3')">
              <el-link type="primary" :underline="false" @click="go('/map/cache')">{{ cacheEntries }}</el-link>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.cache4')">{{ cacheBytesText }}</el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.text33')">
              <el-link type="primary" :underline="false" @click="go('/map/trash')">{{ deletedCount }} {{ $t('misc.auto.text34') }}</el-link>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('misc.auto.config')">
              <el-link type="primary" :underline="false" @click="go('/map/settings')">{{ $t('misc.auto.rows') }}</el-link>
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
              <span>{{ $t('misc.auto.text35') }}</span>
              <el-link type="primary" :underline="false" @click="go('/map/engine')">{{ $t('misc.auto.text36') }}</el-link>
            </div>
          </template>
          <el-table :data="engineRows" size="small" border max-height="220">
            <el-table-column prop="engineName" :label="$t('misc.auto.text37')" min-width="130" show-overflow-tooltip />
            <el-table-column :label="$t('misc.auto.text23')" width="80" align="center">
              <template #default="s">
                <el-tag :type="s.row.up ? 'success' : 'danger'" size="small" effect="dark">
                  {{ s.row.up ? $t('misc.auto.text23') : $t('misc.auto.text24') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="count" :label="$t('misc.auto.text38')" width="80" align="center" />
            <el-table-column :label="$t('misc.auto.role')" width="90" align="center">
              <template #default="s">
                <el-tag v-if="s.row.isDefault" size="small" type="warning" effect="plain">{{ $t('misc.auto.default') }}</el-tag>
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
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
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
    title: t('misc.auto.text39'),
    value: services.value.filter((s) => s.state !== 'deleted').length,
    foot: `${t('misc.auto.inService')}${servingCount.value}${t('misc.auto.text40')}${activeCount.value}`,
    path: '/map/service',
    color: '#0092C8'
  },
  {
    title: t('misc.auto.n24h'),
    value: totalHits.value,
    foot: t('misc.auto.text41'),
    path: '/map/dashboard'
  },
  {
    title: '3D Tiles',
    value: tilesServing.value,
    sub: ` / ${tiles3d.value.length}`,
    foot: t('misc.auto.inService2'),
    path: '/map/tiles'
  },
  {
    title: t('misc.auto.text42'),
    value: engineStats.value.filter((e) => e.up).length,
    sub: ` / ${engineRows.value.length}`,
    foot: t('misc.auto.text43'),
    path: '/map/engine'
  },
  {
    title: t('misc.auto.cache4'),
    value: cacheBytesText.value,
    foot: `${cacheEntries.value}${t('misc.auto.text44')}`,
    path: '/map/cache'
  },
  {
    title: t('misc.auto.text33'),
    value: deletedCount.value,
    foot: deletedCount.value ? t('misc.auto.text45') : t('misc.auto.text46'),
    path: '/map/trash',
    color: deletedCount.value ? '#e6a23c' : undefined
  }
]);

// ---------- 分类/状态标签 ----------
const kindLabel = (k: string) => {
  const map: Record<string, string> = {
    xyz: t('misc.auto.xyz'),
    tms: t('misc.auto.tms'),
    cog: t('misc.auto.cogRaster'),
    vector: t('misc.auto.vector'),
    postgis: 'PostGIS',
    mbtiles: 'MBTiles',
    pmtiles: 'PMTiles',
    arcgis: t('misc.auto.arcgisTiles'),
    '3dtiles': '3D Tiles'
  };
  return map[k] || k;
};
const stateLabel = (s: string) => {
  const map: Record<string, string> = { active: t('misc.auto.text47'), stopped: t('misc.auto.text48'), paused: t('misc.auto.text49'), deleted: t('misc.auto.delete') };
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
// 后端所有接口统一返回 { code, msg, data } 包装层（见 utils/request.ts 响应拦截器），
// 首页各块必须先从 data 取值，否则字段全是 undefined，页面会一直显示离线/0。
function unwrap<T = any>(r: any): T {
  const v = r && typeof r === 'object' && 'code' in r && 'data' in r ? r.data : r;
  return (v ?? {}) as T;
}

async function loadServices() {
  try {
    const info: any = unwrap(await listServices());
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
    comp.value = unwrap(await getComponents()) as unknown as ComponentsResp;
  } catch {
    /* map-api 离线时保持默认 */
  }
}

async function loadUsage() {
  try {
    const u: any = unwrap(await usage({ period: period.value, group_by: 'name' }));
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
    const r: any = unwrap(await listAccessLogs({ limit: 1 }));
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
    const r: any = unwrap(await listStyles());
    styleCount.value = r.count ?? (r.styles || []).length;
  } catch {
    styleCount.value = 0;
  }
}

async function loadCache() {
  try {
    cache.value = unwrap(await cacheStats()) as unknown as CacheStatsResp;
  } catch {
    cache.value = null;
  }
}

async function loadEngines() {
  try {
    const r: any = unwrap(await listEngines());
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
