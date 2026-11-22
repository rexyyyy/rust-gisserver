import i18n from '@/lang';
import type { PublishedVO } from '@/api/map/catalog/types';

/**
 * Shared display rules for published map services.
 * Pure functions only: no component instance, no reactive state, safe to import
 * from both pages and presentational components.
 */

/** Layer kind labels. Resolved on each call so runtime language switches take effect. */
export const kindLabel = (k: string): string => {
  const labels: Record<string, string> = {
    cog: i18n.global.t('map.auto.cogRaster'),
    vector: i18n.global.t('map.auto.vector'),
    pmtiles: 'PMTiles',
    mbtiles: 'MBTiles',
    postgis: 'PostGIS',
    arcgis: i18n.global.t('map.auto.arcgisTiles'),
    '3dtiles': '3D Tiles',
    terrain: i18n.global.t('map.auto.terrain')
  };
  return labels[k] || k;
};

/** Engine may return serviceTypes as "mvt,wms" or as ["mvt", "wms"]. */
export const toTypes = (v: string | string[] | undefined | null): string[] =>
  Array.isArray(v)
    ? v
    : v
      ? v
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean)
      : [];

/** Style editor applies only to vector / PostGIS services that speak MVT. */
export const canEditStyle = (row: PublishedVO): boolean =>
  ['postgis', 'vector'].includes(row.mapserverKind) && toTypes(row.serviceTypes).includes('mvt');

export type GeometryType = 'point' | 'line' | 'polygon' | 'mixed';

/** Prefer the top-level geometryType field, fall back to meta variants. */
export const geomTypeOf = (row: PublishedVO): GeometryType => {
  const r = row as PublishedVO & {
    geometry_type?: string;
    meta?: { geometry_type?: string; geometryType?: string };
  };
  const raw = (r.geometryType || r.geometry_type || r.meta?.geometry_type || r.meta?.geometryType || '').toUpperCase();
  if (!raw) return 'mixed';
  if (raw === 'POINT' || raw === 'MULTIPOINT') return 'point';
  if (raw === 'LINESTRING' || raw === 'MULTILINESTRING') return 'line';
  if (raw === 'POLYGON' || raw === 'MULTIPOLYGON') return 'polygon';
  return 'mixed';
};
