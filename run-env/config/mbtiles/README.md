# mbtiles/ — MBTiles 图层目录

MBTiles（SQLite 瓦片包）图层由 map-api 原生服务（rusqlite bundled，不依赖系统
sqlite3.dll）。启动时只读打开校验 `tiles` 表并解析 `metadata`；每瓦片请求在
`spawn_blocking` 内按需开只读连接，无跨线程共享句柄。

一个图层一个 `*.xml`，`<source>` 的 `type` 必须与目录名一致：

```xml
<layer name="topo" title="Topo raster (MBTiles)" src_crs="EPSG:3857" tile_px="256">
  <source type="mbtiles" path="data/topo.mbtiles"/>
  <grids>
    <item>WebMercatorQuad</item>
  </grids>
</layer>
```

服务路径（XYZ 寻址，服务端内部把 y 翻转成 MBTiles 的 TMS 行）：

```text
/mbtiles/{name}                    归档信息（format / bounds / zoom_range / metadata）
/mbtiles/{name}/{z}/{x}/{y}        瓦片（容忍 .png / .pbf 后缀）
```

MBTiles 层同时注册进 XYZ / TMS / WMTS / MVT 前端——Leaflet 可直接
`/tiles/{name}/{z}/{x}/{y}.png`，MapLibre 可走 `/mvt/{name}/{z}/{x}/{y}.pbf`
（矢量包，gzip 透传 `Content-Encoding: gzip`）。

约定：
- `metadata` 的 `bounds`（west,south,east,north）供 WMS/WMTS 能力文档使用；
  `minzoom`/`maxzoom` 只是声明值，越界请求按 404 处理。
- 瓦片为 TMS 行存储（row 0 在底部），与 TMS 路由的底行起算天然对消。
- 本目录自带回归样例 `test.xml`（PNG 包）与 `test-vector.xml`（gzip pbf 包），
  由 `tools/make_mbtiles_fixture.py` 生成。
