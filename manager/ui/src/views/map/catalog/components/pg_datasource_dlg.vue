<template>
  <el-dialog
    v-model="visible"
    :title="$t('map.auto.newPostgisDatasource')"
    width="820px"
    append-to-body
    @open="resetForm"
  >
    <el-form ref="formRef" :model="form" :rules="rules" label-width="130px">
      <el-row :gutter="12">
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.text77')" prop="workspaceId">
            <el-select v-model="form.workspaceId" style="width: 100%">
              <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
            </el-select>
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.name')" prop="name">
            <el-input v-model="form.name" :placeholder="$t('map.auto.wuhanCog')" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.host')" prop="host">
            <el-input v-model="form.host" :placeholder="$t('map.auto.n127001')" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.text78')" prop="port">
            <el-input-number v-model="form.port" :min="1" :max="65535" controls-position="right" style="width: 100%" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.user')" prop="username">
            <el-input v-model="form.username" placeholder="postgres" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.text79')" prop="password">
            <el-input v-model="form.password" type="password" show-password :placeholder="$t('map.auto.text80')" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.text81')" prop="database">
            <el-input v-model="form.database" :placeholder="$t('map.auto.gis')" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item label="Schema">
            <el-input v-model="form.schema" placeholder="public" />
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.text82')">
            <el-button type="primary" plain icon="Connection" :loading="testing" :disabled="!canTest" @click="testConnection">
              {{ $t('map.auto.text188') }}
            </el-button>
          </el-form-item>
        </el-col>
        <el-col :span="12">
          <el-form-item :label="$t('map.auto.remark')">
            <el-input v-model="form.remark" type="textarea" :rows="2" />
          </el-form-item>
        </el-col>
      </el-row>
      <div v-if="testResult" class="text-xs -mt-2 mb-2" :class="testResult.ok ? 'text-green-600' : 'text-red-500'">
        {{ testResult.detail }}
      </div>
    </el-form>

    <template #footer>
      <el-button :disabled="saving" @click="visible = false">{{ $t('map.auto.cancel') }}</el-button>
      <el-button type="primary" :loading="saving" @click="save">{{ $t('map.auto.ok') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { computed, getCurrentInstance, reactive, ref } from 'vue';
import { addStore, testStoreConnectionPreflight } from '@/api/map/catalog';
import type { StoreVO, WorkspaceVO } from '@/api/map/catalog/types';

const props = defineProps<{
  modelValue: boolean;
  workspaces: WorkspaceVO[];
  defaultWorkspaceId?: number;
}>();
const emit = defineEmits<{
  (e: 'update:modelValue', value: boolean): void;
  (e: 'created', store: StoreVO): void;
}>();

const proxy = getCurrentInstance()?.proxy as any;
const visible = computed({
  get: () => props.modelValue,
  set: (v: boolean) => emit('update:modelValue', v)
});

const formRef = ref<ElFormInstance>();
const testing = ref(false);
const saving = ref(false);
const testResult = ref<{ ok: boolean; detail: string } | null>(null);

const form = reactive({
  workspaceId: 0,
  name: '',
  host: '',
  port: 5432,
  username: '',
  password: '',
  database: '',
  schema: 'public',
  remark: ''
});

const canTest = computed(
  () => !!form.host.trim() && !!form.username.trim() && !!form.password && !!form.database.trim()
);

const rules = {
  workspaceId: [{ required: true, message: t('map.auto.pleaseSelect2'), trigger: 'change' }],
  name: [
    { required: true, message: t('map.auto.nameisRequired'), trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,128}$/, message: t('map.auto.text88'), trigger: 'blur' }
  ],
  host: [{ required: true, message: t('map.auto.hostisRequired'), trigger: 'blur' }],
  username: [{ required: true, message: t('map.auto.userisRequired'), trigger: 'blur' }],
  password: [{ required: true, message: t('map.auto.isRequired'), trigger: 'blur' }],
  database: [{ required: true, message: t('map.auto.isRequired2'), trigger: 'blur' }]
};

const resetForm = () => {
  Object.assign(form, {
    workspaceId: props.defaultWorkspaceId || props.workspaces[0]?.id || 0,
    name: '',
    host: '',
    port: 5432,
    username: '',
    password: '',
    database: '',
    schema: 'public',
    remark: ''
  });
  testResult.value = null;
  nextTick(() => formRef.value?.clearValidate());
};

const testConnection = async () => {
  testing.value = true;
  testResult.value = null;
  try {
    const res = await testStoreConnectionPreflight({
      host: form.host.trim(),
      port: form.port,
      username: form.username.trim(),
      password: form.password,
      database: form.database.trim()
    });
    testResult.value = { ok: !!res.data.ok, detail: res.data.detail || t('map.auto.success') };
  } catch (e: any) {
    testResult.value = { ok: false, detail: e?.message || t('map.auto.failed') };
  } finally {
    testing.value = false;
  }
};

const save = async () => {
  const valid = await formRef.value?.validate().catch(() => false);
  if (!valid) return;
  saving.value = true;
  try {
    // 用户名/口令按 RFC 3986 编码，避免特殊字符破坏 DSN。
    const dsn = `postgres://${encodeURIComponent(form.username.trim())}:${encodeURIComponent(form.password)}@${form.host.trim()}:${form.port}/${encodeURIComponent(form.database.trim())}`;
    const res = await addStore({
      workspaceId: form.workspaceId,
      name: form.name.trim(),
      storeType: 'postgis',
      basePath: '',
      pgDsn: dsn,
      pgSchema: form.schema.trim() || 'public',
      remark: form.remark
    });
    proxy?.$modal.msgSuccess(t('map.auto.text89'));
    visible.value = false;
    emit('created', res.data);
  } finally {
    saving.value = false;
  }
};
</script>
