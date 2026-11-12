<template>
  <!-- 选择服务器上的 ArcGIS Conf.xml，解析出发布所需的瓦片网格（CRS/原点/LOD） -->
  <el-dialog v-model="visible" title="选择瓦片配置 XML" width="820px" append-to-body @open="onOpen">
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        选择工作空间数据根目录下的 ArcGIS <b>Conf.xml</b>（或其所在缓存目录），读取坐标系、原点、瓦片尺寸与 LOD 阶梯。
      </template>
    </el-alert>

    <div class="flex items-center gap-2 mb-[8px]">
      <el-button size="small" icon="Back" :disabled="!canGoUp" @click="goUp">上一级</el-button>
      <span class="text-xs text-[#888] truncate" :title="curPath">{{ relPath || root || '未配置数据根目录' }}</span>
    </div>

    <el-table
      v-loading="loading"
      :data="entries"
      size="small"
      border
      height="240"
      highlight-current-row
      @row-click="onRowClick"
    >
      <el-table-column label="名称" min-width="320" :show-overflow-tooltip="true">
        <template #default="{ row }">
          <span :class="row.is_dir ? 'text-[#2f6fd0]' : ''">{{ row.is_dir ? '[目录] ' + row.name : row.name }}</span>
        </template>
      </el-table-column>
      <el-table-column label="类型" width="90" align="center">
        <template #default="{ row }">
          <el-tag size="small" effect="plain" :type="row.is_dir ? 'info' : 'success'">
            {{ row.is_dir ? '目录' : 'XML' }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column label="大小" width="110" align="right">
        <template #default="{ row }">{{ row.is_dir ? '—' : fmtSize(row.size) }}</template>
      </el-table-column>
    </el-table>

    <div class="mt-[12px] flex items-center gap-2">
      <span class="text-sm shrink-0">XML 路径</span>
      <el-input v-model="pathInput" placeholder="可直接粘贴绝对路径，或在上表中选择 Conf.xml" />
      <el-button type="primary" :loading="parsing" @click="parse(pathInput)">解析</el-button>
    </div>

    <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon class="mt-[10px]" />

    <div v-if="parsed" class="mt-[10px]">
      <div class="text-xs">
        坐标系 <b>{{ parsed.crs }}</b> · 瓦片 {{ parsed.tile_px || parsed.tile_cols || '—' }} px · 原点
        [{{ (parsed.origin || []).join(', ') }}] · 层级 {{ parsed.min_zoom ?? '—' }}–{{ parsed.max_zoom ?? '—' }}
        （{{ (parsed.lods || []).length }} 级）
      </div>
      <div class="text-xs text-gray-400 truncate" :title="parsed.source">来源：{{ parsed.source }}</div>
      <el-table :data="lods" size="small" border max-height="150" class="mt-1">
        <el-table-column prop="level" label="级别" width="70" align="center" />
        <el-table-column prop="scale" label="比例尺" align="right" />
        <el-table-column prop="resolution" label="分辨率（地图单位/像素）" align="right" />
      </el-table>
    </div>

    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :disabled="!parsed" @click="confirm">确定</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { getCurrentInstance, ComponentInternalInstance, computed, ref } from 'vue';
import { listWorkspaceDirs, parseTileGrid } from '@/api/map/catalog';
import type { DirEntry, NativeGridLevel, TileGridParseResp } from '@/api/map/catalog/types';

const props = defineProps<{
  modelValue: boolean;
  workspaceId: number;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void;
  (e: 'confirm', r: TileGridParseResp): void;
}>();

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v)
});

const loading = ref(false);
const parsing = ref(false);
const root = ref('');
const curPath = ref('');
const parentPath = ref('');
const entries = ref<DirEntry[]>([]);
const pathInput = ref('');
const parsed = ref<TileGridParseResp | null>(null);
const error = ref('');

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

const lods = computed<NativeGridLevel[]>(() => {
  const p = parsed.value;
  if (!p) return [];
  if (p.lods?.length) return p.lods;
  return (p.levels || []).map((level, i) => ({
    level,
    scale: p.scales?.[i] ?? 0,
    resolution: p.resolutions?.[i] ?? 0
  }));
});

const load = async (path?: string) => {
  if (!props.workspaceId) {
    error.value = '未能确定所属工作空间，无法浏览服务器目录';
    return;
  }
  loading.value = true;
  error.value = '';
  try {
    const res = await listWorkspaceDirs({ workspaceId: props.workspaceId, path });
    root.value = res.data.root || '';
    curPath.value = res.data.path || '';
    parentPath.value = res.data.parent || '';
    // 只保留子目录与 .xml 文件
    entries.value = (res.data.entries || [])
      .filter((e) => e.is_dir || e.name.toLowerCase().endsWith('.xml'))
      .sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1));
  } catch (e: any) {
    proxy?.$modal.msgError(e?.message || '目录读取失败');
  } finally {
    loading.value = false;
  }
};

const onOpen = () => {
  parsed.value = null;
  error.value = '';
  pathInput.value = '';
  entries.value = [];
  load(undefined);
};

const goUp = () => {
  if (canGoUp.value) load(parentPath.value);
};

const parse = async (raw: string) => {
  const path = (raw || '').trim();
  if (!path) {
    error.value = '请选择或填写 Conf.xml 路径';
    return;
  }
  parsing.value = true;
  error.value = '';
  try {
    const res = await parseTileGrid({ workspaceId: props.workspaceId, path });
    parsed.value = res.data;
    pathInput.value = res.data.source || path;
  } catch (e: any) {
    parsed.value = null;
    error.value = e?.message || '解析失败';
  } finally {
    parsing.value = false;
  }
};

const onRowClick = (row: DirEntry) => {
  if (row.is_dir) {
    load(joinPath(curPath.value, row.name));
    return;
  }
  pathInput.value = joinPath(curPath.value, row.name);
  parse(pathInput.value);
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

const confirm = () => {
  if (!parsed.value) return;
  emit('confirm', parsed.value);
  visible.value = false;
};
</script>
