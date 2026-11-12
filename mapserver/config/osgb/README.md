# osgb/ — OSGB 图层目录（预留）

OSGB 是倾斜摄影原始格式（gis-wt 的 `gislab.osgb2tiles` 插件负责 OSGB → 3D Tiles
转换）。转换产物（tileset.json + b3dm）落在 `3dtiles/` 目录登记服务，因此本目录
当前无独立服务类型，仅作数据组织预留。

若未来需要"OSGB 直读服务"（不落 3D Tiles、按需动态转换），在此目录放每图层一个
`*.xml` 配置，并实现对应的 `SourceKind` 与 handler。
