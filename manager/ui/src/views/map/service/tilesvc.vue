<template>
  <!-- 瓦片服务：PMTiles + MBTiles 已发布图层 -->
  <PublishedServiceList :kinds="['pmtiles', 'mbtiles']" />
</template>

<script setup lang="ts">
import PublishedServiceList from './components/PublishedServiceList.vue';
</script>
