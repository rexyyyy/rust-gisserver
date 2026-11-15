<template>
  <div class="p-2">
    <!-- 红线提示：这里只登记命名空间容器，不含任何数据本体 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        {{ $t('map.auto.yesof') }}<b>{{ $t('map.auto.text95') }}</b>。{{ $t('map.auto.deleteInServiceHour') }}
      </template>
    </el-alert>

    <el-card shadow="hover">
      <CatalogToolbar>
        <template #filters>
          <el-input v-model="keyword" :placeholder="$t('map.auto.bynameShow')" clearable style="width: 220px" prefix-icon="Search" />
          <el-checkbox v-model="includeDeleted" @change="load">{{ $t('map.auto.showdelete') }}</el-checkbox>
        </template>
        <template #actions>
          <el-button icon="Refresh" @click="load">{{ $t('map.auto.refresh') }}</el-button>
          <el-button v-hasPermi="['map:catalog:edit']" type="primary" plain icon="Plus" @click="openAdd">{{ $t('map.auto.text96') }}</el-button>
        </template>
      </CatalogToolbar>

      <el-table v-loading="loading" border :data="pagedView" size="small" style="width: 100%">
        <el-table-column :label="$t('map.auto.name')" prop="name" min-width="120" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.show')" prop="title" min-width="120" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.text97')" min-width="140" :show-overflow-tooltip="true">
          <template #default="{ row }">
            {{ engineLabel(row.engineId) }}
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text98')" prop="dataRoot" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.text52')" align="center" width="90">
          <template #default="{ row }">
            <router-link :to="`/data/store?workspaceId=${row.id}`" class="link-type">{{ row.storeCount }}</router-link>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.inService3')" align="center" width="90">
          <template #default="{ row }">
            <router-link :to="`/data/published?workspaceId=${row.id}`" class="link-type">{{ row.publishedCount }}</router-link>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.status')" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="row.delFlag === 1 ? 'danger' : 'success'" size="small" effect="dark">
              {{ row.delFlag === 1 ? $t('map.auto.text44') : $t('map.auto.normal') }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.remark')" prop="remark" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.actions')" align="center" width="220" fixed="right">
          <template #default="{ row }">
            <el-button v-hasPermi="['map:catalog:edit']" link type="primary" icon="Edit" @click="openEdit(row)">{{ $t('map.auto.text99') }}</el-button>
            <el-button link type="success" icon="Coin" @click="goStore(row)">{{ $t('map.auto.text52') }}</el-button>
            <el-button
              v-hasPermi="['map:catalog:edit']"
              link
              type="danger"
              icon="Delete"
              :disabled="row.delFlag === 1"
              @click="handleRemove(row)"
            >
              {{ $t('map.auto.delete6') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        {{ $t('map.auto.workspaceHint', { n: view.length }) }}
      </div>
      <div class="mt-3 flex justify-end">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :page-sizes="[10, 20, 50, 100]"
          :total="view.length"
          layout="total, sizes, prev, pager, next, jumper"
          background
        />
      </div>
    </el-card>

    <!-- 新增 / 编辑 -->
    <el-dialog v-model="dlg.visible" :title="dlg.isEdit ? $t('map.auto.text100') : $t('map.auto.text96')" width="480px" append-to-body>
      <el-form ref="formRef" :model="form" :rules="rules" label-width="90px">
        <el-form-item :label="$t('map.auto.name')" prop="name">
          <el-input v-model="form.name" :placeholder="$t('map.auto.ws1')" :disabled="dlg.isEdit && form.name === 'ws1'" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.show')" prop="title">
          <el-input v-model="form.title" :placeholder="$t('map.auto.default3')" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.text97')" prop="engineId">
          <el-select v-model="form.engineId" :placeholder="$t('map.auto.default4')" clearable style="width: 100%">
            <el-option
              v-for="eng in engines"
              :key="eng.engineId"
              :label="eng.baseUrl"
              :value="eng.engineId"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('map.auto.text98')" prop="dataRoot">
          <el-input v-model="form.dataRoot" :placeholder="$t('map.auto.eData1EData1')" @blur="normalizePath" />
        </el-form-item>
        <el-form-item :label="$t('map.auto.remark')" prop="remark">
          <el-input v-model="form.remark" type="textarea" :rows="2" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dlg.visible = false">{{ $t('map.auto.cancel') }}</el-button>
        <el-button type="primary" :loading="dlg.saving" @click="save">{{ $t('map.auto.ok') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup name="MapCatalogWorkspace" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { addWorkspace, deleteWorkspace, listWorkspaces, updateWorkspace } from '@/api/map/catalog';
import { WorkspaceBody, WorkspaceVO } from '@/api/map/catalog/types';
import { listEngines } from '@/api/map/engine';
import { EngineVO } from '@/api/map/engine/types';
import CatalogToolbar from './components/CatalogToolbar.vue';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const router = useRouter();

const loading = ref(false);
const includeDeleted = ref(false);
const keyword = ref('');
const rows = ref<WorkspaceVO[]>([]);
const engines = ref<EngineVO[]>([]);

const view = computed(() => {
  const k = keyword.value.trim().toLowerCase();
  if (!k) return rows.value;
  return rows.value.filter((w) => w.name.toLowerCase().includes(k) || (w.title || '').toLowerCase().includes(k));
});

const page = ref(1);
const pageSize = ref(10);
const pagedView = computed(() => {
  const start = (page.value - 1) * pageSize.value;
  return view.value.slice(start, start + pageSize.value);
});
watch([keyword, includeDeleted], () => {
  page.value = 1;
});

const load = async () => {
  loading.value = true;
  try {
    const res = await listWorkspaces(includeDeleted.value ? { includeDeleted: 1 } : undefined);
    rows.value = res.data.workspaces || [];
  } finally {
    loading.value = false;
  }
};

const dlg = reactive({ visible: false, isEdit: false, saving: false });
const formRef = ref<ElFormInstance>();
const form = reactive<WorkspaceBody & { id: number }>({ id: 0, name: '', title: '', engineId: '', dataRoot: '', remark: '' });
const rules = {
  name: [
    { required: true, message: t('map.auto.nameisRequired'), trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,64}$/, message: t('map.auto.text88'), trigger: 'blur' }
  ]
};

const openAdd = () => {
  dlg.isEdit = false;
  Object.assign(form, { id: 0, name: '', title: '', engineId: '', dataRoot: '', remark: '' });
  dlg.visible = true;
  nextTick(() => formRef.value?.clearValidate());
};

const openEdit = (w: WorkspaceVO) => {
  dlg.isEdit = true;
  Object.assign(form, { id: w.id, name: w.name, title: w.title, engineId: w.engineId || '', dataRoot: w.dataRoot || '', remark: w.remark });
  dlg.visible = true;
  nextTick(() => formRef.value?.clearValidate());
};

const engineLabel = (engineId: string) => {
  if (!engineId) return t('map.auto.default5');
  const eng = engines.value.find((e) => e.engineId === engineId);
  return eng ? eng.baseUrl : engineId;
};

const loadEngines = async () => {
  try {
    const res = await listEngines();
    engines.value = (res.data.engines || []).filter((e: EngineVO) => e.status === '0');
  } catch {
    engines.value = [];
  }
};

const normalizePath = () => {
  const s = form.dataRoot.trim().replace(/\\/g, '/');
  form.dataRoot = s && !s.endsWith('/') ? s + '/' : s;
};

const save = async () => {
  const valid = await formRef.value?.validate().catch(() => false);
  if (!valid) return;
  dlg.saving = true;
  try {
    const { id, ...body } = form;
    if (dlg.isEdit) await updateWorkspace(id, body);
    else await addWorkspace(body);
    proxy?.$modal.msgSuccess(t('map.auto.save5'));
    dlg.visible = false;
    await load();
  } finally {
    dlg.saving = false;
  }
};

const handleRemove = async (w: WorkspaceVO) => {
  await proxy?.$modal.confirm(
    `${t('map.auto.confirmdelete2')}${w.title || w.name}${t('map.auto.inServiceActions')}`
  );
  await deleteWorkspace(w.id);
  proxy?.$modal.msgSuccess(t('map.auto.delete'));
  await load();
};

const goStore = (w: WorkspaceVO) => router.push(`/data/store?workspaceId=${w.id}`);

onMounted(() => {
  load();
  loadEngines();
});
</script>

<style scoped>
.link-type {
  color: var(--el-color-primary);
}
</style>
