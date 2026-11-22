<template>
  <div>
    <el-table v-loading="loading" border :data="rows" style="width: 100%">
      <el-table-column :label="$t('map.auto.no')" width="55" align="center">
        <template #default="scope">{{ (pageModel - 1) * pageSizeModel + scope.$index + 1 }}</template>
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
          <el-tag v-for="proto in toTypes(row.serviceTypes)" :key="proto" size="small" effect="plain" class="mr-1">
            {{ protoLabel(proto) }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column :label="$t('map.auto.crs8')" align="center" prop="srcCrs" width="100" />
      <el-table-column :label="$t('map.auto.status')" align="center" width="80">
        <template #default="{ row }">
          <el-tag :type="stateTag(row.state)" size="small" effect="dark">{{ stateLabel(row.state) }}</el-tag>
        </template>
      </el-table-column>

      <!-- Page-specific columns, rendered before the action column -->
      <slot name="columns" />

      <el-table-column :label="$t('map.auto.actions')" align="center" :width="actionsWidth" fixed="right">
        <template #default="{ row }">
          <el-button link type="success" icon="View" @click="emit('preview', row)">{{ $t('map.auto.preview3') }}</el-button>
          <el-button
            v-if="enableStyle && canEditStyle(row)"
            link
            type="primary"
            icon="Brush"
            :disabled="row.state === 'deleted'"
            @click="emit('style', row)"
          >
            {{ $t('map.auto.style6') }}
          </el-button>
          <el-button
            v-hasPermi="['map:catalog:publish']"
            link
            type="primary"
            icon="Edit"
            :disabled="row.state === 'deleted'"
            @click="emit('edit', row)"
          >
            {{ $t('map.auto.edit') }}
          </el-button>
          <template v-if="row.state === 'active'">
            <el-button v-hasPermi="['map:catalog:publish']" link type="warning" icon="VideoPause" @click="emit('state', row, 'stopped')">
              {{ $t('map.auto.cancel2') }}
            </el-button>
          </template>
          <template v-else-if="row.state === 'stopped' || row.state === 'paused'">
            <el-button v-hasPermi="['map:catalog:publish']" link type="success" icon="VideoPlay" @click="emit('state', row, 'active')">
              {{ $t('map.auto.text201') }}
            </el-button>
          </template>
          <el-button
            v-hasPermi="['map:catalog:publish']"
            link
            type="danger"
            icon="Delete"
            :disabled="row.state === 'deleted'"
            @click="emit('state', row, 'deleted')"
          >
            {{ $t('map.auto.delete6') }}
          </el-button>

          <!-- Page-specific row actions -->
          <slot name="row-actions" :row="row" />
        </template>
      </el-table-column>
    </el-table>

    <div class="mt-3 flex justify-end">
      <el-pagination
        v-model:current-page="pageModel"
        v-model:page-size="pageSizeModel"
        :page-sizes="[10, 20, 50, 100]"
        :total="total"
        layout="total, sizes, prev, pager, next, jumper"
        background
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import type { PublishedVO } from '@/api/map/catalog/types';
import { nativeGridInfo, nativeGridTip, protoLabel, stateLabel, stateTag } from '@/views/map/catalog/composables/useCatalog';
import { canEditStyle, kindLabel, toTypes } from '../composables/serviceMeta';

/**
 * Base published-service table. Common columns and lifecycle actions live here;
 * a service type page injects its own columns via `columns` and extra buttons
 * via `row-actions`, and controls the action column width / style button.
 */
withDefaults(
  defineProps<{
    rows: PublishedVO[];
    total: number;
    loading?: boolean;
    enableStyle?: boolean;
    actionsWidth?: number;
  }>(),
  { loading: false, enableStyle: false, actionsWidth: 320 }
);

const pageModel = defineModel<number>('page', { default: 1 });
const pageSizeModel = defineModel<number>('pageSize', { default: 20 });

const emit = defineEmits<{
  preview: [row: PublishedVO];
  style: [row: PublishedVO];
  edit: [row: PublishedVO];
  state: [row: PublishedVO, state: string];
}>();
</script>

<style scoped lang="scss">
.mr-1 {
  margin-right: 4px;
}
</style>
