<template>
  <div class="app-container">
    <el-row :gutter="16">
      <!-- ① 服务统计：类型 × 状态 -->
      <el-col :span="8">
        <el-card shadow="never" :header="$t('map.auto.typeStatus')">
          <el-table :data="kindRows" size="small" border height="260">
            <el-table-column prop="kind" :label="$t('map.auto.type')" width="110" />
            <el-table-column prop="count" :label="$t('map.auto.text101')" width="80" align="center" />
            <el-table-column :label="$t('map.auto.statusminute')" min-width="160">
              <template #default="scope">
                <el-tag
                  v-for="(n, s) in scope.row.states"
                  :key="s"
                  size="small"
                  class="mr-1"
                  :type="s === 'active' ? 'success' : s === 'deleted' ? 'danger' : 'warning'"
                >{{ s }}:{{ n }}</el-tag>
              </template>
            </el-table-column>
          </el-table>
          <div class="mt-2 text-sm">
            {{ $t('map.auto.text193') }}<b>{{ totalServices }}</b> {{ $t('map.auto.text194') }}·
            {{ $t('map.auto.inService2') }}<b>{{ activeCount }}</b> ·
            {{ $t('map.auto.text195') }}<b>{{ deletedCount }}</b>
          </div>
        </el-card>
      </el-col>

      <!-- ② 组件探测 -->
      <el-col :span="8">
        <el-card shadow="never" :header="$t('map.auto.mapApiRows')">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item :label="$t('map.auto.mapApi')">
              <el-tag :type="comp.map_api_up ? 'success' : 'danger'">
                {{ comp.map_api_up ? $t('map.auto.text102') : $t('map.auto.text103') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="PostGIS">
              <el-tag :type="comp.pg_enabled ? 'success' : 'info'">
                {{ comp.pg_enabled ? $t('map.auto.enable2') : $t('map.auto.enable3') }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.proj')">
              <el-tag :type="comp.proj_loaded ? 'success' : 'info'">
                {{ comp.proj_loaded ? $t('map.auto.text104') : $t('map.auto.text105') }}
              </el-tag>
              <el-tooltip :content="comp.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
              </el-tooltip>
            </el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.inServiceLayer')">{{ comp.served_layers }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.n3dTiles')">{{ comp.tilesets }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.cacheMiB')">{{ comp.cache_mib }}</el-descriptions-item>
            <el-descriptions-item :label="$t('map.auto.cacheTTLSecond')">{{ comp.cache_ttl_secs }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <!-- ③ 访问计数 -->
      <el-col :span="8">
        <el-card shadow="never" :header="$t('map.auto.text106')">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm">{{ $t('map.auto.text107') }}</span>
            <el-select v-model="period" size="small" style="width: 140px" @change="loadUsage">
              <el-option :label="$t('map.auto.n24Hour')" value="day" />
              <el-option :label="$t('map.auto.n7')" value="week" />
              <el-option :label="$t('map.auto.n30Month')" value="month" />
              <el-option :label="$t('map.auto.n365Year')" value="year" />
            </el-select>
          </div>
          <div class="text-3xl font-bold mb-2">{{ totalHits }} <span class="text-sm font-normal">{{ $t('map.auto.text108') }}</span></div>
          <el-table :data="topRows" size="small" border height="200">
            <el-table-column prop="name" :label="$t('map.auto.text109')" min-width="120" show-overflow-tooltip />
            <el-table-column prop="kind" :label="$t('map.auto.type')" width="100" />
            <el-table-column prop="hits" :label="$t('map.auto.text110')" width="90" align="center" sortable />
          </el-table>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts" name="MapDashboard">
import { ref, onMounted } from 'vue';
import { CRS_SUPPORT_HINT, PROJ_MISSING_HINT } from '@/utils/crs';
import {
  listServices,
  usage,
  getComponents
} from '@/api/map/service';
import type {
  MapServiceListResp,
  UsageResp,
  ComponentsResp
} from '@/api/map/service/types';

const totalServices = ref(0);
const activeCount = ref(0);
const deletedCount = ref(0);
const kindRows = ref<{ kind: string; count: number; states: Record<string, number> }[]>([]);

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

const period = ref('day');
const totalHits = ref(0);
const topRows = ref<{ kind: string; name: string; hits: number }[]>([]);

async function loadServices() {
  const list: any = await listServices();
  const byKind = new Map<string, { count: number; states: Record<string, number> }>();
  let active = 0;
  let deleted = 0;
  for (const s of list.services || []) {
    const e = byKind.get(s.kind) || { count: 0, states: {} };
    e.count += 1;
    e.states[s.state] = (e.states[s.state] || 0) + 1;
    byKind.set(s.kind, e);
    if (s.state === 'active') active += 1;
    if (s.state === 'deleted') deleted += 1;
  }
  kindRows.value = Array.from(byKind.entries()).map(([kind, v]) => ({ kind, ...v }));
  totalServices.value = (list.services || []).length;
  activeCount.value = active;
  deletedCount.value = deleted;
}

async function loadComponents() {
  comp.value = (await getComponents()) as unknown as ComponentsResp;
}

async function loadUsage() {
  const u: any = await usage({ period: period.value, group_by: 'name' });
  totalHits.value = (u.rows || []).reduce((a, r) => a + (r.hits || 0), 0);
  topRows.value = [...(u.rows || [])]
    .sort((a, b) => (b.hits || 0) - (a.hits || 0))
    .slice(0, 10)
    .map((r) => ({ kind: r.kind, name: r.name, hits: r.hits }));
}

onMounted(() => {
  loadServices();
  loadComponents();
  loadUsage();
});
</script>

<style scoped>
.mr-1 { margin-right: 4px; }
.mt-2 { margin-top: 8px; }
</style>
