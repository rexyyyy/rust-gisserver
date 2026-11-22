<template>
  <div class="p-2">
    <!-- 操作条 -->
    <el-card shadow="hover" class="mb-[10px]">
      <el-form :inline="true">
        <el-form-item>
          <span class="text-xs text-gray-400">
            {{ $t('map.auto.proxyForwardHint') }}
          </span>
        </el-form-item>
        <el-form-item>
          <el-button icon="Refresh" @click="getList">{{ $t('map.auto.refresh') }}</el-button>
          <el-button type="info" plain icon="Document" @click="openEvents()">{{ $t('map.auto.eventLog') }}</el-button>
          <el-button
            v-hasPermi="['map:engine:edit']"
            type="primary"
            plain
            icon="Plus"
            @click="openAdd"
          >
            {{ $t('map.auto.text112') }}
          </el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <el-card shadow="hover">
      <el-table v-loading="loading" border :data="pagedEngines" style="width: 100%">
        <el-table-column :label="$t('map.auto.no')" width="50" type="index" align="center" />
        <el-table-column :label="$t('map.auto.engineIdLabel')" align="center" prop="engineId" min-width="110">
          <template #default="{ row }">
            <span>{{ row.engineId }}</span>
            <el-tag v-if="row.isDefault === '1'" type="success" size="small" effect="dark" class="ml-1">{{ $t('map.auto.default10') }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.name')" align="center" prop="engineName" min-width="150" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.type')" align="center" prop="engineType" width="120">
          <template #default="{ row }">
            <el-tag type="info" effect="plain">{{ row.engineType }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="Base URL" align="center" prop="baseUrl" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.serviceCount')" align="center" width="80">
          <template #default="scope">
            {{ serviceCountMap[scope.row.engineId] ?? '—' }}
          </template>
        </el-table-column>
        <el-table-column label="Token" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="row.tokenConfigured ? 'warning' : 'info'" effect="plain" size="small">
              {{ row.tokenConfigured ? $t('map.auto.text145') : $t('map.auto.none') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.status')" align="center" width="90">
          <template #default="{ row }">
            <el-tag :type="row.status === '0' ? 'success' : 'danger'" effect="dark" size="small">
              {{ row.status === '0' ? $t('map.auto.enable') : $t('map.auto.disable') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.health')" align="center" width="170">
          <template #default="{ row }">
            <template v-if="healthMap[row.engineId]">
              <el-tag :type="healthMap[row.engineId].up ? 'success' : 'danger'" effect="dark" size="small">
                {{ healthMap[row.engineId].up ? $t('map.auto.text102') : $t('map.auto.text90') }}
              </el-tag>
              <span class="ml-1 text-xs text-gray-400">{{ healthMap[row.engineId].latencyMs }}ms</span>
            </template>
            <span v-else class="text-xs text-gray-400">{{ $t('map.auto.notChecked') }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.remark')" align="center" prop="remark" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.actions')" align="center" width="310" fixed="right">
          <template #default="{ row }">
            <el-button link type="info" icon="View" @click="openDetail(row)">{{ $t('map.auto.text123') }}</el-button>
            <el-button link type="success" icon="Connection" @click="probeOne(row)">{{ $t('map.auto.probe') }}</el-button>
            <el-button
              v-if="row.isDefault !== '1' && row.status === '0'"
              v-hasPermi="['map:engine:edit']"
              link
              type="warning"
              icon="Star"
              @click="handleSetDefault(row)"
            >
              {{ $t('map.auto.setDefault') }}
            </el-button>
            <el-button
              v-hasPermi="['map:engine:edit']"
              link
              type="primary"
              icon="Edit"
              @click="openEdit(row)"
            >
              {{ $t('map.auto.text99') }}
            </el-button>
            <el-button
              v-hasPermi="['map:engine:edit']"
              link
              type="danger"
              icon="Delete"
              @click="handleDelete(row)"
            >
              {{ $t('map.auto.delete6') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-3 flex justify-end">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :page-sizes="[10, 20, 50, 100]"
          :total="engineList.length"
          layout="total, sizes, prev, pager, next, jumper"
          background
        />
      </div>
    </el-card>

    <!-- 新增/编辑对话框 -->
    <el-dialog v-model="dlg.visible" :title="dlg.isEdit ? $t('map.auto.text111') : $t('map.auto.text112')" width="620px" append-to-body>
      <el-form ref="formRef" :model="form" :rules="rules" label-width="140px">
        <el-form-item :label="$t('map.auto.id')" prop="engineId">
          <el-input v-model="form.engineId" :disabled="dlg.isEdit" :placeholder="$t('map.auto.mapApi2')" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.name')" prop="engineName">
          <el-input v-model="form.engineName" :placeholder="$t('map.auto.mapApi3')" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.type')" prop="engineType">
          <el-select v-model="form.engineType" style="width: 100%">
            <el-option label="rust-gisserver (map-api)" value="rust-gisserver" />
          </el-select>
        </el-form-item>
        <el-form-item label="Base URL" prop="baseUrl">
          <el-input v-model="form.baseUrl" placeholder="http://127.0.0.1:8095" />
        </el-form-item>
        <el-form-item label="Admin Token" prop="adminToken">
          <el-input
            v-model="form.adminToken"
            type="password"
            show-password
            :placeholder="dlg.isEdit && form.tokenConfigured ? $t('map.auto.text87') : $t('map.auto.mapApiAdminTokenOne')"
          />
        </el-form-item>
        <el-form-item :label="$t('map.auto.status')" prop="status">
          <el-radio-group v-model="form.status">
            <el-radio value="0">{{ $t('map.auto.enable') }}</el-radio>
            <el-radio value="1">{{ $t('map.auto.disable') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="$t('map.auto.default6')" prop="isDefault">
          <el-switch v-model="form.isDefault" active-value="1" inactive-value="0" />
          <span class="ml-2 text-xs text-gray-400">{{ $t('map.auto.default7') }}</span>
        </el-form-item>
        <el-form-item :label="$t('map.auto.order')" prop="sort">
          <el-input-number v-model="form.sort" :min="0" :max="999" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.remark')" prop="remark">
          <el-input v-model="form.remark" type="textarea" :rows="2" :placeholder="$t('map.auto.text113')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dlg.visible = false">{{ $t('map.auto.cancel') }}</el-button>
        <el-button type="primary" :loading="dlg.saving" @click="handleSave">{{ $t('map.auto.ok') }}</el-button>
      </template>
    </el-dialog>

    <!-- 引擎详情抽屉：概览（运行环境/服务统计）+ 事件日志 -->
    <el-drawer v-model="detailVisible" :title="`${t('map.auto.text114')}${detail?.engine.engineName || ''}`" size="720px" append-to-body>
      <div v-loading="detailLoading">
        <template v-if="detail">
          <el-tabs v-model="detailTab">
            <el-tab-pane :label="$t('map.auto.text115')" name="overview">
              <el-descriptions :column="2" border size="small" class="mb-3">
                <el-descriptions-item :label="$t('map.auto.id')">{{ detail.engine.engineId }}</el-descriptions-item>
                <el-descriptions-item :label="$t('map.auto.name')">{{ detail.engine.engineName }}</el-descriptions-item>
                <el-descriptions-item :label="$t('map.auto.type')">{{ detail.engine.engineType }}</el-descriptions-item>
                <el-descriptions-item label="Base URL">{{ detail.engine.baseUrl }}</el-descriptions-item>
                <el-descriptions-item :label="$t('map.auto.status')">
                  <el-tag :type="detail.engine.status === '0' ? 'success' : 'danger'" size="small">
                    {{ detail.engine.status === '0' ? $t('map.auto.enable') : $t('map.auto.disable') }}
                  </el-tag>
                </el-descriptions-item>
                <el-descriptions-item :label="$t('map.auto.default5')">
                  <el-tag :type="detail.engine.isDefault === '1' ? 'success' : 'info'" size="small">
                    {{ detail.engine.isDefault === '1' ? $t('map.auto.yes2') : $t('map.auto.no2') }}
                  </el-tag>
                </el-descriptions-item>
                <el-descriptions-item :label="$t('map.auto.text116')" :span="2">
                  <el-tag :type="detail.health.up ? 'success' : 'danger'" effect="dark" size="small">
                    {{ detail.health.up ? $t('map.auto.text102') : $t('map.auto.text90') }}
                  </el-tag>
                  <span class="ml-2 text-xs text-gray-400">{{ detail.health.detail }}（{{ detail.health.latencyMs }}ms）</span>
                </el-descriptions-item>
              </el-descriptions>

              <!-- 服务统计 -->
              <h4 class="mb-2">{{ $t('map.auto.text117') }}</h4>
              <el-descriptions :column="2" border size="small" class="mb-3">
                <el-descriptions-item :label="$t('map.auto.text118')">{{ detail.serviceTotal }}</el-descriptions-item>
                <el-descriptions-item :label="$t('map.auto.inService4')">
                  <el-tag :type="detail.serviceServing === detail.serviceTotal ? 'success' : 'warning'" size="small">
                    {{ detail.serviceServing }} / {{ detail.serviceTotal }}
                  </el-tag>
                </el-descriptions-item>
              </el-descriptions>

              <!-- 服务器运行环境（map-api /admin/components） -->
              <h4 class="mb-2">{{ $t('map.auto.rows2') }}</h4>
              <template v-if="detail.components">
                <el-descriptions :column="2" border size="small">
                  <el-descriptions-item :label="$t('map.auto.proj2')">
                    <el-tag :type="detail.components.proj_loaded ? 'success' : 'danger'" size="small">
                      {{ detail.components.proj_loaded ? $t('map.auto.text104') : $t('map.auto.crs10') }}
                    </el-tag>
                    <el-tooltip :content="detail.components.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                      <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
                    </el-tooltip>
                  </el-descriptions-item>
                  <el-descriptions-item label="PostGIS">
                    <el-tag :type="detail.components.pg_enabled ? 'success' : 'info'" size="small">
                      {{ detail.components.pg_enabled ? $t('map.auto.enable2') : $t('map.auto.enable3') }}
                    </el-tag>
                  </el-descriptions-item>
                  <el-descriptions-item :label="$t('map.auto.catalog7')" :span="2">
                    {{ detail.components.catalog_root }}
                    <el-tag :type="detail.components.catalog_root_exists ? 'success' : 'danger'" size="small" class="ml-1">
                      {{ detail.components.catalog_root_exists ? $t('map.auto.text119') : $t('map.auto.text120') }}
                    </el-tag>
                  </el-descriptions-item>
                  <el-descriptions-item :label="$t('map.auto.n3dTilesCatalog')" :span="2">
                    {{ detail.components.tilesets_dir }}
                    <el-tag :type="detail.components.tilesets_dir_exists ? 'success' : 'danger'" size="small" class="ml-1">
                      {{ detail.components.tilesets_dir_exists ? $t('map.auto.text119') : $t('map.auto.text120') }}
                    </el-tag>
                  </el-descriptions-item>
                  <el-descriptions-item :label="$t('map.auto.inServiceLayer')">{{ detail.components.served_layers }}</el-descriptions-item>
                  <el-descriptions-item label="3D Tiles">{{ detail.components.tilesets }}</el-descriptions-item>
                  <el-descriptions-item :label="$t('map.auto.cache3')">{{ detail.components.cache_mib }} MiB</el-descriptions-item>
                  <el-descriptions-item :label="$t('map.auto.cacheTTL')">{{ detail.components.cache_ttl_secs }}s</el-descriptions-item>
                </el-descriptions>
              </template>
              <el-alert v-else type="warning" :closable="false" :title="$t('map.auto.rowsinfo')" />
            </el-tab-pane>

            <el-tab-pane :label="$t('map.auto.text121')" name="events">
              <el-table :data="detail.recentEvents" border size="small" max-height="520">
                <el-table-column :label="$t('map.auto.hour')" prop="createTime" width="160" />
                <el-table-column :label="$t('map.auto.type')" width="110" align="center">
                  <template #default="scope">
                    <el-tag :type="eventTypeTag(scope.row.eventType)" size="small">{{ eventTypeLabel(scope.row.eventType) }}</el-tag>
                  </template>
                </el-table-column>
                <el-table-column :label="$t('map.auto.text20')" prop="summary" :show-overflow-tooltip="true" />
              </el-table>
            </el-tab-pane>
          </el-tabs>
        </template>
      </div>
    </el-drawer>

    <!-- 事件日志抽屉（全局，可按引擎/类型过滤） -->
    <el-drawer v-model="evVisible" :title="$t('map.auto.day')" size="860px" append-to-body>
      <el-form :inline="true" class="mb-2">
        <el-form-item :label="$t('map.auto.text122')">
          <el-select v-model="evQuery.engineId" :placeholder="$t('map.auto.all2')" clearable style="width: 180px">
            <el-option v-for="e in engineList" :key="e.engineId" :label="e.engineName" :value="e.engineId" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('map.auto.type')">
          <el-select v-model="evQuery.eventType" :placeholder="$t('map.auto.all2')" clearable style="width: 160px">
            <el-option v-for="(label, key) in EVENT_TYPES" :key="key" :label="label" :value="key" />
          </el-select>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" icon="Search" @click="loadEvents">{{ $t('map.auto.query2') }}</el-button>
        </el-form-item>
      </el-form>
      <el-table v-loading="evLoading" :data="evList" border size="small" max-height="560">
        <el-table-column :label="$t('map.auto.hour')" prop="createTime" width="165" />
        <el-table-column :label="$t('map.auto.text122')" width="130" align="center">
          <template #default="scope">{{ engineLabelOf(scope.row.engineId) }}</template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.type')" width="110" align="center">
          <template #default="scope">
            <el-tag :type="eventTypeTag(scope.row.eventType)" size="small">{{ eventTypeLabel(scope.row.eventType) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text20')" prop="summary" min-width="220" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.text123')" prop="detail" min-width="180" :show-overflow-tooltip="true" />
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        {{ $t('map.auto.text139') }}{{ evList.length }} {{ $t('map.auto.text197') }}500 {{ $t('map.auto.text198') }}30 {{ $t('map.auto.second3') }}
      </div>
    </el-drawer>
  </div>
</template>

<script setup name="MapEngine" lang="ts">
import { useI18n } from 'vue-i18n';
import type { FormInstance, FormRules } from 'element-plus';
const { t } = useI18n();
import { addEngine, deleteEngine, engineDetail, engineHealth, listEngines, listEngineEvents, setDefaultEngine, updateEngine } from '@/api/map/engine';
import { EngineDetailResp, EngineEvent, EngineHealth, EngineVO } from '@/api/map/engine/types';
import { listServices } from '@/api/map/service';
import { CRS_SUPPORT_HINT, PROJ_MISSING_HINT } from '@/utils/crs';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const loading = ref(false);
const engineList = ref<EngineVO[]>([]);
const healthMap = ref<Record<string, EngineHealth>>({});
// 各引擎承载的服务数（来自跨引擎聚合清单）
const serviceCountMap = ref<Record<string, number>>({});

const page = ref(1);
const pageSize = ref(10);
const pagedEngines = computed(() => {
  const start = (page.value - 1) * pageSize.value;
  return engineList.value.slice(start, start + pageSize.value);
});

const getList = async () => {
  loading.value = true;
  try {
    const res = await listEngines();
    engineList.value = res.data.engines || [];
    // 顺带刷新服务数（聚合失败不影响引擎列表展示）
    try {
      const sRes = await listServices();
      const map: Record<string, number> = {};
      for (const s of sRes.data.services || []) {
        const eid = s.engineId || '';
        map[eid] = (map[eid] || 0) + 1;
      }
      serviceCountMap.value = map;
    } catch {
      /* 忽略：聚合不可用时服务数显示 — */
    }
  } finally {
    loading.value = false;
  }
};

/** 探测单个引擎 */
const probeOne = async (row: EngineVO) => {
  const res = await engineHealth(row.engineId);
  healthMap.value[row.engineId] = res.data;
  if (res.data.up) {
    proxy?.$modal.msgSuccess(`${row.engineName}${t('map.auto.text124')}${res.data.latencyMs}ms)`);
  } else {
    proxy?.$modal.msgWarning(res.data.detail || `${row.engineName}${t('map.auto.text125')}`);
  }
};

/** 设为默认引擎 */
const handleSetDefault = async (row: EngineVO) => {
  await proxy?.$modal.confirm(`${t('map.auto.confirm4')}${row.engineName}${t('map.auto.default8')}`);
  await setDefaultEngine(row.engineId);
  proxy?.$modal.msgSuccess(t('map.auto.default9'));
  await getList();
};

// ================= 引擎详情（概览 / 最近事件） =================
const detailVisible = ref(false);
const detailLoading = ref(false);
const detail = ref<EngineDetailResp | null>(null);
const detailTab = ref('overview');

const openDetail = async (row: EngineVO) => {
  detailVisible.value = true;
  detailLoading.value = true;
  detail.value = null;
  detailTab.value = 'overview';
  try {
    const res = await engineDetail(row.engineId);
    detail.value = res.data;
  } finally {
    detailLoading.value = false;
  }
};

// ================= 事件日志（全局抽屉） =================
const EVENT_TYPES: Record<string, string> = {
  engine_online: t('map.auto.text126'),
  engine_offline: t('map.auto.text127'),
  service_added: t('map.auto.text128'),
  service_removed: t('map.auto.text129'),
  service_state: t('map.auto.status2'),
  service_up: t('map.auto.inService5'),
  service_down: t('map.auto.text130')
};
const eventTypeLabel = (t: string) => EVENT_TYPES[t] || t;
const eventTypeTag = (t: string): 'success' | 'primary' | 'info' | 'warning' | 'danger' => {
  const map: Record<string, 'success' | 'primary' | 'info' | 'warning' | 'danger'> = {
    engine_online: 'success',
    engine_offline: 'danger',
    service_added: 'primary',
    service_removed: 'info',
    service_state: 'warning',
    service_up: 'success',
    service_down: 'danger'
  };
  return map[t] || 'info';
};
const engineLabelOf = (engineId: string) => engineList.value.find((e) => e.engineId === engineId)?.engineName || engineId;

const evVisible = ref(false);
const evLoading = ref(false);
const evQuery = reactive({ engineId: '', eventType: '', limit: 200 });
const evList = ref<EngineEvent[]>([]);

const loadEvents = async () => {
  evLoading.value = true;
  try {
    const res = await listEngineEvents({
      engineId: evQuery.engineId || undefined,
      eventType: evQuery.eventType || undefined,
      limit: evQuery.limit
    });
    evList.value = res.data.events || [];
  } finally {
    evLoading.value = false;
  }
};

const openEvents = async () => {
  evVisible.value = true;
  await loadEvents();
};

/** 删除引擎 */
const handleDelete = async (row: EngineVO) => {
  const tip =
    row.isDefault === '1'
      ? `${t('map.auto.confirmdeletedefault')}${row.engineName}${t('map.auto.deleteManagerXmlOfMapsvc')}`
      : `${t('map.auto.confirmdelete3')}${row.engineName}"?`;
  await proxy?.$modal.confirm(tip);
  await deleteEngine(row.engineId);
  proxy?.$modal.msgSuccess(t('map.auto.deletedSuccessfully'));
  await getList();
};

// ---------- 新增/编辑 ----------
const formRef = ref<FormInstance>();
const dlg = reactive({ visible: false, isEdit: false, saving: false });

const emptyForm = () => ({
  engineId: '',
  engineName: '',
  engineType: 'rust-gisserver',
  baseUrl: 'http://127.0.0.1:8095',
  adminToken: '',
  tokenConfigured: false,
  status: '0',
  isDefault: '0',
  sort: 0,
  remark: ''
});
const form = reactive(emptyForm());

const rules: FormRules = {
  engineId: [
    { required: true, message: t('map.auto.idIsRequired'), trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]+$/, message: t('map.auto.text131'), trigger: 'blur' }
  ],
  engineName: [{ required: true, message: t('map.auto.nameisRequired'), trigger: 'blur' }],
  baseUrl: [
    { required: true, message: t('map.auto.baseURLIsRequired'), trigger: 'blur' },
    { pattern: /^https?:\/\//, message: t('map.auto.baseUrlSchemeHint'), trigger: 'blur' }
  ]
};

const openAdd = () => {
  Object.assign(form, emptyForm());
  dlg.isEdit = false;
  dlg.visible = true;
  nextTick(() => formRef.value?.clearValidate());
};

const openEdit = (row: EngineVO) => {
  Object.assign(form, {
    engineId: row.engineId,
    engineName: row.engineName,
    engineType: row.engineType,
    baseUrl: row.baseUrl,
    adminToken: '',
    tokenConfigured: row.tokenConfigured,
    status: row.status,
    isDefault: row.isDefault,
    sort: row.sort,
    remark: row.remark
  });
  dlg.isEdit = true;
  dlg.visible = true;
  nextTick(() => formRef.value?.clearValidate());
};

const handleSave = async () => {
  await formRef.value?.validate();
  dlg.saving = true;
  try {
    const body = {
      engineId: form.engineId,
      engineName: form.engineName,
      engineType: form.engineType,
      baseUrl: form.baseUrl,
      adminToken: form.adminToken,
      status: form.status,
      isDefault: form.isDefault,
      sort: form.sort,
      remark: form.remark
    };
    if (dlg.isEdit) {
      await updateEngine(form.engineId, body);
      proxy?.$modal.msgSuccess(t('map.auto.success2'));
    } else {
      await addEngine(body);
      proxy?.$modal.msgSuccess(t('map.auto.success3'));
    }
    dlg.visible = false;
    await getList();
  } finally {
    dlg.saving = false;
  }
};

onMounted(getList);
</script>
