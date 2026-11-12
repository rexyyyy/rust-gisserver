# rust-gisserver

**rust-gisserver** 是一套用 Rust 编写的开源自包含 GIS 服务器全家桶：从纯 Rust 核心库，到地图服务器引擎，再到 Web 管理控制台，覆盖「发布—管理—复用」全链路。

核心路径（TIFF/COG 解析、瓦片解码、MVT 编码、PMTiles 归档读写、投影）均依据公开格式规范独立实现，**不依赖 GDAL 等重型 C 绑定**；PROJ 仅作为可选运行时后端，不存在时自动降级到内置投影路径。

## 三组件结构

```text
rust-gisserver/
├── gisserver/   # 核心库：15 个 gisserver-* 库 crate，可发布到 crates.io 供开发引用
├── mapserver/   # 地图服务器：自包含瓦片/影像/要素服务引擎（二进制 map-api）
├── manager/     # 管理控制台：Rust backend + Vite ui（服务目录/发布/日志/样式管理）
└── docs/        # 架构说明
```

| 组件 | 角色 | 说明 | 许可证 |
|---|---|---|---|
| **gisserver** | 核心库 | 纯 Rust 数据与协议层：COG/BigTIFF 解析与写入、瓦片渲染、MVT 编码、PMTiles/MBTiles 归档、WMS/WFS/OGC API 协议、投影、PostGIS 数据源、路径编码推断。**单包发布**到 crates.io，功能域作为模块由 feature 门控，可直接作为开发依赖复用 | MIT OR Apache-2.0 |
| **mapserver** | 地图服务器 | 开箱即用的 HTTP 服务：WMS 1.1.1/1.3.0、WMTS 1.0.0、TMS/XYZ、MVT 矢量瓦片（含 PostGIS 动态矢量）、PMTiles/MBTiles/COG 直读、WFS、OGC API - Features/Tiles、3D Tiles/terrain 直通；内置 Leaflet/Cesium 查看器与缓存体系 | AGPL-3.0-only + 商业授权例外 |
| **manager** | 管理控制台 | Web 控制台：服务目录管理、地图服务发布向导（坐标系/范围校验）、运行时监控、样式管理、缓存管理、日志审计 | AGPL-3.0-only |

> 许可说明：核心库 **gisserver** 采用 MIT OR Apache-2.0 双许可，可自由闭源/商用集成，无需授权；**mapserver** 在 AGPL-3.0-only 基础上附**商业授权例外**——内部使用/个人学习/开源集成免费，SaaS 服务或闭源商业集成需购买商业授权（条款见 [mapserver/LICENSE](mapserver/LICENSE) 末尾，咨询 rex yang <13556388@qq.com>）。

## 快速开始

### 1) 地图服务器（mapserver）

```powershell
cd mapserver
cargo build --release
cargo run -p mapserver -- serve        # 默认读取 config/mapserver.xml，监听 0.0.0.0:8080
# 浏览器访问 http://localhost:8080/viewer
```

详见 [mapserver/README.md](mapserver/README.md)（协议矩阵、坐标系支持、缓存调优、实测数据）。

### 2) 管理控制台（manager）

```powershell
cd manager
cargo run -p manager                    # 后端（默认代理 map-api 管理面）
cd ui && npm install && npm run dev     # 前端（Vite）
```

### 3) 核心库（gisserver）

```toml
[dependencies]
gisserver = "0.8"                                   # 全量
# 或按需：default-features = false, features = ["mvt", "projection"]
```

```rust
use gisserver::cog::{Cog, GeoTransform};   // 模块路径 = gisserver::<module>::
```

详见 [gisserver/README.md](gisserver/README.md)。

## 特性一览

- **零重型依赖**：纯 Rust 实现，无 GDAL/tiff/mapbox C 绑定；PROJ 为可选动态后端
- **全协议覆盖**：WMS / WMTS / TMS / XYZ / MVT / PMTiles / MBTiles / COG / WFS / OGC API / 3D Tiles
- **中国坐标系友好**：CGCS2000（4490）、高斯-克吕格 3°/6° 分带、西安 80，恒等路径免 PROJ 直出
- **性能内建**：五段缓存（foyer）、定位读零拷贝、MVT bbox 预过滤、渲染投影快速路径（2.9×）
- **区域自适应**：路径编码由 `.env` 的 `GIS_ENCODING_REGION` 显式声明（zh-CN/zh-TW/en/ko/ja 预设），核心库默认零 FFI

## 许可证

- `gisserver/`（核心库）：**MIT OR Apache-2.0** 双许可，见 [gisserver/LICENSE](gisserver/LICENSE)
- `mapserver/`（地图服务器）与 `manager/`（管理控制台）：**AGPL-3.0-only**，见各自 `LICENSE`；第三方组件声明见 [mapserver/THIRD-PARTY-NOTICES.md](mapserver/THIRD-PARTY-NOTICES.md)

Copyright (c) 2026 rex yang &lt;13556388@qq.com&gt; and contributors.
