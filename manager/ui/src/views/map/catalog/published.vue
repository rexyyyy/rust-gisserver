<template>
  <div class="p-2">
    <!-- 红线提示：账本与引擎配置同源，生命周期只动引擎登记簿 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        已发布服务是发布动作的<b>账本</b>：实体配置落在 rust-gisserver 的 XML 里，
        启停 / 暂停 / 删除只改引擎登记簿并热重载，不物理删除配置文件。
      </template>
    </el-alert>

    <el-card shadow="hover">
      <CatalogToolbar>
        <template #filters>
          <el-select v-model="wsId" placeholder="全部工作空间" clearable style="width: 210px" @change="load">
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
          <el-select v-model="pubDataType" style="width: 170px">
            <el-option label="全部数据类型" value="" />
            <el-option v-for="d in dataTypeOptions" :key="d" :label="d" :value="d" />
          </el-select>
          <el-checkbox v-model="includeDeleted" @change="load">显示已删除</el-checkbox>
        </template>
        <template #actions>
          <el-button icon="Refresh" @click="reloadAll">刷新</el-button>
          <router-link to="/data/resource" class="link-type">去发布数据图层</router-link>
        </template>
      </CatalogToolbar>

      <el-radio-group v-model="pubProto" size="small" class="mb-[10px]">
        <el-radio-button v-for="p in protoTabs" :key="p.value" :value="p.value">
          {{ p.label }}
          <span class="text-xs text-gray-400">{{ p.count }}</span>
        </el-radio-button>
      </el-radio-group>

      <el-table v-loading="loading" border :data="view" size="small" style="width: 100%">
        <el-table-column label="服务名" prop="layerName" min-width="190" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span>{{ row.layerName }}</span>
            <el-tooltip v-if="nativeGridInfo(row)" :content="nativeGridTip(row, nativeGridInfo(row)!)" placement="top" :show-after="200">
              <el-tag type="warning" effect="plain" size="small" class="ml-1">自定义坐标/网格</el-tag>
            </el-tooltip>
          </template>
        </el-table-column>
        <el-table-column label="标题" prop="title" min-width="130" :show-overflow-tooltip="true" />
        <el-table-column label="工作空间" align="center" width="100">
          <template #default="{ row }">
            <router-link to="/data/workspace" class="link-type">{{ row.workspaceName || wsName(row.workspaceId) }}</router-link>
          </template>
        </el-table-column>
        <el-table-column label="数据类型" align="center" width="120">
          <template #default="{ row }">
            <el-tag effect="plain" size="small">{{ dataTypeOf(row) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="来源" align="center" min-width="170" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <router-link v-if="storeOf(row)" :to="`/data/resource?storeId=${storeOf(row)}`" class="link-type text-xs">
              {{ [row.storeName, row.resourceName].filter(Boolean).join(' / ') || '—' }}
            </router-link>
            <span v-else class="text-xs">{{ [row.storeName, row.resourceName].filter(Boolean).join(' / ') || '—' }}</span>
          </template>
        </el-table-column>
        <el-table-column label="协议" align="center" min-width="170">
          <template #default="{ row }">
            <el-tag v-for="t in row.serviceTypes" :key="t" size="small" effect="plain" class="mr-1">
              {{ protoLabel(t) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="坐标系" align="center" prop="srcCrs" width="100" />
        <el-table-column label="四至" align="center" width="150">
          <template #default="{ row }">
            <span class="text-xs">{{ bboxText(row.bbox) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="状态" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="stateTag(row.state)" size="small" effect="dark">{{ stateLabel(row.state) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="操作" align="center" width="320" fixed="right">
          <template #default="{ row }">
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="primary"
              icon="Edit"
              :disabled="row.state === 'deleted'"
              @click="pubRef?.openEdit(row)"
            >
              修改
            </el-button>
            <el-button link type="success" icon="View" @click="pvRef?.open(row)">预览</el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="success"
              icon="VideoPlay"
              :disabled="row.state === 'active'"
              @click="handleState(row, 'active')"
            >
              {{ row.state === 'deleted' ? '恢复' : '启动' }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="primary"
              icon="Clock"
              :disabled="row.state !== 'active'"
              @click="handleState(row, 'paused')"
            >
              暂停
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="warning"
              icon="VideoPause"
              :disabled="row.state === 'stopped' || row.state === 'deleted'"
              @click="handleState(row, 'stopped')"
            >
              停止
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="danger"
              icon="Delete"
              :disabled="row.state === 'deleted'"
              @click="handleState(row, 'deleted')"
            >
              删除
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        当前分组 {{ view.length }} 个服务（共 {{ total }} 个）。「停止」与「暂停」都不再对外提供（引擎登记簿
        state≠active 即摘除），只是管理态标签不同；「删除」进垃圾箱但保留配置文件，勾「显示已删除」后可在此恢复。
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

    <PublishDialog ref="pubRef" :caps="caps" @saved="reloadAll" />
    <PreviewDialog ref="pvRef" />
  </div>
</template>

<script setup name="MapCatalogPublished" lang="ts">
import { listPublished, listStores, listWorkspaces, setPublishedState } from '@/api/map/catalog';
import { PublishedVO, StoreVO, WorkspaceVO } from '@/api/map/catalog/types';
import CatalogToolbar from './components/CatalogToolbar.vue';
import PublishDialog from './components/PublishDialog.vue';
import PreviewDialog from './components/PreviewDialog.vue';
import { bboxText, nativeGridInfo, nativeGridTip, protoLabel, stateLabel, stateTag, useCaps } from './composables/useCatalog';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const route = useRoute();
const { caps, loadCaps } = useCaps();

const loading = ref(false);
const wsId = ref<number | undefined>(undefined);
const includeDeleted = ref(false);
// 分组视图：服务类型（''=全部）与数据类型（''=全部）
const pubProto = ref('');
const pubDataType = ref('');
const workspaces = ref<WorkspaceVO[]>([]);
const stores = ref<StoreVO[]>([]);
const rows = ref<PublishedVO[]>([]);
const total = ref(0);

const page = ref(1);
const pageSize = ref(10);
// 后端分页：page/pageSize 变化即重新拉取对应页（服务端按 workspace/includeDeleted 过滤）
watch([page, pageSize], () => {
  load();
});
watch([wsId, pubProto, pubDataType, includeDeleted], () => {
  page.value = 1;
  load();
});

const pubRef = ref<InstanceType<typeof PublishDialog>>();
const pvRef = ref<InstanceType<typeof PreviewDialog>>();

const wsName = (id: number) => workspaces.value.find((w) => w.id === id)?.name || `#${id}`;
/** 来源列的深链：资源属于某个数据源，点了直接带着 storeId 跳到「数据图层」页。
 *  账本里只存 store 名，故先按同工作空间匹配，再退回全局同名（跳不过去就当普通文本） */
const storeOf = (row: PublishedVO) =>
  stores.value.find((s) => s.name === row.storeName && s.workspaceId === row.workspaceId)?.id ||
  stores.value.find((s) => s.name === row.storeName)?.id ||
  0;

const dataTypeOf = (p: PublishedVO) => p.dataType || p.mapserverKind;

/** 已删服务默认不混在清单里，勾选后才显示（在此直接恢复，不再单设垃圾箱页） */
const liveRows = computed(() => (includeDeleted.value ? rows.value : rows.value.filter((p) => p.state !== 'deleted')));

/** 协议顺序跟着引擎能力矩阵，出现过的未知协议追加在后 */
const protoOrder = computed<string[]>(() => {
  const known = caps.value?.service_types || [];
  const seen = new Set(known);
  const extra = [...new Set(rows.value.flatMap((p) => p.serviceTypes || []))].filter((t) => !seen.has(t));
  return [...known, ...extra];
});

const protoTabs = computed(() => {
  const list = liveRows.value;
  const tabs: { value: string; label: string; count: number }[] = [{ value: '', label: '全部', count: list.length }];
  for (const t of protoOrder.value) {
    const count = list.filter((p) => (p.serviceTypes || []).includes(t)).length;
    if (count) tabs.push({ value: t, label: protoLabel(t), count });
  }
  return tabs;
});

const dataTypeOptions = computed(() => [...new Set(liveRows.value.map(dataTypeOf))]);

const view = computed(() =>
  liveRows.value.filter((p) => {
    if (pubProto.value && !(p.serviceTypes || []).includes(pubProto.value)) return false;
    if (pubDataType.value && dataTypeOf(p) !== pubDataType.value) return false;
    return true;
  })
);

const load = async () => {
  loading.value = true;
  try {
    const params: { workspaceId?: number; includeDeleted?: number; page?: number; pageSize?: number } = {};
    if (wsId.value) params.workspaceId = wsId.value;
    if (includeDeleted.value) params.includeDeleted = 1;
    params.page = page.value;
    params.pageSize = pageSize.value;
    const res = await listPublished(params);
    rows.value = res.data.published || [];
    total.value = res.data.total || 0;
  } finally {
    loading.value = false;
  }
};

const reloadAll = async () => {
  await Promise.all([load(), loadStores()]);
};

const loadStores = async () => {
  const res = await listStores({ includeDeleted: 1 });
  stores.value = res.data.stores || [];
};

const handleState = async (row: PublishedVO, state: string) => {
  const tip: Record<string, string> = {
    active: `确认启动/恢复服务「${row.layerName}」？引擎将重新纳入该服务并对外提供。`,
    stopped: `确认停止服务「${row.layerName}」？配置文件保留，仅停止对外提供。`,
    paused: `确认暂停服务「${row.layerName}」？与停止同样不再对外提供，只是管理态标记不同。`,
    deleted: `确认删除服务「${row.layerName}」？引擎停止提供并进垃圾箱（配置不物理删除）。`
  };
  await proxy?.$modal.confirm(tip[state] || `确认将「${row.layerName}」置为 ${state}？`);
  await setPublishedState(row.id, state);
  proxy?.$modal.msgSuccess('已更新');
  await reloadAll();
};

onMounted(async () => {
  const q = Number(route.query.workspaceId);
  if (q > 0) wsId.value = q;
  await Promise.all([loadCaps(), listWorkspaces().then((r) => (workspaces.value = r.data.workspaces || [])), loadStores(), load()]);
});
</script>

<style scoped>
.link-type {
  color: var(--el-color-primary);
}
</style>

