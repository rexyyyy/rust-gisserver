<template>
  <!-- 矢量服务：vector + postgis 已发布图层 -->
  <PublishedServiceList :kinds="['vector', 'postgis']" />
</template>

<script setup lang="ts">
import PublishedServiceList from './components/PublishedServiceList.vue';
</script>
