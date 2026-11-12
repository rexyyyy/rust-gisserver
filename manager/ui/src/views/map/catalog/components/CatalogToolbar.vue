<template>
  <div class="catalog-toolbar">
    <div class="catalog-toolbar__filters">
      <slot name="filters" />
    </div>
    <div class="catalog-toolbar__actions">
      <slot name="actions" />
    </div>
  </div>
</template>

<script setup name="CatalogToolbar" lang="ts">
/**
 * 目录类页面的统一操作栏。
 *
 * 约定：
 * - filters 插槽放筛选控件（左），actions 插槽放动作按钮（右）；
 * - 动作组固定靠右（margin-left:auto），控件换行时仍停在最右；
 * - 控件间距 12px、动作间距 8px，并压掉 Element Plus 自带的按钮左外边距，避免双重间距；
 * - 底部一条分隔线把操作栏与表格分开，不再"贴着"数据区。
 */
</script>

<style>
.catalog-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--el-border-color-light);
}

.catalog-toolbar__filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  min-width: 0;
}

.catalog-toolbar__actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

/* Element Plus 对相邻按钮自带 12px 左外边距，与 gap 叠加会显得过散 */
.catalog-toolbar__actions .el-button + .el-button {
  margin-left: 0;
}
</style>
