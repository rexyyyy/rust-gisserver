<template>
  <div class="p-2">
    <!-- 编辑引导（§9.5.5：不做 Web 编辑器，引导 QGIS） -->
    <div class="mb-[10px]">
      <el-card shadow="hover">
        <template #header>
          <span class="font-bold">样式库（SLD 1.1 / SE，只读管理）</span>
        </template>
        <el-alert
          type="info"
          :closable="false"
          class="mb-2"
          title="编辑引导：建议使用 QGIS 编辑后放回图层同名目录"
          description="图层属性 → 样式 → 导出 SLD 1.1（.sld）；将文件放在对应图层同名目录（vector/ 或 postgis/ 下），map-api 按请求读取、purge 即生效，无需 reload。本页不提供规则级 Web 设计器。"
        />
        <el-descriptions :column="2" border size="small">
          <el-descriptions-item label="样式目录">{{ styleDir || '—' }}</el-descriptions-item>
          <el-descriptions-item label="样式数量">{{ styleList.length }}</el-descriptions-item>
        </el-descriptions>
      </el-card>
    </div>

    <el-card shadow="hover">
      <template #header>
        <el-row :gutter="10" class="mb8">
          <el-col :span="1.5">
            <el-button icon="Refresh" @click="getList">刷新</el-button>
          </el-col>
          <el-col :span="1.5">
            <el-button v-hasPermi="['map:style:upload']" type="primary" icon="Upload" @click="openUpload">上传/替换样式</el-button>
          </el-col>
          <right-toolbar v-model:show-search="showSearch" @query-table="getList" />
        </el-row>
      </template>

      <el-table v-loading="loading" border :data="pagedStyles" style="width: 100%" max-height="560">
        <el-table-column label="样式名" align="center" prop="name" min-width="180" :show-overflow-tooltip="true" />
        <el-table-column label="大小" align="center" width="100">
          <template #default="scope">{{ (scope.row.size / 1024).toFixed(1) }} KB</template>
        </el-table-column>
        <el-table-column label="修改时间" align="center" width="170">
          <template #default="scope">
            <span class="text-xs">{{ proxy?.parseTime(scope.row.modified) }}</span>
          </template>
        </el-table-column>
        <el-table-column label="样式标题" align="center" prop="style_name" min-width="140" :show-overflow-tooltip="true">
          <template #default="scope">
            <span v-if="scope.row.style_name">{{ scope.row.style_name }}</span>
            <span v-else class="text-gray-400">—</span>
          </template>
        </el-table-column>
        <el-table-column label="关联图层" align="center" prop="layer_name" min-width="140" :show-overflow-tooltip="true">
          <template #default="scope">
            <span v-if="scope.row.layer_name">{{ scope.row.layer_name }}</span>
            <span v-else class="text-gray-400">—</span>
          </template>
        </el-table-column>
        <el-table-column label="规则数" align="center" width="90" prop="rules" />
        <el-table-column label="解析" align="center" width="90">
          <template #default="scope">
            <el-tag :type="scope.row.parse_ok ? 'success' : 'danger'" effect="dark" size="small">
              {{ scope.row.parse_ok ? '正常' : '异常' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="被引用" align="center" min-width="160">
          <template #default="scope">
            <el-tag v-for="u in scope.row.used_by || []" :key="u" class="mr-1 mb-1" type="info" effect="plain" size="small">{{ u }}</el-tag>
            <span v-if="!scope.row.used_by || !scope.row.used_by.length" class="text-gray-400">未引用</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" align="center" width="200" class-name="small-padding fixed-width">
          <template #default="scope">
            <el-button v-hasPermi="['map:style:list']" link type="primary" @click="openContent(scope.row)">查看</el-button>
            <el-button v-hasPermi="['map:style:list']" link type="success" @click="openLegend(scope.row)">图例</el-button>
            <el-button v-hasPermi="['map:style:remove']" link type="danger" @click="handleDelete(scope.row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
      <el-empty v-if="!loading && styleList.length === 0" description="样式库为空" />
      <div class="mt-3 flex justify-end">
        <el-pagination
          v-model:current-page="page"
          v-model:page-size="pageSize"
          :page-sizes="[10, 20, 50, 100]"
          :total="styleList.length"
          layout="total, sizes, prev, pager, next, jumper"
          background
        />
      </div>
    </el-card>

    <!-- 样式原文（只读） -->
    <el-dialog v-model="contentVisible" :title="`样式原文 — ${contentRow?.name || ''}`" width="760px" append-to-body>
      <el-input
        v-model="contentText"
        type="textarea"
        :rows="20"
        readonly
        style="font-family: monospace; font-size: 12px"
      />
    </el-dialog>

    <!-- 图例预览 -->
    <el-dialog v-model="legendVisible" :title="`图例 — ${legendRow?.name || ''}`" width="420px" append-to-body @closed="closeLegend">
      <div class="flex justify-center">
        <img v-if="legendUrl" :src="legendUrl" alt="legend" style="max-width: 100%; border: 1px solid var(--el-border-color); border-radius: 4px" />
        <el-empty v-else description="图例生成失败" :image-size="60" />
      </div>
    </el-dialog>

    <!-- 上传/替换 -->
    <el-dialog v-model="uploadVisible" title="上传 / 替换 SLD 样式" width="720px" append-to-body @closed="resetUpload">
      <el-form label-width="90px">
        <el-form-item label="样式名" required>
          <el-input v-model="uploadName" placeholder="不含扩展名，如 buildings" />
        </el-form-item>
        <el-form-item label="SLD 内容" required>
          <el-input v-model="uploadContent" type="textarea" :rows="16" placeholder="粘贴 SLD 1.1 / SE XML 内容" style="font-family: monospace; font-size: 12px" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="uploadVisible = false">取消</el-button>
        <el-button type="primary" :loading="uploading" @click="handleUpload">提交（服务端解析校验后落盘）</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts" name="MapStyle">
import { listStyles, styleContent, styleLegend, uploadStyle, deleteStyle } from '@/api/map/service';
import { StyleVO, StyleListResp } from '@/api/map/service/types';
import { to } from 'await-to-js';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const loading = ref(true);
const showSearch = ref(true);
const styleList = ref<StyleVO[]>([]);
const styleDir = ref('');

const page = ref(1);
const pageSize = ref(10);
const pagedStyles = computed(() => {
  const start = (page.value - 1) * pageSize.value;
  return styleList.value.slice(start, start + pageSize.value);
});

// 原文查看
const contentVisible = ref(false);
const contentRow = ref<StyleVO | null>(null);
const contentText = ref('');

// 图例
const legendVisible = ref(false);
const legendRow = ref<StyleVO | null>(null);
const legendUrl = ref('');

// 上传
const uploadVisible = ref(false);
const uploading = ref(false);
const uploadName = ref('');
const uploadContent = ref('');

const getList = async () => {
  loading.value = true;
  const [err, res] = await to(listStyles());
  loading.value = false;
  if (err) {
    proxy?.$modal.msgError('样式库获取失败：' + (err as Error).message);
    return;
  }
  const data = (res?.data ?? {}) as StyleListResp;
  styleList.value = data.styles || [];
  styleDir.value = data.dir || '';
};

const openContent = async (row: StyleVO) => {
  contentRow.value = row;
  contentVisible.value = true;
  contentText.value = '加载中…';
  const [err, res] = await to(styleContent(row.name));
  if (err) {
    contentText.value = '读取失败：' + (err as Error).message;
    return;
  }
  contentText.value = (res?.data as any)?.content || '';
};

const openLegend = async (row: StyleVO) => {
  legendRow.value = row;
  legendVisible.value = true;
  // 先释放旧 URL
  if (legendUrl.value) {
    URL.revokeObjectURL(legendUrl.value);
    legendUrl.value = '';
  }
  const [err, res] = await to(styleLegend(row.name, { width: 256, height: 256 }));
  if (err) {
    proxy?.$modal.msgError('图例生成失败：' + (err as Error).message);
    return;
  }
  const blob = res?.data as Blob;
  if (blob && blob.size > 0) {
    legendUrl.value = URL.createObjectURL(blob);
  }
};

const closeLegend = () => {
  if (legendUrl.value) {
    URL.revokeObjectURL(legendUrl.value);
    legendUrl.value = '';
  }
};

const openUpload = () => {
  uploadVisible.value = true;
};

const resetUpload = () => {
  uploadName.value = '';
  uploadContent.value = '';
  uploading.value = false;
};

const handleUpload = async () => {
  if (!uploadName.value.trim()) {
    proxy?.$modal.msgWarning('请填写样式名');
    return;
  }
  if (!uploadContent.value.trim()) {
    proxy?.$modal.msgWarning('请粘贴 SLD 内容');
    return;
  }
  uploading.value = true;
  const [err] = await to(uploadStyle(uploadName.value.trim(), uploadContent.value));
  uploading.value = false;
  if (err) {
    proxy?.$modal.msgError('上传失败：' + (err as Error).message);
    return;
  }
  proxy?.$modal.msgSuccess('上传成功（服务端已解析校验）');
  uploadVisible.value = false;
  await getList();
};

const handleDelete = async (row: StyleVO) => {
  const [err] = await to(proxy?.$modal.confirm(`确认删除样式「${row.name}」？删除后对应图层将回退到默认样式。`) as any);
  if (err) return;
  const [delErr] = await to(deleteStyle(row.name));
  if (delErr) {
    proxy?.$modal.msgError('删除失败：' + (delErr as Error).message);
    return;
  }
  proxy?.$modal.msgSuccess('删除成功');
  await getList();
};

onMounted(() => {
  getList();
});
</script>
