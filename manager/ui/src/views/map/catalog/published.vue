<template>
  <div class="p-2">
    <!-- 红线提示：账本与引擎配置同源，生命周期只动引擎登记簿 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        {{ $t('map.auto.publishedLedgerHint') }}
        {{ $t('map.auto.text181')}} / {{ $t('map.auto.text43') }} / {{ $t('map.auto.deleteDelete') }}
      </template>
    </el-alert>

    <el-card shadow="hover">
      <CatalogToolbar>
        <template #filters>
          <el-select v-model="wsId" :placeholder="$t('map.auto.all')" clearable style="width: 210px" @change="load">
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
          <el-select v-model="pubDataType" style="width: 170px">
            <el-option :label="$t('map.auto.alldataType')" value="" />
            <el-option v-for="d in dataTypeOptions" :key="d" :label="d" :value="d" />
          </el-select>
          <el-checkbox v-model="includeDeleted" @change="load">{{ $t('map.auto.showdelete') }}</el-checkbox>
        </template>
        <template #actions>
          <el-button icon="Refresh" @click="reloadAll">{{ $t('map.auto.refresh') }}</el-button>
          <router-link to="/data/resource" class="link-type">{{ $t('map.auto.dataLayer2') }}</router-link>
        </template>
      </CatalogToolbar>

      <el-radio-group v-model="pubProto" size="small" class="mb-[10px]">
        <el-radio-button v-for="p in protoTabs" :key="p.value" :value="p.value">
          {{ p.label }}
          <span class="text-xs text-gray-400">{{ p.count }}</span>
        </el-radio-button>
      </el-radio-group>

      <el-table v-loading="loading" border :data="view" size="small" style="width: 100%">
        <el-table-column :label="$t('map.auto.serviceName')" prop="layerName" min-width="190" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span>{{ row.layerName }}</span>
            <el-tooltip v-if="nativeGridInfo(row)" :content="nativeGridTip(row, nativeGridInfo(row)!)" placement="top" :show-after="200">
              <el-tag type="warning" effect="plain" size="small" class="ml-1">{{ $t('map.auto.customCRSGrid') }}</el-tag>
            </el-tooltip>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.title')" prop="title" min-width="130" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.text51')" align="center" width="100">
          <template #default="{ row }">
            <router-link to="/data/workspace" class="link-type">{{ row.workspaceName || wsName(row.workspaceId) }}</router-link>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.dataType')" align="center" width="120">
          <template #default="{ row }">
            <el-tag effect="plain" size="small">{{ dataTypeOf(row) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text54')" align="center" min-width="170" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <router-link v-if="storeOf(row)" :to="`/data/resource?storeId=${storeOf(row)}`" class="link-type text-xs">
              {{ [row.storeName, row.resourceName].filter(Boolean).join(' / ') || '—' }}
            </router-link>
            <span v-else class="text-xs">{{ [row.storeName, row.resourceName].filter(Boolean).join(' / ') || '—' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.protocol5')" align="center" min-width="170">
          <template #default="{ row }">
            <el-tag v-for="t in row.serviceTypes" :key="t" size="small" effect="plain" class="mr-1">
              {{ protoLabel(t) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.crs8')" align="center" prop="srcCrs" width="100" />
        <el-table-column :label="$t('map.auto.four')" align="center" width="150">
          <template #default="{ row }">
            <span class="text-xs">{{ bboxText(row.bbox) }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.status')" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="stateTag(row.state)" size="small" effect="dark">{{ stateLabel(row.state) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.actions')" align="center" width="320" fixed="right">
          <template #default="{ row }">
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="primary"
              icon="Edit"
              :disabled="row.state === 'deleted'"
              @click="pubRef?.openEdit(row)"
            >
              {{ $t('map.auto.edit') }}
            </el-button>
            <el-button link type="success" icon="View" @click="pvRef?.open(row)">{{ $t('map.auto.preview3') }}</el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="success"
              icon="VideoPlay"
              :disabled="row.state === 'active'"
              @click="handleState(row, 'active')"
            >
              {{ row.state === 'deleted' ? $t('map.auto.text55') : $t('map.auto.text56') }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="primary"
              icon="Clock"
              :disabled="row.state !== 'active'"
              @click="handleState(row, 'paused')"
            >
              {{ $t('map.auto.text43') }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="warning"
              icon="VideoPause"
              :disabled="row.state === 'stopped' || row.state === 'deleted'"
              @click="handleState(row, 'stopped')"
            >
              {{ $t('map.auto.text42') }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="danger"
              icon="Delete"
              :disabled="row.state === 'deleted'"
              @click="handleState(row, 'deleted')"
            >
              {{ $t('map.auto.delete6') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
      <div class="mt-2 text-xs text-gray-400">
        {{ $t('map.auto.minute2') }}{{ view.length }} {{ $t('map.auto.text182') }}{{ total }} {{ $t('map.auto.text183') }}
        state≠active {{ $t('map.auto.yesDeleteShowDelete') }}
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
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
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
  const tabs: { value: string; label: string; count: number }[] = [{ value: '', label: t('map.auto.all2'), count: list.length }];
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
    active: `${t('map.auto.confirm')}${row.layerName}${t('map.auto.text57')}`,
    stopped: `${t('map.auto.confirm2')}${row.layerName}${t('map.auto.text58')}`,
    paused: `${t('map.auto.confirm3')}${row.layerName}${t('map.auto.yes')}`,
    deleted: `${t('map.auto.confirmDeleteService')}${row.layerName}${t('map.auto.delete2')}`
  };
  await proxy?.$modal.confirm(tip[state] || `${t('map.auto.confirm4')}${row.layerName}${t('map.auto.text59')}${state}?`);
  await setPublishedState(row.id, state);
  proxy?.$modal.msgSuccess(t('map.auto.updated2'));
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

