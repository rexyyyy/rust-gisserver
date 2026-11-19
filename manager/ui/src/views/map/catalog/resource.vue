<template>
  <div class="p-2">
    <!-- 红线提示：这里是「发现」出来的元数据清单，不是数据本体 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]" :title="$t('map.auto.catalogResourceHint')" />

    <el-card shadow="hover">
      <div class="catalog-split">
        <!-- 左侧边栏：工作空间下拉 + 该工作空间下的数据源 -->
        <aside class="catalog-side" :style="{ width: sideWidth + 'px' }">
          <el-select
            v-model="wsId"
            :placeholder="$t('map.auto.all3')"
            clearable
            style="width: 100% !important"
            :title="wsId ? wsLabel(wsId) : $t('map.auto.all')"
            @change="onWsChange"
          >
            <el-option v-for="w in workspaces" :key="w.id" :label="`${w.name}${w.title ? ' — ' + w.title : ''}`" :value="w.id" />
          </el-select>
          <div class="catalog-side__list">
            <template v-for="grp in sideGroups" :key="grp.key">
              <div v-if="grp.wsName" class="catalog-side__group">{{ grp.wsName }}</div>
              <div
                v-for="s in grp.stores"
                :key="s.id"
                class="catalog-side__item"
                :class="{ 'is-active': s.id === storeId, 'is-del': s.delFlag === 1 }"
                :title="`${s.name} (${s.storeType})${s.delFlag === 1 ? $t('map.auto.delete3') : ''}`"
                @click="selectStore(s)"
              >
                <span class="name">{{ s.name }}</span>
                <span class="meta">{{ s.resourceCount }}</span>
              </div>
            </template>
            <el-empty v-if="!sideGroups.length" :image-size="48" :description="$t('map.auto.text60')" />
          </div>
          <div class="catalog-side__resize" @mousedown="startResize" :title="$t('map.auto.text61')" />
        </aside>

        <!-- 右侧：数据图层列表 -->
        <section class="catalog-main">
          <CatalogToolbar>
            <template #filters>
              <el-select
                v-if="currentStore && !currentStore.isPostgis"
                v-model="discoverDepth"
                style="width: 130px"
                :title="$t('map.auto.catalogOf2')"
              >
                <el-option v-for="d in [1, 2, 3, 4]" :key="d" :label="`${t('map.auto.text62')}${d}`" :value="d" />
              </el-select>
              <el-button
                v-hasPermi="['map:catalog:publish']"
                type="primary"
                plain
                icon="Search"
                :loading="discovering"
                :disabled="!currentStore || currentStore.delFlag === 1"
                @click="handleDiscover"
              >
                {{ $t('map.auto.text185') }}
              </el-button>
              <el-checkbox v-model="includeDeleted" @change="onFilterChange">{{ $t('map.auto.showdelete') }}</el-checkbox>
            </template>
            <template #actions>
              <el-button icon="Refresh" @click="reloadAll">{{ $t('map.auto.refresh') }}</el-button>
              <router-link to="/data/store" class="link-type">{{ $t('map.auto.text63') }}</router-link>
            </template>
          </CatalogToolbar>

          <template v-if="currentStore">
            <div class="mb-[10px] flex flex-wrap items-center gap-2">
              <el-tag type="primary" effect="dark">{{ wsName(currentStore.workspaceId) }} / {{ currentStore.name }}</el-tag>
              <el-tag type="info" effect="plain">{{ currentStore.storeType }}</el-tag>
              <span class="text-xs text-gray-500">{{ currentStore.isPostgis ? currentStore.pgDsnMasked || $t('map.auto.text64') : currentStore.basePath }}</span>
            </div>
            <el-table
              v-loading="loading"
              border
              :data="resources"
              :empty-text="emptyText"
              size="small"
              style="width: 100%"
            >
              <el-table-column :label="$t('map.auto.text65')" min-width="180">
                <template #default="{ row }">
                  <el-tooltip placement="top">
                    <template #content>
                      <div>{{ $t('map.auto.dataType2') }}{{ row.dataType || '—' }}</div>
                      <div>
                        {{ $t('map.auto.crs12') }}{{ row.srcCrs || $t('map.auto.text67') }}<span v-if="row.crsSuspicious && !is3dOrTerrain(row)" class="crs-warn">{{ $t('map.auto.text66') }}</span>
                      </div>
                      <div>{{ $t('map.auto.four2') }}{{ row.bbox ? bboxText(row.bbox) : $t('map.auto.text68') }}</div>
                    </template>
                    <span class="res-name">
                      <span class="name-text">{{ row.name }}</span>
                      <el-tag
                        v-if="row.crsSuspicious && !is3dOrTerrain(row)"
                        type="warning"
                        size="small"
                        effect="plain"
                        class="ml-1"
                      >{{ $t('map.auto.text69') }}</el-tag>
                      <el-tag
                        v-if="needsNativeGrid(row)"
                        type="warning"
                        size="small"
                        effect="light"
                        class="ml-1"
                        :title="nativeGridTip(row)"
                      >{{ $t('map.auto.customCRSGrid') }}</el-tag>
                    </span>
                  </el-tooltip>
                </template>
              </el-table-column>
              <el-table-column :label="$t('map.auto.text70')" prop="source" min-width="190" :show-overflow-tooltip="true" />
              <el-table-column :label="$t('map.auto.text71')" align="center" min-width="110">
                <template #default="{ row }">
                  <template v-if="publishedOf(row).length">
                    <div class="flex flex-wrap items-center justify-center gap-1">
                      <el-tag
                        v-for="p in publishedOf(row)"
                        :key="p.id"
                        :type="stateTag(p.state)"
                        size="small"
                        effect="dark"
                      >{{ stateLabel(p.state) }}</el-tag>
                    </div>
                  </template>
                  <el-tag v-else type="info" size="small" effect="plain">{{ $t('map.auto.text72') }}</el-tag>
                </template>
              </el-table-column>
              <el-table-column :label="$t('map.auto.actions')" align="center" width="300" fixed="right">
                <template #default="{ row }">
                  <!-- PostGIS 普通表：提供数据查询服务（不发布图层） -->
                  <el-button
                    v-if="row.dataType === 'pg_table'"
                    v-hasPermi="['map:catalog:list']"
                    link
                    type="primary"
                    icon="Search"
                    :disabled="row.delFlag === 1"
                    @click="openQuery(row)"
                  >
                    {{ $t('map.auto.query4') }}
                  </el-button>
                  <!-- 空间图层：发布为图层服务 -->
                  <el-button
                    v-else
                    v-hasPermi="['map:catalog:publish']"
                    link
                    type="primary"
                    icon="Upload"
                    :disabled="!row.publishable || publishedOf(row).length > 0 || row.delFlag === 1"
                    @click="pubRef?.open(row, currentStore?.workspaceId || 0)"
                  >
                    {{ $t('map.auto.text37') }}
                  </el-button>
                  <template v-if="solePublished(row)">
                    <el-button link type="success" icon="View" @click="pvRef?.open(solePublished(row)!)">{{ $t('map.auto.preview3') }}</el-button>
                    <el-button
                      v-hasPermi="['map:catalog:publish']"
                      link
                      type="warning"
                      icon="Edit"
                      @click="pubRef?.openEdit(solePublished(row)!)"
                    >
                      {{ $t('map.auto.edit') }}
                    </el-button>
                  </template>
                  <el-button
                    v-if="row.delFlag !== 1"
                    v-hasPermi="['map:catalog:publish']"
                    link
                    type="primary"
                    icon="Refresh"
                    :loading="refreshingId === row.id"
                    @click="handleRefresh(row)"
                  >
                    {{ $t('map.auto.refresh') }}
                  </el-button>
                  <el-button
                    v-if="row.delFlag === 1"
                    v-hasPermi="['map:catalog:publish']"
                    link
                    type="success"
                    icon="RefreshLeft"
                    @click="handleRestore(row)"
                  >
                    {{ $t('map.auto.text55') }}
                  </el-button>
                </template>
              </el-table-column>
            </el-table>
            <div class="mt-2 text-xs text-gray-400">
              {{ $t('map.auto.resourceCountHint', { n: total }) }}
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
          </template>
          <el-empty v-else :description="$t('map.auto.oneOfRows')" />
        </section>
      </div>
    </el-card>

    <PublishDialog ref="pubRef" :caps="caps" @saved="reloadAll" />
    <PreviewDialog ref="pvRef" />

    <!-- PostGIS 普通表数据查询对话框 -->
    <el-dialog v-model="queryDlg.visible" :title="`${t('map.auto.query')}${queryDlg.tableName}`" width="900px" append-to-body>
      <div class="mb-2 flex items-center gap-2">
        <span class="text-xs text-gray-500">{{ queryDlg.schema }}.{{ queryDlg.table }}</span>
        <el-input-number v-model="queryDlg.limit" :min="1" :max="1000" size="small" controls-position="right" style="width: 120px" />
        <el-button type="primary" plain size="small" icon="Search" :loading="queryDlg.loading" @click="runQuery">{{ $t('map.auto.query2') }}</el-button>
        <span class="text-xs text-gray-400">{{ $t('map.auto.back') }}{{ queryDlg.rowCount }} {{ $t('map.auto.rows') }}{{ queryDlg.limit }}）</span>
      </div>
      <el-table v-loading="queryDlg.loading" :data="queryDlg.tableData" border size="small" max-height="500" style="width: 100%">
        <el-table-column
          v-for="col in queryDlg.columns"
          :key="col"
          :prop="col"
          :label="col"
          min-width="120"
          :show-overflow-tooltip="true"
        />
      </el-table>
      <div v-if="queryDlg.error" class="mt-2 text-xs text-red-500">{{ queryDlg.error }}</div>
    </el-dialog>
  </div>
</template>

<script setup name="MapCatalogResource" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import {
  discoverStore,
  listPublished,
  listResources,
  listStores,
  listWorkspaces,
  queryResource,
  refreshResource,
  restoreResource
} from '@/api/map/catalog';
import { PublishedVO, ResourceVO, StoreVO, WorkspaceVO } from '@/api/map/catalog/types';
import CatalogToolbar from './components/CatalogToolbar.vue';
import PublishDialog from './components/PublishDialog.vue';
import PreviewDialog from './components/PreviewDialog.vue';
import { bboxText, stateLabel, stateTag, useCaps } from './composables/useCatalog';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const route = useRoute();
const { caps, loadCaps } = useCaps();

const loading = ref(false);
const discovering = ref(false);
/** 正在单条刷新的图层 id（按钮 loading） */
const refreshingId = ref<number | null>(null);

/** PostGIS 普通表数据查询对话框状态 */
const queryDlg = reactive({
  visible: false,
  loading: false,
  resourceId: 0,
  tableName: '',
  schema: '',
  table: '',
  limit: 100,
  columns: [] as string[],
  tableData: [] as Record<string, string | null>[],
  rowCount: 0,
  error: ''
});

/** 打开普通表数据查询对话框 */
const openQuery = (row: ResourceVO) => {
  const meta = (row.meta || {}) as Record<string, unknown>;
  const sch = String(meta.schema || '');
  const tbl = String(meta.table || row.name);
  queryDlg.resourceId = row.id;
  queryDlg.tableName = row.name;
  queryDlg.schema = sch;
  queryDlg.table = tbl;
  queryDlg.limit = 100;
  queryDlg.columns = [];
  queryDlg.tableData = [];
  queryDlg.rowCount = 0;
  queryDlg.error = '';
  queryDlg.visible = true;
  runQuery();
};

/** 执行普通表数据查询（转发引擎 SELECT * LIMIT n） */
const runQuery = async () => {
  if (!queryDlg.resourceId) return;
  queryDlg.loading = true;
  queryDlg.error = '';
  try {
    const res = await queryResource(queryDlg.resourceId, queryDlg.limit);
    const cols = res.data.columns || [];
    const rows = res.data.rows || [];
    queryDlg.columns = cols;
    queryDlg.tableData = rows.map((r) => {
      const obj: Record<string, string | null> = {};
      cols.forEach((c, i) => (obj[c] = r[i] ?? null));
      return obj;
    });
    queryDlg.rowCount = res.data.row_count ?? rows.length;
  } catch (e: any) {
    queryDlg.error = e?.message || t('map.auto.queryfailed');
    queryDlg.columns = [];
    queryDlg.tableData = [];
    queryDlg.rowCount = 0;
  } finally {
    queryDlg.loading = false;
  }
};
const includeDeleted = ref(false);
const discoverDepth = ref(2);
const wsId = ref<number | undefined>(undefined);
const storeId = ref<number | undefined>(undefined);
const workspaces = ref<WorkspaceVO[]>([]);
const stores = ref<StoreVO[]>([]);
const resources = ref<ResourceVO[]>([]);
const published = ref<PublishedVO[]>([]);
const total = ref(0);

const page = ref(1);
const pageSize = ref(10);
// 后端分页：page/pageSize 变化即重新拉取对应页
watch([page, pageSize], () => {
  load();
});
// 切换数据源不在这里刷：选中数据源时可能要先自动发现，由 selectStore 串行处理，避免空列表闪烁
watch(includeDeleted, () => {
  page.value = 1;
  load();
});

const pubRef = ref<InstanceType<typeof PublishDialog>>();
const pvRef = ref<InstanceType<typeof PreviewDialog>>();

const storeOptions = computed(() => stores.value.filter((s) => !wsId.value || s.workspaceId === wsId.value));
const currentStore = computed(() => stores.value.find((s) => s.id === storeId.value) || null);
const wsName = (id: number) => workspaces.value.find((w) => w.id === id)?.name || `#${id}`;
/** 工作空间完整名（侧栏窄时下拉框只显示得下简称，完整名靠 title 提示） */
const wsLabel = (id: number) => {
  const w = workspaces.value.find((x) => x.id === id);
  return w ? `${w.name}${w.title ? ' — ' + w.title : ''}` : `#${id}`;
};

// ================= 左侧边栏 =================
/** 侧栏宽度可拖拽，范围 120–240px；记忆到本地，避免每次进来都要拉一次 */
const SIDE_MIN = 120;
const SIDE_MAX = 240;
const SIDE_KEY = 'rust-gisserver:catalog:side-width';
const sideWidth = ref(Math.min(SIDE_MAX, Math.max(SIDE_MIN, Number(localStorage.getItem(SIDE_KEY)) || 200)));

interface SideGroup {
  key: string;
  /** 分组标题（指定工作空间时不重复显示标题） */
  wsName: string;
  stores: StoreVO[];
}

/** 侧栏数据源列表：指定工作空间时平铺，未指定时按工作空间分组 */
const sideGroups = computed<SideGroup[]>(() => {
  if (wsId.value) {
    return [{ key: 'current', wsName: '', stores: stores.value.slice() }];
  }
  const map = new Map<number, SideGroup>();
  for (const w of workspaces.value) map.set(w.id, { key: `ws-${w.id}`, wsName: w.name, stores: [] });
  for (const s of stores.value) {
    if (!map.has(s.workspaceId)) {
      map.set(s.workspaceId, { key: `ws-${s.workspaceId}`, wsName: `#${s.workspaceId}`, stores: [] });
    }
    map.get(s.workspaceId)!.stores.push(s);
  }
  return [...map.values()].filter((g) => g.stores.length > 0);
});

/** 拖拽手柄调宽：手柄在侧栏右缘，向右拖变宽 */
const startResize = (e: MouseEvent) => {
  e.preventDefault();
  const startX = e.clientX;
  const startW = sideWidth.value;
  const onMove = (ev: MouseEvent) => {
    sideWidth.value = Math.min(SIDE_MAX, Math.max(SIDE_MIN, startW + (ev.clientX - startX)));
  };
  const onUp = () => {
    document.removeEventListener('mousemove', onMove);
    document.removeEventListener('mouseup', onUp);
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
    localStorage.setItem(SIDE_KEY, String(sideWidth.value));
  };
  document.addEventListener('mousemove', onMove);
  document.addEventListener('mouseup', onUp);
  document.body.style.cursor = 'col-resize';
  document.body.style.userSelect = 'none';
};

/** 点侧栏里的数据源 = 选中它；空账本先自动发现一次，再把图层列到右侧 */
const selectStore = async (s: StoreVO) => {
  if (storeId.value === s.id) return;
  storeId.value = s.id;
  page.value = 1;
  resources.value = [];
  total.value = 0;
  await ensureDiscovered(s);
  await load();
};

/** 勾选「显示已删除」时侧栏列表也要跟着变，故两边一起刷 */
const onFilterChange = async () => {
  await Promise.all([loadStores(), load()]);
};

const loadPublished = async () => {
  const res = await listPublished({ includeDeleted: 1 });
  published.value = res.data.published || [];
};

const load = async () => {
  // 后端按 store_id 精确匹配资源账本（storeId=0 查不到），未选数据源时不请求
  if (!storeId.value) {
    resources.value = [];
    total.value = 0;
    return;
  }
  loading.value = true;
  try {
    const params: { storeId: number; includeDeleted?: number; page?: number; pageSize?: number } = { storeId: storeId.value };
    if (includeDeleted.value) params.includeDeleted = 1;
    params.page = page.value;
    params.pageSize = pageSize.value;
    const [rRes] = await Promise.all([listResources(params), loadPublished()]);
    resources.value = rRes.data.resources || [];
    total.value = rRes.data.total || 0;
  } finally {
    loading.value = false;
  }
};

const reloadAll = async () => {
  await Promise.all([loadStores(), load(), loadPublished()]);
};

const loadStores = async () => {
  const res = await listStores({ ...(wsId.value ? { workspaceId: wsId.value } : {}), ...(includeDeleted.value ? { includeDeleted: 1 } : {}) });
  stores.value = res.data.stores || [];
  if (storeId.value && !stores.value.some((s) => s.id === storeId.value)) {
    storeId.value = undefined;
    resources.value = [];
    total.value = 0;
  }
};

const onWsChange = async () => {
  storeId.value = undefined;
  await Promise.all([loadStores(), load()]);
};

/** 资源 → 已发布服务（同一资源可能被发布成多个服务，逐个列出用于状态展示） */
const publishedOf = (row: ResourceVO) => published.value.filter((p) => p.resourceId === row.id);

/** 3dtiles / terrain 本身按 WGS 84/ECEF 规范渲染，不需要也不记录 EPSG，
 *  源坐标系可疑校验与自定义坐标/网格标记都应跳过这两类。 */
const is3dOrTerrain = (row: ResourceVO) => ['3dtiles', 'terrain'].includes((row.dataType || '').toLowerCase());

/** 源坐标系不是 WebMercator 的数据源：发布时必须选对应网格（4490 → CGCS2000Quad /
 *  天地图 LOD / 自定义 XML），否则发布后用默认 WebMercator 预览或浏览会白屏。
 *  3dtiles / terrain 本身按 WGS 84/ECEF 规范渲染，不走瓦片网格，跳过。 */
const needsNativeGrid = (row: ResourceVO) => {
  if (is3dOrTerrain(row)) return false;
  const m = String(row.srcCrs || '').match(/(\d+)\s*$/);
  return !!m && !['3857', '900913'].includes(m[1]);
};
const nativeGridTip = (row: ResourceVO) =>
  `${t('map.auto.crs9')}${row.srcCrs}${t('map.auto.yesWebMercatorHourCRSof4490')}` +
  t('map.auto.cgcs2000quadLODXMLPreviewByDefaultWebMerca');
/** 仅一个服务时才给「预览 / 修改」，多个则到「已发布服务」页分别操作 */
const solePublished = (row: ResourceVO) => (publishedOf(row).length === 1 ? publishedOf(row)[0] : undefined);

const handleDiscover = async () => {
  const s = currentStore.value;
  if (!s) return;
  await runDiscover(s);
};

/** 执行一次只读发现并刷新账本 + 侧栏计数 */
const runDiscover = async (s: StoreVO) => {
  discovering.value = true;
  try {
    const res = await discoverStore(s.id, s.isPostgis ? { schema: s.pgSchema } : { depth: discoverDepth.value });
    proxy?.$modal.msgSuccess(`${t('map.auto.text73')}${res.data.discovered}${t('map.auto.refresh3')}${res.data.stored}${t('map.auto.text74')}`);
    await loadStores();
    await load();
  } finally {
    discovering.value = false;
  }
};

/** 每个数据源每个页面会话只自动发现一次，避免反复扫描大目录 */
const autoDiscovered = new Set<number>();

/** 选中数据源时若账本为空则自动发现：这就是「选数据源 → 右侧出图层」的操作闭环 */
const ensureDiscovered = async (s: StoreVO) => {
  if (s.delFlag === 1 || (s.resourceCount ?? 0) > 0 || autoDiscovered.has(s.id)) return;
  autoDiscovered.add(s.id);
  try {
    await runDiscover(s);
  } catch {
    // 请求层已弹出错误（如引擎 data_root 未覆盖该目录），保留空态提示即可
  }
};

/** 空表格提示：把「为什么没有图层」和下一步动作讲清楚 */
const emptyText = computed(() => {
  const s = currentStore.value;
  if (!s) return t('map.auto.one3');
  if (s.delFlag === 1) return t('map.auto.delete4');
  return t('map.auto.dataLayerPleaseConfirmCatalogofTifMbtilesP');
});

const handleRefresh = async (row: ResourceVO) => {
  refreshingId.value = row.id;
  try {
    await refreshResource(row.id);
    proxy?.$modal.msgSuccess(`${t('map.auto.refresh4')}${row.name}${t('map.auto.of3')}`);
    await load();
  } finally {
    refreshingId.value = null;
  }
};

const handleRestore = async (row: ResourceVO) => {
  await proxy?.$modal.confirm(`${t('map.auto.confirmdataLayer')}${row.name}${t('map.auto.publishedServices2')}`);
  await restoreResource(row.id);
  proxy?.$modal.msgSuccess(t('map.auto.text75'));
  await reloadAll();
};

onMounted(async () => {
  // 深链：总览页目录树、数据源页的「图层」列都能带参数跳进来
  const qWs = Number(route.query.workspaceId);
  if (qWs > 0) wsId.value = qWs;
  const qStore = Number(route.query.storeId);
  await Promise.all([loadCaps(), listWorkspaces().then((r) => (workspaces.value = r.data.workspaces || [])), loadStores()]);
  if (qStore > 0 && stores.value.some((s) => s.id === qStore)) {
    storeId.value = qStore;
    const hit = stores.value.find((s) => s.id === qStore);
    if (!qWs && hit) wsId.value = hit.workspaceId;
  } else storeId.value = storeOptions.value.find((s) => s.delFlag === 0)?.id;
  // 深链/默认选中空数据源时也补一次自动发现，保证右侧立刻有图层
  const picked = stores.value.find((s) => s.id === storeId.value);
  if (picked) await ensureDiscovered(picked);
  await load();
});
</script>

<style scoped>
.link-type {
  color: var(--el-color-primary);
}

/* 左右分栏：左侧固定宽（可拖），右侧自适应 */
.catalog-split {
  display: flex;
  align-items: flex-start;
  gap: 16px;
}

.catalog-side {
  position: relative;
  flex: none;
  /* 覆盖全局 aside 规则（src/assets/styles/index.scss）：该规则 padding:8px 24px、浅底、margin 等，
     会污染本侧栏并把内容区压窄（窄边栏下下拉框只剩 79px）。这里显式归零。 */
  padding: 0 16px 0 0;
  margin-bottom: 0;
  background: transparent;
  border-radius: 0;
  line-height: normal;
  font-size: inherit;
  border-right: 1px solid var(--el-border-color-lighter);
}

.catalog-side__list {
  margin-top: 10px;
  max-height: 62vh;
  overflow-y: auto;
}

/* Element Plus 2.13 的 el-select 不把行内 width:100% 透传到根元素，
   这里用作用域规则 + !important 强制铺满侧栏内容宽度（窄边栏下也不塌） */
.catalog-side :deep(.el-select),
.catalog-side :deep(.el-select__wrapper) {
  width: 100% !important;
}

.catalog-side__group {
  padding: 8px 6px 2px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.catalog-side__item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border-radius: 6px;
  font-size: 13px;
  line-height: 1.4;
  cursor: pointer;
  user-select: none;
}

.catalog-side__item:hover {
  background: var(--el-fill-color-light);
}

.catalog-side__item.is-active {
  background: var(--el-color-primary-light-9);
  color: var(--el-color-primary);
  font-weight: 500;
}

.catalog-side__item.is-del {
  opacity: 0.55;
  text-decoration: line-through;
}

.catalog-side__item .name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.catalog-side__item .meta {
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

.catalog-side__resize {
  position: absolute;
  top: 0;
  right: -6px;
  width: 12px;
  height: 100%;
  cursor: col-resize;
}

.catalog-side__resize:hover {
  background: var(--el-color-primary-light-8);
}

.catalog-main {
  flex: 1;
  min-width: 0;
}

/* 资源名列：悬停显示数据类型/源坐标系/四至，行内仅保留关键的「可疑」警示 */
.res-name {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  cursor: help;
}

.res-name .name-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.crs-warn {
  color: var(--el-color-warning);
  font-weight: 600;
}
</style>
