<template>
  <div class="service-filter-bar">
    <el-form :inline="true" class="service-filter-bar__form">
      <el-form-item :label="$t('map.auto.serviceName2')">
        <el-input v-model="nameModel" :placeholder="$t('map.auto.filterByServiceNameTitle')" clearable style="width: 200px" />
      </el-form-item>
      <el-form-item :label="$t('map.auto.status')">
        <el-select v-model="stateModel" :placeholder="$t('map.auto.all2')" clearable style="width: 140px">
          <el-option :label="$t('map.auto.inService2')" value="active" />
          <el-option :label="$t('map.auto.stopped')" value="stopped" />
          <el-option :label="$t('map.auto.paused')" value="paused" />
        </el-select>
      </el-form-item>
      <!-- Page-specific filter controls -->
      <slot name="filters" />
    </el-form>

    <div class="service-filter-bar__tags">
      <el-tag v-for="kind in kinds" :key="kind" type="primary" effect="plain" size="small">
        {{ kindLabel(kind) }}
      </el-tag>
      <!-- Page-specific status tags -->
      <slot name="tags" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { kindLabel } from '../composables/serviceMeta';

defineProps<{ kinds?: string[] }>();
const nameModel = defineModel<string>('name', { default: '' });
const stateModel = defineModel<string>('state', { default: '' });
</script>

<style scoped lang="scss">
.service-filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;

  &__form {
    flex: 1;
    min-width: 0;
  }

  &__tags {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
}
</style>
