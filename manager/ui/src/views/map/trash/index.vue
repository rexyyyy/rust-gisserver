<template>
  <div class="p-2">
    <!-- 说明 + 操作条 -->
    <div class="mb-[10px]">
      <el-card shadow="hover">
        <el-alert
          type="info"
          :closable="false"
          class="mb-2"
          title="垃圾箱中的服务不会被加载，也不会出现在「服务管理」列表；恢复后立即生效（重建服务集并清理缓存）。"
        />
        <el-form :inline="true">
          <el-form-item label="服务名称">
            <el-input v-model="queryName" placeholder="按服务名/标题过滤" clearable style="width: 200px" />
          </el-form-item>
          <el-form-item label="服务类">
            <el-select v-model="queryKind" placeholder="全部" clearable style="width: 160px">
              <el-option v-for="k in kindOptions" :key="k" :label="kindLabel(k)" :value="k" />
            </el-select>
          </el-form-item>
          <el-form-item>
            <el-button icon="Refresh" @click="getList">刷新</el-button>
          </el-form-item>
        </el-form>
      </el-card>
    </div>

    <el-card shadow="hover">
      <el-table v-loading="loading" border :data="filteredList" style="width: 100%">
        <el-table-column label="序号" width="50" type="index" align="center" />
        <el-table-column label="服务名称" align="center" prop="name" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column label="标题" align="center" prop="title" min-width="140" :show-overflow-tooltip="true" />
        <el-table-column label="服务类" align="center" width="110">
          <template #default="scope">
            <el-tag type="info" effect="plain">{{ kindLabel(scope.row.kind) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="数据位置" align="center" prop="source" min-width="200" :show-overflow-tooltip="true" />
        <el-table-column label="登记状态" align="center" width="100">
          <template #default="scope">
            <el-tag type="info">{{ stateLabel(scope.row.state) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="操作" align="center" width="160" class-name="small-padding fixed-width">
          <template #default="scope">
            <el-button v-hasPermi="['map:service:state']" link type="success" @click="handleRestore(scope.row)">恢复</el-button>
          </template>
        </el-table-column>
      </el-table>
      <el-empty v-if="!loading && filteredList.length === 0" description="垃圾箱为空" />
    </el-card>
  </div>
</template>

<script setup lang="ts" name="MapTrash">
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
    xyz: 'XYZ瓦片',
    tms: 'TMS瓦片',
    cog: 'COG栅格',
    mbtiles: 'MBTiles',
    pmtiles: 'PMTiles',
    postgis: 'PostGIS',
    arcgis: 'ArcGIS瓦片',
    '3dtiles': '3D Tiles'
  };
  return map[k] || k;
};

const stateLabel = (s: string) => {
  const map: Record<string, string> = { active: '在用', stopped: '已停止', paused: '已暂停', deleted: '已删除' };
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
  const [err] = await to(proxy?.$modal.confirm(`确认从垃圾箱恢复服务「${row.name}」？`) as any);
  if (err) return;
  await changeServiceState(row.kind, row.name, 'active');
  proxy?.$modal.msgSuccess('恢复成功');
  await getList();
};

onMounted(() => {
  getList();
});
</script>
