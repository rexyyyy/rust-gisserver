<template>
  <div class="app-container">
    <el-row :gutter="16">
      <!-- ① 服务统计：类型 × 状态 -->
      <el-col :span="8">
        <el-card shadow="never" header="服务统计（类型 × 状态）">
          <el-table :data="kindRows" size="small" border height="260">
            <el-table-column prop="kind" label="类型" width="110" />
            <el-table-column prop="count" label="数量" width="80" align="center" />
            <el-table-column label="状态分布" min-width="160">
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
            合计 <b>{{ totalServices }}</b> 个服务 ·
            在服 <b>{{ activeCount }}</b> ·
            回收站 <b>{{ deletedCount }}</b>
          </div>
        </el-card>
      </el-col>

      <!-- ② 组件探测 -->
      <el-col :span="8">
        <el-card shadow="never" header="组件探测（map-api 运行环境）">
          <el-descriptions :column="1" border size="small">
            <el-descriptions-item label="map-api 存活">
              <el-tag :type="comp.map_api_up ? 'success' : 'danger'">
                {{ comp.map_api_up ? '在线' : '离线' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="PostGIS">
              <el-tag :type="comp.pg_enabled ? 'success' : 'info'">
                {{ comp.pg_enabled ? '已启用' : '未启用' }}
              </el-tag>
            </el-descriptions-item>
            <el-descriptions-item label="PROJ 重投影">
              <el-tag :type="comp.proj_loaded ? 'success' : 'info'">
                {{ comp.proj_loaded ? '已加载' : '未加载' }}
              </el-tag>
              <el-tooltip :content="comp.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
              </el-tooltip>
            </el-descriptions-item>
            <el-descriptions-item label="在服图层">{{ comp.served_layers }}</el-descriptions-item>
            <el-descriptions-item label="3D Tiles 服务">{{ comp.tilesets }}</el-descriptions-item>
            <el-descriptions-item label="缓存上限(MiB)">{{ comp.cache_mib }}</el-descriptions-item>
            <el-descriptions-item label="缓存 TTL(秒)">{{ comp.cache_ttl_secs }}</el-descriptions-item>
          </el-descriptions>
        </el-card>
      </el-col>

      <!-- ③ 访问计数 -->
      <el-col :span="8">
        <el-card shadow="never" header="访问计数">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm">统计周期</span>
            <el-select v-model="period" size="small" style="width: 140px" @change="loadUsage">
              <el-option label="近 24 小时（天）" value="day" />
              <el-option label="近 7 天（周）" value="week" />
              <el-option label="近 30 天（月）" value="month" />
              <el-option label="近 365 天（年）" value="year" />
            </el-select>
          </div>
          <div class="text-3xl font-bold mb-2">{{ totalHits }} <span class="text-sm font-normal">次请求</span></div>
          <el-table :data="topRows" size="small" border height="200">
            <el-table-column prop="name" label="服务" min-width="120" show-overflow-tooltip />
            <el-table-column prop="kind" label="类型" width="100" />
            <el-table-column prop="hits" label="命中" width="90" align="center" sortable />
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
  const { data } = await listServices();
  const list: MapServiceListResp = data.data || data;
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
  const { data } = await getComponents();
  comp.value = (data.data || data) as ComponentsResp;
}

async function loadUsage() {
  const { data } = await usage({ period: period.value, group_by: 'name' });
  const u: UsageResp = data.data || data;
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
