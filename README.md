# rust-gisserver

|                            |                 |
| -------------------------- | --------------- |
| [English](README.en-US.md) | [中文](README.md) |

> **先说重点：rust-gisserver 支持的服务类型非常多。** 一套引擎同时提供 WMS 1.1.1/1.3.0、WMTS 1.0.0、TMS/XYZ、MVT 矢量瓦片（含 PostGIS 动态矢量）、WFS、OGC API - Features、OGC API - Tiles、3D Tiles/terrain 直通，以及 COG / PMTiles / MBTiles / ArcGIS 紧凑缓存（`.tpk`/`.tpkx`）直读；并面向 CGCS2000、高斯-克吕格 3°/6° 分带、UTM 等中国常用坐标系提供恒等直出与可选 PROJ 跨坐标转换。

**rust-gisserver** 基于 Rust 开发，**运行资源占用低、运行性能很高、并发量很大**，是面向国产化坐标系与大规模并发场景的新一代开放 GIS 服务器标杆产品。核心字节码路径（TIFF/COG 解析、瓦片解码、MVT 编码、PMTiles 归档读写、投影）均依据公开格式规范，不依赖 GDAL 等重型 C 绑定；PROJ 仅作为可选运行时后端，缺失时自动降级到内置投影路径。

> **四个先进性支点（迁移友好）：**
> ① **传统瓦片格式平滑迁移** —— XYZ/TMS 目录、MBTiles、PMTiles 直接挂接，**不重切图、不转格式**；
> ② **ArcGIS 紧凑缓存原样读取** —— Server 缓存目录、`.tpk`（compact v1）、`.tpkx`（compact v2）、gis-wt `.webpdle`（WTC v1）一套逻辑通吃；
> ③ **新型 COG 直接服务** —— HTTP Range 按字节直出，**无需切片入库、无需预处理**；
> ④ **基于 PostGIS 的高性能动态 MVT** —— 空间表/视图实时生成矢量瓦片，**改库即改图**。
> 详见 [先进性与数据兼容（迁移友好）](#先进性与数据兼容迁移友好)。

## 三组件结构

```text
rust-gisserver/
├── gisserver/   # 核心库：单包 gisserver-* 模块，feature 门控，可发布到 crates.io
├── mapserver/   # 地图服务器：自包含瓦片/影像/要素服务引擎（二进制 mapserve）
├── manager/     # 管理控制台：Rust backend + Vite UI（服务目录/发布/日志/样式管理）
├── docs/        # 架构说明与按服务类型的发布/操作指南
└── installscript/ # PostgreSQL/PostGIS 与服务安装资产、SQL 基线
```

| 组件            | 角色    | 说明                                                                                                                                 | 许可证                    |
| ------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| **gisserver** | 核心库   | 纯 Rust 数据与协议层：COG/BigTIFF 解析与写入、瓦片渲染、MVT 编码、PMTiles/MBTiles 归档、WMS/WFS/OGC API 协议、投影、PostGIS 数据源、路径编码推断                            | MIT OR Apache-2.0      |
| **mapserver** | 地图服务器 | 开箱即用的 HTTP 服务：WMS / WMTS / TMS / XYZ / MVT / PMTiles / MBTiles / COG / WFS / OGC API / 3D Tiles；五段缓存 + 可选磁盘 L2；预览与 UI 由 manager 托管 | AGPL-3.0-only + 商业授权例外 |
| **manager**   | 管理控制台 | Web 控制台：服务目录管理、地图服务发布向导（坐标系/范围校验）、运行时监控、样式管理、缓存管理、日志审计                                                                             | AGPL-3.0-only + 商业授权例外 |

> 许可说明：核心库 **gisserver** 采用 MIT OR Apache-2.0 双许可，可自由闭源/商用集成；**mapserver** 与 **manager** 在 AGPL-3.0-only 基础上附**商业授权例外**——内部使用/个人学习/开源集成免费，SaaS 服务或闭源商业集成需购买商业授权（条款见 [mapserver/LICENSE](mapserver/LICENSE) / [manager/LICENSE](manager/LICENSE) 末尾，咨询 rex yang <13556388@qq.com>）。

## 为什么快

- **零重型依赖**：纯 Rust 实现，无 GDAL/tiff/mapbox C 绑定；PROJ 为可选动态后端，默认构建零 FFI、零 unsafe。
- **调用方式优先**：MVT 缓存命中 16–19ms，XYZ 栅格瓦片 10–21ms/片，归档/COG/3D Tiles 走 HTTP Range 零解码直通；WMS GetMap 是兜底，不当底图缩放主链路。
- **五段缓存**：输出瓦片、图层索引、元数据文档、目录元数据、要素键值缓存，统一由 `CacheHub` 收口，支持 TTL、手动 purge、可选磁盘 L2。
- **并发模型**：渲染/编码/阻塞 IO 全部走 `spawn_blocking`，tokio worker 只做调度；本地文件与 HTTP Range 源全部定位读（`read_at`/`seek_read`），共享 fd 并发无竞态；远程源隔离到专用 rayon 池。
- **渲染快速路径**：请求 CRS == 源 CRS 且 geotransform 仿射时，512×256 GetMap 的 262,144 次逐点投影降为 5 次（release micro-bench 2.9×）。

详细实测与实现要点见 [mapserver/README.md](mapserver/README.md)。

---

## 先进性与数据兼容（迁移友好）

一句话概括：**已有的瓦片资产不用重切、不用转格式、不用上 GDAL，挂上就能服务；新数据走 COG 直读与 PostGIS 动态出图，全程零预处理。** 四个支点展开如下。

### 1) 传统瓦片格式：平滑迁移，不重切图

| 已有资产 | 迁移方式 | 直接获得的服务 |
| --- | --- | --- |
| `z/x/y.{png,jpg,pbf}` 标准目录树 | 放入图层目录即可发现 | XYZ / TMS / WMTS（顶行/底行 y-flip 自动处理） |
| MBTiles（SQLite 瓦片包） | 只读直读，内置连接池 + `prepare_cached` | XYZ / TMS / WMTS / MVT 全部前端 |
| PMTiles v3 归档 | HTTP Range 整档直出 | `/pmtiles/{name}` 与 `pmtiles://` 客户端直连 |
| 静态 `z/x/y` 瓦片目录 | 可选打包为 PMTiles | Hilbert 聚类、blob 去重、gzip 重编码 |

MBTiles 同时支持 PNG/JPEG/WebP 栅格与 gzip pbf 矢量包，TMS 行号自动翻转；一个归档发布即获得全部瓦片前端，无需为每种协议各切一套。

### 2) ArcGIS 紧凑缓存：四类形态，一套读取逻辑

对历史 ArcGIS 资产做到**原样读取**——不解压重切、不转换格式：

| 形态 | 载体 | 字节布局 | 网格配置 |
| --- | --- | --- | --- |
| ArcGIS Server 缓存目录 | 目录 | compact v1 / v2 | `Conf.xml` + `conf.cdi` |
| `.tpk` 瓦片包 | ZIP | compact v1（独立 `.bundlx`、列优先） | `Conf.xml` + `conf.cdi` |
| `.tpkx`（Pro Compact Tile Package） | ZIP | compact v2（索引内嵌、行优先） | `root.json` |
| gis-wt `.webpdle`（WTC v1，自有） | 目录 | WTC v1 稀疏索引 | `Conf.xml` + `conf.cdi` |

- **一套逻辑**：四类形态的网格方案统一归一为 `gisserver::arcgis_cache::definition::CacheDefinition`；差异只在目录布局与瓦片字节布局，读瓦片代码只有一套。
- **定义记录复用**：按内容指纹（CRS、原点、瓦片尺寸、DPI、LOD、PacketSize、瓦片根目录名）复用；同一套网格装不同范围的数据也共享记录，不重复建条目。
- **一键导入**：`POST /admin/import-tpk` 完成解压 → 提升缓存根 → 原子发布到 `data_root/arcgis_tpk/`；临时目录 + 原子改名，失败不留半成品，同名重导覆盖且可回滚。
- **实测样本**：`road.tpk`（EPSG:4490，compact v1，L00–L08）、`bai.tpkx`（EPSG:4522，compact v2，L00–L04）导入后均被 `/admin/discover` 识别为 `arcgis` 且可发布。

格式细节见 [docs/arcgis-cache-formats.md](docs/arcgis-cache-formats.md)，发布操作见 [ArcGIS 缓存指南](docs/services/arcgis-cache.zh-CN.md)。

### 3) 新型 COG：直接服务，无需切片入库

- Cloud Optimized GeoTIFF 直接读取：本地路径或 HTTP(S) / S3 预签名 URL，走 **HTTP Range 206** 按字节取数据，QGIS / rasterio 可远程打开，GB 级影像不整读进内存。
- 自研 TIFF / BigTIFF 解析（经典 TIFF 与 BigTIFF 的 IFD 链、overview 金字塔选层、rayon 并行解码），**无 GDAL 等重型 C 绑定**。
- 一份 COG 同时服务 WMS / WMTS / TMS / XYZ，避免"先切片、再入库、再发布"的冗长链路；`/cog/{layer}` 提供零解码直通端点。
- 兼容 CGCS2000、高斯-克吕格 3°/6° 分带、UTM 等常用坐标系；同坐标系恒等直出，跨坐标系交给可选 PROJ 后端。

### 4) 基于 PostGIS 的高性能动态 MVT 瓦片

- **改库即改图**：从 PostGIS 空间表、视图或 SQL 表达式实时生成 MVT 矢量瓦片，无需预切瓦片、无需刷新缓存目录。
- **双坐标接口**：`/{z}/{x}/{y}.pbf`（EPSG:3857）与 `/{srid}/{z}/{x}/{y}.pbf`（源 CRS 自身单位，如 4543 米、4326 度，服务端不做 `ST_Transform`）；TileJSON 按接口分别宣告 bounds/crs/grid。
- **为高并发而设计**：按 DSN 共享连接池，版本预检 PG≥11 / PostGIS≥3.0，启动自检 GiST 索引；编码前按瓦片 bbox 预过滤，低 z 用 `ST_SimplifyPreserveTopology` 分级简化；输出缓存命中约 16–25ms，可选磁盘 L2 跨重启保留。
- **免落表发布**：`from` 表达式直接发布 SQL / 视图图层；bounds/count 走统计估计，缺失自动回退精确扫描。

发布与调优见 [MVT 指南](docs/services/mvt.zh-CN.md) 与 [架构说明](docs/architecture.md)。

> **为什么这更先进**：传统 GIS 服务器把"格式转换 + 金字塔切片 + 入库"当作发布前置条件，数据一变就要重跑整条链路。rust-gisserver 把字节层做到自研，直接面向已有资产（传统瓦片、ArcGIS 紧凑缓存）与新型标准（COG、PostGIS 动态 MVT）：**迁移零重切、新数据零预处理、发布即服务**；叠加纯 Rust 的零重型依赖、五段缓存、定位读与 `spawn_blocking` 并发模型，在国产化坐标系与大规模并发场景下兼具兼容性与性能优势。

---

## 数据格式与目录对照

系统按 catalog 的 kind 组织目录：`cog/`、`vector/`、`pmtiles/`、`mbtiles/`、`postgis/`、`3dtiles/`、`terrain/`、`arcgis/`；`mvt/`、`osgb/`、`styles/` 为预留/辅目录。作者本机测试数据集 `E:\data\1`（其 `README.md` §1.1 有逐格式说明）覆盖下列资产，可作为格式迁移的实测参考：

| 格式 / 资产 | 系统支持 | 可发布服务 | 样本位置（`E:\data\1`） |
| --- | --- | --- | --- |
| COG / BigTIFF（`.tif`） | ✅ 原生 | WMS / WMTS / XYZ | `cog/`（EPSG:4326 真实影像 + fixture） |
| GeoJSON | ✅ 原生（→ `vector/`） | MVT / WMS / WMTS / XYZ / WFS / OAF | `geojson/`、`recon/` |
| MBTiles（栅格 / 矢量） | ✅ 原生 | 栅格 → WMTS/XYZ；矢量 → MVT | `mbtiles/`、`recon/` |
| PMTiles（栅格 / 矢量） | ✅ 原生 | 栅格 → WMTS/XYZ；矢量 → MVT | `pmtiles/` |
| ArcGIS Server 缓存（compact v1 / v2） | ✅ 原生（→ `arcgis/`） | WMTS / XYZ | `arcgis_cache/`（3×v1 + 1×v2） |
| `.tpk` / `.tpkx` 紧凑瓦片包 | ✅ 原生（→ `arcgis/`） | 导入后 WMTS / XYZ | `arcgis_tpk/`、`arcgis_cache/` |
| 3D Tiles（`tileset.json` + `.b3dm`） | ✅ 原生 | `/3dtiles` 直通 | `3dtiles/`（recon / kmczc / kmczc_full） |
| Quantized-Mesh（`.terrainpack`） | ✅ 原生直通 | `/terrain/{name}/layer.json` | `terrain/` |
| PostGIS 空间表 / 视图 | ✅ 原生 | **动态 MVT** / WMS / WFS / OAF | `postgis/`（pg_dump + 视图导出） |
| 静态 `z/x/y` 瓦片目录 | ⚠️ 需打包 | 打包为 PMTiles/MBTiles 后 WMTS/XYZ | `xyz-tms/`（样本待补） |
| OSGB 原始倾斜摄影 | ⚠️ 需转换 | 转 3D Tiles 后发布 | `osgb/`（样本待补） |
| 静态 Shapefile | ⚠️ 仅发现 | **不可发布**（无 shp 读取器） | `shapefile/` |
| 静态 MVT 瓦片目录 | ⚠️ 仅发现 | **不可发布**（无 SourceKind） | `mvt/`（预留） |
| Esri `.mmpk` / FileGDB `.gdb` | ❌ 不支持 | 需取出内部 COG | `cog/cog.mmpk`、缓存目录内 `Status.gdb` |

> 格式能力以 `GET /admin/capabilities` 的 `data_type_caps` 为权威矩阵；「可发现」不等于「可发布」。gisserver 核心库未来补齐 shp 读取器后，`shapefile/` 一行即可转为可发布。

---

## Linux 安装

### 1) Docker 方式（推荐）

部署入口按「语言 × 平台」拆分，Linux 宿主机用：

| 目录                            | compose 项目名              | 首建库种子                    |
| ----------------------------- | ------------------------ | ------------------------ |
| `manager/deploy_zh-cn/linux/` | `gis-data-base-zh-linux` | `system.sql` → `map.sql` |
| `manager/deploy_en/linux/`    | `gis-data-base-en-linux` | `.en_US` 版种子             |

两个项目端口相同（6180 / 8080 / `MAPSERVER_DB_PORT` 默认 5434），**不能同时运行**；卷按项目名隔离，切换语言不需要 `down -v`。

```bash
cd manager/deploy_zh-cn/linux          # 英文环境：cd ../../deploy_en/linux 或 manager/deploy_en/linux
cp ../../deploy/.env.example .env
# 编辑 .env：至少设置 MANAGER_JWT_SECRET（openssl rand -hex 32），并改掉 MAPSERVER_DB_PASSWORD
bash ../../deploy/start.sh --dir .     # up --build → 等 PostGIS healthy → validate
```

也可直接手工：

```bash
docker compose up -d --build
docker compose ps
docker compose logs -f map-api
```

访问：管理后台 <http://localhost:6180/>，地图服务健康 <http://localhost:8080/health>，WMS 能力 <http://localhost:8080/wms?service=WMS\&request=GetCapabilities>。

数据挂载点 `GISDATA_DIR`（默认 `./gisdata`）只读挂到 map-api 的 `/app/data`；把图层 XML 放进部署模板 `config/mapserver/`，数据路径写 `/app/data/...` 即可。详见 [manager/deploy/README.md](manager/deploy/README.md)。

### 2) 编译运行

环境：Rust 1.80+（项目在 1.98.1 上验证）、build-essential、pkg-config、PostgreSQL 客户端；跨 CRS 转换需要 PROJ 运行库。

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl libproj-dev proj-bin libsqlite3-0 postgresql-client

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"

cargo build --release --manifest-path mapserver/Cargo.toml
cargo build --release --manifest-path manager/backend/Cargo.toml

cd manager/ui
corepack enable
pnpm install --frozen-lockfile
pnpm run build:prod
cd ../../..
```

初始化 PostgreSQL 16 + PostGIS（库名带连字符，需双引号）：

```bash
psql -h localhost -p 5434 -U postgres -c 'CREATE DATABASE "mapserver-manager"'
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/system.sql
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/map.sql
# 英文基线：改用 system.en_US.sql / map.en_US.sql，二选一，不要两套都跑
```

运行（在仓库根目录执行，保证 `static_dir="manager/ui/dist"` 与 `data_root` 相对路径可解析）：

```bash
export MANAGER_DB_PASSWORD='<你的口令>'
export MANAGER_JWT_SECRET='<随机长串>'
export MAPSERVER_DB_PASSWORD='<你的口令>'
export PROJ_DATA=/usr/share/proj
# 可选：export DATA_ROOT=/srv/gisdata

./mapserver/target/release/mapserve serve --config mapserver/config/mapserver.xml   # :8080
./manager/backend/target/release/manager --config manager/backend/config/manager.xml # :6180
```

> Linux 容器/源码部署均需显式安装 PROJ 运行库（`libproj-dev proj-bin libsqlite3-0`）；缺失时服务照常启动，但 4490/45xx/UTM 等跨 CRS 互转会失败。通过 `/health` 的 `proj` 字段判定。

数据库安装指南：[中文](docs/postgresql16-postgis-linux-install.zh-CN.md) / [English](docs/postgresql16-postgis-linux-install.en-US.md)。

---

## Windows 安装

### 1) 构建与安装

环境：Windows 10/11 x64、Rust MSVC toolchain、VS 2019/2022 Build Tools、可选 Node.js + pnpm。

```powershell
cd <repo>\build\script
.\build_all.bat            # mapserver + manager
.\build_all.bat frontend   # 同时构建前端（推荐，否则管理后台 UI 404）
.\install.bat              # 默认装到 <repo>\..\..\..\rmap_target
# 或指定目录：.\install.bat D:\gis\rmap_target
```

安装目录生成 `bin\`（`mapserve.exe` / `manager.exe` / PROJ DLL）、`config\`、`web\`、`data\`、`cache\`、`logs\`、`share\proj\`、`start-mapserver.bat`、`start-manager.bat`。复制 `.env.example` 为 `.env`，至少设置 `MANAGER_DB_PASSWORD`、`MANAGER_JWT_SECRET`、`MAPSERVER_DB_PASSWORD`。完整路径与踩坑见 [build/script/README.md](build/script/README.md)。

### 2) 以 Windows 服务运行（NSSM）

```powershell
nssm install map-api "<安装目录>\bin\mapserve.exe"
nssm install manager  "<安装目录>\bin\manager.exe"
nssm set map-api AppDirectory "<安装目录>"
nssm set manager  AppDirectory "<安装目录>"
nssm start map-api
nssm start manager
```

**必须设 `AppDirectory`**：`data_root` 默认是相对路径 `data`，NSSM 不设时工作目录会落在 `bin\`，导致管理面报 `data_root 不可访问`。任何自定义启动方式（systemd / 计划任务）同理。

### 3) 命令行直接启动

```powershell
cd <安装目录>
.\start-mapserver.bat   # map-api，默认 :8080，脚本会清 PROJ_LIB 并设 PROJ_DATA
.\start-manager.bat     # manager，默认 :6180
# 或手动：
.\bin\mapserve.exe serve --config config\mapserver.xml
.\bin\manager.exe --config config\manager.xml
```

端口不读 `.env`：map-api 取 `config/mapserver.xml` 的 `<server port>` 或 `--listen`，manager 取 `config/manager.xml` 的 `<server addr>`。改端口后需同步 `manager.xml` 的 `<mapsvc base_url>`。详见 [mapserver/install/windows/README.md](mapserver/install/windows/README.md)。

---

## 系统架构

```text
HTTP (axum 0.8 / tokio)     mapserver/services/map-api
  ├─ handlers/  WMS · WMTS · TMS · XYZ · MVT · PMTiles · MBTiles · COG · WFS · OAF · OAT · 3DTiles · health
  ├─ admin/     发现/发布向导 · 服务登记簿 · 样式 · 访问日志 · 缓存 · 导入 TPK
  ├─ state.rs   AppState（图层运行时、五段缓存、基 URL、manager 模式）
  └─ config.rs  XML → serde 结构体 → TileMatrixSet

协议层                      gisserver::wms_protocol / wfs_protocol / ogcapi_features / ogcapi_tiles
数据层                      gisserver::projection / storage / cog / renderer / mvt / pmtiles / pg_source / arcgis_cache
管理平面                    manager/backend (axum) + manager/ui (Vue 3 + Vite + Element Plus)
```

- map-api 是纯地图引擎，**不托管 UI**；`<manager>` 配置只做短超时探活，决定 `managed` / `standalone` 模式。
- 图层配置一律 XML、一图层一文件，落在 `<catalog.root>/<类型>/`；管理面新建/编辑服务写同一位置，带语法校验、`.xml.bak` 备份与重载失败回滚。
- `RangeSource` 是唯一读取缝：本地文件、HTTP(S) 对象、S3 预签名 URL 统一为定位读。

分层、数据流、并发与缓存约定详见 [docs/architecture.md](docs/architecture.md)。

---

## 各类服务如何发布

发布主链路（推荐）：**工作空间 → 数据源 Store → 发现数据图层 → 发布向导勾选服务类型 → 已发布服务预览/复制端点**。底层由 manager 调 map-api 的 `/admin/services` 写图层 XML 并热重载。

**迁移与动态数据优先**：既有 XYZ/TMS 瓦片目录、MBTiles/PMTiles 归档、ArcGIS 紧凑缓存可直接挂接发布（[迁移能力](#1-传统瓦片格式平滑迁移不重切图)）；新数据推荐 COG 直读（栅格）与 PostGIS 动态 MVT（矢量），改库即改图，无需预切瓦片。

| 服务类型                   | 可发布的数据来源                                 | 详细说明                                               | 操作说明                                                 |
| ---------------------- | ---------------------------------------- | -------------------------------------------------- | ---------------------------------------------------- |
| **WMS** 1.1.1/1.3.0    | COG、GeoJSON、PostGIS 空间表                  | [WMS 指南](docs/services/wms.zh-CN.md)               | [发布步骤](docs/services/wms.zh-CN.md#操作说明管理后台)          |
| **WMTS** 1.0.0         | COG、GeoJSON、栅格 PMTiles/MBTiles、ArcGIS 缓存 | [WMTS 指南](docs/services/wmts.zh-CN.md)             | [发布步骤](docs/services/wmts.zh-CN.md#操作说明管理后台)         |
| **XYZ / TMS**          | 同 WMTS；传统 `z/x/y` 瓦片目录免重切迁移        | [XYZ/TMS 指南](docs/services/xyz-tms.zh-CN.md)       | [发布步骤](docs/services/xyz-tms.zh-CN.md#操作说明管理后台)      |
| **MVT** 矢量瓦片           | **PostGIS 动态生成**、GeoJSON、矢量 PMTiles/MBTiles | [MVT 指南](docs/services/mvt.zh-CN.md)               | [发布步骤](docs/services/mvt.zh-CN.md#操作说明管理后台)          |
| **WFS** 2.0/1.1/1.0    | GeoJSON、PostGIS 空间表                      | [WFS 指南](docs/services/wfs.zh-CN.md)               | [发布步骤](docs/services/wfs.zh-CN.md#操作说明管理后台)          |
| **OGC API - Features** | GeoJSON、PostGIS 空间表                      | [OAF 指南](docs/services/oaf.zh-CN.md)               | [发布步骤](docs/services/oaf.zh-CN.md#操作说明管理后台)          |
| **OGC API - Tiles**    | COG、GeoJSON、栅格归档、ArcGIS 缓存               | [OAT 指南](docs/services/oat.zh-CN.md)               | [发布步骤](docs/services/oat.zh-CN.md#操作说明管理后台)          |
| **3D Tiles**           | `tileset.json` 目录（OSGB 需先转 3D Tiles）     | [3D Tiles 指南](docs/services/3dtiles.zh-CN.md)      | [发布步骤](docs/services/3dtiles.zh-CN.md#操作说明管理后台)      |
| **ArcGIS 缓存**          | `Conf.xml` 缓存目录、`.tpk`、`.tpkx`           | [ArcGIS 缓存指南](docs/services/arcgis-cache.zh-CN.md) | [发布步骤](docs/services/arcgis-cache.zh-CN.md#操作说明管理后台) |

> 静态 Shapefile 目录、静态 MVT 瓦片目录可被发现但暂不可发布；PostGIS 普通表（无 geometry 列）仅数据查询，不发布图层服务。权威矩阵来自 `GET /admin/capabilities`。

全部指南索引：[中文](docs/services/README.zh-CN.md) / [English](docs/services/README.en-US.md)。协议矩阵、请求样例与限制见 [mapserver/README.md](mapserver/README.md)。

---

## 下一步计划

以下为路线图方向，不承诺具体时间表：

1. **换机不碎（P1）**：运行期发布的服务把 `source.path` 规约为 `${DATA_ROOT}` 相对存储，serve 端按 `data_root` 拼接；files 与 db 两种后端统一。详见 [manager/deploy/data-root.md](manager/deploy/data-root.md)。
2. **更多数据源**：在已支持传统瓦片格式迁移、ArcGIS 紧凑缓存、COG 直读与 PostGIS 动态 MVT 的基础上，继续补齐静态 Shapefile / GeoPackage 读取、ArcGIS 缓存发布链路增强、COG JPEG/WEBP 瓦片解码与写入。
3. **安全与治理**：数据面鉴权/RBAC、TLS 终结、集群部署与监控告警；当前版本建议置于反向代理之后。
4. **OGC 完整性**：WMS 多图层合成、SLD 更多符号与过滤能力、更多 conformance class 与自动化测试。
5. **性能持续调优**：重采样/调色板并行化、WMS GetMap 输出缓存、rayon 池与 MBTiles 连接池可配置化。
6. **交付体验**：Linux 一键安装包与 systemd 单元、双语文档与示例数据同步完善。

---

## 许可证

- `gisserver/`（核心库）：**MIT OR Apache-2.0** 双许可，见 [gisserver/LICENSE](gisserver/LICENSE)
- `mapserver/` 与 `manager/`：**AGPL-3.0-only** + 商业授权例外，见各自 `LICENSE`；第三方组件声明见 [mapserver/THIRD-PARTY-NOTICES.md](mapserver/THIRD-PARTY-NOTICES.md)

Copyright (c) 2026 rex yang <13556388@qq.com> and contributors.
