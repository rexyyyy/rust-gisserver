/**
 * MVT 矢量瓦片前端样式 JSON 读写。
 * 路径：/manager-api/map/services/:kind/:name/mvt-style
 */
import request from '@/utils/request';

export interface Stroke {
  color: string;
  width: number;
}
export interface Fill {
  color: string;
}
export interface PointStyle {
  radius: number;
  fill: Fill;
  stroke: Stroke;
}
export interface LineStyle {
  stroke: Stroke;
}
export interface PolygonStyle {
  fill: Fill;
  stroke: Stroke;
}
export interface LabelStyle {
  field: string;
  font: string;
  fill: Fill;
  stroke: Stroke;
  offsetY: number;
}
export interface MvtStyleJson {
  version: number;
  point?: PointStyle;
  line?: LineStyle;
  polygon?: PolygonStyle;
  label?: LabelStyle | null;
}

export function getMvtStyle(kind: string, name: string) {
  return request.get<any, any>(`/map/services/${kind}/${name}/mvt-style`);
}

export function putMvtStyle(kind: string, name: string, style: MvtStyleJson) {
  return request.put<any, any>(`/map/services/${kind}/${name}/mvt-style`, { style });
}
