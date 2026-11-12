<template>
  <!-- 3D Tiles：已发布图层 -->
  <PublishedServiceList :kinds="['3dtiles']" />
</template>

<script setup lang="ts">
import PublishedServiceList from '@/views/map/service/components/PublishedServiceList.vue';
</script>
