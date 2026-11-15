<template>
  <div class="app-container">
    <el-card shadow="never">
      <template #header>
        <span>{{ $t('map.auto.apiKeyRustGisserver') }}</span>
      </template>

      <el-alert
        class="mb-3"
        type="info"
        show-icon
        :closable="false"
        :title="$t('map.auto.one')"
        :description="$t('map.auto.adminManagerServerDefaultofXAdminTokenClie')"
      />

      <el-row :gutter="12" class="mb-3" align="middle">
        <el-col :span="14">
          <span class="text-sm">
            {{ $t('map.auto.text139') }}<b>{{ rows.length }}</b> {{ $t('map.auto.text159') }}
            <el-tag size="small" type="success" effect="plain">{{ effectiveCount }}</el-tag>
            {{ $t('map.auto.enable4') }}
          </span>
        </el-col>
        <el-col :span="10" class="text-right">
          <el-button icon="Refresh" :loading="loading" @click="getList">{{ $t('map.auto.refresh') }}</el-button>
          <el-button
            v-hasPermi="['map:apikey:edit']"
            type="primary"
            icon="Plus"
            @click="openCreate"
          >
            {{ $t('map.auto.text10') }}API Key
          </el-button>
        </el-col>
      </el-row>

      <el-table v-loading="loading" :data="rows" border size="small" style="width: 100%">
        <el-table-column :label="$t('map.auto.no')" type="index" width="55" align="center" />
        <el-table-column :label="$t('map.auto.name')" prop="keyName" min-width="140" show-overflow-tooltip />
        <el-table-column label="Key" min-width="310">
          <template #default="{ row }">
            <span class="font-mono">{{ revealed[row.keyId] ? row.apiKey : masked(row.apiKey) }}</span>
            <el-button link type="primary" :icon="revealed[row.keyId] ? 'Hide' : 'View'" @click="toggleReveal(row.keyId)" />
            <el-button link type="info" icon="DocumentCopy" @click="copy(row.apiKey, t('map.auto.keyCopied'))" />
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text')" width="175" align="center">
          <template #default="{ row }">
            <el-tag :type="row.purpose === 'admin' ? 'warning' : 'info'" effect="plain" size="small">
              {{ row.purpose === 'admin' ? $t('map.auto.admin') : $t('map.auto.clientData') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.status')" width="100" align="center">
          <template #default="{ row }">
            <el-tag :type="row.status !== '0' ? 'info' : row.expired ? 'danger' : 'success'" effect="dark" size="small">
              {{ row.status !== '0' ? $t('map.auto.disable') : row.expired ? $t('map.auto.text2') : $t('map.auto.enable') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text3')" width="160" align="center">
          <template #default="{ row }">{{ row.expiresAt || $t('map.auto.text4') }}</template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.createdAt')" prop="createTime" width="160" align="center" />
        <el-table-column :label="$t('map.auto.remark')" prop="remark" min-width="130" show-overflow-tooltip />
        <el-table-column :label="$t('map.auto.actions')" width="350" fixed="right" align="center">
          <template #default="{ row }">
            <el-button link type="info" icon="InfoFilled" @click="openUsage(row)">{{ $t('map.auto.text5') }}</el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="primary"
              icon="Edit"
              @click="openEdit(row)"
            >
              {{ $t('map.auto.text99') }}
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="warning"
              icon="RefreshRight"
              @click="onRotate(row)"
            >
              {{ $t('map.auto.reset2') }}
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="success"
              icon="Connection"
              @click="onApply(row)"
            >
              {{ row.purpose === 'admin' ? $t('map.auto.text6') : $t('map.auto.text7') }}
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              :type="row.status === '0' ? 'warning' : 'success'"
              @click="onToggle(row)"
            >
              {{ row.status === '0' ? $t('map.auto.disable') : $t('map.auto.enable') }}
            </el-button>
            <el-button
              v-hasPermi="['map:apikey:edit']"
              link
              type="danger"
              icon="Delete"
              @click="onDelete(row)"
            >
              {{ $t('map.auto.delete6') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 生成 / 编辑 -->
    <el-dialog v-model="formOpen" :title="formMode === 'create' ? $t('map.auto.apiKey') : $t('map.auto.apiKey2')" width="560px" append-to-body>
      <el-form ref="formRef" :model="form" :rules="formRules" label-width="100px">
        <el-form-item :label="$t('map.auto.name')" prop="keyName">
          <el-input v-model="form.keyName" :placeholder="$t('map.auto.clientManager')" maxlength="50" show-word-limit />
        </el-form-item>
        <el-form-item :label="$t('map.auto.text')" prop="purpose">
          <el-radio-group v-model="form.purpose">
            <el-radio value="data">{{ $t('map.auto.clientData2') }}</el-radio>
            <el-radio value="admin">{{ $t('map.auto.admin2') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="$t('map.auto.status')">
          <el-radio-group v-model="form.status">
            <el-radio value="0">{{ $t('map.auto.enable') }}</el-radio>
            <el-radio value="1">{{ $t('map.auto.disable') }}</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="$t('map.auto.text3')">
          <div style="width: 100%">
            <el-radio-group v-model="form.expiresAt" class="mb-2">
              <el-radio-button value="">{{ $t('map.auto.text8') }}</el-radio-button>
              <el-radio-button :value="getExpiryDate(1)">{{ $t('map.auto.n1Month') }}</el-radio-button>
              <el-radio-button :value="getExpiryDate(3)">{{ $t('map.auto.n3Month') }}</el-radio-button>
              <el-radio-button :value="getExpiryDate(6)">{{ $t('map.auto.n6Month') }}</el-radio-button>
              <el-radio-button :value="getExpiryDate(12)">{{ $t('map.auto.n1Year') }}</el-radio-button>
            </el-radio-group>
            <el-date-picker
              v-model="form.expiresAt"
              type="datetime"
              value-format="YYYY-MM-DD HH:mm:ss"
              :placeholder="$t('map.auto.dayhour')"
              style="width: 100%"
            />
          </div>
        </el-form-item>
        <el-form-item :label="$t('map.auto.remark')">
          <el-input v-model="form.remark" type="textarea" :rows="2" maxlength="250" show-word-limit :placeholder="$t('map.auto.text9')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="formOpen = false">{{ $t('map.auto.cancel') }}</el-button>
        <el-button type="primary" :loading="submitting" @click="submitForm">
          {{ formMode === 'create' ? $t('map.auto.text10') : $t('map.auto.save') }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 新 Key 一次性醒目展示（生成 / 重置） -->
    <el-dialog v-model="createdOpen" :title="$t('map.auto.key')" width="620px" append-to-body>
      <el-alert type="warning" show-icon :closable="false" class="mb-3" :title="$t('map.auto.keyHour')" />
      <el-form label-width="110px">
        <el-form-item :label="$t('map.auto.name')">{{ created?.keyName }}</el-form-item>
        <el-form-item :label="$t('map.auto.text')">{{ created?.purposeName }}</el-form-item>
        <el-form-item label="API Key">
          <div style="width: 100%">
            <el-input :model-value="created?.apiKey" readonly class="font-mono" />
            <el-button class="mt-2" type="primary" plain icon="DocumentCopy" @click="copy(created?.apiKey || '', t('map.auto.keyCopied'))">
              {{ $t('map.auto.copyKey') }}
            </el-button>
          </div>
        </el-form-item>
        <el-form-item :label="$t('map.auto.text11')">
          <div class="font-mono">{{ created?.usage?.sample }}</div>
          <el-button link type="primary" icon="DocumentCopy" @click="copy(created?.usage?.sample || '', t('map.auto.sampleCopied'))">{{ $t('map.auto.text12') }}</el-button>
        </el-form-item>
        <el-form-item v-if="created?.usage?.bearer" :label="$t('map.auto.bearer')">
          <div class="font-mono">{{ created.usage.bearer }}</div>
        </el-form-item>
        <el-form-item v-if="created?.usage?.query" :label="$t('map.auto.urlConfig')">
          <div class="font-mono">{{ created.usage.query }}</div>
        </el-form-item>
      </el-form>
      <div class="text-sm">{{ created?.usage?.note }}</div>
      <template #footer>
        <el-button type="primary" @click="createdOpen = false">{{ $t('map.auto.save2') }}</el-button>
      </template>
    </el-dialog>

    <!-- 用法 / 下发说明 -->
    <el-dialog v-model="usageOpen" :title="$t('map.auto.apiKey3')" width="620px" append-to-body>
      <el-form label-width="110px" v-if="usageTarget">
        <el-form-item :label="$t('map.auto.name')">{{ usageTarget.keyName }}</el-form-item>
        <el-form-item label="API Key">
          <div style="width: 100%">
            <el-input :model-value="usageTarget.apiKey" readonly class="font-mono" />
            <el-button class="mt-2" plain icon="DocumentCopy" @click="copy(usageTarget.apiKey, t('map.auto.keyCopied'))">{{ $t('map.auto.key2') }}</el-button>
          </div>
        </el-form-item>
        <el-form-item :label="$t('map.auto.text13')">
          <div class="font-mono">{{ usageTarget.usage.sample }}</div>
          <el-button link type="primary" icon="DocumentCopy" @click="copy(usageTarget.usage.sample, t('map.auto.sampleCopied'))">{{ $t('map.auto.text12') }}</el-button>
        </el-form-item>
        <el-form-item v-if="usageTarget.usage.bearer" label="Bearer">
          <div class="font-mono">{{ usageTarget.usage.bearer }}</div>
        </el-form-item>
        <el-form-item v-if="usageTarget.usage.query" :label="$t('map.auto.urlConfig2')">
          <div class="font-mono">{{ usageTarget.usage.query }}</div>
          <el-button link type="primary" icon="DocumentCopy" @click="copy(usageTarget.usage.query || '', t('map.auto.configCopied'))">{{ $t('map.auto.text12') }}</el-button>
        </el-form-item>
      </el-form>
      <el-alert v-if="applyResult" class="mb-3" :type="applyResult.applied ? 'success' : 'info'" show-icon :closable="false"
        :title="applyResult.applied
          ? `${t('map.auto.default')}${applyResult.engineId}${t('map.auto.managerServerKey')}`
          : $t('map.auto.clientKey')" />
      <div class="text-sm">{{ usageTarget?.usage?.note }}</div>
      <template #footer>
        <el-button type="primary" @click="usageOpen = false">{{ $t('map.auto.close') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts" name="MapApiKey">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
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
    ElMessage.warning(t('map.auto.browser'));
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
  keyName: [{ required: true, message: t('map.auto.pleaseFillInName'), trigger: 'blur' }],
  purpose: [{ required: true, message: t('map.auto.pleaseSelect'), trigger: 'change' }]
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
      ElMessage.success(t('map.auto.apiKey4'));
    } else {
      await updateApiKey(editingId.value, body);
      ElMessage.success(t('map.auto.savedSuccessfully'));
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
    if (res.applied) ElMessage.success(t('map.auto.default2'));
  } catch (e: any) {
    ElMessage.error(e?.msg || e?.message || t('map.auto.applyFailed'));
  }
}

// ---------- 重置 / 启停 / 删除 ----------
async function onRotate(row: ApiKeyVO) {
  await ElMessageBox.confirm(
    `${t('map.auto.reset')}${row.keyName}${t('map.auto.ofKeyClientofYesNo')}`,
    t('map.auto.resetKey'),
    { type: 'warning' }
  );
  const fresh = pick<ApiKeyVO>(await rotateApiKey(row.keyId));
  showCreated(fresh);
  await getList();
}

async function onToggle(row: ApiKeyVO) {
  const next = row.status === '0' ? '1' : '0';
  await updateApiKey(row.keyId, toBody(row, next));
  ElMessage.success(next === '0' ? t('map.auto.enable2') : t('map.auto.disable2'));
  await getList();
}

async function onDelete(row: ApiKeyVO) {
  await ElMessageBox.confirm(`${t('map.auto.okdelete')}${row.keyName}${t('map.auto.key3')}`, t('map.auto.deleteAPIKey'), {
    type: 'warning'
  });
  await deleteApiKey(row.keyId);
  ElMessage.success(t('map.auto.delete'));
  await getList();
}

onMounted(() => getList());
</script>
