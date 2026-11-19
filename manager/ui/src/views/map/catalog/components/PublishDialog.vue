<template>
  <!-- 发布 / 修改发布信息 向导：服务协议的可选项一律取自引擎能力矩阵 -->
  <el-dialog v-model="visible" :title="isEdit ? $t('map.auto.editinfo') : $t('map.auto.text31')" width="640px" append-to-body>
    <el-form ref="formRef" :model="form" :rules="rules" label-width="140px">
      <el-alert
        v-if="form.crsAlert"
        :title="form.crsAlertTitle"
        :description="form.crsAlertDesc"
        type="warning"
        show-icon
        :closable="false"
        class="mb-[10px]"
      />
      <el-form-item :label="$t('map.auto.dataLayer')">
        <span class="text-xs">{{ form.resourceName }}（{{ form.dataType }}）</span>
      </el-form-item>
      <el-form-item :label="$t('map.auto.serviceName')" prop="layerName">
        <el-input v-model="form.layerName" :placeholder="$t('map.auto.one2')" />
        <span class="text-xs text-gray-400">
          {{ $t('map.auto.byone') }}
        </span>
      </el-form-item>
      <el-form-item :label="$t('map.auto.title')" prop="title">
        <el-input v-model="form.title" :placeholder="$t('map.auto.text32')" />
      </el-form-item>
      <el-form-item v-if="!form.is3dtiles" :label="$t('map.auto.crs')" prop="srcCrs">
        <el-select
          v-model="form.srcCrs"
          filterable
          allow-create
          default-first-option
          clearable
          :disabled="!!nativeGrid"
          :placeholder="$t('map.auto.crsEPSGCode')"
          style="width: 100%"
        >
          <el-option-group v-for="g in crsGroups" :key="g.label" :label="g.label">
            <el-option v-for="o in g.options" :key="o.value" :label="`${o.label}（${o.value}）`" :value="o.value" />
          </el-option-group>
        </el-select>
        <span class="text-xs text-gray-400">
          {{
            nativeGrid
              ? $t('map.auto.cacheConfXmlOfHour')
              : $t('map.auto.oneWGS84OfCGCS2000Of')
          }}
        </span>
      </el-form-item>
      <el-form-item v-if="!form.is3dtiles && !nativeGrid" :label="$t('map.auto.text33')">
        <div class="w-full">
          <el-radio-group v-model="form.gridMode">
            <el-radio v-if="standardAllowed" value="ogc">OGC {{ $t('map.auto.text34') }}LOD（{{ ogcGridId }}）</el-radio>
            <el-radio v-if="crsCode === 4490" value="tdt">{{ $t('map.auto.lod4490118') }}</el-radio>
            <el-radio v-if="!standardAllowed" value="">{{ $t('map.auto.webmercatorquad') }}</el-radio>
            <el-radio value="custom">{{ $t('map.auto.lodConfXml') }}</el-radio>
          </el-radio-group>
          <div class="text-xs text-gray-400">
            <template v-if="!standardAllowed">
              {{ $t('map.auto.projectedCrsLodHint') }}
            </template>
            <template v-else-if="crsCode === 4490">
              4490 {{ $t('map.auto.text167') }}OGC {{ $t('map.auto.text34') }}LOD、{{ $t('map.auto.text168') }}LOD {{ $t('map.auto.text35') }}LOD。
            </template>
            <template v-else>{{ $t('map.auto.crs2') }}OGC {{ $t('map.auto.text34') }}LOD（{{ ogcGridId }}）{{ $t('map.auto.text35') }}LOD。</template>
          </div>

          <div v-if="form.gridMode === 'custom'" class="mt-2">
            <div class="flex items-center gap-2">
              <el-input v-model="xmlPath" :placeholder="$t('map.auto.confXml')" />
              <el-button @click="tileDlgVisible = true">{{ $t('map.auto.xml') }}</el-button>
              <el-button :loading="parsingXml" @click="parseXml">{{ $t('map.auto.parse') }}</el-button>
            </div>
            <el-alert v-if="xmlError" :title="xmlError" type="error" :closable="false" show-icon class="mt-1" />
            <div v-if="customGrid" class="text-xs mt-1">
              {{ customGrid.crs }} · {{ $t('map.auto.text169') }}{{ customGrid.tile_px || customGrid.tile_cols || '—' }} px · {{ $t('map.auto.text170') }}
              [{{ (customGrid.origin || []).join(', ') }}] · {{ $t('map.auto.text171') }}
              {{ customGrid.min_zoom ?? '—' }}–{{ customGrid.max_zoom ?? '—' }}（{{ customGrid.levels.length }} {{ $t('map.auto.text172') }}
              <el-table :data="customGrid.levels" size="small" border max-height="140" class="mt-1">
                <el-table-column prop="level" :label="$t('map.auto.level')" width="70" align="center" />
                <el-table-column prop="scale" :label="$t('map.auto.scale')" align="right" />
                <el-table-column prop="resolution" :label="$t('map.auto.resolutionMapUnitsPixel')" align="right" />
              </el-table>
            </div>
          </div>
        </div>
      </el-form-item>
      <el-form-item v-if="nativeGrid" :label="$t('map.auto.text36')">
        <div class="w-full">
          <div class="text-xs">
            {{ nativeGrid.crs || $t('map.auto.detectCRS') }} · {{ $t('map.auto.text169') }}{{ tileText }} · DPI {{ nativeGrid.dpi ?? '—' }} ·
            {{ $t('map.auto.text171') }}{{ nativeGrid.min_zoom ?? '—' }}–{{ nativeGrid.max_zoom ?? '—' }}（{{ nativeGrid.levels.length }} {{ $t('map.auto.text172') }}
          </div>
          <div class="text-xs text-gray-400">
            {{ $t('map.auto.text170') }}[{{ (nativeGrid.origin || []).join(', ') }}] · {{ $t('map.auto.cache5') }}Conf.xml，{{ $t('map.auto.hourpublishedServices') }}
          </div>
          <el-table v-if="nativeGrid.levels.length" :data="nativeGrid.levels" size="small" border max-height="150" class="mt-1">
            <el-table-column prop="level" :label="$t('map.auto.level')" width="70" align="center" />
            <el-table-column prop="scale" :label="$t('map.auto.scale')" align="right" />
            <el-table-column prop="resolution" :label="$t('map.auto.resolutionMapUnitsPixel')" align="right" />
          </el-table>
        </div>
      </el-form-item>
      <el-form-item v-if="!form.is3dtiles" :label="$t('map.auto.four')" prop="bbox">
        <div class="flex items-center gap-1">
          <el-input-number v-model="box.minx" :controls="false" :precision="6" placeholder="minx" style="width: 120px" />
          <el-input-number v-model="box.miny" :controls="false" :precision="6" placeholder="miny" style="width: 120px" />
          <el-input-number v-model="box.maxx" :controls="false" :precision="6" placeholder="maxx" style="width: 120px" />
          <el-input-number v-model="box.maxy" :controls="false" :precision="6" placeholder="maxy" style="width: 120px" />
        </div>
        <span v-if="form.bboxRequired" class="text-xs text-warning">
          {{ $t('map.auto.fourPleaseFillInNoLayer') }}
        </span>
        <span v-else class="text-xs text-gray-400">{{ $t('map.auto.byMinLtMax') }}</span>
      </el-form-item>
      <el-form-item :label="$t('map.auto.protocol3')" prop="serviceTypes">
        <el-checkbox-group v-model="form.serviceTypes">
          <el-checkbox v-for="t in cap?.service_types || []" :key="t" :value="t" :label="t">
            {{ protoLabel(t) }}
          </el-checkbox>
        </el-checkbox-group>
        <span v-if="cap?.note" class="text-xs text-gray-400">{{ cap.note }}</span>
      </el-form-item>
      <el-form-item :label="$t('map.auto.remark')" prop="remark">
        <el-input v-model="form.remark" type="textarea" :rows="2" />
      </el-form-item>
    </el-form>

    <!-- 选择服务器上的 ArcGIS Conf.xml（append-to-body，可与本对话框叠加） -->
    <TileConfigDialog v-model="tileDlgVisible" :workspace-id="workspaceId" @confirm="onTileParsed" />

    <template #footer>
      <el-button @click="visible = false">{{ $t('map.auto.cancel') }}</el-button>
      <el-button type="primary" :loading="saving" @click="save">{{ isEdit ? $t('map.auto.save3') : $t('map.auto.text37') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
const { t } = useI18n();
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
    xmlError.value = `${t('map.auto.crs3')}${r.crs}${t('map.auto.crs4')}${form.srcCrs}${t('map.auto.oneCRSXML')}`;
    return;
  }
  xmlError.value = '';
  xmlPath.value = r.source || xmlPath.value;
  customGrid.value = toGridSnapshot(r);
};

const parseXml = async () => {
  const path = xmlPath.value.trim();
  if (!path) {
    xmlError.value = t('map.auto.selectOrEnterTheConfXmlPath');
    return;
  }
  parsingXml.value = true;
  xmlError.value = '';
  try {
    const res = await parseTileGrid({ workspaceId: workspaceId.value, path });
    applyParsedGrid(res.data);
  } catch (e: any) {
    customGrid.value = null;
    xmlError.value = e?.message || t('map.auto.parseFailed');
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
      xmlError.value = `${t('map.auto.crs3')}${customGrid.value.crs}${t('map.auto.crs4')}${v}${t('map.auto.oneXML')}`;
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
        const label = path.join(' / ') || t('map.auto.crs5');
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
    { required: true, message: t('map.auto.serviceNameisRequired'), trigger: 'blur' },
    { pattern: /^[A-Za-z0-9_-]{1,64}$/, message: t('map.auto.text38'), trigger: 'blur' }
  ],
  srcCrs: [
    {
      validator: (_r: any, v: string, cb: (e?: Error) => void) => {
        const val = (v || '').trim();
        if (!val) {
          if (form.crsRequired) cb(new Error(t('map.auto.crsCRS')));
          else cb();
          return;
        }
        if (!/^EPSG:\d+$/i.test(val)) cb(new Error(t('map.auto.epsg4326')));
        else cb();
      },
      trigger: 'change'
    }
  ],
  serviceTypes: [
    {
      validator: (_r: any, _v: string[], cb: (e?: Error) => void) => {
        if (!form.serviceTypes.length) cb(new Error(t('map.auto.oneprotocol')));
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
  if (filled < 4) return { err: t('map.auto.fourfourAllBy'), value: null };
  const [minx, miny, maxx, maxy] = v as number[];
  if (!(minx < maxx && miny < maxy)) return { err: t('map.auto.fourMinxMaxxMinyMaxy'), value: null };
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
    crsAlertTitle: !row.srcCrs ? t('map.auto.crs6') : t('map.auto.crs7'),
    crsAlertDesc: !row.srcCrs
      ? t('map.auto.crsGISCRSImportCRSOneWGS84OfCGCS2000Of')
      : `${t('map.auto.text39')}${row.srcCrs}${t('map.auto.crsofCRSCRSHour')}`
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
      proxy?.$modal.msgError(t('map.auto.lodParseXML'));
      return;
    }
    if (epsgOf(customGrid.value.crs) !== crsCode.value) {
      proxy?.$modal.msgError(t('map.auto.crscrsoneCRSXML'));
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
    proxy?.$modal.msgSuccess(isEdit.value ? t('map.auto.updated') : t('map.auto.text40'));
    visible.value = false;
    emit('saved');
  } finally {
    saving.value = false;
  }
};

defineExpose({ open, openEdit });
</script>
