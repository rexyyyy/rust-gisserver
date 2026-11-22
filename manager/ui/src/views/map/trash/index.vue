<template>
  <div class="p-2">
    <!-- 说明 + 操作条 -->
    <div class="mb-[10px]">
      <el-card shadow="hover">
        <el-alert
          type="info"
          :closable="false"
          class="mb-2"
          :title="$t('map.auto.ofCache')"
        />
        <el-form :inline="true">
          <el-form-item :label="$t('map.auto.serviceName2')">
            <el-input v-model="queryName" :placeholder="$t('map.auto.filterByServiceNameTitle')" clearable style="width: 200px" />
          </el-form-item>
          <el-form-item :label="$t('map.auto.text135')">
            <el-select v-model="queryKind" :placeholder="$t('map.auto.all2')" clearable style="width: 160px">
              <el-option v-for="k in kindOptions" :key="k" :label="kindLabel(k)" :value="k" />
            </el-select>
          </el-form-item>
          <el-form-item>
            <el-button icon="Refresh" @click="getList">{{ $t('map.auto.refresh') }}</el-button>
          </el-form-item>
        </el-form>
      </el-card>
    </div>

    <el-card shadow="hover">
      <el-table v-loading="loading" border :data="filteredList" style="width: 100%">
        <el-table-column :label="$t('map.auto.no')" width="50" type="index" align="center" />
        <el-table-column :label="$t('map.auto.serviceName2')" align="center" prop="name" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.title')" align="center" prop="title" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.text135')" align="center" width="110">
          <template #default="scope">
            <el-tag type="info" effect="plain">{{ kindLabel(scope.row.kind) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.text156')" align="center" prop="source" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column :label="$t('map.auto.status4')" align="center" width="100">
          <template #default="scope">
            <el-tag type="info">{{ stateLabel(scope.row.state) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="$t('map.auto.actions')" align="center" width="160" class-name="small-padding fixed-width">
          <template #default="scope">
            <el-button v-hasPermi="['map:service:state']" link type="success" @click="handleRestore(scope.row)">{{ $t('map.auto.text55') }}</el-button>
          </template>
        </el-table-column>
      </el-table>
      <el-empty v-if="!loading && filteredList.length === 0" :description="$t('map.auto.text157')" />
    </el-card>
  </div>
</template>

<script setup lang="ts" name="MapTrash">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
import { changeServiceState, listServices } from '@/api/map/service';
import { MapServiceVO } from '@/api/map/service/types';
import { to } from 'await-to-js';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const loading = ref(true);
const trashList = ref<MapServiceVO[]>([]);

const queryName = ref('');
const queryKind = ref('');

const kindOptions = computed(() => [...new Set(trashList.value.map((s) => s.kind))]);

const filteredList = computed(() =>
  trashList.value.filter((s) => {
    if (queryKind.value && s.kind !== queryKind.value) return false;
    if (queryName.value && !(s.name + s.title).toLowerCase().includes(queryName.value.toLowerCase())) return false;
    return true;
  })
);

const kindLabel = (k: string) => {
  const map: Record<string, string> = {
    xyz: t('map.auto.xyz'),
    tms: t('map.auto.tms'),
    cog: t('map.auto.cogRaster'),
    mbtiles: 'MBTiles',
    pmtiles: 'PMTiles',
    postgis: 'PostGIS',
    arcgis: t('map.auto.arcgisTiles'),
    '3dtiles': '3D Tiles'
  };
  return map[k] || k;
};

const stateLabel = (s: string) => {
  const map: Record<string, string> = { active: t('map.auto.text158'), stopped: t('map.auto.stopped'), paused: t('map.auto.paused'), deleted: t('map.auto.delete') };
  return map[s] || s;
};

const getList = async () => {
  loading.value = true;
  try {
    const res = await listServices();
    trashList.value = (res.data.services || []).filter((s) => s.state === 'deleted');
  } finally {
    loading.value = false;
  }
};

// 恢复：deleted → active，即时生效
const handleRestore = async (row: MapServiceVO) => {
  const [err] = await to(proxy?.$modal.confirm(`${t('map.auto.confirm6')}${row.name}"?`) as any);
  if (err) return;
  await changeServiceState(row.kind, row.name, 'active');
  proxy?.$modal.msgSuccess(t('map.auto.success4'));
  await getList();
};

onMounted(() => {
  getList();
});
</script>
