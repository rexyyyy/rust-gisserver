# Service publishing and operation guides (English)

This directory contains one guide per service type / data source. Each guide includes both **details** and an **operation guide**. The authoritative publish matrix comes from `GET /admin/capabilities` on map-api.

| Service / data type | Guide | Summary |
|---|---|---|
| WMS | [wms.en-US.md](wms.en-US.md) | Raster/vector fallback, single-frame output and legacy clients |
| WMTS | [wmts.en-US.md](wmts.en-US.md) | Standard OGC tiles, KVP + RESTful |
| XYZ / TMS | [xyz-tms.en-US.md](xyz-tms.en-US.md) | Leaflet/MapLibre and legacy TMS, recommended basemap path |
| MVT | [mvt.en-US.md](mvt.en-US.md) | Vector tiles, 16–19 ms cache hits |
| WFS | [wfs.en-US.md](wfs.en-US.md) | Feature queries, GML / GeoJSON / CSV |
| OGC API - Features | [oaf.en-US.md](oaf.en-US.md) | REST feature API + OpenAPI |
| OGC API - Tiles | [oat.en-US.md](oat.en-US.md) | REST tile API |
| 3D Tiles | [3dtiles.en-US.md](3dtiles.en-US.md) | One XML per service, Cesium ready |
| ArcGIS cache | [arcgis-cache.en-US.md](arcgis-cache.en-US.md) | `Conf.xml` / `.tpk` / `.tpkx` published on their native grid |

## Common publishing flow

1. **Workspace**: `POST /map/catalog/workspaces`.
2. **Data-source Store**: `POST /map/catalog/stores`; see each guide for `storeType`, and set `basePath` to the data root.
3. **Discover**: `POST /map/catalog/stores/{id}/discover` produces DataResources with `dataType`, `publishable`, `srcCrs` and bbox.
4. **Publish**: `POST /map/catalog/publish`; core body fields:

   ```json
   {
     "resourceId": 123,
     "layerName": "world",
     "title": "World",
     "srcCrs": "EPSG:4326",
     "bbox": [-180, -90, 180, 90],
     "serviceTypes": ["wms", "wmts", "xyz"]
   }
   ```

5. **Endpoints**: `GET /map/catalog/published/{id}/endpoints` returns preview/copy URLs.
6. **Lifecycle**: `PUT /map/catalog/published/{id}/state` accepts `active|stopped|paused|deleted`.

Publishing ultimately calls map-api `POST /admin/services` to write `<catalog.root>/<kind>/<name>.xml` and hot-reload. The admin plane validates XML syntax, keeps a `.xml.bak` backup and rolls back on failed reload.

## Data type → publishable service types

| `dataType` | `mapserver_kind` | Publishable services | Defaults |
|---|---|---|---|
| `cog` | `cog` | wms, wmts, xyz | wms, wmts |
| `geojson` | `vector` | wms, wmts, xyz, mvt, wfs, oaf | mvt |
| `pmtiles` | `pmtiles` | wmts, xyz | wmts |
| `pmtiles_mvt` | `pmtiles` | mvt | mvt |
| `mbtiles` | `mbtiles` | wmts, xyz | wmts |
| `mbtiles_mvt` | `mbtiles` | mvt | mvt |
| `3dtiles` | `3dtiles` | 3dtiles | 3dtiles |
| `arcgis` | `arcgis` | wmts, xyz | wmts |
| `postgis_table` | `postgis` | wms, mvt, wfs, oaf | mvt, wms |
| `pg_table` | — | none (data query only) | — |
| `shapefile` | — | none natively; **import into PostGIS → `postgis_table` publishes mvt/wms/wfs/oaf** | mvt |
| `mvt` | — | none (static directory is discoverable only) | — |

## Related docs

- [mapserver README: protocol matrix, samples and limits](../../mapserver/README.md)
- [Architecture](../architecture.md)
- [ArcGIS cache formats](../arcgis-cache-formats.md)
- [中文索引](README.zh-CN.md)
