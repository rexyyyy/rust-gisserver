<template>
  <!-- 选择服务器上的 ArcGIS Conf.xml，解析出发布所需的瓦片网格（CRS/原点/LOD） -->
  <el-dialog v-model="visible" :title="$t('map.auto.selectTileConfigurationXML')" width="820px" append-to-body @open="onOpen">
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        {{ $t('map.auto.arcgisCacheHint') }}
      </template>
    </el-alert>

    <div class="flex items-center gap-2 mb-[8px]">
      <el-button size="small" icon="Back" :disabled="!canGoUp" @click="goUp">{{ $t('map.auto.parent') }}</el-button>
      <span class="text-xs text-[#888] truncate" :title="curPath">{{ relPath || root || $t('map.auto.dataRootIsNotConfigured') }}</span>
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
      <el-table-column :label="$t('map.auto.name')" min-width="320" :show-overflow-tooltip="true">
        <template #default="{ row }">
          <span :class="row.is_dir ? 'text-[#2f6fd0]' : ''">{{ row.is_dir ? $t('map.auto.dir') + row.name : row.name }}</span>
        </template>
      </el-table-column>
      <el-table-column :label="$t('map.auto.type')" width="90" align="center">
        <template #default="{ row }">
          <el-tag size="small" effect="plain" :type="row.is_dir ? 'info' : 'success'">
            {{ row.is_dir ? $t('map.auto.catalog2') : 'XML' }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="$t('map.auto.size')" width="110" align="right">
        <template #default="{ row }">{{ row.is_dir ? '—' : fmtSize(row.size) }}</template>
      </el-table-column>
    </el-table>

    <div class="mt-[12px] flex items-center gap-2">
      <span class="text-sm shrink-0">{{ $t('map.auto.xmlPath') }}</span>
      <el-input v-model="pathInput" :placeholder="$t('map.auto.pasteAnAbsolutePathOrSelectConfXmlFromTheT')" />
      <el-button type="primary" :loading="parsing" @click="parse(pathInput)">{{ $t('map.auto.parse') }}</el-button>
    </div>

    <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon class="mt-[10px]" />

    <div v-if="parsed" class="mt-[10px]">
      <div class="text-xs">
        {{ $t('map.auto.crs8') }} <b>{{ parsed.crs }}</b> · {{ $t('map.auto.text169') }} {{ parsed.tile_px || parsed.tile_cols || '—' }} px · {{ $t('map.auto.text170') }}
        [{{ (parsed.origin || []).join(', ') }}] · {{ $t('map.auto.level') }} {{ parsed.min_zoom ?? '—' }}–{{ parsed.max_zoom ?? '—' }}
        ({{ (parsed.lods || []).length }} {{ $t('map.auto.text172') }})
      </div>
      <div class="text-xs text-gray-400 truncate" :title="parsed.source">{{ $t('map.auto.text41') }}{{ parsed.source }}</div>
      <el-table :data="lods" size="small" border max-height="150" class="mt-1">
        <el-table-column prop="level" :label="$t('map.auto.level')" width="70" align="center" />
        <el-table-column prop="scale" :label="$t('map.auto.scale')" align="right" />
        <el-table-column prop="resolution" :label="$t('map.auto.resolutionMapUnitsPixel')" align="right" />
      </el-table>
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ $t('map.auto.cancel') }}</el-button>
      <el-button type="primary" :disabled="!parsed" @click="confirm">{{ $t('map.auto.ok') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
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
    error.value = t('map.auto.cannotDetermineTheWorkspaceUnableToBrowseT');
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
    proxy?.$modal.msgError(e?.message || t('map.auto.failedToReadDirectory'));
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
    error.value = t('map.auto.selectOrEnterTheConfXmlPath');
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
    error.value = e?.message || t('map.auto.parseFailed');
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
