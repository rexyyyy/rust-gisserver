<template>
  <!-- ArcGIS 瓦片图层：已发布图层 -->
  <PublishedServiceList :kinds="['arcgis']" />
</template>

<script setup lang="ts">
import PublishedServiceList from './components/PublishedServiceList.vue';
</script>
