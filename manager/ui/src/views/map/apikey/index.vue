<template>
  <div class="app-container">
    <el-card shadow="never">
      <template #header>
        <span>API Key 管理（r-mapserver 访问凭证）</span>
      </template>

      <el-alert
        class="mb-3"
        type="info"
        show-icon
        :closable="false"
        title="一处生成，两种用途"
        description="管理面（admin）：manager 连接 server，点「应用到引擎」即写入默认引擎的 X-Admin-Token；客户端（data）：下发给客户端访问 /tiles /wms /wmts /mvt，支持 X-Data-Token、Authorization: Bearer 与 ?token=。server 侧启用强制校验需在 mapserver.xml 配置主令牌（admin token / data_token），本库启用中的 Key 由 r-mapserver 自动认可。"
      />

      <el-row :gutter="12" class="mb-3" align="middle">
        <el-col :span="14">
          <span class="text-sm">
            共 <b>{{ rows.length }}</b> 枚 Key，其中
            <el-tag size="small" type="success" effect="plain">{{ effectiveCount }}</el-tag>
            枚有效（启用且未过期）
          </span>
        </el-col>
        <el-col :span="10" class="text-right">
          <el-button icon="Refresh" :loading="loading" @click="getList">刷新</el-button>
          <el-button
            v-hasPermi="['map:apikey:edit']"
            type="primary"
            icon="Plus"
            @click="openCreate"
          >
            生成 API Key
          </el-button>
        </el-col>
      </el-row>

      <el-table v-loading="loading" :data="rows" border size="small" style="width: 100%">
        <el-table-column label="序号" type="index" width="55" align="center" />
        <el-table-column label="名称" prop="keyName" min-width="140" show-overflow-tooltip />
        <el-table-column label="Key" min-width="310">
          <template #default="{ row }">
            <span class="font-mono">{{ revealed[row.keyId] ? row.apiKey : masked(row.apiKey) }}</span>
            <el-button link type="primary" :icon="revealed[row.keyId] ? 'Hide' : 'View'" @click="toggleReveal(row.keyId)" />
            <el-button link type="info" icon="DocumentCopy" @click="copy(row.apiKey, 'Key 已复制')" />
          </template>
        </el-table-column>
        <el-table-column label="用途" width="175" align="center">
          <template #default="{ row }">
            <el-tag :type="row.purpose === 'admin' ? 'warning' : 'info'" effect="plain" size="small">
              {{ row.purpose === 'admin' ? '管理面 admin' : '客户端 data' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="状态" width="100" align="center">
          <template #default="{ row }">
            <el-tag :type="row.status !== '0' ? 'info' : row.expired ? 'danger' : 'success'" effect="dark" size="small">
              {{ row.status !== '0' ? '停用' : row.expired ? '已过期' : '启用' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="有效期" width="160" align="center">
          <template #default="{ row }">{{ row.expiresAt || '永久有效' }}</template>
        </el-table-column>
        <el-table-column label="创建时间" prop="createTime" width="160" align="center" />
        <el-table-column label="备注" prop="remark" min-width="130" show-overflow-tooltip />
        <el-table-column label="操作" width="350" fixed="right" align="center">
          <template #default="{ row }">
            <el-button link type="info" icon="InfoFilled" @click="openUsage(row)">用法</el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="primary"
              icon="Edit"
              @click="openEdit(row)"
            >
              编辑
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="warning"
              icon="RefreshRight"
              @click="onRotate(row)"
            >
              重置
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="success"
              icon="Connection"
              @click="onApply(row)"
            >
              {{ row.purpose === 'admin' ? '应用' : '下发' }}
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              :type="row.status === '0' ? 'warning' : 'success'"
              @click="onToggle(row)"
            >
              {{ row.status === '0' ? '停用' : '启用' }}
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="danger"
              icon="Delete"
              @click="onDelete(row)"
            >
              删除
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 生成 / 编辑 -->
    <el-dialog v-model="formOpen" :title="formMode === 'create' ? '生成 API Key' : '编辑 API Key'" width="560px" append-to-body>
      <el-form ref="formRef" :model="form" :rules="formRules" label-width="100px">
        <el-form-item label="名称" prop="keyName">
          <el-input v-model="form.keyName" placeholder="如：桌面客户端-昆明项目部 / manager 主通道" maxlength="50" show-word-limit />
        </el-form-item>
        <el-form-item label="用途" prop="purpose">
          <el-radio-group v-model="form.purpose">
            <el-radio value="data">客户端访问（data）</el-radio>
            <el-radio value="admin">管理面连接（admin）</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item label="状态">
          <el-radio-group v-model="form.status">
            <el-radio value="0">启用</el-radio>
            <el-radio value="1">停用</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item label="有效期">
          <div style="width: 100%">
            <el-radio-group v-model="form.expiresAt" class="mb-2">
              <el-radio-button value="">永不过期</el-radio-button>
              <el-radio-button :value="getExpiryDate(1)">1 个月</el-radio-button>
              <el-radio-button :value="getExpiryDate(3)">3 个月</el-radio-button>
              <el-radio-button :value="getExpiryDate(6)">6 个月</el-radio-button>
              <el-radio-button :value="getExpiryDate(12)">1 年</el-radio-button>
            </el-radio-group>
            <el-date-picker
              v-model="form.expiresAt"
              type="datetime"
              value-format="YYYY-MM-DD HH:mm:ss"
              placeholder="或选择自定义日期时间"
              style="width: 100%"
            />
          </div>
        </el-form-item>
        <el-form-item label="备注">
          <el-input v-model="form.remark" type="textarea" :rows="2" maxlength="250" show-word-limit placeholder="持有人、下发渠道、用途说明等" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="formOpen = false">取消</el-button>
        <el-button type="primary" :loading="submitting" @click="submitForm">
          {{ formMode === 'create' ? '生成' : '保存' }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 新 Key 一次性醒目展示（生成 / 重置） -->
    <el-dialog v-model="createdOpen" title="Key 已生成，请妥善保管" width="620px" append-to-body>
      <el-alert type="warning" show-icon :closable="false" class="mb-3" title="完整 Key 仅在管理端可见，下发时请使用安全通道" />
      <el-form label-width="110px">
        <el-form-item label="名称">{{ created?.keyName }}</el-form-item>
        <el-form-item label="用途">{{ created?.purposeName }}</el-form-item>
        <el-form-item label="API Key">
          <div style="width: 100%">
            <el-input :model-value="created?.apiKey" readonly class="font-mono" />
            <el-button class="mt-2" type="primary" plain icon="DocumentCopy" @click="copy(created?.apiKey || '', 'Key 已复制')">
              复制 Key
            </el-button>
          </div>
        </el-form-item>
        <el-form-item label="请求头示例">
          <div class="font-mono">{{ created?.usage?.sample }}</div>
          <el-button link type="primary" icon="DocumentCopy" @click="copy(created?.usage?.sample || '', '示例已复制')">复制</el-button>
        </el-form-item>
        <el-form-item v-if="created?.usage?.bearer" label="Bearer 示例">
          <div class="font-mono">{{ created.usage.bearer }}</div>
        </el-form-item>
        <el-form-item v-if="created?.usage?.query" label="URL 参数示例">
          <div class="font-mono">{{ created.usage.query }}</div>
        </el-form-item>
      </el-form>
      <div class="text-sm">{{ created?.usage?.note }}</div>
      <template #footer>
        <el-button type="primary" @click="createdOpen = false">我已保存</el-button>
      </template>
    </el-dialog>

    <!-- 用法 / 下发说明 -->
    <el-dialog v-model="usageOpen" title="API Key 使用方式" width="620px" append-to-body>
      <el-form label-width="110px" v-if="usageTarget">
        <el-form-item label="名称">{{ usageTarget.keyName }}</el-form-item>
        <el-form-item label="API Key">
          <div style="width: 100%">
            <el-input :model-value="usageTarget.apiKey" readonly class="font-mono" />
            <el-button class="mt-2" plain icon="DocumentCopy" @click="copy(usageTarget.apiKey, 'Key 已复制')">复制 Key</el-button>
          </div>
        </el-form-item>
        <el-form-item label="请求头">
          <div class="font-mono">{{ usageTarget.usage.sample }}</div>
          <el-button link type="primary" icon="DocumentCopy" @click="copy(usageTarget.usage.sample, '示例已复制')">复制</el-button>
        </el-form-item>
        <el-form-item v-if="usageTarget.usage.bearer" label="Bearer">
          <div class="font-mono">{{ usageTarget.usage.bearer }}</div>
        </el-form-item>
        <el-form-item v-if="usageTarget.usage.query" label="URL 参数">
          <div class="font-mono">{{ usageTarget.usage.query }}</div>
          <el-button link type="primary" icon="DocumentCopy" @click="copy(usageTarget.usage.query || '', '参数已复制')">复制</el-button>
        </el-form-item>
      </el-form>
      <el-alert v-if="applyResult" class="mb-3" :type="applyResult.applied ? 'success' : 'info'" show-icon :closable="false"
        :title="applyResult.applied
          ? `已写入默认引擎 ${applyResult.engineId}，manager 连接 server 立即使用该 Key`
          : '客户端 Key 无需写入引擎，复制上方用法下发即可'" />
      <div class="text-sm">{{ usageTarget?.usage?.note }}</div>
      <template #footer>
        <el-button type="primary" @click="usageOpen = false">关闭</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts" name="MapApiKey">
import { computed, onMounted, reactive, ref } from 'vue';
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus';
import {
  applyApiKey,
  createApiKey,
  deleteApiKey,
  listApiKeys,
  rotateApiKey,
  updateApiKey
} from '@/api/map/apikey';
import { ApiKeyApplyResp, ApiKeyBody, ApiKeyVO } from '@/api/map/apikey/types';

const loading = ref(false);
const rows = ref<ApiKeyVO[]>([]);
const revealed = ref<Record<string, boolean>>({});
const effectiveCount = computed(() => rows.value.filter((r) => r.effective).length);

/** request 拦截器在不同部署下可能已剥一层 data，这里做兼容取值。 */
function pick<T>(r: any): T {
  return (r?.data ?? r) as T;
}

function masked(key: string): string {
  if (!key || key.length < 20) return key || '';
  return `${key.slice(0, 16)}••••••••••••${key.slice(-6)}`;
}

async function copy(text: string, tip: string) {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    ElMessage.success(tip);
  } catch {
    ElMessage.warning('浏览器不允许访问剪贴板，请手动选中复制');
  }
}

function toggleReveal(id: string) {
  revealed.value = { ...revealed.value, [id]: !revealed.value[id] };
}

async function getList() {
  loading.value = true;
  try {
    const r = await listApiKeys();
    rows.value = pick<{ keys: ApiKeyVO[] }>(r)?.keys ?? [];
  } finally {
    loading.value = false;
  }
}

function toBody(row: ApiKeyVO, status?: string): ApiKeyBody {
  return {
    keyName: row.keyName,
    purpose: row.purpose,
    status: status ?? row.status,
    expiresAt: row.expiresAt || '',
    remark: row.remark || ''
  };
}

// ---------- 生成 / 编辑 ----------
const formOpen = ref(false);
const formMode = ref<'create' | 'edit'>('create');
const submitting = ref(false);
const formRef = ref<FormInstance>();
const editingId = ref('');
const form = reactive<ApiKeyBody>({
  keyName: '',
  purpose: 'data',
  status: '0',
  expiresAt: '',
  remark: ''
});
const formRules: FormRules = {
  keyName: [{ required: true, message: '请填写名称', trigger: 'blur' }],
  purpose: [{ required: true, message: '请选择用途', trigger: 'change' }]
};

function resetForm() {
  form.keyName = '';
  form.purpose = 'data';
  form.status = '0';
  form.expiresAt = '';
  form.remark = '';
  formRef.value?.clearValidate();
}

/** 计算 N 个月后的到期时间（YYYY-MM-DD HH:mm:ss） */
function getExpiryDate(months: number): string {
  const now = new Date();
  now.setMonth(now.getMonth() + months);
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, '0');
  const d = String(now.getDate()).padStart(2, '0');
  const h = String(now.getHours()).padStart(2, '0');
  const min = String(now.getMinutes()).padStart(2, '0');
  const s = String(now.getSeconds()).padStart(2, '0');
  return `${y}-${m}-${d} ${h}:${min}:${s}`;
}

function openCreate() {
  formMode.value = 'create';
  editingId.value = '';
  resetForm();
  formOpen.value = true;
}

function openEdit(row: ApiKeyVO) {
  formMode.value = 'edit';
  editingId.value = row.keyId;
  form.keyName = row.keyName;
  form.purpose = row.purpose;
  form.status = row.status;
  form.expiresAt = row.expiresAt || '';
  form.remark = row.remark || '';
  formRef.value?.clearValidate();
  formOpen.value = true;
}

async function submitForm() {
  if (!(await formRef.value?.validate().catch(() => false))) return;
  submitting.value = true;
  try {
    const body: ApiKeyBody = {
      keyName: form.keyName,
      purpose: form.purpose,
      status: form.status,
      expiresAt: form.expiresAt || '',
      remark: form.remark || ''
    };
    if (formMode.value === 'create') {
      const created = pick<ApiKeyVO>(await createApiKey(body));
      showCreated(created);
      ElMessage.success('API Key 已生成');
    } else {
      await updateApiKey(editingId.value, body);
      ElMessage.success('保存成功');
    }
    formOpen.value = false;
    await getList();
  } finally {
    submitting.value = false;
  }
}

// ---------- 新 Key 展示 ----------
const createdOpen = ref(false);
const created = ref<ApiKeyVO | null>(null);
function showCreated(v: ApiKeyVO) {
  created.value = v;
  revealed.value = { ...revealed.value, [v.keyId]: true };
  createdOpen.value = true;
}

// ---------- 用法 / 应用 ----------
const usageOpen = ref(false);
const usageTarget = ref<ApiKeyVO | null>(null);
const applyResult = ref<ApiKeyApplyResp | null>(null);

function openUsage(row: ApiKeyVO) {
  usageTarget.value = row;
  applyResult.value = null;
  usageOpen.value = true;
}

async function onApply(row: ApiKeyVO) {
  try {
    const res = pick<ApiKeyApplyResp>(await applyApiKey(row.keyId));
    usageTarget.value = row;
    applyResult.value = res;
    usageOpen.value = true;
    if (res.applied) ElMessage.success('已应用到默认引擎');
  } catch (e: any) {
    ElMessage.error(e?.msg || e?.message || '应用失败');
  }
}

// ---------- 重置 / 启停 / 删除 ----------
async function onRotate(row: ApiKeyVO) {
  await ElMessageBox.confirm(
    `重置后「${row.keyName}」的旧 Key 立即失效，已下发到客户端的凭证需同步更新。是否继续？`,
    '重置 Key',
    { type: 'warning' }
  );
  const fresh = pick<ApiKeyVO>(await rotateApiKey(row.keyId));
  showCreated(fresh);
  await getList();
}

async function onToggle(row: ApiKeyVO) {
  const next = row.status === '0' ? '1' : '0';
  await updateApiKey(row.keyId, toBody(row, next));
  ElMessage.success(next === '0' ? '已启用' : '已停用');
  await getList();
}

async function onDelete(row: ApiKeyVO) {
  await ElMessageBox.confirm(`确定删除「${row.keyName}」？该 Key 将立即失效且不可恢复。`, '删除 API Key', {
    type: 'warning'
  });
  await deleteApiKey(row.keyId);
  ElMessage.success('已删除');
  await getList();
}

onMounted(() => getList());
</script>
