# 服务发布与操作指南（中文）

本目录按服务类型/数据来源给出一份一类说明文件，每份都同时包含**详细说明**与**操作说明**。权威发布矩阵来自 map-api 的 `GET /admin/capabilities`。

| 服务 / 数据类型 | 指南 | 一句话 |
|---|---|---|
| WMS | [wms.zh-CN.md](wms.zh-CN.md) | 栅格/矢量兜底协议，单帧出图与老客户端 |
| WMTS | [wmts.zh-CN.md](wmts.zh-CN.md) | 标准 OGC 瓦片接口，KVP + RESTful |
| XYZ / TMS | [xyz-tms.zh-CN.md](xyz-tms.zh-CN.md) | Leaflet/MapLibre 与老 TMS 客户端，底图推荐 |
| MVT | [mvt.zh-CN.md](mvt.zh-CN.md) | 矢量瓦片，缓存命中 16–19ms |
| WFS | [wfs.zh-CN.md](wfs.zh-CN.md) | 要素查询，GML / GeoJSON / CSV |
| OGC API - Features | [oaf.zh-CN.md](oaf.zh-CN.md) | REST 要素 API + OpenAPI |
| OGC API - Tiles | [oat.zh-CN.md](oat.zh-CN.md) | REST 瓦片 API |
| 3D Tiles | [3dtiles.zh-CN.md](3dtiles.zh-CN.md) | 一个 XML 一个服务，Cesium 直连 |
| ArcGIS 缓存 | [arcgis-cache.zh-CN.md](arcgis-cache.zh-CN.md) | `Conf.xml` / `.tpk` / `.tpkx` 按原生网格发布 |

## 通用发布链路

1. **工作空间**：`POST /map/catalog/workspaces`。
2. **数据源 Store**：`POST /map/catalog/stores`，`storeType` 见各指南；`basePath` 指向数据根目录。
3. **发现**：`POST /map/catalog/stores/{id}/discover`，产出 DataResource，带 `dataType` / `publishable` / `srcCrs` / bbox。
4. **发布**：`POST /map/catalog/publish`，请求体核心字段：

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

5. **端点**：`GET /map/catalog/published/{id}/endpoints` 返回预览/复制地址。
6. **生命周期**：`PUT /map/catalog/published/{id}/state` 支持 `active|stopped|paused|deleted`。

发布动作最终由 manager 调 map-api `POST /admin/services` 写 `<catalog.root>/<kind>/<name>.xml` 并热重载；配置写前做 XML 语法校验与 `.xml.bak` 备份，重载失败回滚。

## 数据类型 → 可发布服务类型

| `dataType` | `mapserver_kind` | 可发布服务 | 默认勾选 |
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
| `pg_table` | — | 无（仅数据查询） | — |
| `shapefile` | — | 无（暂无读取器） | — |
| `mvt` | — | 无（静态目录只发现） | — |

## 相关文档

- [mapserver README：协议矩阵、请求样例与限制](../../mapserver/README.md)
- [架构说明](../architecture.md)
- [ArcGIS 缓存格式](../arcgis-cache-formats.md)
- [English index](README.en-US.md)
