<template>
  <!-- 瓦片服务页（PMTiles + MBTiles）：页面元素由本文件独立持有，扩展只影响本类型 -->
  <div class="p-2">
    <el-alert type="info" :closable="false" show-icon class="mb-[10px]">
      <template #title>{{ $t('map.auto.publishedListHint') }}</template>
    </el-alert>

    <el-card shadow="hover">
      <!-- 扩展位 A：#filters 放瓦片格式/级别范围等专属筛选，#tags 放专属标记 -->
      <ServiceFilterBar v-model:name="queryName" v-model:state="queryState" :kinds="kinds" />

      <!-- 扩展位 B：#left 放本类型统计，#actions 放瓦片校验等专属按钮 -->
      <ServiceTableToolbar :count="filteredRows.length" @refresh="load" @publish="goPublish" />

      <!-- 扩展位 C：#columns / #row-actions 加专属列与行操作，actionsWidth 控制操作列宽度。
           示例：把 ServiceTable 改为成对标签后插入
           <template #columns>
             <el-table-column :label="$t('map.auto.type')" width="120">
               <template #default="{ row }">{{ row.mapserverKind }}</template>
             </el-table-column>
           </template> -->
      <ServiceTable
        v-model:page="page"
        v-model:page-size="pageSize"
        :rows="pagedRows"
        :total="filteredRows.length"
        :loading="loading"
        :enable-style="enableStyle"
        :actions-width="actionsWidth"
        @preview="openPreview"
        @style="openStyle"
        @edit="openEdit"
        @state="handleState"
      />
    </el-card>

    <PublishDialog ref="pubRef" :caps="caps" @saved="load" />
    <PreviewDialog ref="pvRef" />
    <StyleJsonEditor v-if="enableStyle" ref="styleRef" />
  </div>
</template>

<script setup lang="ts">
import PublishDialog from '@/views/map/catalog/components/PublishDialog.vue';
import PreviewDialog from '@/views/map/catalog/components/PreviewDialog.vue';
import StyleJsonEditor from '@/views/map/service/components/StyleJsonEditor.vue';
import ServiceFilterBar from './components/ServiceFilterBar.vue';
import ServiceTableToolbar from './components/ServiceTableToolbar.vue';
import ServiceTable from './components/ServiceTable.vue';
import { usePublishedServiceList } from './composables/usePublishedServiceList';

// 本页配置：只列 PMTiles / MBTiles；瓦片包无 MVT 样式编辑
const kinds = ['pmtiles', 'mbtiles'];
const enableStyle = false;
const actionsWidth = 320;
// 需要追加本类型专属数据口径时，用 usePublishedServiceList({ kinds, extraFilter })

const {
  loading,
  filteredRows,
  pagedRows,
  page,
  pageSize,
  queryName,
  queryState,
  caps,
  pubRef,
  pvRef,
  styleRef,
  load,
  goPublish,
  openPreview,
  openEdit,
  openStyle,
  handleState
} = usePublishedServiceList({ kinds });
</script>
