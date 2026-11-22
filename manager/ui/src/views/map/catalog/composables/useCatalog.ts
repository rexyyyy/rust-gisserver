import i18n from '@/lang';
/**
 * 数据功能各页（工作空间 / 数据源 / 数据图层 / 已发布服务 / 总览）的共享口径。
 *
 * 只放「口径」不放数据：能力矩阵的唯一来源仍是引擎 `GET /admin/capabilities`，
 * 管理端不另存一份矩阵副本，否则又会变成「文档说支持、接口不认」的第二个真相源。
 */
import { getCapabilities } from '@/api/map/catalog';
import { CatalogCaps, DataTypeCap, NativeGrid } from '@/api/map/catalog/types';

/** 引擎登记簿四态：各页必须叫同一个名字，否则同一个状态在两个页面两种说法 */
export const STATE_LABELS: Record<string, string> = {
  active: i18n.global.t('map.auto.inService2'),
  stopped: i18n.global.t('map.auto.text42'),
  paused: i18n.global.t('map.auto.text43'),
  deleted: i18n.global.t('map.auto.text44')
};

export const stateLabel = (s: string) => STATE_LABELS[s] || s;

export const stateTag = (s: string) => (s === 'active' ? 'success' : s === 'deleted' ? 'danger' : 'warning');

export const bboxText = (b: number[] | null) =>
  b && b.length === 4 ? b.map((x) => Number(x).toFixed(3)).join(', ') : '—';

/** 服务协议展示名：OGC API 一族太长，表格里统一收成短名 */
export const PROTO_LABELS: Record<string, string> = {
  wms: 'WMS',
  wmts: 'WMTS',
  xyz: 'XYZ',
  wfs: 'WFS',
  mvt: 'MVT',
  oaf: 'OGC API',
  '3dtiles': '3D Tiles'
};

export const protoLabel = (t: string) => PROTO_LABELS[t] || t.toUpperCase();

/** 按数据类型取能力行（纯函数，便于组件拿着 caps prop 直接用） */
export const capOfProto = (caps: CatalogCaps | null, dataType?: string | null): DataTypeCap | undefined =>
  dataType ? caps?.data_types.find((d) => d.data_type === dataType) : undefined;

/** 该图层是否必须按「原生坐标系 + 自定义 LOD 网格」调用。
 *
 *  判定口径与引擎的 `default_grid_id` 一致（`grids[0]`，缺省 WebMercatorQuad）：
 *  - ArcGIS 预切片缓存一律是自定义网格（`arcgis_<name>`），只能按缓存行列号读；
 *  - 其它图层只要发布时固化的网格 CRS 不是 3857（4490 / 4326 / Conf.xml 自定义），
 *    瓦片行列号就与 WebMercator XYZ 对不上，必须带 grid 段。
 *
 *  返回 null 表示可直接用默认 WebMercator 预览/浏览。 */
export function nativeGridInfo(row: {
  mapserverKind?: string | null;
  layerName?: string;
  srcCrs?: string | null;
  grid?: NativeGrid | null;
}): { crs: string; grid: string } | null {
  if (row.mapserverKind === 'arcgis') {
    return { crs: row.grid?.crs || row.srcCrs || '', grid: `arcgis_${row.layerName || ''}` };
  }
  const g = row.grid;
  if (!g) return null;
  const mode = (g.mode || '').toLowerCase();
  const crs = (g.crs || '').toUpperCase();
  const nonMercator = crs !== '' && !crs.includes('3857');
  if (mode === 'custom' || mode === 'native' || nonMercator) {
    return { crs: g.crs || row.srcCrs || '', grid: g.id || '' };
  }
  return null;
}

/** 标记的悬浮说明：写清该图层该用什么坐标系 / 网格，以及默认预览为什么白屏 */
export function nativeGridTip(
  row: { mapserverKind?: string | null; layerName?: string },
  info: { crs: string; grid: string }
): string {
  const where = [info.crs, info.grid].filter(Boolean).join(' · ');
  if (row.mapserverKind === 'arcgis') {
    return i18n.global.t('map.nativeGrid.arcgis', { where, layer: row.layerName });
  }
  return i18n.global.t('map.nativeGrid.dynamic', {
    where,
    layer: row.layerName,
    grid: info.grid || '{grid}'
  });
}

/** 能力矩阵 + 按数据类型取行（发布勾选取于此） */
export function useCaps() {
  const caps = ref<CatalogCaps | null>(null);

  const loadCaps = async () => {
    try {
      caps.value = (await getCapabilities()).data;
    } catch {
      /* 引擎不可用时向导渲染不出勾选项；保存时后端仍会再取一次矩阵并拒绝越界组合 */
    }
  };

  const capOf = (dataType?: string | null) => capOfProto(caps.value, dataType);

  return { caps, loadCaps, capOf };
}
