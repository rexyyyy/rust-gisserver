<template>
  <!-- 发布 / 修改发布信息 向导：服务协议的可选项一律取自引擎能力矩阵 -->
  <el-dialog v-model="visible" :title="isEdit ? '修改发布信息' : '发布为地图服务'" width="640px" append-to-body>
    <el-form ref="formRef" :model="form" :rules="rules" label-width="110px">
      <el-alert
        v-if="form.crsAlert"
        :title="form.crsAlertTitle"
        :description="form.crsAlertDesc"
        type="warning"
        show-icon
        :closable="false"
        class="mb-[10px]"
      />
      <el-form-item label="数据图层">
        <span class="text-xs">{{ form.resourceName }}（{{ form.dataType }}）</span>
      </el-form-item>
      <el-form-item label="服务名" prop="layerName">
        <el-input v-model="form.layerName" placeholder="全局唯一，字母/数字/-/_，不能含点号" />
        <span class="text-xs text-gray-400">
          引擎按扁平唯一名落配置文件；点号会被当成扩展名截断，因此入口就禁止。
        </span>
      </el-form-item>
      <el-form-item label="标题" prop="title">
        <el-input v-model="form.title" placeholder="留空则用资源名" />
      </el-form-item>
      <el-form-item v-if="!form.is3dtiles" label="源坐标系" prop="srcCrs">
        <el-select
          v-model="form.srcCrs"
          filterable
          allow-create
          default-first-option
          clearable
          :disabled="!!nativeGrid"
          placeholder="从坐标系树选择，或手动输入 EPSG 编码"
          style="width: 100%"
        >
          <el-option-group v-for="g in crsGroups" :key="g.label" :label="g.label">
            <el-option v-for="o in g.options" :key="o.value" :label="`${o.label}（${o.value}）`" :value="o.value" />
          </el-option-group>
        </el-select>
        <span class="text-xs text-gray-400">
          {{
            nativeGrid
              ? '由缓存 Conf.xml 的原生网格决定，发布时以该定义为准，不接受表单覆盖。'
              : '须与数据实际坐标一致（WGS84 系的地理坐标/投影、CGCS2000 系的地理坐标与高斯投影）；选错会被服务端拦截。'
          }}
        </span>
      </el-form-item>
      <el-form-item v-if="!form.is3dtiles && !nativeGrid" label="瓦片网格">
        <div class="w-full">
          <el-radio-group v-model="form.gridMode">
            <el-radio v-if="standardAllowed" value="ogc">OGC 标准 LOD（{{ ogcGridId }}）</el-radio>
            <el-radio v-if="crsCode === 4490" value="tdt">天地图 LOD（4490，1–18 级）</el-radio>
            <el-radio v-if="!standardAllowed" value="">WebMercatorQuad（服务端重投影）</el-radio>
            <el-radio value="custom">自定义 LOD（选择 Conf.xml）</el-radio>
          </el-radio-group>
          <div class="text-xs text-gray-400">
            <template v-if="!standardAllowed">
              当前为投影坐标系，只能选择「自定义 LOD」；如需标准 LOD 请先转换为 EPSG:4326/4490/3857。
            </template>
            <template v-else-if="crsCode === 4490">
              4490 可选 OGC 标准 LOD、天地图 LOD 或自定义 LOD。
            </template>
            <template v-else>该坐标系可选 OGC 标准 LOD（{{ ogcGridId }}）或自定义 LOD。</template>
          </div>

          <div v-if="form.gridMode === 'custom'" class="mt-2">
            <div class="flex items-center gap-2">
              <el-input v-model="xmlPath" placeholder="Conf.xml 绝对路径，或点右侧选择" />
              <el-button @click="tileDlgVisible = true">选择 XML</el-button>
              <el-button :loading="parsingXml" @click="parseXml">解析</el-button>
            </div>
            <el-alert v-if="xmlError" :title="xmlError" type="error" :closable="false" show-icon class="mt-1" />
            <div v-if="customGrid" class="text-xs mt-1">
              {{ customGrid.crs }} · 瓦片 {{ customGrid.tile_px || customGrid.tile_cols || '—' }} px · 原点
              [{{ (customGrid.origin || []).join(', ') }}] · 层级
              {{ customGrid.min_zoom ?? '—' }}–{{ customGrid.max_zoom ?? '—' }}（{{ customGrid.levels.length }} 级）
              <el-table :data="customGrid.levels" size="small" border max-height="140" class="mt-1">
                <el-table-column prop="level" label="级别" width="70" align="center" />
                <el-table-column prop="scale" label="比例尺" align="right" />
                <el-table-column prop="resolution" label="分辨率（地图单位/像素）" align="right" />
              </el-table>
            </div>
          </div>
        </div>
      </el-form-item>
      <el-form-item v-if="nativeGrid" label="原生网格">
        <div class="w-full">
          <div class="text-xs">
            {{ nativeGrid.crs || '（未识别 CRS）' }} · 瓦片 {{ tileText }} · DPI {{ nativeGrid.dpi ?? '—' }} ·
            层级 {{ nativeGrid.min_zoom ?? '—' }}–{{ nativeGrid.max_zoom ?? '—' }}（{{ nativeGrid.levels.length }} 级）
          </div>
          <div class="text-xs text-gray-400">
            原点 [{{ (nativeGrid.origin || []).join(', ') }}] · 来自缓存 Conf.xml，发布时固化到已发布服务账本
          </div>
          <el-table v-if="nativeGrid.levels.length" :data="nativeGrid.levels" size="small" border max-height="150" class="mt-1">
            <el-table-column prop="level" label="级别" width="70" align="center" />
            <el-table-column prop="scale" label="比例尺" align="right" />
            <el-table-column prop="resolution" label="分辨率（地图单位/像素）" align="right" />
          </el-table>
        </div>
      </el-form-item>
      <el-form-item v-if="!form.is3dtiles" label="四至" prop="bbox">
        <div class="flex items-center gap-1">
          <el-input-number v-model="box.minx" :controls="false" :precision="6" placeholder="minx" style="width: 120px" />
          <el-input-number v-model="box.miny" :controls="false" :precision="6" placeholder="miny" style="width: 120px" />
          <el-input-number v-model="box.maxx" :controls="false" :precision="6" placeholder="maxx" style="width: 120px" />
          <el-input-number v-model="box.maxy" :controls="false" :precision="6" placeholder="maxy" style="width: 120px" />
        </div>
        <span v-if="form.bboxRequired" class="text-xs text-warning">
          探测不到四至，请填写，否则前端地图定位不到该图层。
        </span>
        <span v-else class="text-xs text-gray-400">可留空由引擎按源数据计算；填写需满足 min &lt; max。</span>
      </el-form-item>
      <el-form-item label="服务协议" prop="serviceTypes">
        <el-checkbox-group v-model="form.serviceTypes">
          <el-checkbox v-for="t in cap?.service_types || []" :key="t" :value="t" :label="t">
            {{ protoLabel(t) }}
          </el-checkbox>
        </el-checkbox-group>
        <span v-if="cap?.note" class="text-xs text-gray-400">{{ cap.note }}</span>
      </el-form-item>
      <el-form-item label="备注" prop="remark">
        <el-input v-model="form.remark" type="textarea" :rows="2" />
      </el-form-item>
    </el-form>

    <!-- 选择服务器上的 ArcGIS Conf.xml（append-to-body，可与本对话框叠加） -->
    <TileConfigDialog v-model="tileDlgVisible" :workspace-id="workspaceId" @confirm="onTileParsed" />

    <template #footer>
      <el-button @click="visible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="save">{{ isEdit ? '保存并重载' : '发布' }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { getCrsTree, parseTileGrid, publishService, updatePublished } from '@/api/map/catalog';
import {
  CatalogCaps,
  CrsTreeNode,
  NativeGrid,
  PublishBody,
  PublishedVO,
  ResourceVO,
  TileGridParseResp
} from '@/api/map/catalog/types';
import { capOfProto, protoLabel } from '../composables/useCatalog';
import TileConfigDialog from './TileConfigDialog.vue';

const props = defineProps<{ caps: CatalogCaps | null }>();
const emit = defineEmits<{ (e: 'saved'): void }>();

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const visible = ref(false);
const isEdit = ref(false);
const saving = ref(false);
const formRef = ref<ElFormInstance>();
const form = reactive({
  resourceId: 0,
  publishedId: 0,
  resourceName: '',
  dataType: '',
  layerName: '',
  title: '',
  srcCrs: 'EPSG:4326',
  remark: '',
  serviceTypes: [] as string[],
  bboxRequired: false,
  is3dtiles: false,
  /** 瓦片网格模式：ogc | tdt | custom | native | ''（仅投影坐标系可留空） */
  gridMode: '' as string,
  crsRequired: false,
  crsAlert: false,
  crsAlertTitle: '',
  crsAlertDesc: ''
});
const box = reactive({
  minx: undefined as number | undefined,
  miny: undefined as number | undefined,
  maxx: undefined as number | undefined,
  maxy: undefined as number | undefined
});

// 勾选项与提示都跟着这一行能力矩阵走：矩阵说不支持的协议，这里就不该出现。
const cap = computed(() => capOfProto(props.caps, form.dataType));

/** ArcGIS 缓存的原生网格：发现时来自 Conf.xml，修改时来自已发布账本快照 */
const nativeGrid = ref<NativeGrid | null>(null);
const tileText = computed(() =>
  nativeGrid.value?.tile_cols && nativeGrid.value?.tile_rows
    ? `${nativeGrid.value.tile_cols}×${nativeGrid.value.tile_rows}`
    : '—'
);

// ---------- 瓦片网格 / LOD 分级 ----------
const workspaceId = ref(0);
const tileDlgVisible = ref(false);
const xmlPath = ref('');
const xmlError = ref('');
const parsingXml = ref(false);
/** 自定义 LOD（Conf.xml 解析结果，按账本快照形状保存） */
const customGrid = ref<NativeGrid | null>(null);

const epsgOf = (crs?: string | null): number => {
  const m = (crs || '').match(/(\d{3,6})/);
  return m ? Number(m[1]) : 0;
};
const crsCode = computed(() => epsgOf(form.srcCrs));
/** 4326/3857/4490 之外视为投影坐标系：只能自定义 LOD */
const standardAllowed = computed(() => [4326, 3857, 4490].includes(crsCode.value));
const ogcGridId = computed(() =>
  crsCode.value === 3857 ? 'WebMercatorQuad' : crsCode.value === 4326 ? 'WorldCRS84Quad' : 'CGCS2000Quad'
);

/** 把引擎解析结果转成账本快照（含引擎重建所需的 grid_def） */
const onTileParsed = (r: TileGridParseResp) => applyParsedGrid(r);

const toGridSnapshot = (r: TileGridParseResp): NativeGrid => {
  const levels = r.levels || [];
  return {
    crs: r.crs,
    tile_cols: r.tile_cols ?? r.tile_px ?? null,
    tile_rows: r.tile_rows ?? r.tile_px ?? null,
    dpi: r.dpi ?? null,
    origin: r.origin || null,
    min_zoom: r.min_zoom ?? null,
    max_zoom: r.max_zoom ?? null,
    levels: r.lods?.length
      ? r.lods
      : levels.map((level, i) => ({
          level,
          scale: r.scales?.[i] ?? 0,
          resolution: r.resolutions?.[i] ?? 0
        })),
    mode: 'custom',
    id: `g_${form.layerName}_xml`,
    source: r.source,
    extent: r.extent || null,
    tile_px: r.tile_px ?? null,
    resolutions: r.resolutions || null,
    scales: r.scales || null,
    grid_def: {
      crs: r.crs,
      origin: r.origin,
      extent: r.extent,
      tile_px: r.tile_px || 256,
      resolutions: r.resolutions || [],
      levels,
      scales: r.scales || []
    }
  };
};

const applyParsedGrid = (r: TileGridParseResp) => {
  if (epsgOf(r.crs) !== crsCode.value) {
    customGrid.value = null;
    xmlError.value = `瓦片配置坐标系 ${r.crs} 与源坐标系 ${form.srcCrs} 不一致，请先转换坐标系后再选择该 XML`;
    return;
  }
  xmlError.value = '';
  xmlPath.value = r.source || xmlPath.value;
  customGrid.value = toGridSnapshot(r);
};

const parseXml = async () => {
  const path = xmlPath.value.trim();
  if (!path) {
    xmlError.value = '请选择或填写 Conf.xml 路径';
    return;
  }
  parsingXml.value = true;
  xmlError.value = '';
  try {
    const res = await parseTileGrid({ workspaceId: workspaceId.value, path });
    applyParsedGrid(res.data);
  } catch (e: any) {
    customGrid.value = null;
    xmlError.value = e?.message || '解析失败';
  } finally {
    parsingXml.value = false;
  }
};

// 用户切换坐标系后，保证网格模式与坐标系匹配（投影坐标不能保留 OGC/天地图）
watch(
  () => form.srcCrs,
  (v) => {
    if (nativeGrid.value || form.is3dtiles) return;
    const code = epsgOf(v);
    const std = [4326, 3857, 4490].includes(code);
    if (form.gridMode === 'ogc' || form.gridMode === 'tdt') {
      if (!std) form.gridMode = '';
      else if (form.gridMode === 'tdt' && code !== 4490) form.gridMode = 'ogc';
    } else if (!form.gridMode && std) {
      form.gridMode = 'ogc';
    }
    if (customGrid.value && epsgOf(customGrid.value.crs) !== code) {
      xmlError.value = `瓦片配置坐标系 ${customGrid.value.crs} 与源坐标系 ${v} 不一致，请重新选择 XML`;
      customGrid.value = null;
    }
  }
);

/** 从资源发现结果 meta 中提取原生网格（仅 ArcGIS 缓存带 lods/crs） */
const gridFromMeta = (meta: Record<string, unknown> | undefined | null): NativeGrid | null => {
  if (!meta) return null;
  const levels = Array.isArray(meta.lods) ? (meta.lods as NativeGrid['levels']) : [];
  if (!meta.crs && !levels.length) return null;
  return {
    crs: String(meta.crs || ''),
    tile_cols: (meta.tile_cols as number) ?? null,
    tile_rows: (meta.tile_rows as number) ?? null,
    dpi: (meta.dpi as number) ?? null,
    origin: Array.isArray(meta.origin) ? (meta.origin as number[]) : null,
    min_zoom: (meta.min_zoom as number) ?? null,
    max_zoom: (meta.max_zoom as number) ?? null,
    levels
  };
};

// 坐标系树（WGS84 系 / CGCS2000 系），供「源坐标系」下拉选择
const crsTree = ref<CrsTreeNode[]>([]);
const crsGroups = computed<{ label: string; options: { label: string; value: string }[] }[]>(() => {
  const groups: { label: string; options: { label: string; value: string }[] }[] = [];
  const walk = (nodes: CrsTreeNode[], path: string[]) => {
    for (const n of nodes) {
      if (n.code) {
        const label = path.join(' / ') || '其他坐标系';
        let g = groups.find((x) => x.label === label);
        if (!g) {
          g = { label, options: [] };
          groups.push(g);
        }
        g.options.push({ label: n.name, value: n.code });
      }
      if (n.children?.length) walk(n.children, [...path, n.name]);
    }
  };
  walk(crsTree.value, []);
  return groups;
});
const loadCrsTree = async () => {
  try {
    const res = await getCrsTree();
    crsTree.value = res.data?.tree || [];
  } catch {
    crsTree.value = [];
  }
};
onMounted(loadCrsTree);

const rules = {
  layerName: [
    { required: true, message: '服务名不能为空', trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,64}$/, message: '仅字母、数字、下划线与短横线，不能含点号', trigger: 'blur' }
  ],
  srcCrs: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        const val = (v || '').trim();
        if (!val) {
          if (form.crsRequired) cb(new Error('该数据坐标系缺失或可疑，请先选择正确坐标系'));
          else cb();
          return;
        }
        if (!/^EPSG:\d+$/i.test(val)) cb(new Error('形如 EPSG:4326'));
        else cb();
      },
      trigger: 'change'
    }
  ],
  serviceTypes: [
    {
      validator: (_r: any, _v: string[], cb: (e?: Error) => void) => {
        if (!form.serviceTypes.length) cb(new Error('至少勾选一种服务协议'));
        else cb();
      },
      trigger: 'change'
    }
  ]
};

const resetBox = (bbox: number[] | null) => {
  box.minx = bbox?.[0];
  box.miny = bbox?.[1];
  box.maxx = bbox?.[2];
  box.maxy = bbox?.[3];
};

const collectBox = (): { err: string; value: number[] | null } => {
  const v = [box.minx, box.miny, box.maxx, box.maxy];
  const filled = v.filter((x) => x !== undefined && x !== null).length;
  if (filled === 0) return { err: '', value: null };
  if (filled < 4) return { err: '四至需四项齐全，或全部留空由引擎按源数据计算', value: null };
  const [minx, miny, maxx, maxy] = v as number[];
  if (!(minx < maxx && miny < maxy)) return { err: '四至无效：需满足 minx < maxx 且 miny < maxy', value: null };
  return { err: '', value: [minx, miny, maxx, maxy] };
};

/** 从「数据图层」行进入：默认勾选该数据类型的推荐协议 */
const open = (row: ResourceVO, wsId = 0) => {
  const c = capOfProto(props.caps, row.dataType);
  const is3d = c?.mapserver_kind === '3dtiles';
  // ArcGIS 缓存自带原生网格：CRS/层级以 Conf.xml 为准，不再提示坐标系可疑
  const grid = gridFromMeta(row.meta);
  nativeGrid.value = grid;
  workspaceId.value = wsId;
  customGrid.value = null;
  xmlPath.value = '';
  xmlError.value = '';
  const code = epsgOf(row.srcCrs);
  // 坐标系缺失或与坐标值不匹配 → 强制先核实，服务端会拦截发布
  const suspicious = !grid && !!row.crsSuspicious && !is3d;
  isEdit.value = false;
  Object.assign(form, {
    resourceId: row.id,
    publishedId: 0,
    resourceName: row.name,
    dataType: row.dataType,
    layerName: row.name,
    title: '',
    srcCrs: grid?.crs || (suspicious ? row.srcCrs || '' : row.srcCrs || 'EPSG:4326'),
    remark: '',
    serviceTypes: [...(c?.default_service_types || [])],
    bboxRequired: !row.bbox,
    is3dtiles: is3d,
    gridMode: is3d ? '' : grid ? 'native' : [4326, 3857, 4490].includes(code) ? 'ogc' : '',
    crsRequired: suspicious,
    crsAlert: suspicious,
    crsAlertTitle: !row.srcCrs ? '数据未声明坐标系' : '坐标系与坐标值范围不匹配',
    crsAlertDesc: !row.srcCrs
      ? '该数据源未声明坐标系。请先用 GIS 软件（如 QGIS）赋予正确坐标系后重新导入，或在下方坐标系树中选择一个（WGS84 系的地理/投影坐标、CGCS2000 系的地理坐标与高斯投影）。'
      : `该数据声明为 ${row.srcCrs}，但其坐标值范围与该坐标系的合理范围明显不符（疑似局部坐标或坐标系写错）。请核实后在下方选择正确坐标系；无法配准时请勿发布。`
  });
  resetBox(row.bbox);
  visible.value = true;
  nextTick(() => formRef.value?.clearValidate());
};

/** 从「已发布服务」行进入：改名即引擎侧改名（配置文档随之重建并热重载） */
const openEdit = (row: PublishedVO) => {
  isEdit.value = true;
  // 只有 ArcGIS 预切片缓存 / 旧版原生网格快照才锁定为「原生网格」
  const isNative = row.dataType === 'arcgis' || row.grid?.mode === 'native';
  nativeGrid.value = isNative ? row.grid || null : null;
  const g = row.grid;
  customGrid.value = !isNative && g?.mode === 'custom' ? g : null;
  xmlPath.value = g?.mode === 'custom' ? g.source || '' : '';
  xmlError.value = '';
  workspaceId.value = row.workspaceId || 0;
  const code = epsgOf(row.srcCrs);
  const std = [4326, 3857, 4490].includes(code);
  Object.assign(form, {
    resourceId: row.resourceId || 0,
    publishedId: row.id,
    resourceName: row.resourceName || row.layerName,
    dataType: row.dataType || '',
    layerName: row.layerName,
    title: row.title,
    srcCrs: row.srcCrs || 'EPSG:4326',
    remark: row.remark,
    serviceTypes: [...(row.serviceTypes || [])],
    bboxRequired: !row.bbox,
    is3dtiles: row.mapserverKind === '3dtiles',
    gridMode: isNative
      ? 'native'
      : g?.mode === 'ogc' || g?.mode === 'tdt' || g?.mode === 'custom'
        ? g.mode
        : std
          ? 'ogc'
          : '',
    crsRequired: false,
    crsAlert: false,
    crsAlertTitle: '',
    crsAlertDesc: ''
  });
  resetBox(row.bbox);
  visible.value = true;
  nextTick(() => formRef.value?.clearValidate());
};

const save = async () => {
  const valid = await formRef.value?.validate().catch(() => false);
  if (!valid) return;
  let bbox: number[] | null = null;
  if (!form.is3dtiles) {
    const parsed = collectBox();
    if (parsed.err) {
      proxy?.$modal.msgError(parsed.err);
      return;
    }
    bbox = parsed.value;
  }
  // 自定义 LOD 必须已解析且坐标系与源坐标系一致
  if (!form.is3dtiles && form.gridMode === 'custom') {
    if (!customGrid.value) {
      proxy?.$modal.msgError('自定义 LOD 需先选择并解析瓦片配置 XML');
      return;
    }
    if (epsgOf(customGrid.value.crs) !== crsCode.value) {
      proxy?.$modal.msgError('瓦片配置坐标系与源坐标系不一致，请先转换坐标系或重新选择 XML');
      return;
    }
  }
  saving.value = true;
  const body: PublishBody = {
    resourceId: form.resourceId,
    layerName: form.layerName.trim(),
    title: form.title.trim(),
    srcCrs: form.is3dtiles ? '' : form.srcCrs.trim(),
    bbox,
    serviceTypes: form.serviceTypes,
    remark: form.remark.trim(),
    gridMode: form.is3dtiles ? '' : form.gridMode,
    // 仅自定义 LOD 需要携带网格定义；标准/天地图/原生由后端按模式生成快照
    grid: !form.is3dtiles && form.gridMode === 'custom' ? customGrid.value : null
  };
  try {
    if (isEdit.value) await updatePublished(form.publishedId, body);
    else await publishService(body);
    proxy?.$modal.msgSuccess(isEdit.value ? '已更新并重载' : '已发布');
    visible.value = false;
    emit('saved');
  } finally {
    saving.value = false;
  }
};

defineExpose({ open, openEdit });
</script>
