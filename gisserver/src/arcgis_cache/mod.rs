//! ArcGIS Server 切片缓存（预切片）读取层。
//!
//! 本 crate 只做三件事，且都不依赖 HTTP/框架层，便于单独测试：
//!
//! 1. [`conf`]：解析缓存服务目录里的 `Conf.xml` / `conf.cdi`，拿到 CRS、
//!    `TileOrigin`、瓦片像素、DPI、LOD 阶梯（分辨率 + 地图比例尺）、存储格式
//!    与 `PacketSize`；
//! 2. [`grid`]：把 `Conf.xml` 的 LOD 阶梯摊成一份 **custom grid**（含每级矩阵
//!    行列数与对齐后的范围），使浏览端按数据自身的分辨率/比例尺取瓦片，而不是
//!    套用与本缓存不等价的预设网格；
//! 3. [`bundle`]：按 compact cache v1/v2 的字节布局做 read-through 取瓦片，
//!    外带一层 bundle 索引表缓存。
//!
//! # 为什么需要它
//!
//! ArcGIS 缓存的瓦片寻址完全由 `Conf.xml` 决定，与 WebMercatorQuad /
//! WorldCRS84Quad 都不等价；E 盘样例更是 CGCS2000 地理坐标系（EPSG:4490）。
//! 用预设网格取瓦片会整体错位且**不报错**，因此网格必须由缓存自身推导。
//!
//! 实现依据 ESRI compact cache v1/v2 的公开字节布局独立编写，字节序与索引
//! 顺序的结论以真实缓存数据的实证为准（见 [`bundle`] 模块文档）。

pub mod bundle;
pub mod conf;
pub mod grid;
pub mod tpk;
pub mod xmlscan;

pub use bundle::{ArcgisCache, CacheProbe, CacheVersion, TileData};
pub use conf::{CacheDocs, CacheInfo, Cdi, ImageType, LodInfo};
pub use grid::{EmittedGrid, GridSpec};
