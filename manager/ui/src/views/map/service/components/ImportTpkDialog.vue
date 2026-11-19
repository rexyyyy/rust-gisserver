<template>
  <!-- 导入 ArcGIS 缓存包（.tpk / .tpkx）：选包 → 解到工作空间数据根下 → 展示缓存定义记录 -->
  <el-dialog v-model="visible" :title="$t('map.auto.importTpk')" width="880px" append-to-body @open="onOpen">
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>{{ $t('map.auto.importTpkHint') }}</template>
    </el-alert>

    <el-form label-width="110px">
      <el-form-item :label="$t('map.auto.workspace')">
        <el-select v-model="workspaceId" style="width: 320px" @change="onWorkspaceChange">
          <el-option
            v-for="w in workspaces"
            :key="w.id"
            :label="`${w.name}${w.title ? ' — ' + w.title : ''}${w.dataRoot ? ' (' + w.dataRoot + ')' : ''}`"
            :value="w.id"
          />
        </el-select>
      </el-form-item>
      <el-form-item :label="$t('map.auto.tpkPackage')">
        <el-input v-model="pathInput" :placeholder="$t('map.auto.selectTpkPackageHint')" class="flex-1" />
        <el-button :loading="browsing" @click="browseVisible = true">{{ $t('map.auto.browse') }}</el-button>
      </el-form-item>
      <el-form-item :label="$t('map.auto.landingDirName')">
        <el-input v-model="name" :placeholder="$t('map.auto.landingDirNameHint')" style="width: 320px" />
        <span class="text-xs text-gray-400">{{ $t('map.auto.landingDirPreview') }}{{ previewDir }}</span>
      </el-form-item>
    </el-form>

    <el-alert v-if="error" :title="error" type="error" :closable="false" show-icon class="mt-[10px]" />

    <!-- 导入结果：缓存定义记录（网格方案） -->
    <div v-if="result" class="mt-[12px]">
      <el-descriptions :column="2" border size="small">
        <el-descriptions-item :label="$t('map.auto.landingDir')">{{ result.dir }}</el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.configFile')">{{ result.config_file }}</el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.packageKind')">
          {{ result.package_kind.toUpperCase() }} · {{ result.config_kind }}
        </el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.cacheFormat')">{{ result.cache_version.toUpperCase() }}</el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.crs')">{{ result.crs }}</el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.text169')">
          {{ result.tile_cols }} × {{ result.tile_rows }} px · DPI {{ result.dpi }}
        </el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.text170')">[{{ (result.origin || []).join(', ') }}]</el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.text171')">
          {{ result.min_zoom }}–{{ result.max_zoom }}（{{ result.lods?.length || 0 }} {{ $t('map.auto.text172') }}）
        </el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.tileRoot')" :span="2">{{ result.tile_root }}</el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.dataRange')" :span="2">
          {{ (result.bounds || []).map((v) => Number(v).toFixed(4)).join(', ') || '—' }}
        </el-descriptions-item>
        <el-descriptions-item :label="$t('map.auto.definitionRecord')" :span="2">
          <el-tag size="small" :type="result.definition_reused ? 'info' : 'success'">
            #{{ result.definition_id }}
          </el-tag>
          <span class="text-xs ml-2">
            {{ result.definition_reused ? $t('map.auto.definitionReused') : $t('map.auto.definitionCreated') }}
          </span>
        </el-descriptions-item>
      </el-descriptions>

      <el-table :data="result.lods || []" size="small" border max-height="160" class="mt-[10px]">
        <el-table-column prop="level" :label="$t('map.auto.level')" width="70" align="center" />
        <el-table-column prop="scale" :label="$t('map.auto.scale')" align="right" />
        <el-table-column prop="resolution" :label="$t('map.auto.resolutionMapUnitsPixel')" align="right" />
      </el-table>
    </div>

    <!-- 包选择：浏览工作空间数据根下的 .tpk / .tpkx -->
    <el-dialog v-model="browseVisible" :title="$t('map.auto.selectTpkPackage')" width="720px" append-to-body>
      <div class="flex items-center gap-2 mb-[8px]">
        <el-button size="small" icon="Back" :disabled="!canGoUp" @click="goUp">{{ $t('map.auto.parent') }}</el-button>
        <span class="text-xs text-[#888] truncate" :title="curPath">{{ relPath || root || '—' }}</span>
      </div>
      <el-table
        v-loading="browsing"
        :data="entries"
        size="small"
        border
        height="240"
        highlight-current-row
        @row-click="onRowClick"
      >
        <el-table-column :label="$t('map.auto.name')" min-width="300" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span :class="row.is_dir ? 'text-[#2f6fd0]' : ''">{{ row.name }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.type')" width="90" align="center">
          <template #default="{ row }">
            <el-tag size="small" effect="plain" :type="row.is_dir ? 'info' : 'success'">
              {{ row.is_dir ? $t('map.auto.catalog2') : row.name.split('.').pop()?.toUpperCase() }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.size')" width="110" align="right">
          <template #default="{ row }">{{ row.is_dir ? '—' : fmtSize(row.size) }}</template>
        </el-table-column>
      </el-table>
      <div class="mt-[10px] flex items-center gap-2">
        <span class="text-sm shrink-0">{{ $t('map.auto.tpkPackage') }}</span>
        <el-input v-model="pathInput" :placeholder="$t('map.auto.pasteAnAbsolutePathOrSelectConfXmlFromTheT')" />
      </div>
      <template #footer>
        <el-button @click="browseVisible = false">{{ $t('map.auto.cancel') }}</el-button>
        <el-button type="primary" :disabled="!selectedPkg" @click="confirmPick">{{ $t('map.auto.ok') }}</el-button>
      </template>
    </el-dialog>

    <template #footer>
      <el-button @click="visible = false">{{ $t('map.auto.cancel') }}</el-button>
      <el-button type="primary" :loading="importing" :disabled="!pathInput.trim()" @click="doImport">
        {{ $t('map.auto.importTpk') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { getCurrentInstance, ComponentInternalInstance, computed, ref } from 'vue';
import { importTpk, listWorkspaceDirs, listWorkspaces } from '@/api/map/catalog';
import type { DirEntry, ImportTpkResp, WorkspaceVO } from '@/api/map/catalog/types';

const props = defineProps<{ modelValue: boolean }>();
const emit = defineEmits<{ (e: 'update:modelValue', v: boolean): void; (e: 'imported', r: ImportTpkResp): void }>();

const { t } = useI18n();
const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v)
});

const importing = ref(false);
const browsing = ref(false);
const browseVisible = ref(false);
const error = ref('');
const result = ref<ImportTpkResp | null>(null);
const workspaces = ref<WorkspaceVO[]>([]);
const workspaceId = ref(0);
const root = ref('');
const curPath = ref('');
const parentPath = ref('');
const entries = ref<DirEntry[]>([]);
const pathInput = ref('');
const name = ref('');
const selectedPkg = ref('');

const separator = computed(() => (root.value.includes('\\') ? '\\' : '/'));
const joinPath = (dir: string, n: string) =>
  dir.endsWith(separator.value) ? `${dir}${n}` : `${dir}${separator.value}${n}`;

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

/** 落点预览：{data_root}/arcgis_tpk/explode_tpk/<name>_tpk */
const previewDir = computed(() => {
  const n = (name.value || '').trim() || t('map.auto.usePackageName');
  const base = root.value ? `${root.value}${separator.value}` : '…';
  return `${base}arcgis_tpk${separator.value}explode_tpk${separator.value}${n}_tpk`;
});

const onOpen = async () => {
  error.value = '';
  result.value = null;
  pathInput.value = '';
  name.value = '';
  selectedPkg.value = '';
  entries.value = [];
  try {
    const res = await listWorkspaces();
    workspaces.value = res.data?.workspaces || [];
    if (!workspaceId.value && workspaces.value.length) {
      workspaceId.value = workspaces.value[0].id;
    }
    if (workspaceId.value) await load(undefined);
  } catch (e: any) {
    proxy?.$modal.msgError(e?.message || t('map.auto.failedToReadDirectory'));
  }
};

const onWorkspaceChange = async () => {
  pathInput.value = '';
  selectedPkg.value = '';
  entries.value = [];
  await load(undefined);
};

const load = async (path?: string) => {
  if (!workspaceId.value) {
    error.value = t('map.auto.cannotDetermineTheWorkspaceUnableToBrowseT');
    return;
  }
  browsing.value = true;
  error.value = '';
  try {
    const res = await listWorkspaceDirs({ workspaceId: workspaceId.value, path });
    root.value = res.data.root || '';
    curPath.value = res.data.path || '';
    parentPath.value = res.data.parent || '';
    // 只留子目录与 .tpk / .tpkx 包
    entries.value = (res.data.entries || [])
      .filter((e) => e.is_dir || /\.(tpk|tpkx)$/i.test(e.name))
      .sort((a, b) => (a.is_dir === b.is_dir ? a.name.localeCompare(b.name) : a.is_dir ? -1 : 1));
  } catch (e: any) {
    proxy?.$modal.msgError(e?.message || t('map.auto.failedToReadDirectory'));
  } finally {
    browsing.value = false;
  }
};

const goUp = () => {
  if (canGoUp.value) load(parentPath.value);
};

const onRowClick = (row: DirEntry) => {
  if (row.is_dir) {
    load(joinPath(curPath.value, row.name));
    return;
  }
  selectedPkg.value = joinPath(curPath.value, row.name);
  pathInput.value = selectedPkg.value;
  if (!name.value) name.value = row.name.replace(/\.(tpk|tpkx)$/i, '');
};

const confirmPick = () => {
  if (!selectedPkg.value) return;
  pathInput.value = selectedPkg.value;
  browseVisible.value = false;
};

const doImport = async () => {
  const path = pathInput.value.trim();
  if (!path) {
    error.value = t('map.auto.selectTpkPackage');
    return;
  }
  importing.value = true;
  error.value = '';
  result.value = null;
  try {
    const res = await importTpk({
      workspaceId: workspaceId.value,
      path,
      name: name.value.trim() || undefined
    });
    result.value = res.data;
    proxy?.$modal.msgSuccess(t('map.auto.importTpkSuccess'));
    emit('imported', res.data);
  } catch (e: any) {
    error.value = e?.message || t('map.auto.importTpkFailed');
  } finally {
    importing.value = false;
  }
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
</script>
