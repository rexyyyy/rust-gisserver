<template>
  <div class="p-2">
    <!-- 红线提示：目录层只存指针与元数据，不碰数据本体 -->
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]" :title="$t('map.auto.catalogIndexHint')" />

    <!-- 功能入口：工作空间 / 数据源 / 图层分工 -->
    <el-row :gutter="10" class="mb-[10px]">
      <el-col v-for="e in entries" :key="e.path" :lg="6" :md="12" :sm="12" :xs="24">
        <el-card shadow="hover" class="entry-card" @click="go(e.path)">
          <div class="flex items-center gap-2">
            <svg-icon :icon-class="e.icon" class-name="entry-icon" />
            <div>
              <div class="font-bold">{{ e.title }}</div>
              <div class="mt-1 text-xs text-gray-500">{{ e.desc }}</div>
            </div>
            <span class="ml-auto text-lg font-bold" :style="{ color: 'var(--el-color-primary)' }">{{ e.value }}</span>
          </div>
        </el-card>
      </el-col>
    </el-row>

    <el-row :gutter="10">
      <!-- 目录树：工作空间 → 数据源（只读跳转，不在本页做 CRUD） -->
      <el-col :lg="9" :md="24" :sm="24" :xs="24">
        <el-card shadow="hover">
          <template #header>
            <div class="flex items-center justify-between">
              <span class="font-bold">{{ $t('map.auto.catalog3') }}</span>
              <el-button link type="primary" icon="Refresh" @click="loadTree">{{ $t('map.auto.refresh') }}</el-button>
            </div>
          </template>
          <el-tree
            v-loading="treeLoading"
            :data="treeData"
            node-key="key"
            :props="{ label: 'label', children: 'children' }"
            :expand-on-click-node="false"
            default-expand-all
            highlight-current
            @node-click="onNodeClick"
          >
            <template #default="{ data }">
              <span class="flex items-center">
                <el-icon><component :is="data.type === 'ws' ? 'Folder' : 'Files'" /></el-icon>
                <span class="ml-1">{{ data.label }}</span>
                <span v-if="data.countText" class="ml-1 text-xs text-gray-400">{{ data.countText }}</span>
              </span>
            </template>
          </el-tree>
          <div v-if="!treeData.length && !treeLoading" class="text-xs text-gray-400">
            {{ $t('map.auto.noWorkspaceHint') }}
          </div>
        </el-card>
      </el-col>

      <!-- 能力矩阵：与引擎 /admin/capabilities 同源，避免管理端另立口径 -->
      <el-col :lg="15" :md="24" :sm="24" :xs="24">
        <el-card shadow="hover">
          <template #header>
            <div class="flex items-center justify-between">
              <span class="font-bold">{{ $t('map.auto.text47') }}</span>
              <span class="text-xs text-gray-400">{{ $t('map.auto.adminCapabilitiesTwo') }}</span>
            </div>
          </template>
          <el-table v-loading="!caps" border :data="caps?.data_types || []" size="small" style="width: 100%">
            <el-table-column :label="$t('map.auto.dataType')" align="center" width="130">
              <template #default="{ row }">
                <el-tag effect="plain" size="small">{{ row.data_type }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="$t('map.auto.text48')" align="center" prop="mapserver_kind" width="110" />
            <el-table-column :label="$t('map.auto.text49')" align="center" width="80">
              <template #default="{ row }">
                <el-tag :type="row.publishable ? 'success' : 'danger'" size="small">
                  {{ row.publishable ? $t('map.auto.text50') : $t('map.auto.no2') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="$t('map.auto.protocol4')" align="center" min-width="200">
              <template #default="{ row }">
                <el-tag v-for="t in row.service_types" :key="t" size="small" effect="plain" class="mr-1">
                  {{ protoLabel(t) }}
                </el-tag>
                <span v-if="!row.service_types?.length" class="text-xs text-gray-400">—</span>
              </template>
            </el-table-column>
            <el-table-column :label="$t('map.auto.defaultprotocol')" align="center" min-width="120">
              <template #default="{ row }">
                <span class="text-xs">{{ (row.default_service_types || []).map(protoLabel).join(' / ') || '—' }}</span>
              </template>
            </el-table-column>
            <el-table-column :label="$t('map.auto.text20')" prop="note" min-width="220" :show-overflow-tooltip="true" />
          </el-table>
          <div v-if="caps && !caps.data_types.length" class="text-xs text-gray-400">
            {{ $t('map.auto.back2') }}map-api {{ $t('map.auto.protocol6') }}
          </div>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup name="MapCatalog" lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { listPublished, listStores, listWorkspaces } from '@/api/map/catalog';
import { PublishedVO, StoreVO, WorkspaceVO } from '@/api/map/catalog/types';
import { protoLabel, useCaps } from './composables/useCatalog';

const router = useRouter();
const { caps, loadCaps } = useCaps();

const treeLoading = ref(false);
const workspaces = ref<WorkspaceVO[]>([]);
const stores = ref<StoreVO[]>([]);
const published = ref<PublishedVO[]>([]);

interface CatalogNode {
  key: string;
  label: string;
  type: 'ws' | 'store';
  countText: string;
  path: string;
  children?: CatalogNode[];
}

const liveStores = computed(() => stores.value.filter((s) => s.delFlag === 0));

const treeData = computed<CatalogNode[]>(() =>
  workspaces.value.map((w) => ({
    key: `ws-${w.id}`,
    label: w.title || w.name,
    type: 'ws' as const,
    countText: `${w.storeCount}/${w.publishedCount}`,
    path: `/data/resource?workspaceId=${w.id}`,
    children: liveStores.value
      .filter((s) => s.workspaceId === w.id)
      .map((s) => ({
        key: `st-${s.id}`,
        label: s.name,
        type: 'store' as const,
        countText: `${s.resourceCount}`,
        path: `/data/resource?storeId=${s.id}`
      }))
  }))
);

// 四个入口的计数都来自既有清单接口，不额外请求（资源数按 store 的 resourceCount 汇总）
const entries = computed(() => [
  {
    title: t('map.auto.text51'),
    desc: t('map.auto.workspace'),
    icon: 'workspace',
    path: '/data/workspace',
    value: workspaces.value.length
  },
  {
    title: t('map.auto.text52'),
    desc: t('map.auto.catalogPostGISStore'),
    icon: 'store',
    path: '/data/store',
    value: liveStores.value.length
  },
  {
    title: t('map.auto.dataLayer'),
    desc: t('map.auto.ofResource'),
    icon: 'layers',
    path: '/data/resource',
    value: liveStores.value.reduce((n, s) => n + (s.resourceCount || 0), 0)
  },
  {
    title: t('map.auto.publishedServices'),
    desc: t('map.auto.previewLayer'),
    icon: 'published',
    path: '/data/published',
    value: published.value.filter((p) => p.state === 'active').length
  }
]);

const loadTree = async () => {
  treeLoading.value = true;
  try {
    const [wRes, sRes, pRes] = await Promise.all([listWorkspaces(), listStores(), listPublished({ includeDeleted: 1 })]);
    workspaces.value = wRes.data.workspaces || [];
    stores.value = sRes.data.stores || [];
    published.value = pRes.data.published || [];
  } finally {
    treeLoading.value = false;
  }
};

const go = (path: string) => router.push(path);

/** 树节点只做跳转：本页不再承载 CRUD，避免同一份表单在两处维护 */
const onNodeClick = (data: CatalogNode) => go(data.path);

onMounted(async () => {
  await Promise.all([loadCaps(), loadTree()]);
});
</script>

<style scoped>
.entry-card {
  cursor: pointer;
  margin-bottom: 10px;
}
.entry-card:hover {
  border-color: var(--el-color-primary);
}
.entry-icon {
  font-size: 26px;
  color: var(--el-color-primary);
}
:deep(.el-tree-node__content) {
  height: 30px;
}
</style>

