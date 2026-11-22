# gisserver

**gisserver** 是 **rust-gisserver** 的核心库（本仓库三组件之一）：一个纯 Rust 的单包
（single-package）核心库，发布到 crates.io 供开发直接引用；地图服务器
[mapserver](../mapserver) 即构建在它之上。

核心字节码路径（TIFF/COG 解析、瓦片解码、MVT 编码、PMTiles 归档读写）均依据公开
格式规范独立实现，不依赖 GDAL/tiff/mapbox 等重型 C 绑定；PROJ 仅作为**可选运行时
后端**，Windows 下动态解析 C ABI（无编译期链接 / build script），不存在时自动降级
到内置投影路径。除 `projection` 的 PROJ 运行时动态加载后端外，其余模块零 FFI、
零 unsafe；不启用 PROJ 时完全零 FFI。

## 模块与 feature

功能域全部作为本包的模块，由 Cargo features 门控——`default-features = false` 只取
所需，重依赖（tokio-postgres / foyer / zstd…）全部 optional：

| 模块 | feature | 说明 |
|---|---|---|
| `wms_common` | （恒编译） | 公共基座：Bbox / CRS / 图层 / 样式 / 错误 / TileMatrixSet |
| `encoding` | `encoding` | 路径编码检测与归一化（GBK/UTF-8，区域由 `GIS_ENCODING_REGION` 声明） |
| `projection` | `projection` | EPSG:4326 ↔ 3857 自写 Web Mercator + 可选 PROJ 后端 |
| `mvt` | `mvt` | MVT spec 2.1 编码器（4096 extent、bbox 预过滤） |
| `vector_query` | `vector-query` | 内存 GeoJSON 要素查询（bbox/分页/GML/GeoJSON/CSV） |
| `storage` | `storage` | RangeSource 定位读（本地/HTTP(S)/S3 预签名）+ foyer 瓦片缓存 |
| `pg_source` | `postgis` | PostGIS 数据源：连接池 / SQL 模板 / OGC-Filter→SQL 下推 |
| `cog` | `cog` | 自研 COG/BigTIFF 解析（IFD 链、overviews、predictor、LZW/ZSTD）+ 写入器 |
| `renderer` | `renderer` | 窗口重采样、调色板、PNG 编码、SLD 符号渲染 |
| `pmtiles` | `pmtiles` | PMTiles v3 读取 + 打包（Hilbert TileID、列式 varint 目录） |
| `arcgis_cache` | `arcgis-cache` | ESRI compact cache v1/v2（bundle/tpk）读取 |
| `wms_protocol` | `wms-protocol` | WMS 1.1.1/1.3.0、WMTS 1.0.0 协议与能力文档 |
| `wfs_protocol` | `wfs-protocol` | WFS 1.0.0/1.1.0/2.0.0 协议 |
| `ogcapi_features` | `ogcapi-features` | OGC API - Features 1.0.1 |
| `ogcapi_tiles` | `ogcapi-tiles` | OGC API - Tiles 1.0.0 |

`features = ["full"]` 一次全量启用；`win-acp` 为可选的 Windows ANSI 代码页兜底
（默认关闭，关闭时 `encoding` 模块零 FFI）。

## 使用

```toml
[dependencies]
# 全量
gisserver = "0.8"
# 或按需（只取 mvt + 投影，不编译 PostGIS/foyer 等重依赖）
gisserver = { version = "0.8", default-features = false, features = ["mvt", "projection"] }
```

```rust
use gisserver::cog::{Cog, GeoTransform};   // 模块路径 = gisserver::<module>::
```

工作区开发：

```powershell
cargo check                       # 默认 feature
cargo check --features full       # 全量
cargo check --no-default-features # 仅 wms_common 基座
cargo test --features full
```

## 许可证 / License

gisserver 采用 **MIT OR Apache-2.0** 双许可（任选其一），全文见 [LICENSE](LICENSE)。

Copyright (c) 2026 rex yang <13556388@qq.com> and contributors.
