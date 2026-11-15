<template>
  <div class="p-2">
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>
        {{ $t('map.auto.publishedListHint') }}
      </template>
    </el-alert>

    <el-card shadow="hover">
      <div class="filter-bar">
        <el-form :inline="true" class="filter-form">
          <el-form-item :label="$t('map.auto.serviceName2')">
            <el-input v-model="queryName" :placeholder="$t('map.auto.filterByServiceNameTitle')" clearable style="width: 200px" />
          </el-form-item>
          <el-form-item :label="$t('map.auto.status')">
            <el-select v-model="queryState" :placeholder="$t('map.auto.all2')" clearable style="width: 140px">
              <el-option :label="$t('map.auto.inService2')" value="active" />
              <el-option :label="$t('map.auto.stopped')" value="stopped" />
              <el-option :label="$t('map.auto.paused')" value="paused" />
            </el-select>
          </el-form-item>
        </el-form>
        <div class="format-tags">
          <el-tag v-for="kind in displayKinds" :key="kind" type="primary" effect="plain" size="small">
            {{ kindLabel(kind) }}
          </el-tag>
        </div>
      </div>

      <div class="table-toolbar">
        <div class="toolbar-left">
          <span class="toolbar-title">{{ $t('map.auto.publishedServices') }}</span>
          <span class="toolbar-count">{{ $t('map.auto.text139') }}{{ filteredRows.length }} {{ $t('map.auto.text140') }}</span>
        </div>
        <div class="toolbar-right">
          <el-button icon="Refresh" @click="load">{{ $t('map.auto.refresh') }}</el-button>
          <el-button type="primary" plain @click="goPublish">
            <el-icon><Plus /></el-icon>
            {{ $t('map.auto.text200') }}
          </el-button>
        </div>
      </div>

      <el-table v-loading="loading" border :data="pagedRows" style="width: 100%">
        <el-table-column :label="$t('map.auto.no')" width="55" align="center">
          <template #default="scope">{{ (page - 1) * pageSize + scope.$index + 1 }}</template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.serviceName')" prop="layerName" min-width="190" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span>{{ row.layerName }}</span>
            <el-tooltip v-if="nativeGridInfo(row)" :content="nativeGridTip(row, nativeGridInfo(row)!)" placement="top" :show-after="200">
              <el-tag type="warning" effect="plain" size="small" class="ml-1">{{ $t('map.auto.customCRSGrid') }}</el-tag>
            </el-tooltip>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.title')" prop="title" min-width="130" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.type')" align="center" width="110">
          <template #default="{ row }">
            <el-tag effect="plain" size="small">{{ kindLabel(row.mapserverKind) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.dataType')" align="center" width="110">
          <template #default="{ row }">
            <el-tag effect="plain" size="small" type="info">{{ row.dataType || row.mapserverKind }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.protocol5')" align="center" min-width="170">
          <template #default="{ row }">
            <el-tag v-for="t in toTypes(row.serviceTypes)" :key="t" size="small" effect="plain" class="mr-1">
              {{ protoLabel(t) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.crs8')" align="center" prop="srcCrs" width="100" />
        <el-table-column :label="$t('map.auto.status')" align="center" width="80">
          <template #default="{ row }">
            <el-tag :type="stateTag(row.state)" size="small" effect="dark">{{ stateLabel(row.state) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.actions')" align="center" width="320" fixed="right">
          <template #default="{ row }">
            <el-button link type="success" icon="View" @click="pvRef?.open(row)">{{ $t('map.auto.preview3') }}</el-button>
            <el-button
              v-if="showStyleBtn(row)"
              link
              type="primary"
              icon="Brush"
              :disabled="row.state === 'deleted'"
              @click="openStyle(row)"
            >
              {{ $t('map.auto.style6') }}
            </el-button>
            <el-button
              v-hasPermi="['map:catalog:publish']"
              link
              type="primary"
              icon="Edit"
              :disabled="row.state === 'deleted'"
              @click="openEdit(row)"
            >
              {{ $t('map.auto.edit') }}
            </el-button>
            <template v-if="row.state === 'active'">
              <el-button
                v-hasPermi="['map:catalog:publish']"
                link
                type="warning"
                icon="VideoPause"
                @click="handleState(row, 'stopped')"
              >
                {{ $t('map.auto.cancel2') }}
              </el-button>
            </template>
            <template v-else-if="row.state === 'stopped' || row.state === 'paused'">
              <el-button
                v-hasPermi="['map:catalog:publish']"
                link
                type="success"
                icon="VideoPlay"
                @click="handleState(row, 'active')"
              >
                {{ $t('map.auto.text201') }}
              </el-button>
            </template>
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

      <div class="mt-3 flex justify-end">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :page-sizes="[10, 20, 50, 100]"
          :total="filteredRows.length"
          layout="total, sizes, prev, pager, next, jumper"
          background
        />
      </div>
    </el-card>

    <PublishDialog ref="pubRef" :caps="caps" @saved="load" />
    <PreviewDialog ref="pvRef" />
    <StyleJsonEditor ref="styleRef" />
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
// Plus 由全局注册的 Element Plus Icons 提供
import { listPublished, setPublishedState } from '@/api/map/catalog';
import { PublishedVO } from '@/api/map/catalog/types';
import PublishDialog from '@/views/map/catalog/components/PublishDialog.vue';
import PreviewDialog from '@/views/map/catalog/components/PreviewDialog.vue';
import StyleJsonEditor from '@/views/map/service/components/StyleJsonEditor.vue';
import { nativeGridInfo, nativeGridTip, protoLabel, stateLabel, stateTag, useCaps } from '@/views/map/catalog/composables/useCatalog';

const props = defineProps<{ kinds?: string[] }>();

const router = useRouter();
const { proxy } = getCurrentInstance() as ComponentInternalInstance;
const { caps, loadCaps } = useCaps();

const loading = ref(false);
const rows = ref<PublishedVO[]>([]);
const queryName = ref('');
const queryState = ref('');
const page = ref(1);
const pageSize = ref(20);

const pubRef = ref<InstanceType<typeof PublishDialog>>();
const pvRef = ref<InstanceType<typeof PreviewDialog>>();
const styleRef = ref<InstanceType<typeof StyleJsonEditor>>();

const displayKinds = computed(() => props.kinds?.length ? props.kinds : []);

const KIND_LABELS: Record<string, string> = {
  cog: t('map.auto.cogRaster'),
  vector: t('map.auto.vector'),
  pmtiles: 'PMTiles',
  mbtiles: 'MBTiles',
  postgis: 'PostGIS',
  arcgis: t('map.auto.arcgisTiles'),
  '3dtiles': '3D Tiles',
  terrain: t('map.auto.terrain')
};
const kindLabel = (k: string) => KIND_LABELS[k] || k;

const filteredRows = computed(() => {
  let list = rows.value;
  if (displayKinds.value.length) {
    list = list.filter((r) => displayKinds.value.includes(r.mapserverKind));
  }
  const q = queryName.value.trim().toLowerCase();
  if (q) {
    list = list.filter(
      (r) =>
        r.layerName.toLowerCase().includes(q) ||
        (r.title || '').toLowerCase().includes(q)
    );
  }
  if (queryState.value) {
    list = list.filter((r) => r.state === queryState.value);
  }
  return list;
});

const pagedRows = computed(() => {
  const start = (page.value - 1) * pageSize.value;
  return filteredRows.value.slice(start, start + pageSize.value);
});

watch([queryName, queryState], () => {
  page.value = 1;
});

const load = async () => {
  loading.value = true;
  try {
    const res = await listPublished({ includeDeleted: 1 });
    rows.value = (res.data.published || []).filter((p) => p.state !== 'deleted');
  } finally {
    loading.value = false;
  }
};

const goPublish = () => {
  router.push('/data/resource');
};

// 后端 serviceTypes 可能是 string ("mvt,wms") 也可能是 array (["mvt","wms"])
const toTypes = (v: string | string[] | undefined | null): string[] =>
  Array.isArray(v) ? v : (v ? v.split(',').map((s) => s.trim()).filter(Boolean) : []);

// "样式" 按钮只对矢量/PostGIS 且支持 MVT 协议的服务显示
const showStyleBtn = (row: PublishedVO) => {
  const kindOk = ['postgis', 'vector'].includes(row.mapserverKind);
  const hasMvt = toTypes(row.serviceTypes).includes('mvt');
  return kindOk && hasMvt;
};
const geomTypeOf = (row: any): 'point' | 'line' | 'polygon' | 'mixed' => {
  // 优先读后端顶层 geometryType 字段（catalog.rs published_json 返回）
  // 兼容 row.meta.geometry_type（其他场景）
  const raw = (
    row?.geometryType ||
    row?.geometry_type ||
    row?.meta?.geometry_type ||
    row?.meta?.geometryType ||
    ''
  ).toUpperCase();
  if (!raw) return 'mixed';
  if (raw === 'POINT' || raw === 'MULTIPOINT') return 'point';
  if (raw === 'LINESTRING' || raw === 'MULTILINESTRING') return 'line';
  if (raw === 'POLYGON' || raw === 'MULTIPOLYGON') return 'polygon';
  return 'mixed'; // GEOMETRY / GEOMETRYCOLLECTION / 未知
};

const openStyle = (row: PublishedVO) => {
  styleRef.value?.open({
    kind: row.mapserverKind,
    name: row.layerName,
    title: row.title || row.layerName,
    geometryType: geomTypeOf(row as any)
  });
};

const openEdit = (row: PublishedVO) => {
  pubRef.value?.openEdit(row);
};

const handleState = async (row: PublishedVO, state: string) => {
  const tips: Record<string, string> = {
    active: `${t('map.auto.confirmRepublish')}${row.layerName}${t('map.auto.theEngineWillPickUpThisServiceAgain')}`,
    stopped: `${t('map.auto.confirmUnpublish')}${row.layerName}${t('map.auto.theServiceWillStopBeingServedButItsConfigu')}`,
    deleted: `${t('map.auto.confirmDeleteService')}${row.layerName}${t('map.auto.itWillBeMovedToTheTrashConfigurationIsNotP')}`
  };
  await proxy?.$modal.confirm(tips[state] || t('map.auto.confirmOperation'));
  await setPublishedState(row.id, state);
  proxy?.$modal.msgSuccess(t('map.auto.updated2'));
  await load();
};

onMounted(async () => {
  await Promise.all([loadCaps(), load()]);
});
</script>

<style scoped lang="scss">
.filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}

.filter-form {
  flex: 1;
  min-width: 0;
}

.format-tags {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.table-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 12px;
}

.toolbar-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--el-text-color-primary);
}

.toolbar-count {
  font-size: 13px;
  color: var(--el-text-color-secondary);
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.mr-1 {
  margin-right: 4px;
}
</style>
