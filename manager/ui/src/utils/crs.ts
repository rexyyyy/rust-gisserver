/**
 * 坐标系（CRS）能力口径与写法工具。
 *
 * 口径必须与 rust-gisserver README《坐标系支持》一致，分三层：
 * ① 内置数学 EPSG:4326 / CRS:84 / EPSG:3857（服务端自写 Web Mercator，零依赖）；
 * ② 恒等路径——请求 CRS 与源 CRS 相同时不做任何投影数学，因此 CGCS2000
 *    （EPSG:4490）或高斯-克吕格分带图层在没有 PROJ 的机器上也能直出；
 * ③ 跨 CRS 互转依赖运行时 PROJ ≥ 8 后端（PROJ_DLL / PROJ_DATA），受理的码由
 *    服务端 crs::is_supported 判定：地理系 4326/4490/4214/4258，投影带
 *    4491..=4554（CGCS2000/北京 54/西安 80 的 3°、6° 高斯-克吕格）、
 *    4800..=4999、32648..=32660（WGS84 UTM 48N–60N）。
 *
 * 因此 `proj_loaded === false` 不等于「只能用 4326/3857」，UI 文案不能这么写，
 * 否则运维会误判同源直出的图层也坏了。
 */

/** 三层能力全量说明，用于表单/详情里的悬浮提示。 */
export const CRS_SUPPORT_HINT =
  '坐标系三层能力：① 内置 EPSG:4326 / CRS:84 / EPSG:3857（自写 Web Mercator，不依赖 PROJ）；' +
  '② 请求 CRS 与源 CRS 相同时走恒等直出，CGCS2000（EPSG:4490）与高斯-克吕格分带图层无 PROJ 也能出图；' +
  '③ 跨 CRS 互转需运行时 PROJ ≥ 8（环境变量 PROJ_DLL / PROJ_DATA）。';

/** PROJ 未加载时的真实影响，用于运行状态标签。 */
export const PROJ_MISSING_HINT =
  'PROJ 未加载：跨 CRS 互转不可用（这类请求回 400，不是 500）；' + '内置 4326 / CRS:84 / 3857 与「请求 CRS == 源 CRS」的原生产物直出不受影响。';

/** 源坐标系输入框占位：常见 datum 都给例子，别只写 4326 让人以为不支持别的。 */
export const SRC_CRS_PLACEHOLDER = 'EPSG:4326 / EPSG:4490（CGCS2000）/ EPSG:4543（3°带）…（可选，缺省 4326）';

/** 图层配置里可填的地理系与投影带族（与服务端 is_supported 同源，供校验/提示用）。 */
export const CRS_FAMILY_HINT =
  '可用坐标系：地理系 EPSG:4326 / 4490（CGCS2000）/ 4214（西安 80）/ 4258；' +
  '投影带 EPSG:4491–4554（CGCS2000/北京 54/西安 80 的 3°、6° 高斯-克吕格）、4800–4999、' +
  '32648–32660（WGS84 UTM 48N–60N）；短名、URN（urn:ogc:def:crs:EPSG::4490）与 ' +
  'def-URI（http://www.opengis.net/def/crs/EPSG/0/4490）写法同等接受。';

/** 地理系（度）CRS 码：范围数值可直接当经纬度用。 */
const GEOGRAPHIC_CODES = ['4326', '4490', '4214', '4258'];

/**
 * 从任意 CRS 写法里取 EPSG 码（与服务端 crs::normalize 一样宽容）。
 *
 * 取的是**尾部**数字：`EPSG:4543`、`urn:ogc:def:crs:EPSG::4543`、
 * `http://www.opengis.net/def/crs/EPSG/0/4543` 都得 4543；
 * `CRS:84` 各写法返回 4326（同一 datum，只是轴序不同）。
 * 取不到返回 null，调用方按缺省 4326 处理。
 */
export function epsgCode(crs?: string): string | null {
  const raw = (crs || '').trim();
  if (!raw) return null;
  if (/(?:CRS:84|CRS84)$/i.test(raw)) return '4326';
  const m = raw.match(/(\d+)\s*$/);
  return m ? m[1] : null;
}

/** 该 CRS 的范围是不是度数（可当经纬度直接用）；投影带返回 false（单位是米）。 */
export function isGeographicCrs(crs?: string): boolean {
  const code = epsgCode(crs);
  return code !== null && GEOGRAPHIC_CODES.includes(code);
}
