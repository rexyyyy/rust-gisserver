<template>
  <div class="p-2">
    <!-- 红线提示：数据源只是指针，不落地数据 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        数据源只登记<b>指针</b>：目录路径或 PostGIS 连接串。「只读发现」仅扫描文件头 / 归档元数据 /
        geometry_columns，不导入数据、不复制文件、不重投影落库。
      </template>
    </el-alert>

    <el-card shadow="hover">
      <CatalogToolbar>
        <template #filters>
          <el-select v-model="wsId" placeholder="全部工作空间" clearable style="width: 210px" @change="load">
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
          <el-select v-model="discoverDepth" style="width: 120px" title="目录型数据源「只读发现」的递归层深">
            <el-option v-for="d in [1, 2, 3, 4]" :key="d" :label="`发现深度 ${d}`" :value="d" />
          </el-select>
          <el-checkbox v-model="includeDeleted" @change="load">显示已删除</el-checkbox>
        </template>
        <template #actions>
          <el-button icon="Refresh" @click="load">刷新</el-button>
          <el-button v-hasPermi="['map:catalog:edit']" type="primary" plain icon="Plus" @click="openAdd">新建数据源</el-button>
        </template>
      </CatalogToolbar>

      <el-table v-loading="loading" border :data="rows" size="small" style="width: 100%">
        <el-table-column label="名称" prop="name" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column label="工作空间" align="center" width="100">
          <template #default="{ row }">
            <router-link :to="`/data/workspace`" class="link-type">{{ wsName(row.workspaceId) }}</router-link>
          </template>
        </el-table-column>
        <el-table-column label="类型" align="center" width="120">
          <template #default="{ row }">
            <el-tag effect="plain" size="small">{{ row.storeType }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="数据定位" min-width="230" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span class="text-xs">{{ row.isPostgis ? row.pgDsnMasked || '未登记连接串' : row.basePath }}</span>
          </template>
        </el-table-column>
        <el-table-column label="图层" align="center" width="80">
          <template #default="{ row }">
            <router-link :to="`/data/resource?storeId=${row.id}`" class="link-type">{{ row.resourceCount }}</router-link>
          </template>
        </el-table-column>
        <el-table-column label="可发布" align="center" width="80" prop="publishableCount" />
        <el-table-column label="状态" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="row.delFlag === 1 ? 'danger' : 'success'" size="small" effect="dark">
              {{ row.delFlag === 1 ? '已删' : '正常' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="备注" prop="remark" min-width="160" :show-overflow-tooltip="true" />
        <el-table-column label="操作" align="center" width="400" fixed="right">
          <template #default="{ row }">
            <el-button
              v-hasPermi="['map:catalog:edit']"
              link
              type="primary"
              icon="Edit"
              :disabled="row.delFlag === 1"
              @click="openEdit(row)"
            >
              编辑
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
              测试连接
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
              只读发现
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
              导入shp
            </el-button>
            <el-button v-if="row.delFlag === 1" v-hasPermi="['map:catalog:edit']" link type="success" icon="RefreshLeft" @click="handleRestore(row)">
              恢复
            </el-button>
            <el-button
              v-else
              v-hasPermi="['map:catalog:edit']"
              link
              type="danger"
              icon="Delete"
              @click="handleRemove(row)"
            >
              删除
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        共 {{ total }} 个数据源。「删除」只把指针与其下数据图层标记进垃圾箱，数据文件 / 库表本身不动；恢复指针后，
        被级联的数据图层与已发布服务还需到「数据图层」「已发布服务」页逐个恢复。目录型改动定位后需重新发现才刷新账本。
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
    <el-dialog v-model="addDlg.visible" title="新建数据源" width="900px" append-to-body>
      <div class="store-add-layout">
        <!-- 上部：选择工作空间 -->
        <div class="store-add-top">
          <el-select v-model="addDlg.workspaceId" placeholder="选择工作空间" style="width: 300px" @change="onAddWsChange">
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
            <el-form ref="addFormRef" :model="addForm" :rules="addRules" label-width="100px">
              <el-form-item label="名称" prop="name">
                <el-input v-model="addForm.name" placeholder="字母/数字/-/_（如 wuhan_cog）" />
              </el-form-item>
              <el-form-item label="类型" prop="storeType">
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
              <el-form-item v-if="!isAddPgStore" label="目录" prop="basePath">
                <el-input v-model="addForm.basePath" readonly placeholder="从左侧目录树选择" />
                <span class="text-xs text-gray-400">点击左侧目录逐级展开；识别到已知图层数据后停止展开并自动填入类型</span>
              </el-form-item>
              <!-- PostGIS 连接参数 -->
              <template v-else>
                <el-form-item label="主机" prop="pgHost">
                  <el-input v-model="addForm.pgHost" placeholder="如 127.0.0.1" />
                </el-form-item>
                <el-form-item label="端口" prop="pgPort">
                  <el-input-number v-model="addForm.pgPort" :min="1" :max="65535" controls-position="right" style="width: 100%" />
                </el-form-item>
                <el-form-item label="用户名" prop="pgUsername">
                  <el-input v-model="addForm.pgUsername" placeholder="postgres" />
                </el-form-item>
                <el-form-item label="密码" prop="pgPassword">
                  <el-input v-model="addForm.pgPassword" type="password" show-password placeholder="数据库密码" />
                </el-form-item>
                <el-form-item label="数据库" prop="pgDatabase">
                  <el-input v-model="addForm.pgDatabase" placeholder="如 gis" />
                </el-form-item>
                <el-form-item label="Schema">
                  <el-input v-model="addForm.pgSchema" placeholder="public（默认）" />
                </el-form-item>
                <el-form-item label="连接测试">
                  <el-button
                    type="primary"
                    plain
                    icon="Connection"
                    :loading="addDlg.testing"
                    :disabled="!pgFormReady"
                    @click="testPgConnection"
                  >
                    测试连接
                  </el-button>
                  <span v-if="addDlg.testResult" class="text-xs ml-2" :class="addDlg.testResult.ok ? 'text-green-600' : 'text-red-500'">
                    {{ addDlg.testResult.detail }}
                  </span>
                </el-form-item>
              </template>
              <el-form-item v-if="!isAddPgStore" label="可发布">
                <el-tag v-if="selectedDirInfo?.storeType" :type="canPublish ? 'success' : 'info'" size="small">
                  {{ canPublish ? '可发布' : '不可发布' }}
                </el-tag>
                <span v-else class="text-xs text-gray-400">请选择目录</span>
              </el-form-item>
              <el-form-item v-if="!isAddPgStore && publishableServices.length" label="服务类型">
                <el-checkbox-group v-model="addForm.serviceTypes" size="small">
                  <el-checkbox v-for="s in publishableServices" :key="s" :label="s" :value="s">{{ s.toUpperCase() }}</el-checkbox>
                </el-checkbox-group>
              </el-form-item>
              <el-form-item label="备注" prop="remark">
                <el-input v-model="addForm.remark" type="textarea" :rows="2" />
              </el-form-item>
            </el-form>
          </div>
        </div>
      </div>

      <template #footer>
        <el-button @click="addDlg.visible = false">取消</el-button>
        <el-button type="primary" :loading="addDlg.saving" @click="saveAdd">确定</el-button>
      </template>
    </el-dialog>

    <!-- 编辑数据源（保持原有对话框） -->
    <el-dialog v-model="editDlg.visible" title="编辑数据源" width="580px" append-to-body>
      <el-form ref="editFormRef" :model="editForm" :rules="editRules" label-width="110px">
        <el-form-item label="工作空间" prop="workspaceId">
          <el-select v-model="editForm.workspaceId" style="width: 100%">
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="名称" prop="name">
          <el-input v-model="editForm.name" placeholder="字母/数字/-/_（如 wuhan_cog）" />
        </el-form-item>
        <el-form-item label="类型" prop="storeType">
          <el-select v-model="editForm.storeType" style="width: 100%" @change="onEditTypeChange">
            <el-option v-for="t in storeTypeOptions" :key="t.value" :label="t.label" :value="t.value" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="!isEditPgStore" label="目录" prop="basePath">
          <el-input v-model="editForm.basePath" placeholder="引擎 data_root 之内的目录，如 D:/data/cog" />
          <span class="text-xs text-gray-400">只读扫描该目录，{{ editExtHint }}。数据不入库、不复制。</span>
        </el-form-item>
        <template v-else>
          <el-form-item label="连接串" prop="pgDsn">
            <el-input
              v-model="editForm.pgDsn"
              type="password"
              show-password
              :placeholder="editForm.pgDsnConfigured ? '已配置，留空保持不变' : 'postgres://user:pass@host:5432/db'"
            />
            <span class="text-xs text-gray-400">服务端原样保存，只用于转发引擎做只读发现；对外一律掩码显示。</span>
          </el-form-item>
          <el-form-item label="Schema" prop="pgSchema">
            <el-input v-model="editForm.pgSchema" placeholder="public" />
          </el-form-item>
        </template>
        <el-form-item label="备注" prop="remark">
          <el-input v-model="editForm.remark" type="textarea" :rows="2" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editDlg.visible = false">取消</el-button>
        <el-button type="primary" :loading="editDlg.saving" @click="saveEdit">确定</el-button>
      </template>
    </el-dialog>

    <!-- 导入 SHP：仅 PostGIS 数据源 -->
    <ImportShpDialog
      v-model="importDlg.visible"
      :store="importDlg.store"
      :data-root="wsDataRoot(importDlg.store)"
      @imported="onShpImported"
    />
  </div>
</template>

<script setup name="MapCatalogStore" lang="ts">
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
  const dir = (caps.value?.store_types || []).map((t) => ({
    value: t.store_type,
    label: `${t.store_type}${t.extensions?.length ? '（' + t.extensions.join('/') + '）' : ''}${
      t.store_type === 'arcgis_dir' ? '（ArcGIS 缓存目录）' : t.store_type === 'mvt_dir' ? '（MVT 瓦片目录）' : ''
    }`
  }));
  return [...dir, { value: 'postgis', label: 'postgis（PostGIS 库表）' }];
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
    ? `已识别：${info.label} → ${info.storeType}`
    : `已识别：${info.label}，当前引擎暂不支持登记为数据源类型`;
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
    { required: true, message: '名称不能为空', trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,128}$/, message: '仅字母、数字、下划线与短横线', trigger: 'blur' }
  ],
  storeType: [{ required: true, message: '请选择类型', trigger: 'change' }],
  basePath: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (!isAddPgStore.value && !v?.trim()) cb(new Error('请从左侧选择目录'));
        else cb();
      },
      trigger: 'change'
    }
  ],
  pgHost: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v?.trim()) cb(new Error('主机不能为空'));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgUsername: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v?.trim()) cb(new Error('用户名不能为空'));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgPassword: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v) cb(new Error('密码不能为空'));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgDatabase: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (isAddPgStore.value && !v?.trim()) cb(new Error('数据库名不能为空'));
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
    addDlg.testResult = { ok: !!res.data.ok, detail: res.data.detail || '连接成功' };
  } catch (e: any) {
    addDlg.testResult = { ok: false, detail: e?.message || '连接测试请求失败' };
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
    const pgDsn = isAddPgStore.value
      ? `postgres://${addForm.pgUsername.trim()}:${addForm.pgPassword}@${addForm.pgHost.trim()}:${addForm.pgPort}/${addForm.pgDatabase.trim()}`
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
    proxy?.$modal.msgSuccess('已创建');
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
  workspaceId: [{ required: true, message: '请选择工作空间', trigger: 'change' }],
  name: [
    { required: true, message: '名称不能为空', trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,128}$/, message: '仅字母、数字、下划线与短横线', trigger: 'blur' }
  ],
  storeType: [{ required: true, message: '请选择类型', trigger: 'change' }],
  basePath: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (!isEditPgStore.value && !v?.trim()) cb(new Error('目录型数据源必须填写目录'));
        else cb();
      },
      trigger: 'blur'
    }
  ],
  pgDsn: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        if (!isEditPgStore.value) return cb();
        if (!v?.trim() && !editForm.pgDsnConfigured) return cb(new Error('PostGIS 数据源必须填写连接串'));
        if (v?.trim() && !/^postgres(ql)?:\/\//.test(v.trim())) return cb(new Error('须为 postgres:// 或 postgresql:// 连接串'));
        cb();
      },
      trigger: 'blur'
    }
  ]
};

const editExtHint = computed(() => {
  const hit = caps.value?.store_types.find((t) => t.store_type === editForm.storeType);
  return hit?.extensions?.length ? `按扩展名 ${hit.extensions.join('/')} 识别` : '按数据集目录识别';
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
    proxy?.$modal.msgSuccess('已保存（指针变更需重新发现）');
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
    if (res.data.ok) proxy?.$modal.msgSuccess(res.data.detail || '连接正常');
    else proxy?.$modal.msgWarning(res.data.detail || '不可达');
  } finally {
    busy.value = { row: 0, kind: '' };
  }
};

const handleDiscover = async (s: StoreVO) => {
  busy.value = { row: s.id, kind: 'discover' };
  try {
    const res = await discoverStore(s.id, s.isPostgis ? { schema: s.pgSchema } : { depth: discoverDepth.value });
    proxy?.$modal.msgSuccess(`发现 ${res.data.discovered} 个实体，刷新账本 ${res.data.stored} 条`);
    await load();
  } finally {
    busy.value = { row: 0, kind: '' };
  }
};

const handleRemove = async (s: StoreVO) => {
  await proxy?.$modal.confirm(
    `确认删除数据源「${s.name}」？其下 ${s.resourceCount} 个数据图层将进垃圾箱，已发布服务随之停止（配置保留）。数据${s.isPostgis ? '库表' : '文件'}本身不受影响。`
  );
  const res = await deleteStore(s.id);
  proxy?.$modal.msgSuccess(`已删除，级联进垃圾箱 ${res.data?.resources ?? 0} 个图层`);
  await load();
};

const handleRestore = async (s: StoreVO) => {
  await proxy?.$modal.confirm(`确认恢复数据源「${s.name}」？其下数据图层与已发布服务需在各自页面逐个恢复。`);
  await restoreStore(s.id);
  proxy?.$modal.msgSuccess('已恢复');
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
  proxy?.$modal.msgSuccess(`已导入 ${r.schema}.${r.table}（${r.features} 个要素，SRID ${r.srid}）`);
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
