<template>
  <div class="service-table-toolbar">
    <div class="service-table-toolbar__left">
      <span class="service-table-toolbar__title">{{ $t('map.auto.publishedServices') }}</span>
      <span class="service-table-toolbar__count">{{ $t('map.auto.text139') }}{{ count }} {{ $t('map.auto.text140') }}</span>
      <!-- Page-specific summary / indicators -->
      <slot name="left" />
    </div>
    <div class="service-table-toolbar__right">
      <!-- Page-specific buttons, placed before the shared actions -->
      <slot name="actions" />
      <el-button icon="Refresh" @click="emit('refresh')">{{ $t('map.auto.refresh') }}</el-button>
      <el-button type="primary" plain @click="emit('publish')">
        <el-icon><Plus /></el-icon>
        {{ $t('map.auto.text200') }}
      </el-button>
      <!-- Page-specific buttons, placed after the shared actions -->
      <slot name="actions-tail" />
    </div>
  </div>
</template>

<script setup lang="ts">
defineProps<{ count: number }>();
const emit = defineEmits<{
  refresh: [];
  publish: [];
}>();
</script>

<style scoped lang="scss">
.service-table-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--el-border-color-lighter);

  &__left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  &__title {
    font-size: 15px;
    font-weight: 600;
    color: var(--el-text-color-primary);
  }

  &__count {
    font-size: 13px;
    color: var(--el-text-color-secondary);
  }

  &__right {
    display: flex;
    align-items: center;
    gap: 8px;
  }
}
</style>
