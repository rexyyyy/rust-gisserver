# gisserver

**gisserver** 是 **rust-gisserver** 的核心库（本仓库三组件之一）：一组纯 Rust 数据与协议库，打包为 `gisserver-*` crate 族发布到 crates.io，可直接作为开发依赖复用；地图服务器 [mapserver](../mapserver) 即构建在这套库之上。

核心字节码路径（TIFF/COG 解析、瓦片解码、MVT 编码、PMTiles 归档读写）均依据公开格式规范独立实现，不依赖 GDAL/tiff/mapbox 等重型 C 绑定；PROJ 仅作为**可选运行时后端**，Windows 下动态解析 C ABI（无编译期链接 / build script），不存在时自动降级到内置投影路径。默认构建**零 FFI、零 unsafe**。

## 库清单

| crate | 说明 |
|---|---|
| `gisserver-wms-common` | Bbox / CRS / 错误 / Layer / Style / TileMatrixSet（公共基座） |
| `gisserver-wms-protocol` | WMS 1.1.1/1.3.0、WMTS 1.0.0 协议解析与能力文档 XML |
| `gisserver-wfs-protocol` | WFS 1.0.0/1.1.0/2.0.0 协议：caps/DescribeFeatureType/GetFeature |
| `gisserver-ogcapi-features` | OGC API - Features 1.0.1：landing/conformance/collections/items |
| `gisserver-ogcapi-tiles` | OGC API - Tiles 1.0.0：元数据文档（瓦片字节复用 WMS 渲染） |
| `gisserver-projection` | EPSG:4326 ↔ EPSG:3857（自写 Web Mercator）+ 可选 PROJ 后端 |
| `gisserver-storage` | RangeSource：本地文件 / HTTP(S) 定位读；foyer 解码瓦片缓存 |
| `gisserver-cog` | 自研 COG/BigTIFF 解析（IFD 链、overviews、predictor、LZW/ZSTD）+ 写入器 |
| `gisserver-renderer` | 窗口重采样（nearest/bilinear）、调色板、PNG 编码、SLD、GeoJSON 栅格化 |
| `gisserver-mvt` | MVT spec 2.1 自研编码器（4096 extent、bbox 预过滤） |
| `gisserver-pmtiles` | PMTiles v3 读取 + 打包（Hilbert TileID、列式 varint 目录、gzip） |
| `gisserver-arcgis-cache` | ESRI compact cache v1/v2（bundle/tpk）读取 |
| `gisserver-vector-query` | 内存 GeoJSON FeatureCollection 查询（bbox/分页/GML/GeoJSON/CSV） |
| `gisserver-pg-source` | PostGIS 数据源：连接池 / SQL 模板 / OGC-Filter→SQL |
| `gisserver-encoding` | 路径/文件名编码推断（区域由 `GIS_ENCODING_REGION` 环境配置声明） |

## 使用

```toml
[dependencies]
gisserver-cog = "0.1"
gisserver-pmtiles = "0.1"
```

工作区开发：

```powershell
cargo check --workspace
cargo test --workspace
```

## 许可证 / License

gisserver 库族采用 **MIT OR Apache-2.0** 双许可（任选其一），全文见 [LICENSE](LICENSE)。

Copyright (c) 2026 rex yang &lt;13556388@qq.com&gt; and contributors.
