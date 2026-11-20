# mvt/ — MVT 图层目录（预留）

MVT（Mapbox Vector Tiles）当前是 **vector / pmtiles 图层的输出能力**
（`/wms`、`/mvt/{layer}` 直接出 pbf），不是独立源类型，因此本目录暂无服务类型。

若未来引入独立 MVT 源（如预切好的 pbf 目录 / 远端瓦片集），在此目录放每图层一个
`*.xml` 配置，并实现对应 `SourceKind`。
