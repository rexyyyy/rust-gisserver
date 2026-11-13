<template>
  <!-- 预览与服务地址：端点 URL 一律由引擎给出，管理端不自行拼接 -->
  <el-dialog v-model="visible" :title="`服务预览：${layerName}`" width="900px" append-to-body>
    <div v-loading="loading">
      <div class="mb-2 flex items-center gap-2">
        <el-tag :type="stateTag(state)" size="small" effect="dark">{{ stateLabel(state) }}</el-tag>
        <el-tag v-if="data.engineName" size="small" effect="plain">{{ data.engineName }}</el-tag>
        <!-- 矢量瓦片预览：把该图层实际加载用的 CRS / 网格 / LOD 常量打成可复制的代码 -->
        <el-button v-if="isCodeable" size="small" type="primary" plain @click="openCode">代码</el-button>
        <el-link v-if="previewUrl" type="primary" underline="never" class="ml-auto" @click="openUrl(previewUrl)">
          新窗口全屏打开
        </el-link>
      </div>
      <el-alert
        v-if="!loading && !data.found"
        class="mb-2"
        type="warning"
        :closable="false"
        show-icon
        :title="data.hint || '引擎目录里没有该服务的配置'"
      />
      <el-alert
        v-else-if="!data.serving"
        class="mb-2"
        type="warning"
        :closable="false"
        show-icon
        :title="`服务当前不在服（${stateLabel(state)}）${data.note ? '：' + data.note : ''}，预览可能无输出，启动后再看。`"
      />
      <iframe
        v-if="previewUrl"
        :src="previewUrl"
        style="width: 100%; height: 380px; border: 1px solid var(--el-border-color); border-radius: 4px"
        allowfullscreen
      ></iframe>
      <el-divider content-position="left">协议端点</el-divider>
      <el-table :data="shownEndpoints" border size="small" max-height="240">
        <el-table-column label="协议 / 用途" width="200" prop="name" />
        <el-table-column label="URL" prop="url" :show-overflow-tooltip="true">
          <template #default="{ row }">
            <span class="text-xs">{{ row.url }}</span>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="130" align="center">
          <template #default="{ row }">
            <el-button link type="primary" size="small" @click="copyUrl(row.url)">复制</el-button>
            <el-button link type="success" size="small" @click="openUrl(row.url)">打开</el-button>
          </template>
        </el-table-column>
      </el-table>
      <div v-if="hiddenCount || showAll" class="mt-1 text-xs" style="color: var(--el-text-color-secondary)">
        另有 {{ hiddenCount }} 个端点引擎按图层能力仍可访问，但发布时未勾选该服务类型。
        <el-link type="primary" underline="never" class="text-xs" @click="showAll = !showAll">
          {{ showAll ? '只看已声明' : '全部显示' }}
        </el-link>
      </div>
    </div>
  </el-dialog>

  <!-- 加载代码：常量（CRS / 网格 / 原点 / LOD 分辨率 / 四至）随图层与发布网格变化 -->
  <el-dialog v-model="codeVisible" :title="`加载代码：${layerName}`" width="820px" append-to-body>
    <div v-loading="codeLoading">
      <pre class="code-pre">{{ codeText }}</pre>
    </div>
    <template #footer>
      <el-button @click="codeVisible = false">关闭</el-button>
      <el-button type="primary" :disabled="codeLoading" @click="copyCode">复制</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { listPublishedEndpoints } from '@/api/map/catalog';
import { PublishedEndpoints, PublishedVO } from '@/api/map/catalog/types';
import { stateLabel, stateTag } from '../composables/useCatalog';

const { proxy } = getCurrentInstance() as ComponentInternalInstance;

const visible = ref(false);
const loading = ref(false);
const layerName = ref('');
const state = ref('');
const data = ref<PublishedEndpoints>({ layerName: '', state: '', found: false, serving: false, endpoints: [] });
// 发布时声明的服务类型：引擎的端点表按图层能力给全（cog 层哪怕只勾了 WMS+XYZ，
// WMTS 也照样能答），这里按声明收口，与列表里的「协议」列保持一致。
const declared = ref<string[]>([]);
const showAll = ref(false);

// 端点名称 → 服务类型。归档信息 / layer.json / 内置预览这类与具体协议无关的
// 条目匹配不到（返回空串），一律保留，避免把有用信息藏掉。
const PROTO_OF: Array<[RegExp, string]> = [
  [/^WMS/, 'wms'],
  [/^图例/, 'wms'],
  [/^WMTS/, 'wmts'],
  [/^XYZ/, 'xyz'],
  [/^MVT|^TileJSON/, 'mvt'],
  [/^WFS/, 'wfs'],
  [/^OGC API/, 'oaf'],
  [/^tileset\.json/, '3dtiles']
];
const protoOf = (name: string) => PROTO_OF.find(([re]) => re.test(name))?.[1] || '';
const shownEndpoints = computed(() => {
  const eps = data.value.endpoints || [];
  if (showAll.value || !declared.value.length) return eps;
  return eps.filter((e) => {
    const p = protoOf(e.name);
    return !p || declared.value.includes(p);
  });
});
const hiddenCount = computed(() => (data.value.endpoints || []).length - shownEndpoints.value.length);

// 引擎在端点里给出「内置预览（OpenLayers / Cesium）」，这里只挑出来，不猜 URL
const previewUrl = computed(() => data.value.endpoints.find((e) => e.name.startsWith('内置预览'))?.url || '');

const open = async (row: PublishedVO) => {
  visible.value = true;
  layerName.value = row.layerName;
  state.value = row.state;
  declared.value = row.serviceTypes || [];
  showAll.value = false;
  loading.value = true;
  data.value = { layerName: row.layerName, state: row.state, found: false, serving: false, endpoints: [] };
  try {
    data.value = (await listPublishedEndpoints(row.id)).data;
    state.value = data.value.state || row.state;
  } finally {
    loading.value = false;
  }
};

const copyUrl = async (url: string) => {
  try {
    await navigator.clipboard.writeText(url);
    proxy?.$modal.msgSuccess('已复制到剪贴板');
  } catch {
    proxy?.$modal.msgWarning('浏览器拒绝访问剪贴板，请手动选择复制');
  }
};

const openUrl = (url: string) => window.open(url, '_blank');

// —— 「代码」按钮：把该图层实际加载用的 CRS / 网格 / LOD 常量打成一段可独立运行的
// OpenLayers 代码。常量来自引擎的 tileMatrixSets 与 TileJSON（与预览页同源同值），
// 因此换图层/换网格时随之变化。矢量瓦片（MVT）与 ArcGIS 缓存（XYZ 栅格）都支持。
const previewKind = computed<'mvt' | 'raster' | ''>(() => {
  const u = previewUrl.value;
  if (u.includes('viewer-mvt')) return 'mvt';
  if (u.includes('viewer-arcgis')) return 'raster';
  return '';
});
const isCodeable = computed(() => !!previewKind.value);
const codeVisible = ref(false);
const codeLoading = ref(false);
const codeText = ref('');

const lonLatTo3857 = (lon: number, lat: number): [number, number] => {
  const R = 20037508.342789244;
  const x = (lon * R) / 180;
  const y = (Math.log(Math.tan(((90 + lat) * Math.PI) / 360)) * R) / Math.PI;
  return [Number.isFinite(x) ? x : 0, Number.isFinite(y) ? y : 0];
};

const buildCode = (
  kind: 'mvt' | 'raster',
  base: string,
  layer: string,
  gridId: string,
  tms: any,
  tj: any,
  tileTpl?: string
): string => {
  const epsg = String(tms?.crs || '').match(/(\d+)\s*$/)?.[1] || '3857';
  const crs = `EPSG:${epsg}`;
  const matrices: any[] = (tms?.tileMatrices || []).slice().sort((a, b) => +a.id - +b.id);
  if (!matrices.length) return '// 该网格没有 tileMatrix 定义，无法生成加载代码';
  const origin: number[] = matrices[0].topLeftCorner;
  const tileW = matrices[0].tileWidth || 256;
  const tileH = matrices[0].tileHeight || 256;
  const res = matrices.map((m) => m.resolutions?.native ?? m.resolution);
  const last = matrices[matrices.length - 1];
  const fullW = last.matrixWidth * tileW * (last.resolutions?.native ?? last.resolution);
  const fullH = last.matrixHeight * tileH * (last.resolutions?.native ?? last.resolution);
  const extent = [origin[0], origin[1] - fullH, origin[0] + fullW, origin[1]];
  // TileJSON 的 bounds 是 WGS84 经纬度；投影网格需换算，否则首屏会 fit 到错误位置
  let bbox: number[] = extent;
  const rawBounds = Array.isArray(tj?.bounds) && tj.bounds.length === 4
    ? tj.bounds.map(Number)
    : (Array.isArray(tms?.bbox) && tms.bbox.length === 4 ? tms.bbox.map(Number) : null);
  if (rawBounds) {
    if (epsg === '4326' || epsg === '4490') bbox = rawBounds;
    else if (epsg === '3857' && Math.abs(rawBounds[2]) <= 180) {
      const a = lonLatTo3857(rawBounds[0], rawBounds[1]);
      const c = lonLatTo3857(rawBounds[2], rawBounds[3]);
      bbox = [a[0], a[1], c[0], c[1]];
    } else {
      bbox = rawBounds;
    }
  }
  const arr = (a: number[]) => `[${a.join(', ')}]`;
  // 瓦片服务地址单独作为常量：便于把同一段代码切到内网/外网（http/https）环境
  let tilePath: string;
  if (kind === 'raster') {
    const tpl = tileTpl || `${base}/tiles/${layer}/{z}/{x}/{y}.png`;
    tilePath = /^https?:\/\//i.test(tpl) ? new URL(tpl).pathname : tpl;
  } else {
    tilePath = `/mvt/${layer}/${gridId}/{z}/{x}/{y}.pbf`;
  }
  const L: string[] = [];
  L.push('// ===== 该图层的加载常量（随图层 / 发布网格变化；由管理端按引擎网格定义生成）=====');
  L.push(`const TILE_SERVER = ${JSON.stringify(base)};   // 瓦片服务地址（http/https）`);
  L.push(`const LAYER = ${JSON.stringify(layer)};`);
  L.push(`const CRS = ${JSON.stringify(crs)};                  // 源坐标系，不做 WebMercator 重投影`);
  L.push(`const GRID = ${JSON.stringify(gridId)};        // 发布网格（LOD 体系）`);
  L.push(`const ORIGIN = ${arr(origin)};           // 网格左上角（CRS 单位）`);
  L.push(`const TILE_SIZE = [${tileW}, ${tileH}];`);
  L.push(`const RESOLUTIONS = ${arr(res)};  // ${res.length} 级，CRS 单位/像素（源 LOD）`);
  L.push(`const EXTENT = ${arr(extent)};   // 网格全幅`);
  L.push(`const BBOX = ${arr(bbox)};   // 图层四至（首屏 fit）`);
  L.push(`const TILE_URL = TILE_SERVER + ${JSON.stringify(tilePath)};`);
  L.push('');
  if (tms?.proj4) {
    L.push('// 1) 非内置坐标系必须先注册 proj4（否则 OpenLayers 不认识该 EPSG）');
    L.push(`proj4.defs(CRS, ${JSON.stringify(tms.proj4)});`);
    L.push('ol.proj.proj4.register(proj4);');
    L.push('');
  }
  L.push('// 2) 按发布网格建 TileGrid：用源 LOD 分辨率，而不是默认 WebMercator 金字塔');
  L.push('const tileGrid = new ol.tilegrid.TileGrid({');
  L.push('  extent: EXTENT,');
  L.push('  origin: ORIGIN,');
  L.push('  resolutions: RESOLUTIONS,');
  L.push('  tileSize: TILE_SIZE');
  L.push('});');
  L.push('');
  if (kind === 'mvt') {
    L.push('// 3) 矢量瓦片源：非 WebMercator 网格必须显式给 projection + tileGrid');
    L.push('const source = new ol.source.VectorTile({');
    L.push('  projection: CRS,');
    L.push('  tileGrid: tileGrid,');
    L.push('  format: new ol.format.MVT(),');
    L.push('  url: TILE_URL');
    L.push('});');
  } else {
    L.push('// 3) 栅格瓦片源：按缓存网格的 (z / col / row) 直接读预切片');
    L.push('const source = new ol.source.XYZ({');
    L.push('  projection: CRS,');
    L.push('  tileGrid: tileGrid,');
    L.push('  url: TILE_URL');
    L.push('});');
  }
  L.push('');
  L.push('const map = new ol.Map({');
  L.push("  target: 'map',");
  L.push(`  layers: [new ol.layer.${kind === 'mvt' ? 'VectorTile' : 'Tile'}({ source: source })],`);
  L.push('  view: new ol.View({');
  L.push('    projection: CRS,');
  L.push('    center: [(BBOX[0] + BBOX[2]) / 2, (BBOX[1] + BBOX[3]) / 2],');
  L.push('    resolution: RESOLUTIONS[0]');
  L.push('  })');
  L.push('});');
  L.push('map.getView().fit(BBOX, { padding: [20, 20, 20, 20] });');
  return L.join('\n');
};

const openCode = async () => {
  codeVisible.value = true;
  codeLoading.value = true;
  codeText.value = '正在读取该图层的网格定义…';
  try {
    const kind = previewKind.value || 'mvt';
    const u = new URL(previewUrl.value, window.location.origin);
    const gridId = u.searchParams.get('grid') || '';
    const base = u.searchParams.get('base') || u.origin;
    const layer = layerName.value;
    if (!gridId) {
      codeText.value = '// 预览地址没有携带网格参数，无法确定该图层的 LOD 体系';
      return;
    }
    const [tms, tj] = await Promise.all([
      fetch(`${base}/ogc/tiles/v1/tileMatrixSets/${encodeURIComponent(gridId)}`).then((r) => r.json()),
      kind === 'mvt'
        ? fetch(`${base}/mvt/${encodeURIComponent(layer)}/tilejson.json`)
            .then((r) => (r.ok ? r.json() : null))
            .catch(() => null)
        : Promise.resolve(null)
    ]);
    // ArcGIS 缓存的瓦片地址就是 endpoints 里的 XYZ 模板（各自缓存自己的 z/col/row）
    const tileTpl = (data.value.endpoints || []).find((e) => e.name.startsWith('XYZ'))?.url;
    codeText.value = buildCode(kind, base, layer, gridId, tms, tj, tileTpl);
  } catch (e: any) {
    codeText.value = `// 读取网格定义失败：${e?.message || e}`;
  } finally {
    codeLoading.value = false;
  }
};

const copyCode = async () => {
  try {
    await navigator.clipboard.writeText(codeText.value);
    proxy?.$modal.msgSuccess('已复制到剪贴板');
  } catch {
    proxy?.$modal.msgWarning('浏览器拒绝访问剪贴板，请手动选择复制');
  }
};

defineExpose({ open });
</script>

<style scoped>
.code-pre {
  margin: 0;
  max-height: 460px;
  overflow: auto;
  padding: 12px;
  border-radius: 4px;
  background: #1e1e1e;
  color: #d4d4d4;
  font: 12px/1.6 Consolas, Monaco, monospace;
  white-space: pre;
}
</style>
