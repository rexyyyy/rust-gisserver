<template>
  <el-dialog v-model="visible" title="导入 SHP 到 PostGIS" width="780px" append-to-body @open="onOpen">
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        从工作空间数据根目录选择 <b>.shp</b> 矢量文件，由 map-api 调用 shp2pgsql 导入数据源
        <b>{{ store?.name }}</b> 的 <b>{{ store?.pgSchema || 'public' }}</b> 模式。
      </template>
    </el-alert>

    <!-- 目录导航 -->
    <div class="flex items-center gap-2 mb-[8px]">
      <el-button size="small" icon="Back" :disabled="!canGoUp" @click="goUp">上一级</el-button>
      <span class="text-xs text-[#888] truncate" :title="curPath">{{ relPath || root || '未配置数据根目录' }}</span>
    </div>

    <!-- 目录 + .shp 列表（其它文件不显示） -->
    <el-table
      v-loading="loading"
      :data="entries"
      size="small"
      border
      height="300"
      highlight-current-row
      @row-click="onRowClick"
    >
      <el-table-column label="名称" min-width="300" :show-overflow-tooltip="true">
        <template #default="{ row }">
          <span :class="row.is_dir ? 'text-[#2f6fd0]' : ''">{{ row.is_dir ? '[目录] ' + row.name : row.name }}</span>
        </template>
      </el-table-column>
      <el-table-column label="类型" width="90" align="center">
        <template #default="{ row }">
          <el-tag size="small" effect="plain" :type="row.is_dir ? 'info' : 'success'">{{ row.is_dir ? '目录' : 'SHP' }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column label="大小" width="110" align="right">
        <template #default="{ row }">{{ row.is_dir ? '—' : fmtSize(row.size) }}</template>
      </el-table-column>
    </el-table>

    <!-- 已选文件 + 数据图层名 -->
    <div class="mt-[12px] grid grid-cols-[80px_1fr] gap-y-3 gap-x-2 items-center">
      <span class="text-sm">已选文件</span>
      <el-input :model-value="selectedName" readonly placeholder="在上表中选择 .shp 文件" />
      <span class="text-sm">数据图层名</span>
      <div>
        <el-input v-model="tableName" maxlength="63" placeholder="小写字母/数字/下划线，字母或下划线开头" />
        <div class="text-xs text-[#999] mt-1">导入后在 PostGIS 中生成同名表，坐标系由 .prj 自动识别</div>
      </div>
    </div>

    <!-- 导入进度（SSE 推送，大数据量时持续刷新） -->
    <div v-if="importing || progressText" class="mt-[12px]">
      <el-progress
        v-if="importing"
        :percentage="100"
        :indeterminate="true"
        :duration="2"
        :show-text="false"
        :stroke-width="8"
        :status="progressError ? 'exception' : undefined"
      />
      <div class="text-xs mt-1" :class="progressError ? 'text-[#d93026]' : 'text-[#666]'">{{ progressText }}</div>
    </div>

    <template #footer>
      <el-button :disabled="importing" @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="importing" :disabled="!selectedPath" @click="handleImport">导入</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { getCurrentInstance, ComponentInternalInstance, computed, ref } from 'vue';
import { importStoreShpStream, listWorkspaceDirs } from '@/api/map/catalog';
import type { DirEntry, ImportShpProgress, ImportShpResp, StoreVO } from '@/api/map/catalog/types';

const props = defineProps<{
  modelValue: boolean;
  store: StoreVO | null;
  /** 该 Store 所属工作空间的数据根目录 */
  dataRoot: string;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void;
  (e: 'imported', r: ImportShpResp): void;
}>();

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v)
});

const loading = ref(false);
const importing = ref(false);
const progressText = ref('');
const progressError = ref(false);
/** done 事件暂存（进度回调里写入，await 之后读取） */
const doneEvent = ref<ImportShpProgress | null>(null);
const root = ref('');
const curPath = ref('');
const parentPath = ref('');
const entries = ref<DirEntry[]>([]);
const selectedName = ref('');
const selectedPath = ref('');
const tableName = ref('');

const separator = computed(() => (root.value.includes('\\') ? '\\' : '/'));
const joinPath = (dir: string, name: string) =>
  dir.endsWith(separator.value) ? `${dir}${name}` : `${dir}${separator.value}${name}`;

const relPath = computed(() => {
  if (!root.value || !curPath.value) return curPath.value;
  const r = root.value.replace(/[\\/]+$/, '');
  if (curPath.value === r) return root.value;
  return curPath.value.startsWith(r) ? curPath.value.slice(r.length).replace(/^[\\/]/, '') : curPath.value;
});

const canGoUp = computed(() => {
  if (!root.value || !parentPath.value) return false;
  return parentPath.value.replace(/[\\/]+$/, '') !== root.value.replace(/[\\/]+$/, '');
});

const load = async (path?: string) => {
  if (!props.store) return;
  loading.value = true;
  try {
    const res = await listWorkspaceDirs({
      workspaceId: props.store.workspaceId,
      path: path || props.dataRoot
    });
    root.value = res.data.root || '';
    curPath.value = res.data.path || '';
    parentPath.value = res.data.parent || '';
    // 只保留子目录与 .shp 文件，其它文件不参与选择
    entries.value = (res.data.entries || [])
      .filter((e) => e.is_dir || e.name.toLowerCase().endsWith('.shp'))
      .sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1));
  } catch (e: any) {
    proxy?.$modal.msgError(e?.message || '目录读取失败');
  } finally {
    loading.value = false;
  }
};

const onOpen = () => {
  selectedName.value = '';
  selectedPath.value = '';
  tableName.value = '';
  entries.value = [];
  progressText.value = '';
  progressError.value = false;
  doneEvent.value = null;
  load(props.dataRoot);
};

const goUp = () => {
  if (canGoUp.value) load(parentPath.value);
};

const suggestTable = (fileName: string) => {
  let t = fileName
    .replace(/\.shp$/i, '')
    .toLowerCase()
    .replace(/[^a-z0-9_]/g, '_')
    .replace(/^_+/, '');
  if (!t) t = 'shp_layer';
  if (/^[0-9]/.test(t)) t = `t_${t}`;
  return t.slice(0, 63);
};

const onRowClick = (row: DirEntry) => {
  if (row.is_dir) {
    load(joinPath(curPath.value, row.name));
    return;
  }
  selectedName.value = row.name;
  selectedPath.value = joinPath(curPath.value, row.name);
  tableName.value = suggestTable(row.name);
};

const fmtSize = (n: number) => {
  if (!n) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
};

const fmtMs = (ms: number) => {
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  const m = Math.floor(s / 60);
  return `${m} 分 ${Math.round(s % 60)} 秒`;
};

const handleImport = async () => {
  if (!props.store || !selectedPath.value) return;
  importing.value = true;
  progressError.value = false;
  doneEvent.value = null;
  progressText.value = '正在连接导入服务…';
  try {
    await importStoreShpStream(
      props.store.id,
      {
        path: selectedPath.value,
        table: tableName.value.trim().toLowerCase()
      },
      (e) => {
        if (e.stage === 'start') {
          progressText.value = `开始导入 ${e.table}（SRID ${e.srid}${e.sridDetected ? '' : '，.prj 未识别按 4326'}）…`;
        } else if (e.stage === 'copy') {
          progressText.value = `正在写入数据：已 ${e.rows ?? 0} 行，耗时 ${fmtMs(e.elapsedMs ?? 0)}`;
        } else if (e.stage === 'done' && e.ok) {
          doneEvent.value = e;
          progressText.value = `导入完成：${e.schema}.${e.table}，${e.features} 个要素，SRID ${e.srid}，耗时 ${fmtMs(e.elapsedMs ?? 0)}`;
        } else if (e.stage === 'done' || e.stage === 'error') {
          progressError.value = true;
          progressText.value = e.error || '导入失败';
        }
      }
    );
    const d = doneEvent.value;
    if (!d) throw new Error(progressError.value ? progressText.value : '导入未完成：连接意外中断');
    proxy?.$modal.msgSuccess(
      `导入完成：${d.schema}.${d.table}，${d.features} 个要素，SRID ${d.srid}${d.sridDetected ? '' : '（.prj 未识别，按 4326 处理）'}`
    );
    emit('imported', d as ImportShpResp);
    visible.value = false;
  } catch (e: any) {
    progressError.value = true;
    progressText.value = e?.message || '导入失败';
    proxy?.$modal.msgError(progressText.value);
  } finally {
    importing.value = false;
  }
};
</script>
