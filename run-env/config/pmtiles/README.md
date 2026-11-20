# pmtiles/ — PMTiles 图层目录

一个图层一个 `*.xml` 配置文件，`<source>` 的 `type` 必须与目录名一致：

```xml
<layer name="cities" title="Cities (PMTiles)" src_crs="EPSG:3857" tile_px="256">
  <source type="pmtiles" path="data/cities.pmtiles"/>
  <grids>
    <item>WebMercatorQuad</item>
  </grids>
</layer>
```

注意：整档 PMTiles 直出（供 pmtiles:// 客户端读取的归档映射）走
`mapserver.xml` 的 `archives` 段，与本目录的"图层"语义不同。
