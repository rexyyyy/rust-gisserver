<template>
  <div class="p-2">
    <!-- 红线提示：这里只登记命名空间容器，不含任何数据本体 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        工作空间是服务与数据源的<b>命名空间容器</b>。删除只作用于容器：其下仍有数据源或在服服务时会被服务端拒绝。
      </template>
    </el-alert>

    <el-card shadow="hover">
      <CatalogToolbar>
        <template #filters>
          <el-input v-model="keyword" placeholder="按名称/显示名过滤" clearable style="width: 220px" prefix-icon="Search" />
          <el-checkbox v-model="includeDeleted" @change="load">显示已删除</el-checkbox>
        </template>
        <template #actions>
          <el-button icon="Refresh" @click="load">刷新</el-button>
          <el-button v-hasPermi="['map:catalog:edit']" type="primary" plain icon="Plus" @click="openAdd">新建工作空间</el-button>
        </template>
      </CatalogToolbar>

      <el-table v-loading="loading" border :data="pagedView" size="small" style="width: 100%">
        <el-table-column label="名称" prop="name" min-width="120" :show-overflow-tooltip="true" />
        <el-table-column label="显示名" prop="title" min-width="120" :show-overflow-tooltip="true" />
        <el-table-column label="绑定引擎" min-width="140" :show-overflow-tooltip="true">
          <template #default="{ row }">
            {{ engineLabel(row.engineId) }}
          </template>
        </el-table-column>
        <el-table-column label="数据根路径" prop="dataRoot" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column label="数据源" align="center" width="90">
          <template #default="{ row }">
            <router-link :to="`/data/store?workspaceId=${row.id}`" class="link-type">{{ row.storeCount }}</router-link>
          </template>
        </el-table-column>
        <el-table-column label="在服服务" align="center" width="90">
          <template #default="{ row }">
            <router-link :to="`/data/published?workspaceId=${row.id}`" class="link-type">{{ row.publishedCount }}</router-link>
          </template>
        </el-table-column>
        <el-table-column label="状态" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="row.delFlag === 1 ? 'danger' : 'success'" size="small" effect="dark">
              {{ row.delFlag === 1 ? '已删' : '正常' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="备注" prop="remark" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column label="操作" align="center" width="220" fixed="right">
          <template #default="{ row }">
            <el-button v-hasPermi="['map:catalog:edit']" link type="primary" icon="Edit" @click="openEdit(row)">编辑</el-button>
            <el-button link type="success" icon="Coin" @click="goStore(row)">数据源</el-button>
            <el-button
              v-hasPermi="['map:catalog:edit']"
              link
              type="danger"
              icon="Delete"
              :disabled="row.delFlag === 1"
              @click="handleRemove(row)"
            >
              删除
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        共 {{ view.length }} 个工作空间。默认工作区 ws1 由数据库初始化脚本（08-catalog.sql）写入；v1 服务名为扁平唯一，
        工作空间用于管理侧分组，暂不进入引擎的 URL 命名空间。
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
    <el-dialog v-model="dlg.visible" :title="dlg.isEdit ? '编辑工作空间' : '新建工作空间'" width="480px" append-to-body>
      <el-form ref="formRef" :model="form" :rules="rules" label-width="90px">
        <el-form-item label="名称" prop="name">
          <el-input v-model="form.name" placeholder="命名空间标识，字母/数字/-/_（如 ws1）" :disabled="dlg.isEdit && form.name === 'ws1'" />
        </el-form-item>
        <el-form-item label="显示名" prop="title">
          <el-input v-model="form.title" placeholder="如 默认工作空间" />
        </el-form-item>
        <el-form-item label="绑定引擎" prop="engineId">
          <el-select v-model="form.engineId" placeholder="留空则使用默认引擎" clearable style="width: 100%">
            <el-option
              v-for="eng in engines"
              :key="eng.engineId"
              :label="eng.baseUrl"
              :value="eng.engineId"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="数据根路径" prop="dataRoot">
          <el-input v-model="form.dataRoot" placeholder="如 E:\data\1（自动规范化为 E:/data/1/）" @blur="normalizePath" />
        </el-form-item>
        <el-form-item label="备注" prop="remark">
          <el-input v-model="form.remark" type="textarea" :rows="2" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dlg.visible = false">取消</el-button>
        <el-button type="primary" :loading="dlg.saving" @click="save">确定</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup name="MapCatalogWorkspace" lang="ts">
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
    { required: true, message: '名称不能为空', trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,64}$/, message: '仅字母、数字、下划线与短横线', trigger: 'blur' }
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
  if (!engineId) return '默认引擎';
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
    proxy?.$modal.msgSuccess('已保存');
    dlg.visible = false;
    await load();
  } finally {
    dlg.saving = false;
  }
};

const handleRemove = async (w: WorkspaceVO) => {
  await proxy?.$modal.confirm(
    `确认删除工作空间「${w.title || w.name}」？其下须已无数据存储与在服服务（数据本体与引擎配置都不受本操作影响）。`
  );
  await deleteWorkspace(w.id);
  proxy?.$modal.msgSuccess('已删除');
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
