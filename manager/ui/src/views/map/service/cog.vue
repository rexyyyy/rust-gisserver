<template>
  <!-- 栅格服务：COG 已发布图层 -->
  <PublishedServiceList :kinds="['cog']" />
</template>

<script setup lang="ts">
import PublishedServiceList from './components/PublishedServiceList.vue';
</script>
