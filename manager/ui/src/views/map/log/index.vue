<template>
  <div class="p-2">
    <!-- 说明 + 统计 -->
    <div class="mb-[10px]">
      <el-card shadow="hover">
        <template #header>
          <span class="font-bold">访问日志（重启以来内存环形缓冲，最多 4000 条）</span>
        </template>
        <el-row :gutter="16">
          <el-col :span="6">
            <el-statistic title="累计请求" :value="stats.total" />
          </el-col>
          <el-col :span="6">
            <el-statistic title="客户端 4xx" :value="stats.client_4xx" />
          </el-col>
          <el-col :span="6">
            <el-statistic title="服务端 5xx" :value="stats.server_5xx" />
          </el-col>
          <el-col :span="6">
            <el-statistic title="当前展示" :value="logList.length" />
          </el-col>
        </el-row>
        <el-divider content-position="left">状态码分布</el-divider>
        <div v-if="statusEntries.length">
          <el-tag v-for="[code, n] in statusEntries" :key="code" class="mr-2 mb-1" :type="statusTagType(Number(code))" effect="plain">
            {{ code }}: {{ n }}
          </el-tag>
        </div>
        <el-empty v-else description="暂无统计" :image-size="60" />
      </el-card>
    </div>

    <!-- 筛选条 -->
    <transition :enter-active-class="proxy?.animate.searchAnimate.enter" :leave-active-class="proxy?.animate.searchAnimate.leave">
      <div v-show="showSearch" class="mb-[10px]">
        <el-card shadow="hover">
          <el-form :inline="true">
            <el-form-item label="状态">
              <el-select v-model="queryParams.status" placeholder="全部" clearable style="width: 140px">
                <el-option label="2xx 成功" value="2xx" />
                <el-option label="3xx 重定向" value="3xx" />
                <el-option label="4xx 客户端" value="4xx" />
                <el-option label="5xx 服务端" value="5xx" />
              </el-select>
            </el-form-item>
            <el-form-item label="服务类">
              <el-input v-model="queryParams.kind" placeholder="如 cog/postgis" clearable style="width: 160px" @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item label="服务名">
              <el-input v-model="queryParams.name" placeholder="图层/服务名" clearable style="width: 160px" @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item label="关键字">
              <el-input v-model="queryParams.q" placeholder="路径/参数模糊匹配" clearable style="width: 180px" @keyup.enter="handleQuery" />
            </el-form-item>
            <el-form-item>
              <el-button type="primary" icon="Search" @click="handleQuery">搜索</el-button>
              <el-button icon="Refresh" @click="resetQuery">重置</el-button>
            </el-form-item>
          </el-form>
        </el-card>
      </div>
    </transition>

    <el-card shadow="hover">
      <template #header>
        <el-row :gutter="10" class="mb8">
          <el-col :span="1.5">
            <el-button icon="Refresh" @click="getList">刷新</el-button>
          </el-col>
          <right-toolbar v-model:show-search="showSearch" @query-table="getList" />
        </el-row>
      </template>

      <el-table v-loading="loading" border :data="logList" style="width: 100%" max-height="560">
        <el-table-column label="时间" align="center" width="180">
          <template #default="scope">
            <span class="text-xs">{{ proxy?.parseTime(scope.row.ts_ms) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="方法" align="center" width="80" prop="method" />
        <el-table-column label="状态码" align="center" width="90">
          <template #default="scope">
            <el-tag :type="statusTagType(scope.row.status)" effect="dark" size="small">{{ scope.row.status }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="耗时(ms)" align="center" width="100" prop="duration_ms" sortable />
        <el-table-column label="路径" align="center" prop="path" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column label="查询串" align="center" prop="query" min-width="160" :show-overflow-tooltip="true" />
        <el-table-column label="服务类" align="center" width="100">
          <template #default="scope">
            <el-tag v-if="scope.row.kind" type="info" effect="plain">{{ scope.row.kind }}</el-tag>
            <span v-else class="text-gray-400">—</span>
          </template>
        </el-table-column>
        <el-table-column label="服务名" align="center" prop="name" width="140" :show-overflow-tooltip="true" />
        <el-table-column label="客户端" align="center" prop="client" width="140" :show-overflow-tooltip="true" />
      </el-table>
      <div class="mt-3 flex justify-end">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :page-sizes="[10, 20, 50, 100]"
          :total="total"
          layout="total, sizes, prev, pager, next, jumper"
          background
        />
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts" name="MapAccessLog">
import { listAccessLogs } from '@/api/map/service';
import { AccessLogRow, AccessLogResp } from '@/api/map/service/types';
import { to } from 'await-to-js';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const loading = ref(true);
const showSearch = ref(true);
const logList = ref<AccessLogRow[]>([]);
// 后端分页总条数（manager 侧强制 upstream limit=2000 拉满后切片，total 为全局总数）
const total = ref(0);
const stats = reactive<{ total: number; client_4xx: number; server_5xx: number }>({
  total: 0,
  client_4xx: 0,
  server_5xx: 0
});
const statusEntries = ref<Array<[string, number]>>([]);

const page = ref(1);
const pageSize = ref(10);

const queryParams = ref<{
  status: string;
  kind: string;
  name: string;
  q: string;
}>({
  status: '',
  kind: '',
  name: '',
  q: ''
});

const statusTagType = (code: number): 'success' | 'info' | 'warning' | 'danger' => {
  if (code >= 500) return 'danger';
  if (code >= 400) return 'warning';
  if (code >= 300) return 'info';
  return 'success';
};

const getList = async () => {
  loading.value = true;
  const [err, res] = await to(
    listAccessLogs({
      page: page.value,
      pageSize: pageSize.value,
      status: queryParams.value.status || undefined,
      kind: queryParams.value.kind || undefined,
      name: queryParams.value.name || undefined,
      q: queryParams.value.q || undefined
    })
  );
  loading.value = false;
  if (err) {
    proxy?.$modal.msgError('访问日志获取失败：' + (err as Error).message);
    return;
  }
  const data = (res?.data ?? {}) as AccessLogResp;
  logList.value = data.rows || [];
  total.value = data.total || 0;
  stats.total = data.total || 0;
  stats.client_4xx = data.errors?.client_4xx || 0;
  stats.server_5xx = data.errors?.server_5xx || 0;
  statusEntries.value = Object.entries(data.status_counts || {}).sort((a, b) => a[0].localeCompare(b[0]));
};

// 翻页 / 改每页条数时重新拉取当前页
watch([page, pageSize], () => {
  getList();
});

const handleQuery = () => {
  page.value = 1;
  getList();
};
const resetQuery = () => {
  queryParams.value = { status: '', kind: '', name: '', q: '' };
  page.value = 1;
  getList();
};

onMounted(() => {
  getList();
});
</script>
