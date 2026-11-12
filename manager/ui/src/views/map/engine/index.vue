<template>
  <div class="p-2">
    <!-- 操作条 -->
    <el-card shadow="hover" class="mb-[10px]">
      <el-form :inline="true">
        <el-form-item>
          <span class="text-xs text-gray-400">
            管理代理（服务总览/日志/样式/缓存等 /map/* 请求）转发到<b>默认引擎</b>；
            无默认引擎时回落 manager.xml 的 mapsvc 配置。
          </span>
        </el-form-item>
        <el-form-item>
          <el-button icon="Refresh" @click="getList">刷新</el-button>
          <el-button type="info" plain icon="Document" @click="openEvents()">事件日志</el-button>
          <el-button
            v-hasPermi="['map:engine:edit']"
            type="primary"
            plain
            icon="Plus"
            @click="openAdd"
          >
            登记引擎
          </el-button>
        </el-form-item>
      </el-form>
    </el-card>

    <el-card shadow="hover">
      <el-table v-loading="loading" border :data="pagedEngines" style="width: 100%">
        <el-table-column label="序号" width="50" type="index" align="center" />
        <el-table-column label="引擎 ID" align="center" prop="engineId" min-width="110">
          <template #default="{ row }">
            <span>{{ row.engineId }}</span>
            <el-tag v-if="row.isDefault === '1'" type="success" size="small" effect="dark" class="ml-1">默认</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="名称" align="center" prop="engineName" min-width="150" :show-overflow-tooltip="true" />
        <el-table-column label="类型" align="center" prop="engineType" width="120">
          <template #default="{ row }">
            <el-tag type="info" effect="plain">{{ row.engineType }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="Base URL" align="center" prop="baseUrl" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column label="服务数" align="center" width="80">
          <template #default="scope">
            {{ serviceCountMap[scope.row.engineId] ?? '—' }}
          </template>
        </el-table-column>
        <el-table-column label="Token" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="row.tokenConfigured ? 'warning' : 'info'" effect="plain" size="small">
              {{ row.tokenConfigured ? '已配置' : '无' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="状态" align="center" width="90">
          <template #default="{ row }">
            <el-tag :type="row.status === '0' ? 'success' : 'danger'" effect="dark" size="small">
              {{ row.status === '0' ? '启用' : '停用' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="健康" align="center" width="170">
          <template #default="{ row }">
            <template v-if="healthMap[row.engineId]">
              <el-tag :type="healthMap[row.engineId].up ? 'success' : 'danger'" effect="dark" size="small">
                {{ healthMap[row.engineId].up ? '在线' : '不可达' }}
              </el-tag>
              <span class="ml-1 text-xs text-gray-400">{{ healthMap[row.engineId].latencyMs }}ms</span>
            </template>
            <span v-else class="text-xs text-gray-400">未检测</span>
          </template>
        </el-table-column>
        <el-table-column label="备注" align="center" prop="remark" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column label="操作" align="center" width="310" fixed="right">
          <template #default="{ row }">
            <el-button link type="info" icon="View" @click="openDetail(row)">详情</el-button>
            <el-button link type="success" icon="Connection" @click="probeOne(row)">探测</el-button>
            <el-button
              v-if="row.isDefault !== '1' && row.status === '0'"
              v-hasPermi="['map:engine:edit']"
              link
              type="warning"
              icon="Star"
              @click="handleSetDefault(row)"
            >
              设默认
            </el-button>
            <el-button
              v-hasPermi="['map:engine:edit']"
              link
              type="primary"
              icon="Edit"
              @click="openEdit(row)"
            >
              编辑
            </el-button>
            <el-button
              v-hasPermi="['map:engine:edit']"
              link
              type="danger"
              icon="Delete"
              @click="handleDelete(row)"
            >
              删除
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
    <el-dialog v-model="dlg.visible" :title="dlg.isEdit ? '编辑引擎' : '登记引擎'" width="560px" append-to-body>
      <el-form ref="formRef" :model="form" :rules="rules" label-width="100px">
        <el-form-item label="引擎 ID" prop="engineId">
          <el-input v-model="form.engineId" :disabled="dlg.isEdit" placeholder="如 map-api-2（字母/数字/-/_）" />
        </el-form-item>
        <el-form-item label="名称" prop="engineName">
          <el-input v-model="form.engineName" placeholder="如 昆明节点 map-api" />
        </el-form-item>
        <el-form-item label="类型" prop="engineType">
          <el-select v-model="form.engineType" style="width: 100%">
            <el-option label="r-mapserver (map-api)" value="r-mapserver" />
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
            :placeholder="dlg.isEdit && form.tokenConfigured ? '已配置，留空保持不变' : '可选，与 map-api admin.token 一致'"
          />
        </el-form-item>
        <el-form-item label="状态" prop="status">
          <el-radio-group v-model="form.status">
            <el-radio value="0">启用</el-radio>
            <el-radio value="1">停用</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item label="设为默认" prop="isDefault">
          <el-switch v-model="form.isDefault" active-value="1" inactive-value="0" />
          <span class="ml-2 text-xs text-gray-400">管理代理将转发到默认引擎</span>
        </el-form-item>
        <el-form-item label="排序" prop="sort">
          <el-input-number v-model="form.sort" :min="0" :max="999" />
        </el-form-item>
        <el-form-item label="备注" prop="remark">
          <el-input v-model="form.remark" type="textarea" :rows="2" placeholder="部署位置、责任人等" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dlg.visible = false">取消</el-button>
        <el-button type="primary" :loading="dlg.saving" @click="handleSave">确定</el-button>
      </template>
    </el-dialog>

    <!-- 引擎详情抽屉：概览（运行环境/服务统计）+ 事件日志 -->
    <el-drawer v-model="detailVisible" :title="`引擎详情 — ${detail?.engine.engineName || ''}`" size="720px" append-to-body>
      <div v-loading="detailLoading">
        <template v-if="detail">
          <el-tabs v-model="detailTab">
            <el-tab-pane label="概览" name="overview">
              <el-descriptions :column="2" border size="small" class="mb-3">
                <el-descriptions-item label="引擎 ID">{{ detail.engine.engineId }}</el-descriptions-item>
                <el-descriptions-item label="名称">{{ detail.engine.engineName }}</el-descriptions-item>
                <el-descriptions-item label="类型">{{ detail.engine.engineType }}</el-descriptions-item>
                <el-descriptions-item label="Base URL">{{ detail.engine.baseUrl }}</el-descriptions-item>
                <el-descriptions-item label="状态">
                  <el-tag :type="detail.engine.status === '0' ? 'success' : 'danger'" size="small">
                    {{ detail.engine.status === '0' ? '启用' : '停用' }}
                  </el-tag>
                </el-descriptions-item>
                <el-descriptions-item label="默认引擎">
                  <el-tag :type="detail.engine.isDefault === '1' ? 'success' : 'info'" size="small">
                    {{ detail.engine.isDefault === '1' ? '是' : '否' }}
                  </el-tag>
                </el-descriptions-item>
                <el-descriptions-item label="存活探测" :span="2">
                  <el-tag :type="detail.health.up ? 'success' : 'danger'" effect="dark" size="small">
                    {{ detail.health.up ? '在线' : '不可达' }}
                  </el-tag>
                  <span class="ml-2 text-xs text-gray-400">{{ detail.health.detail }}（{{ detail.health.latencyMs }}ms）</span>
                </el-descriptions-item>
              </el-descriptions>

              <!-- 服务统计 -->
              <h4 class="mb-2">服务统计</h4>
              <el-descriptions :column="2" border size="small" class="mb-3">
                <el-descriptions-item label="服务总数">{{ detail.serviceTotal }}</el-descriptions-item>
                <el-descriptions-item label="在服数量">
                  <el-tag :type="detail.serviceServing === detail.serviceTotal ? 'success' : 'warning'" size="small">
                    {{ detail.serviceServing }} / {{ detail.serviceTotal }}
                  </el-tag>
                </el-descriptions-item>
              </el-descriptions>

              <!-- 服务器运行环境（map-api /admin/components） -->
              <h4 class="mb-2">服务器运行环境</h4>
              <template v-if="detail.components">
                <el-descriptions :column="2" border size="small">
                  <el-descriptions-item label="PROJ 坐标库">
                    <el-tag :type="detail.components.proj_loaded ? 'success' : 'danger'" size="small">
                      {{ detail.components.proj_loaded ? '已加载' : '未加载(跨 CRS 不可用)' }}
                    </el-tag>
                    <el-tooltip :content="detail.components.proj_loaded ? CRS_SUPPORT_HINT : PROJ_MISSING_HINT" placement="bottom">
                      <el-icon style="margin-left: 6px; vertical-align: middle"><question-filled /></el-icon>
                    </el-tooltip>
                  </el-descriptions-item>
                  <el-descriptions-item label="PostGIS">
                    <el-tag :type="detail.components.pg_enabled ? 'success' : 'info'" size="small">
                      {{ detail.components.pg_enabled ? '已启用' : '未启用' }}
                    </el-tag>
                  </el-descriptions-item>
                  <el-descriptions-item label="服务目录" :span="2">
                    {{ detail.components.catalog_root }}
                    <el-tag :type="detail.components.catalog_root_exists ? 'success' : 'danger'" size="small" class="ml-1">
                      {{ detail.components.catalog_root_exists ? '存在' : '缺失' }}
                    </el-tag>
                  </el-descriptions-item>
                  <el-descriptions-item label="3D Tiles 目录" :span="2">
                    {{ detail.components.tilesets_dir }}
                    <el-tag :type="detail.components.tilesets_dir_exists ? 'success' : 'danger'" size="small" class="ml-1">
                      {{ detail.components.tilesets_dir_exists ? '存在' : '缺失' }}
                    </el-tag>
                  </el-descriptions-item>
                  <el-descriptions-item label="在服图层">{{ detail.components.served_layers }}</el-descriptions-item>
                  <el-descriptions-item label="3D Tiles">{{ detail.components.tilesets }}</el-descriptions-item>
                  <el-descriptions-item label="缓存占用">{{ detail.components.cache_mib }} MiB</el-descriptions-item>
                  <el-descriptions-item label="缓存 TTL">{{ detail.components.cache_ttl_secs }}s</el-descriptions-item>
                </el-descriptions>
              </template>
              <el-alert v-else type="warning" :closable="false" title="引擎不可达，无法获取运行环境信息" />
            </el-tab-pane>

            <el-tab-pane label="最近事件" name="events">
              <el-table :data="detail.recentEvents" border size="small" max-height="520">
                <el-table-column label="时间" prop="createTime" width="160" />
                <el-table-column label="类型" width="110" align="center">
                  <template #default="scope">
                    <el-tag :type="eventTypeTag(scope.row.eventType)" size="small">{{ eventTypeLabel(scope.row.eventType) }}</el-tag>
                  </template>
                </el-table-column>
                <el-table-column label="说明" prop="summary" :show-overflow-tooltip="true" />
              </el-table>
            </el-tab-pane>
          </el-tabs>
        </template>
      </div>
    </el-drawer>

    <!-- 事件日志抽屉（全局，可按引擎/类型过滤） -->
    <el-drawer v-model="evVisible" title="引擎事件日志" size="860px" append-to-body>
      <el-form :inline="true" class="mb-2">
        <el-form-item label="引擎">
          <el-select v-model="evQuery.engineId" placeholder="全部" clearable style="width: 180px">
            <el-option v-for="e in engineList" :key="e.engineId" :label="e.engineName" :value="e.engineId" />
          </el-select>
        </el-form-item>
        <el-form-item label="类型">
          <el-select v-model="evQuery.eventType" placeholder="全部" clearable style="width: 160px">
            <el-option v-for="(label, key) in EVENT_TYPES" :key="key" :label="label" :value="key" />
          </el-select>
        </el-form-item>
        <el-form-item>
          <el-button type="primary" icon="Search" @click="loadEvents">查询</el-button>
        </el-form-item>
      </el-form>
      <el-table v-loading="evLoading" :data="evList" border size="small" max-height="560">
        <el-table-column label="时间" prop="createTime" width="165" />
        <el-table-column label="引擎" width="130" align="center">
          <template #default="scope">{{ engineLabelOf(scope.row.engineId) }}</template>
        </el-table-column>
        <el-table-column label="类型" width="110" align="center">
          <template #default="scope">
            <el-tag :type="eventTypeTag(scope.row.eventType)" size="small">{{ eventTypeLabel(scope.row.eventType) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="说明" prop="summary" min-width="220" :show-overflow-tooltip="true" />
        <el-table-column label="详情" prop="detail" min-width="180" :show-overflow-tooltip="true" />
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        共 {{ evList.length }} 条（最近记录优先，最多 500 条；巡检周期 30 秒）
      </div>
    </el-drawer>
  </div>
</template>

<script setup name="MapEngine" lang="ts">
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
    proxy?.$modal.msgSuccess(`${row.engineName} 在线（${res.data.latencyMs}ms）`);
  } else {
    proxy?.$modal.msgWarning(res.data.detail || `${row.engineName} 不可达`);
  }
};

/** 设为默认引擎 */
const handleSetDefault = async (row: EngineVO) => {
  await proxy?.$modal.confirm(`确认将「${row.engineName}」设为默认引擎？管理代理将转发到该引擎。`);
  await setDefaultEngine(row.engineId);
  proxy?.$modal.msgSuccess('已设为默认引擎');
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
  engine_online: '引擎上线',
  engine_offline: '引擎离线',
  service_added: '服务登记',
  service_removed: '服务移除',
  service_state: '服务状态变更',
  service_up: '服务恢复在服',
  service_down: '服务离线'
};
const eventTypeLabel = (t: string) => EVENT_TYPES[t] || t;
const eventTypeTag = (t: string) => {
  const map: Record<string, string> = {
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
      ? `确认删除默认引擎「${row.engineName}」？删除后管理代理回落 manager.xml 的 mapsvc 配置。`
      : `确认删除引擎「${row.engineName}」？`;
  await proxy?.$modal.confirm(tip);
  await deleteEngine(row.engineId);
  proxy?.$modal.msgSuccess('删除成功');
  await getList();
};

// ---------- 新增/编辑 ----------
const formRef = ref<FormInstance>();
const dlg = reactive({ visible: false, isEdit: false, saving: false });

const emptyForm = () => ({
  engineId: '',
  engineName: '',
  engineType: 'r-mapserver',
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
    { required: true, message: '引擎 ID 不能为空', trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]+$/, message: '仅允许字母/数字/-/_', trigger: 'blur' }
  ],
  engineName: [{ required: true, message: '名称不能为空', trigger: 'blur' }],
  baseUrl: [
    { required: true, message: 'Base URL 不能为空', trigger: 'blur' },
    { pattern: /^https?:\/\//, message: '必须以 http:// 或 https:// 开头', trigger: 'blur' }
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
      proxy?.$modal.msgSuccess('更新成功');
    } else {
      await addEngine(body);
      proxy?.$modal.msgSuccess('登记成功');
    }
    dlg.visible = false;
    await getList();
  } finally {
    dlg.saving = false;
  }
};

onMounted(getList);
</script>
