//! # gisserver
//!
//! rust-gisserver 的核心库：纯 Rust 数据与协议层，供地图服务器
//! [mapserver](../mapserver) 与第三方开发直接复用。
//!
//! 模块按功能域划分，经 Cargo features 门控（`default-features = false` 可只取所需；
//! `features = ["full"]` 全量启用）：
//!
//! | 模块 | feature | 说明 |
//! |---|---|---|
//! | [`wms_common`] | （恒编译） | 公共基座：bbox/CRS/图层/样式/错误/TileMatrixSet |
//! | [`encoding`] | `encoding` | 路径编码检测与归一化（GBK/UTF-8） |
//! | [`projection`] | `projection` | EPSG:4326↔3857 Web Mercator |
//! | [`mvt`] | `mvt` | MVT spec 2.1 编码器 |
//! | [`vector_query`] | `vector-query` | 内存 GeoJSON 要素查询 |
//! | [`storage`] | `storage` | RangeSource 定位读 / foyer 瓦片缓存 |
//! | [`pg_source`] | `postgis` | PostGIS 数据源（连接池/SQL 模板/OGC-Filter 下推） |
//! | [`cog`] | `cog` | COG/BigTIFF 解析与写入 |
//! | [`renderer`] | `renderer` | 窗口重采样/PNG/SLD 符号渲染 |
//! | [`pmtiles`] | `pmtiles` | PMTiles v3 读取与打包 |
//! | [`arcgis_cache`] | `arcgis-cache` | ArcGIS compact cache v1/v2 读取 |
//! | [`wms_protocol`] | `wms-protocol` | WMS/WMTS 协议与能力文档 |
//! | [`wfs_protocol`] | `wfs-protocol` | WFS 协议 |
//! | [`ogcapi_features`] | `ogcapi-features` | OGC API - Features |
//! | [`ogcapi_tiles`] | `ogcapi-tiles` | OGC API - Tiles |
//!
//! 核心字节码路径不依赖重型 C 绑定；
//! PROJ 仅作为可选运行时后端（见 [`projection`]）。默认零 FFI / 零 unsafe。

pub mod wms_common;

#[cfg(feature = "encoding")]       pub mod encoding;
#[cfg(feature = "projection")]     pub mod projection;
#[cfg(feature = "mvt")]            pub mod mvt;
#[cfg(feature = "vector-query")]   pub mod vector_query;
#[cfg(feature = "storage")]        pub mod storage;
#[cfg(feature = "postgis")]        pub mod pg_source;
#[cfg(feature = "cog")]            pub mod cog;
#[cfg(feature = "renderer")]       pub mod renderer;
#[cfg(feature = "pmtiles")]        pub mod pmtiles;
#[cfg(feature = "arcgis-cache")]   pub mod arcgis_cache;
#[cfg(feature = "wms-protocol")]   pub mod wms_protocol;
#[cfg(feature = "wfs-protocol")]   pub mod wfs_protocol;
#[cfg(feature = "ogcapi-features")] pub mod ogcapi_features;
#[cfg(feature = "ogcapi-tiles")]   pub mod ogcapi_tiles;
