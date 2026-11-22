<template>
  <div class="p-2">
    <!-- 红线提示：数据源只是指针，不落地数据 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        {{ $t('map.auto.storeAlertHint') }}
      </template>
    </el-alert>

    <el-card shadow="hover">
      <CatalogToolbar>
        <template #filters>
          <el-select v-model="wsId" :placeholder="$t('map.auto.all')" clearable style="width: 210px" @change="load">
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
          <el-select v-model="discoverDepth" style="width: 120px" :title="$t('map.auto.catalogOf2')">
            <el-option v-for="d in [1, 2, 3, 4]" :key="d" :label="`${t('map.auto.text62')}${d}`" :value="d" />
          </el-select>
          <el-checkbox v-model="includeDeleted" @change="load">{{ $t('map.auto.showdelete') }}</el-checkbox>
        </template>
        <template #actions>
          <el-button icon="Refresh" @click="load">{{ $t('map.auto.refresh') }}</el-button>
          <el-button v-hasPermi="['map:catalog:edit']" type="primary" plain icon="Plus" @click="openAdd">{{ $t('map.auto.text76') }}</el-button>
          <el-button v-hasPermi="['map:catalog:edit']" type="success" plain icon="Plus" @click="openPgAdd">{{ $t('map.auto.newPostgisDatasource') }}</el-button>
        </template>
      </CatalogToolbar>

      <el-table v-loading="loading" border :data="rows" size="small" style="width: 100%">
        <el-table-column :label="$t('map.auto.name')" prop="name" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.text51')" align="center" width="100">
          <template #default="{ row }">
            <router-link :to="`/data/workspace`" class="link-type">{{ wsName(row.workspaceId) }}</router-link>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.type')" align="center" width="120">
          <template #default="{ row }">
            <el-tag effect="plain" size="small">{{ row.storeType }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text70')" min-width="230" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span class="text-xs">{{ row.isPostgis ? row.pgDsnMasked || $t('map.auto.text64') : row.basePath }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.layer2')" align="center" width="80">
          <template #default="{ row }">
            <router-link :to="`/data/resource?storeId=${row.id}`" class="link-type">{{ row.resourceCount }}</router-link>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text49')" align="center" width="80" prop="publishableCount" />
        <el-table-column :label="$t('map.auto.status')" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="row.delFlag === 1 ? 'danger' : 'success'" size="small" effect="dark">
              {{ row.delFlag === 1 ? $t('map.auto.text44') : $t('map.auto.normal') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.remark')" prop="remark" min-width="160" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.actions')" align="center" width="400" fixed="right">
          <template #default="{ row }">
            <el-button
              v-hasPermi="['map:catalog:edit']"
              link
              type="primary"
              icon="Edit"
              :disabled="row.delFlag === 1 || (row.publishedCount || 0) > 0"
              :title="(row.publishedCount || 0) > 0 ? $t('map.auto.publishedStoreLocked') : ''"
              @click="openEdit(row)"
            >
              {{ $t('map.auto.text99') }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:edit']"
              link
              type="info"
              icon="Connection"
              :disabled="row.delFlag === 1"
              :loading="busy.row === row.id && busy.kind === 'connect'"
              @click="handleTestConn(row)"
            >
              {{ $t('map.auto.text188') }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="primary"
              icon="Search"
              :disabled="row.delFlag === 1"
              :loading="busy.row === row.id && busy.kind === 'discover'"
              @click="handleDiscover(row)"
            >
              {{ $t('map.auto.text185') }}
            </el-button>
            <el-button
              v-if="row.isPostgis"
              v-hasPermi="['map:catalog:edit']"
              link
              type="warning"
              icon="Upload"
              :disabled="row.delFlag === 1"
              @click="openImport(row)"
            >
              {{ $t('map.auto.import') }}shp
            </el-button>
            <el-button v-if="row.delFlag === 1" v-hasPermi="['map:catalog:edit']" link type="success" icon="RefreshLeft" @click="handleRestore(row)">
              {{ $t('map.auto.text55') }}
            </el-button>
            <el-button
              v-else
              v-hasPermi="['map:catalog:edit']"
              link
              type="danger"
              icon="Delete"
              :disabled="(row.publishedCount || 0) > 0"
              :title="(row.publishedCount || 0) > 0 ? $t('map.auto.publishedStoreLocked') : ''"
              @click="handleRemove(row)"
            >
              {{ $t('map.auto.delete6') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        {{ $t('map.auto.storeCountHint', { n: total }) }}
      </div>
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

    <!-- 新建数据源：三栏布局 -->
    <el-dialog v-model="addDlg.visible" :title="$t('map.auto.text76')" width="900px" append-to-body>
      <div class="store-add-layout">
        <!-- 上部：选择工作空间 -->
        <div class="store-add-top">
          <el-select v-model="addDlg.workspaceId" :placeholder="$t('map.auto.text77')" style="width: 300px" @change="onAddWsChange">
            <el-option
              v-for="w in workspaces"
              :key="w.id"
              :label="`${w.name}${w.title ? ' — ' + w.title : ''}${w.dataRoot ? ' (' + w.dataRoot + ')' : ''}`"
              :value="w.id"
            />
          </el-select>
        </div>

        <div class="store-add-main">
          <!-- 左侧：目录树 -->
          <div v-if="!isAddPgStore" class="store-add-left">
            <DirTree
              ref="dirTreeRef"
              :data-root="selectedWsDataRoot"
              :workspace-id="addDlg.workspaceId"
              @select="onDirSelect"
            />
          </div>

          <!-- 右侧：编辑表单 -->
          <div class="store-add-right">
            <el-form ref="addFormRef" :model="addForm" :rules="addRules" label-width="140px">
              <el-form-item :label="$t('map.auto.name')" prop="name">
                <el-input v-model="addForm.name" :placeholder="$t('map.auto.wuhanCog')" />
              </el-form-item>
              <el-form-item :label="$t('map.auto.type')" prop="storeType">
                <el-select v-model="addForm.storeType" style="width: 100%" @change="onAddTypeChange">
                  <el-option v-for="t in storeTypeOptions" :key="t.value" :label="t.label" :value="t.value" />
                </el-select>
                <span
                  v-if="!isAddPgStore && dirDetectHint"
                  class="text-xs"
                  :class="selectedDirInfo?.storeType ? 'dir-detect-hint' : 'dir-detect-warn'"
                >
                  {{ dirDetectHint }}
                </span>
              </el-form-item>
              <el-form-item v-if="!isAddPgStore" :label="$t('map.auto.catalog2')" prop="basePath">
                <el-input v-model="addForm.basePath" readonly :placeholder="$t('map.auto.catalog4')" />
                <span class="text-xs text-gray-400">{{ $t('map.auto.catalogDetectLayerType') }}</span>
              </el-form-item>
              <!-- PostGIS 连接参数 -->
              <template v-else>
                <el-form-item :label="$t('map.auto.host')" prop="pgHost">
                  <el-input v-model="addForm.pgHost" :placeholder="$t('map.auto.n127001')" />
                </el-form-item>
                <el-form-item :label="$t('map.auto.text78')" prop="pgPort">
                  <el-input-number v-model="addForm.pgPort" :min="1" :max="65535" controls-position="right" style="width: 100%" />
                </el-form-item>
                <el-form-item :label="$t('map.auto.user')" prop="pgUsername">
                  <el-input v-model="addForm.pgUsername" placeholder="postgres" />
                </el-form-item>
                <el-form-item :label="$t('map.auto.text79')" prop="pgPassword">
                  <el-input v-model="addForm.pgPassword" type="password" show-password :placeholder="$t('map.auto.text80')" />
                </el-form-item>
                <el-form-item :label="$t('map.auto.text81')" prop="pgDatabase">
                  <el-input v-model="addForm.pgDatabase" :placeholder="$t('map.auto.gis')" />
                </el-form-item>
                <el-form-item label="Schema">
                  <el-input v-model="addForm.pgSchema" :placeholder="$t('map.auto.publicDefault')" />
                </el-form-item>
                <el-form-item :label="$t('map.auto.text82')">
                  <el-button
                    type="primary"
                    plain
                    icon="Connection"
                    :loading="addDlg.testing"
                    :disabled="!pgFormReady"
                    @click="testPgConnection"
                  >
                    {{ $t('map.auto.text188') }}
                  </el-button>
                  <span v-if="addDlg.testResult" class="text-xs ml-2" :class="addDlg.testResult.ok ? 'text-green-600' : 'text-red-500'">
                    {{ addDlg.testResult.detail }}
                  </span>
                </el-form-item>
              </template>
              <el-form-item v-if="!isAddPgStore" :label="$t('map.auto.text49')">
                <el-tag v-if="selectedDirInfo?.storeType" :type="canPublish ? 'success' : 'info'" size="small">
                  {{ canPublish ? $t('map.auto.text49') : $t('map.auto.text83') }}
                </el-tag>
                <span v-else class="text-xs text-gray-400">{{ $t('map.auto.pleaseSelectCatalog') }}</span>
              </el-form-item>
              <el-form-item v-if="!isAddPgStore && publishableServices.length" :label="$t('map.auto.type2')">
                <el-checkbox-group v-model="addForm.serviceTypes" size="small">
                  <el-checkbox v-for="s in publishableServices" :key="s" :label="s" :value="s">{{ s.toUpperCase() }}</el-checkbox>
                </el-checkbox-group>
              </el-form-item>
              <el-form-item :label="$t('map.auto.remark')" prop="remark">
                <el-input v-model="addForm.remark" type="textarea" :rows="2" />
              </el-form-item>
            </el-form>
          </div>
        </div>
      </div>

      <template #footer>
        <el-button @click="addDlg.visible = false">{{ $t('map.auto.cancel') }}</el-button>
        <el-button type="primary" :loading="addDlg.saving" @click="saveAdd">{{ $t('map.auto.ok') }}</el-button>
      </template>
    </el-dialog>

    <!-- 编辑数据源（保持原有对话框） -->
    <el-dialog v-model="editDlg.visible" :title="$t('map.auto.text84')" width="680px" append-to-body>
      <el-form ref="editFormRef" :model="editForm" :rules="editRules" label-width="150px">
        <el-form-item :label="$t('map.auto.text51')" prop="workspaceId">
          <el-select v-model="editForm.workspaceId" style="width: 100%" :disabled="isEditPgStore">
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('map.auto.name')" prop="name">
          <el-input v-model="editForm.name" :placeholder="$t('map.auto.wuhanCog')" :disabled="isEditPgStore" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.type')" prop="storeType">
          <el-select v-model="editForm.storeType" style="width: 100%" :disabled="isEditPgStore" @change="onEditTypeChange">
            <el-option v-for="t in storeTypeOptions" :key="t.value" :label="t.label" :value="t.value" />
          </el-select>
          <span v-if="isEditPgStore" class="text-xs text-gray-400">{{ $t('map.auto.postgisEditOnlyConnection') }}</span>
        </el-form-item>
        <el-form-item v-if="!isEditPgStore" :label="$t('map.auto.catalog2')" prop="basePath">
          <el-input v-model="editForm.basePath" :placeholder="$t('map.auto.dataRootOfCatalogDDataCog')" />
          <span class="text-xs text-gray-400">{{ $t('map.auto.catalog5') }}{{ editExtHint }}。{{ $t('map.auto.text85') }}</span>
        </el-form-item>
        <template v-else>
          <el-form-item :label="$t('map.auto.text86')" prop="pgDsn">
            <el-input
              v-model="editForm.pgDsn"
              type="password"
              show-password
              :placeholder="editForm.pgDsnConfigured ? $t('map.auto.text87') : 'postgres://user:pass@host:5432/db'"
            />
            <span class="text-xs text-gray-400">{{ $t('map.auto.saveOneShow') }}</span>
          </el-form-item>
          <el-form-item label="Schema" prop="pgSchema">
            <el-input v-model="editForm.pgSchema" placeholder="public" />
          </el-form-item>
        </template>
        <el-form-item :label="$t('map.auto.remark')" prop="remark">
          <el-input v-model="editForm.remark" type="textarea" :rows="2" :disabled="isEditPgStore" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editDlg.visible = false">{{ $t('map.auto.cancel') }}</el-button>
        <el-button type="primary" :loading="editDlg.saving" @click="saveEdit">{{ $t('map.auto.ok') }}</el-button>
      </template>
    </el-dialog>

    <!-- 导入 SHP：仅 PostGIS 数据源 -->
    <ImportShpDialog
      v-model="importDlg.visible"
      :store="importDlg.store"
      :data-root="wsDataRoot(importDlg.store)"
      @imported="onShpImported"
    />

    <!-- 专用新建 PostGIS 数据源弹窗 -->
    <PgDatasourceDlg
      v-model="pgDlg.visible"
      :workspaces="workspaces"
      :default-workspace-id="wsId || workspaces[0]?.id || 0"
      @created="onPgCreated"
    />
  </div>
</template>

<script setup name="MapCatalogStore" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import {
  addStore,
  deleteStore,
  discoverStore,
  listStores,
  listWorkspaces,
  restoreStore,
  testStoreConnection,
  testStoreConnectionPreflight,
  updateStore
} from '@/api/map/catalog';
import { DirEntry, ImportShpResp, StoreBody, StoreVO, WorkspaceVO } from '@/api/map/catalog/types';
import CatalogToolbar from './components/CatalogToolbar.vue';
import DirTree from './components/DirTree.vue';
import ImportShpDialog from './components/ImportShpDialog.vue';
import PgDatasourceDlg from './components/pg_datasource_dlg.vue';
import { useCaps } from './composables/useCatalog';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const route = useRoute();
const { caps, loadCaps } = useCaps();

const loading = ref(false);
const wsId = ref<number | undefined>(undefined);
const includeDeleted = ref(false);
const discoverDepth = ref(2);
const workspaces = ref<WorkspaceVO[]>([]);
const rows = ref<StoreVO[]>([]);
const total = ref(0);

const page = ref(1);
const pageSize = ref(10);
watch([page, pageSize], () => {
  load();
});
watch([wsId, includeDeleted], () => {
  page.value = 1;
  load();
});
const busy = ref<{ row: number; kind: string }>({ row: 0, kind: '' });
const pgDlg = reactive({ visible: false });

const openPgAdd = () => {
  pgDlg.visible = true;
};

const onPgCreated = async () => {
  await load();
};

const wsName = (id: number) => workspaces.value.find((w) => w.id === id)?.name || `#${id}`;

const load = async () => {
  loading.value = true;
  try {
    const params: { workspaceId?: number; includeDeleted?: number; page?: number; pageSize?: number } = {};
    if (wsId.value) params.workspaceId = wsId.value;
    if (includeDeleted.value) params.includeDeleted = 1;
    params.page = page.value;
    params.pageSize = pageSize.value;
    const res = await listStores(params);
    rows.value = res.data.stores || [];
    total.value = res.data.total || 0;
  } finally {
    loading.value = false;
  }
};

const loadWorkspaces = async () => {
  const res = await listWorkspaces();
  workspaces.value = res.data.workspaces || [];
};

// ================= 类型清单 =================
const storeTypeOptions = computed(() => {
  const dir = (caps.value?.store_types || []).map((st) => ({
    value: st.store_type,
    label: `${st.store_type}${st.extensions?.length ? '（' + st.extensions.join('/') + '）' : ''}${
      st.store_type === 'arcgis_dir' ? t('map.auto.arcgisCacheCatalog') : st.store_type === 'mvt_dir' ? t('map.auto.mvtCatalog') : ''
    }`
  }));
  return [...dir, { value: 'postgis', label: t('map.auto.postgisPostGIS') }];
});

// 数据类型 → 发布能力（来自 discover.rs data_type_caps）
const dataTypeStoreTypeMap: Record<string, string> = {
  cog: 'cog_dir',
  geojson: 'geojson_dir',
  pmtiles: 'pmtiles_dir',
  pmtiles_mvt: 'pmtiles_dir',
  mbtiles: 'mbtiles_dir',
  mbtiles_mvt: 'mbtiles_dir',
  '3dtiles': '3dtiles_dir',
  arcgis: 'arcgis_dir',
  mvt: 'mvt_dir',
  postgis_table: 'postgis',
  shapefile: 'shapefile_dir',
};

const getDataTypeFromStoreType = (storeType: string): string | null => {
  const cap = caps.value?.data_types?.find((d) => {
    const mapped = dataTypeStoreTypeMap[d.data_type];
    return mapped === storeType;
  });
  return cap?.data_type || null;
};

const isPublishableStoreType = (storeType: string): boolean => {
  const dt = getDataTypeFromStoreType(storeType);
  if (!dt) return false;
  const cap = caps.value?.data_types?.find((d) => d.data_type === dt);
  return cap?.publishable || false;
};

// ================= 新建数据源（三栏布局） =================
const addDlg = reactive({
  visible: false,
  saving: false,
  workspaceId: 0 as number,
  testing: false,
  testResult: null as { ok: boolean; detail: string } | null
});
const addFormRef = ref<ElFormInstance>();
const dirTreeRef = ref<InstanceType<typeof DirTree> | null>(null);

const addForm = reactive({
  name: '',
  storeType: '',
  basePath: '',
  remark: '',
  serviceTypes: [] as string[],
  // PostGIS 连接参数
  pgHost: '',
  pgPort: 54321,
  pgUsername: '',
  pgPassword: '',
  pgDatabase: '',
  pgSchema: 'public'
});

/** 新建表单是否选中了 PostGIS 类型 */
const isAddPgStore = computed(() => addForm.storeType === 'postgis');

/** PostGIS 表单是否填齐了可测试连接的必要字段 */
const pgFormReady = computed(
  () => !!addForm.pgHost.trim() && !!addForm.pgUsername.trim() && !!addForm.pgPassword && !!addForm.pgDatabase.trim()
);

const selectedDirInfo = ref<{ path: string; storeType: string; label: string; entries: DirEntry[] } | null>(null);

const dirDetectHint = computed(() => {
  const info = selectedDirInfo.value;
  if (!info?.label) return '';
  return info.storeType
    ? `${t('map.auto.detected')}${info.label} → ${info.storeType}`
    : `${t('map.auto.detected')}${info.label}${t('map.auto.theCurrentEngineCannotRegisterThisDataSour')}`;
});

const selectedWsDataRoot = computed(() => {
  const ws = workspaces.value.find((w) => w.id === addDlg.workspaceId);
  return ws?.dataRoot || '';
});

const canPublish = computed(() => {
  if (!selectedDirInfo.value?.storeType) return false;
  return isPublishableStoreType(selectedDirInfo.value.storeType);
});

const publishableServices = computed(() => {
  if (!selectedDirInfo.value?.storeType) return [];
  const dt = getDataTypeFromStoreType(selectedDirInfo.value.storeType);
  if (!dt) return [];
  const cap = caps.value?.data_types?.find((d) => d.data_type === dt);
  return cap?.default_service_types || [];
});

const addRules = {
  name: [
    { required: true, message: t('map.auto.nameisRequired'), trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,128}$/, message: t('map.auto.text88'), trigger: 'blur' }
  ],
  storeType: [{ required: true, message: t('map.auto.pleaseSelectType'), trigger: 'change' }],
  basePath: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (!isAddPgStore.value && !v?.trim()) cb(new Error(t('map.auto.catalog6')));
        else cb();
      },
      trigger: 'change'
    }
  ],
  pgHost: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v?.trim()) cb(new Error(t('map.auto.hostisRequired')));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgUsername: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v?.trim()) cb(new Error(t('map.auto.userisRequired')));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgPassword: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v) cb(new Error(t('map.auto.isRequired')));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgDatabase: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v?.trim()) cb(new Error(t('map.auto.isRequired2')));
        else cb();
      },
      trigger: 'blur'
    }
  ]
};

/** 切换新建数据源类型：PostGIS 清掉目录字段，目录型清掉连接字段 */
const onAddTypeChange = () => {
  addDlg.testResult = null;
  if (isAddPgStore.value) {
    addForm.basePath = '';
    selectedDirInfo.value = null;
  } else {
    addForm.pgHost = '';
    addForm.pgPort = 54321;
    addForm.pgUsername = '';
    addForm.pgPassword = '';
    addForm.pgDatabase = '';
    addForm.pgSchema = 'public';
  }
};

/** 新建 PostGIS 数据源的「测试连接」：用表单字段直接探测，不落库 */
const testPgConnection = async () => {
  addDlg.testing = true;
  addDlg.testResult = null;
  try {
    const res = await testStoreConnectionPreflight({
      host: addForm.pgHost.trim(),
      port: addForm.pgPort,
      username: addForm.pgUsername.trim(),
      password: addForm.pgPassword,
      database: addForm.pgDatabase.trim()
    });
    addDlg.testResult = { ok: !!res.data.ok, detail: res.data.detail || t('map.auto.success') };
  } catch (e: any) {
    addDlg.testResult = { ok: false, detail: e?.message || t('map.auto.failed') };
  } finally {
    addDlg.testing = false;
  }
};

const onAddWsChange = () => {
  selectedDirInfo.value = null;
  Object.assign(addForm, {
    name: '',
    storeType: '',
    basePath: '',
    remark: '',
    serviceTypes: [],
    pgHost: '',
    pgPort: 54321,
    pgUsername: '',
    pgPassword: '',
    pgDatabase: '',
    pgSchema: 'public'
  });
  addDlg.testResult = null;
  nextTick(() => dirTreeRef.value?.refresh());
};

const onDirSelect = (payload: { path: string; storeType: string; label: string; entries: DirEntry[] }) => {
  selectedDirInfo.value = payload;
  const parts = payload.path.replace(/\\/g, '/').split('/').filter(Boolean);
  const dirName = parts[parts.length - 1] || '';
  addForm.name = dirName.replace(/[^A-Za-z0-9_-]/g, '_');
  addForm.basePath = payload.path;
  addForm.storeType = payload.storeType;
  addForm.serviceTypes = payload.storeType ? [...publishableServices.value] : [];
};

const openAdd = () => {
  selectedDirInfo.value = null;
  Object.assign(addForm, {
    name: '',
    storeType: '',
    basePath: '',
    remark: '',
    serviceTypes: [],
    pgHost: '',
    pgPort: 54321,
    pgUsername: '',
    pgPassword: '',
    pgDatabase: '',
    pgSchema: 'public'
  });
  addDlg.testResult = null;
  addDlg.workspaceId = wsId.value || workspaces.value[0]?.id || 0;
  addDlg.visible = true;
  nextTick(() => {
    addFormRef.value?.clearValidate();
    dirTreeRef.value?.refresh();
  });
};

const saveAdd = async () => {
  const valid = await addFormRef.value?.validate().catch(() => false);
  if (!valid) return;
  addDlg.saving = true;
  try {
    // PostGIS：由表单字段拼装 DSN；目录型：basePath 已在表单填好。
    // 用户信息按 RFC 3986 编码（encodeURIComponent），避免密码含 @ : / # ? 时拼坏连接串。
    const pgDsn = isAddPgStore.value
      ? `postgres://${encodeURIComponent(addForm.pgUsername.trim())}:${encodeURIComponent(addForm.pgPassword)}@${addForm.pgHost.trim()}:${addForm.pgPort}/${encodeURIComponent(addForm.pgDatabase.trim())}`
      : '';
    const body: StoreBody = {
      workspaceId: addDlg.workspaceId,
      name: addForm.name,
      storeType: addForm.storeType,
      basePath: addForm.basePath,
      pgDsn,
      pgSchema: isAddPgStore.value ? addForm.pgSchema.trim() || 'public' : '',
      remark: addForm.remark
    };
    await addStore(body);
    proxy?.$modal.msgSuccess(t('map.auto.text89'));
    addDlg.visible = false;
    await load();
  } finally {
    addDlg.saving = false;
  }
};

// ================= 编辑数据源 =================
const editDlg = reactive({ visible: false, saving: false });
const editFormRef = ref<ElFormInstance>();
const editForm = reactive<StoreBody & { id: number; pgDsnConfigured: boolean }>({
  id: 0,
  workspaceId: 0,
  name: '',
  storeType: 'cog_dir',
  basePath: '',
  pgDsn: '',
  pgSchema: 'public',
  remark: '',
  pgDsnConfigured: false
});

const isEditPgStore = computed(() => editForm.storeType === 'postgis');

const editRules = {
  workspaceId: [{ required: true, message: t('map.auto.pleaseSelect2'), trigger: 'change' }],
  name: [
    { required: true, message: t('map.auto.nameisRequired'), trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,128}$/, message: t('map.auto.text88'), trigger: 'blur' }
  ],
  storeType: [{ required: true, message: t('map.auto.pleaseSelectType'), trigger: 'change' }],
  basePath: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (!isEditPgStore.value && !v?.trim()) cb(new Error(t('map.auto.catalogcatalog')));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgDsn: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (!isEditPgStore.value) return cb();
        if (!v?.trim() && !editForm.pgDsnConfigured) return cb(new Error(t('map.auto.postgis')));
        if (v?.trim() && !/^postgres(ql)?:\/\//.test(v.trim())) return cb(new Error('须为 postgres:// 或 postgresql:// 连接串'));
        cb();
      },
      trigger: 'blur'
    }
  ]
};

const editExtHint = computed(() => {
  const hit = caps.value?.store_types.find((t) => t.store_type === editForm.storeType);
  return hit?.extensions?.length ? `${t('map.auto.byExtension')}${hit.extensions.join('/')}${t('map.auto.detect')}` : t('map.auto.detectByDatasetDirectory');
});

const onEditTypeChange = () => {
  if (isEditPgStore.value) editForm.basePath = '';
  else editForm.pgDsn = '';
};

const openEdit = (s: StoreVO) => {
  Object.assign(editForm, {
    id: s.id,
    workspaceId: s.workspaceId,
    name: s.name,
    storeType: s.storeType,
    basePath: s.basePath,
    pgDsn: '',
    pgSchema: s.pgSchema,
    remark: s.remark,
    pgDsnConfigured: s.pgDsnConfigured
  });
  editDlg.visible = true;
  nextTick(() => editFormRef.value?.clearValidate());
};

const saveEdit = async () => {
  const valid = await editFormRef.value?.validate().catch(() => false);
  if (!valid) return;
  editDlg.saving = true;
  try {
    const { id, pgDsnConfigured, ...body } = editForm;
    await updateStore(id, body);
    proxy?.$modal.msgSuccess(t('map.auto.save4'));
    editDlg.visible = false;
    await load();
  } finally {
    editDlg.saving = false;
  }
};

// ================= 行内动作 =================
const handleTestConn = async (s: StoreVO) => {
  busy.value = { row: s.id, kind: 'connect' };
  try {
    const res = await testStoreConnection(s.id);
    if (res.data.ok) proxy?.$modal.msgSuccess(res.data.detail || t('map.auto.normal2'));
    else proxy?.$modal.msgWarning(res.data.detail || t('map.auto.text90'));
  } finally {
    busy.value = { row: 0, kind: '' };
  }
};

const handleDiscover = async (s: StoreVO) => {
  busy.value = { row: s.id, kind: 'discover' };
  try {
    const res = await discoverStore(s.id, s.isPostgis ? { schema: s.pgSchema } : { depth: discoverDepth.value });
    proxy?.$modal.msgSuccess(`${t('map.auto.text73')}${res.data.discovered}${t('map.auto.refresh3')}${res.data.stored}${t('map.auto.text74')}`);
    await load();
  } finally {
    busy.value = { row: 0, kind: '' };
  }
};

const handleRemove = async (s: StoreVO) => {
  await proxy?.$modal.confirm(
    `${t('map.auto.confirmdelete')}${s.name}${t('map.auto.text93')}${s.resourceCount}${t('map.auto.dataLayerPublishedServices')}${s.isPostgis ? t('map.auto.text91') : t('map.auto.text92')}${t('map.auto.text94')}`
  );
  const res = await deleteStore(s.id);
  proxy?.$modal.msgSuccess(`${t('map.auto.delete5')}${res.data?.resources ?? 0}${t('map.auto.layer3')}`);
  await load();
};

const handleRestore = async (s: StoreVO) => {
  await proxy?.$modal.confirm(`${t('map.auto.confirm5')}${s.name}${t('map.auto.dataLayerPublishedServices2')}`);
  await restoreStore(s.id);
  proxy?.$modal.msgSuccess(t('map.auto.text75'));
  await load();
};

// ================= 导入 SHP =================
const importDlg = reactive<{ visible: boolean; store: StoreVO | null }>({ visible: false, store: null });

const wsDataRoot = (s: StoreVO | null) => (s ? workspaces.value.find((w) => w.id === s.workspaceId)?.dataRoot || '' : '');

const openImport = (s: StoreVO) => {
  importDlg.store = s;
  importDlg.visible = true;
};

const onShpImported = async (r: ImportShpResp) => {
  proxy?.$modal.msgSuccess(`${t('map.auto.imported')}${r.schema}.${r.table} (${r.features}${t('map.auto.featuresSRID')}${r.srid})`);
  const s = importDlg.store;
  if (!s) {
    await load();
    return;
  }
  // 导入后立即重新发现，让新表出现在数据图层列表
  try {
    await discoverStore(s.id, { schema: s.pgSchema });
  } catch {
    // 发现失败不阻断：用户可手动点「只读发现」
  }
  await load();
};

onMounted(async () => {
  const q = Number(route.query.workspaceId);
  if (q > 0) wsId.value = q;
  await Promise.all([loadCaps(), loadWorkspaces(), load()]);
});
</script>

<style scoped>
.link-type {
  color: var(--el-color-primary);
}

.dir-detect-hint {
  color: var(--el-text-color-secondary);
}

.dir-detect-warn {
  color: var(--el-color-warning);
}

.store-add-layout {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.store-add-top {
  padding-bottom: 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.store-add-main {
  display: flex;
  gap: 16px;
  min-height: 400px;
}

.store-add-left {
  flex: 0 0 320px;
  overflow: auto;
}

.store-add-right {
  flex: 1;
  overflow: auto;
}
</style>
