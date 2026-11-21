# mapserver

mapserver 是 **rust-gisserver** 的地图服务器（引擎组件）——一个用 Rust 编写的自包含瓦片服务器，提供 **WMS · WMTS · TMS · XYZ · MVT（vector tiles）· PMTiles · MBTiles · COG** 等瓦片/栅格/要素服务。它依赖核心库 [gisserver](../gisserver)（单包，`features = ["full"]` 全量启用），自身只包含可运行的 HTTP 服务与部署资产。

项目是独立 Cargo workspace（`members = ["services/*"]`），HTTP 服务包名为 `mapserver`（二进制 `map-api`）。核心字节码路径（TIFF/COG 解析、瓦片解码、MVT 编码、PMTiles 归档读写）由核心库 gisserver 依据公开格式规范独立实现，不依赖 GDAL/tiff/mapbox 等重型 C 绑定；PROJ 仅作为**可选运行时后端**，Windows 下动态解析 C ABI（无编译期链接 / build script），不存在时自动降级到内置投影路径。

```text
mapserver/
├── Cargo.toml               # workspace + 统一依赖版本
├── (核心库 ../gisserver —— 单包 gisserver，模块经 feature 门控，见其 README)  # 纯 Rust 数据与协议层（无 HTTP）
├── services/
│   └── map-api/             # axum HTTP 服务：router 装配 + 数据面 handler + admin 管理面
│       ├── handlers/        # 数据面：wms/wfs/oaf/oat/wmts/tms/xyz/mvt/pmtiles/mbtiles/cog/terrain/3dtiles
│       └── admin/           # 管理面（独立子层）：catalog_admin/http/registry/styles/usage/access_log/check
├── config/                  # mapserver.xml（server 级）+ 图层类型目录（一图层一文件）+ 样式
├── fixtures/                # 示例数据（sample.tif、sample.geojson、test.mbtiles、test-vector.mbtiles）
├── tools/                   # 辅助脚本（make_mbtiles_fixture.py 生成 MBTiles 测试包）
├── (UI 在 ../manager/ui/public/preview/)  # 预览页由 manager 托管，引擎不内嵌页面
├── docs/                    # 架构说明
└── scripts/                 # （预留）```

## 支持的服务

| 协议 | 入口 | 说明 |
|---|---|---|
| **WMS** 1.1.1 / 1.3.0 | `/wms` | GetCapabilities / GetMap / GetFeatureInfo（栅格采样 + 矢量命中测试，INFO_FORMAT 支持 text/plain、text/html、application/json，无命中返回 200 空体——同 GeoServer）/ GetLegendGraphic（符号图例 PNG，默认高度随行数自适应），1.3.0 轴序处理、ServiceException/ExceptionReport |
| **PostGIS 动态矢量瓦片** | `/mvt/{layer}/…` | 双坐标接口：`/{z}/{x}/{y}.pbf`（EPSG:3857 经纬度 XYZ）与 `/{srid}/{z}/{x}/{y}.pbf`（源 CRS 自身单位建矩阵，如 4543 米制、4326 度数，无 ST_Transform）；TileJSON 按接口分别宣告 bounds/crs/grid 与 vector_layers 字段类型；foyer output 缓存（命中 ~2-25ms；`server.disk_cache` 可选磁盘 L2 跨重启保留）；仅当图层配置 `source.type: postgis` 时启用（按 DSN 共享连接池，版本预检 PG≥11/PostGIS≥3.0）。P3 增强：`simplify`（低 z 服务端 `ST_SimplifyPreserveTopology` 分级简化）；`from` 表达式发布**SQL/视图图层**（免落表）；OAF items 支持 `sortby` 下推；启动自检 GiST 索引、bounds/count 走统计估计（缺失自动回退精确扫描） |
| **WMTS** 1.0.0 | `/wmts`、`/wmts/1.0.0/WMTSCapabilities.xml`、`/wmts/rest/...`、`/tileMatrixSets/:id` | KVP + RESTful + 能力文档；预设 TileMatrixSet（WebMercatorQuad / WorldCRS84Quad）及自定义 |
| **TMS** 1.0.0 | `/tms/1.0.0/{layer}/{z}/{x}/{y}.png` | 底行起算 y-flip |
| **XYZ**（别名） | `/tiles/{layer}/{z}/{x}/{y}.png` | 顶行起算，Leaflet/MapLibre 直连 |
| **MVT** vector tiles | `/mvt/{layer}/{z}/{x}/{y}.pbf`、`/mvt/{layer}/{tms}/{z}/{x}/{y}.pbf` | GeoJSON → MVT 编码；PMTiles(MVT) 直读；附 `tilejson.json` 与 `style.json` |
| **PMTiles** 归档 | `/pmtiles/{name}` | HTTP Range 直出，支持 `pmtiles://` 客户端（JS 库等） |
| **MBTiles** 归档 | `/mbtiles/{name}`、`/mbtiles/{name}/{z}/{x}/{y}[.ext]` | SQLite 瓦片包直读（PNG/JPEG/WebP 栅格或 gzip pbf 矢量）；归档信息 + XYZ 瓦片；同样可经 XYZ / TMS / WMTS / MVT 前端访问 |
| **COG** 直读 | `/cog/{layer}` | HTTP Range 直出，QGIS/rasterio 可远程打开；无 Range 时全量流式 |
| **3D Tiles** 服务 | `/3dtiles`、`/3dtiles/{name}/tileset.json`、`/3dtiles/{name}/{path...}` | `<catalog.root>/3dtiles` 下**每个 XML 一个服务**；Cesium 直连，预览页由 manager 提供 |
| **WFS** 2.0.0 / 1.1.0 / 1.0.0 | `/wfs` | GetCapabilities / DescribeFeatureType / GetFeature；GML 3.2(3.1) / GeoJSON / CSV 输出；bbox（CRS 轴序）/ featureID / count / startIndex / propertyName / resultType=hits；覆盖全部 `vector` 图层 |
| **OGC API - Features** 1.0.1 | `/ogc/features/v1` | landing / conformance / api(OpenAPI 3.0) / collections / items；GeoJSON + HTML；limit/offset/bbox 分页；声明 core/oas30/html/geojson 四个 conformance class |
| **OGC API - Tiles** 1.0.0 | `/ogc/tiles/v1` | landing / conformance / tileMatrixSets（预设 + 自定义网格）/ collections / tiles / tileset 元数据；瓦片字节复用 WMS 渲染路径（`{tileMatrix}/{tileRow}/{tileCol}` 顶行起算） |
| 运维 | `/health`、`/ready`、`/api/config`、`/api/cache`、`/api/cache/purge` | 探活（含运行模式）、当前配置、缓存统计、缓存刷新；预览 / UI 由 manager 托管 |

所有瓦片响应均带 `ETag`（FNV-1a）+ `304 Not Modified` + `Cache-Control`；PMTiles MVT 直读透传 `Content-Encoding: gzip` 与 `Vary`。

## 运行模式与归属管理端

map-api 是纯地图引擎，**不托管 UI**。`<manager/>` 配置声明「受哪个 manager 管理」：

```xml
<manager url="${MAPSERVER_MANAGER_URL:-http://127.0.0.1:6180}" probe_timeout_ms="1500"/>
```

启动时对 `<url>/health` 做一次短超时探活，结果写入启动日志与 `GET /health`
的 `mode` / `manager` 字段：

| 结果 | 模式 | 行为 |
|---|---|---|
| 探活成功 | `managed` | 界面与预览由该 manager 托管；能力列表给出 `/preview/*.html?...&base=<引擎地址>` 预览入口 |
| 失败 / 未配置 | `standalone` | 独立引擎，仅提供数据面、运维面、管理面；不挂任何页面路由，能力列表不含预览项 |

`url` 只用于服务端探活，不会下发到浏览器；预览页里的引擎地址由 manager 通过
`?base=` 注入（取自 manager 侧的引擎配置），跨机部署时该地址须对浏览器可达。
删除该配置段或把 `url` 留空即回到零配置的独立运行方式。

## 快速开始

环境：Rust 1.80+（本项目在 1.98.1 上验证）。

```powershell
# 1) 构建
cargo build --release

# 2) 生成示例 COG（fixtures/sample.tif，1024x512 渐变 RGB）
cargo run -p mapserver -- cog-create

# 3) 启动（默认读取 config/mapserver.xml，监听 0.0.0.0:8080）
cargo run -p mapserver -- serve

# 4) 数据面就绪探活（预览入口在 manager 控制台，引擎不托管页面）
#    curl http://localhost:8080/health
```

### 常用请求示例

```text
# WMS GetCapabilities
/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0

# WMS GetMap（1.3.0 轴序：BBOX = miny,minx,maxy,maxx）
/wms?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=world&CRS=EPSG:4326&BBOX=36,100,40,108&WIDTH=512&HEIGHT=512&FORMAT=image/png&TRANSPARENT=TRUE

# WMTS KVP 瓦片（WebMercatorQuad）
/wmts?REQUEST=GetTile&VERSION=1.0.0&LAYER=world&STYLE=default&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=6&TILEROW=27&TILECOL=50&FORMAT=image/png

# WMTS RESTful 瓦片
/wmts/rest/world/default/WebMercatorQuad/6/27/50.png

# TMS / XYZ
/tms/1.0.0/world/6/50/36.png
/tiles/world/6/50/27.png

# MVT + TileJSON + Style
/mvt/places/6/50/27.pbf
/mvt/places/tilejson.json
/mvt/places/style.json

# PMTiles / COG 直读（支持 Range）
/pmtiles/cities
/cog/world

# MBTiles 瓦片包（归档信息 + XYZ 瓦片；TMS 行内部翻转）
/mbtiles/test
/mbtiles/test/6/50/36.png
/tiles/test/6/50/36.png                          # MBTiles 层同样走 XYZ 前端

# 3D Tiles 服务（Cesium 直连）
/3dtiles                                  # 服务目录（JSON 清单）
/3dtiles/kmczc/tileset.json               # kmczc 服务的入口 tileset.json
/3dtiles/kmczc/Tile_1/tile.b3dm           # tileset.json 引用的瓦片文件
# WFS GetFeature（GML 3.2 默认输出；outputFormat=application/json 出 GeoJSON）
curl "http://localhost:8080/wfs?service=WFS&request=GetFeature&version=2.0.0&typeNames=places&count=10"

# OGC API - Features（collections + items 分页）
curl "http://localhost:8080/ogc/features/v1/collections"
curl "http://localhost:8080/ogc/features/v1/collections/places/items?limit=10&offset=0"

# OGC API - Tiles（tileset 元数据 + 瓦片，行号顶行起算与 XYZ 一致）
curl "http://localhost:8080/ogc/tiles/v1/collections/places/tiles/WebMercatorQuad"
curl -o tile.pbf "http://localhost:8080/ogc/tiles/v1/collections/places/tiles/WebMercatorQuad/6/27/50"

# 三维浏览页由 manager 托管（自动加载 /3dtiles 下全部服务）
#    manager 控制台 → 服务目录 → 内置预览
```

### 3D Tiles 服务（一个 XML = 一个服务）

在 `<catalog.root>/3dtiles`（缺省 `config/3dtiles/`）目录下，**每个 `*.xml` 配置文件定义一个
3D Tiles 服务**，启动时扫描加载（单个文件解析失败只告警跳过）。XML 格式：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<TilesetService>
  <Name>kmczc</Name>      <!-- 服务名，URL 段 /3dtiles/{Name}/...，缺省取文件名 -->
  <Title>kmczc 倾斜摄影（西安80 34度带 EPSG:2358）</Title>
  <Root>D:\data\3dtiles\kmczc_3dtiles</Root>  <!-- 3D Tiles 数据集根目录（必填） -->
  <Entry>tileset.json</Entry>                 <!-- 入口 json，缺省 tileset.json -->
</TilesetService>
```

说明：
- `Root` 支持 `%VAR%` / `${VAR}` 环境变量展开；文件访问严格限制在根目录内（拒绝 `..` 穿越）。
- Cesium 端直接 `Cesium3DTileset.fromUrl("http://host:8080/3dtiles/kmczc/tileset.json")`；
  已启用 CORS，跨域页面亦可加载。
- gis-wt 的 OSGB→3D Tiles 转换产物（gislab.osgb2tiles）可直接按上述方式发布。

### 工具 CLI

```powershell
# 打包目录 z/x/y.{pbf,png} -> .pmtiles
cargo run -p mapserver -- pmtiles-pack --input data/tiles --output data/cities.pmtiles --tile-type mvt --compression none
```

## 配置

**所有配置文件都是 XML**（经 `services/map-api/src/xmlcfg.rs` 桥接成 `serde_json::Value`
后按同一套 serde 结构体反序列化，因此字段名与早期的 YAML 写法一致）。

`config/mapserver.xml` 只放 **server 级**配置（监听、缓存、catalog 根）；
图层相关一律在图层配置文件里（`<catalog.root>/<类型>/`，**一图层一文件**）：

```xml
<!-- config/mapserver.xml —— 仅 server 级 -->
<config>
  <server host="0.0.0.0" port="8080"
          cache_mib="512" cache_ttl_secs="300" cache_stats_secs="60">
    <!-- cache_mib：输出/解码瓦片缓存上限（MiB，0=禁用）
         cache_ttl_secs：全部缓存统一 TTL（秒，0=不过期）
         cache_stats_secs：缓存统计日志 + 内部维护周期（秒，0=关闭）
         可选 public_url="http://example.com"：能力文档公布的 URL（缺省按请求 Host） -->
    <disk_cache dir="cache/foyer" capacity_mib="1024"/>
  </server>
  <catalog root="config" backend="files"/>
  <!-- backend: files（当前实现）| db（预留：数据库管理图层配置） -->
</config>
```

> 整档 PMTiles 直出（供 `pmtiles://` 客户端读取的归档映射）走 `<config>` 下的
> `archives` 段，与 `pmtiles/` 目录里的「图层」语义不同。

```xml
<!-- config/cog/world.xml —— 图层文件（source type 必须与目录名一致） -->
<layer name="world" title="World gradient COG" src_crs="EPSG:4326"
       style="config/styles/world.xml" tile_px="256">
  <source type="cog" path="fixtures/sample.tif"/>
  <grids>
    <item>WebMercatorQuad</item>
    <item>WorldCRS84Quad</item>
  </grids>
  <!-- 自定义 TileMatrixSet（定义随图层文件走，重名先到先得；数组/元组一律用 <item> 子元素）：
  <custom_grids>
    <item id="CGCS2000Geo" crs="EPSG:4490" tile_px="256">
      <origin><item>-180</item><item>90</item></origin>
      <extent><item>-180</item><item>-90</item><item>180</item><item>90</item></extent>
      <resolutions><item>0.703125</item><item>0.3515625</item></resolutions>
    </item>
  </custom_grids>
  -->
</layer>
```

XML → 内部结构的映射约定见 `services/map-api/src/xmlcfg.rs` 模块头（属性→同名键；
带 `id` 的子元素→以 id 为键的映射；子元素全为 `<item>`→数组；其余→以标签为键的对象）。

- 图层类型目录（`CatalogOpts::dir_source_type`）：`cog/`（光栅）、`vector/`（GeoJSON 矢量）、
  `pmtiles/`（PMTiles）、`mbtiles/`（MBTiles 瓦片包）、`postgis/`（PostGIS 动态矢量），
  每图层一个 `*.xml`；`3dtiles/` 每 `*.xml` 一个 3D Tiles 服务（另有 `terrain/` 放
  `*.terrainpack`）；`mvt/ osgb/ styles/` 为预留/辅目录。往预留目录里放 `*.xml`
  只会出一条 "catalog dir '…' is reserved; N layer file(s) ignored" 告警，不阻断启动。
- catalog 扫描只认 `*.xml` 扩展名（其他后缀直接忽略），**不再兼容 `.yaml`/`.yml`**；
  管理面新建/编辑服务同样落 `<catalog.root>/<kind>/<name>.xml`（写前做 XML 语法
  校验 + `.xml.bak` 备份，重载失败回滚）。
- `grids` 引用预设 `WebMercatorQuad`（z0–25）/ `WorldCRS84Quad`（z0–24）、
  带尺寸后缀（`WebMercatorQuad@512` 或 `WebMercatorQuad_512`，两种分隔符均可；
  实际瓦片像素取图层 `tile_px`），或本图层文件 `custom_grids:` 定义的
  自定义网格（经 `/tileMatrixSets/{id}` 提供）。
- 矢量层同时服务栅格协议（WMS/WMTS/TMS，内部栅格化）与 `/mvt/*` 矢量协议。
- 远程 COG 使用 `http(s)://` 路径，通过 HTTP Range 定位读取（含 S3 预签名 URL）。
- **PROJ 投影后端属环境配置**（不在 mapserver.xml）：解析顺序为
  显式参数（当前未暴露）→ 环境变量 `PROJ_DLL` / `PROJ_DATA` → 运行目录默认值
  （工作目录下的 `proj_9.dll` 与 `<工作目录>/share/proj`）→ **可执行文件旁**
  （`<exe目录>/proj_9.dll`、`<exe目录>/share/proj`、`<exe目录>/../share/proj`，
  对应安装包 `bin/` + `share/proj/` 布局，服务从任意 CWD 启动均可）。PROJ ≥ 8 均可
  （实测用 PostGIS 自带的 `libproj_8_2.dll` + `proj.db`）。未找到时只禁用**跨 CRS**
  互转，内置 4326↔3857 与「请求 CRS == 源 CRS」的原生产物直出不受影响。
  部署时**不要**设置 `PROJ_LIB`（系统 PostGIS 常指向旧版 proj.db），详见 `THIRD-PARTY-NOTICES.md`。

## 坐标系支持

坐标系统不是「只支持 4326/3857」：判定分三层。请求校验
（`gisserver::wms_common::crs::is_supported`）与实际可构造的变换（`gisserver::projection::Transformer::new_any`）
共用同一套规则，不会互相漂移；能力文档宣告的是地理系（`4326/4490/4214/4258`）
+ `3857` 加上图层自身的 `src_crs`（故 `EPSG:4543` 图层会带出 4543），
投影带不逐一列举（可达数十个码，且能否真建管线由运行时 PROJ 决定），
但按表内规则直接受理。

| 层级 | CRS | 是否依赖 PROJ | 说明 |
|---|---|---|---|
| 内置数学 | `EPSG:4326`、`CRS:84`、`EPSG:3857`（含 `900913`/`3785`） | 否 | 自写 Web Mercator，`MERC_MAX = 20037508.3427892`、`MAX_LAT = 85.0511287798066` |
| 恒等路径 | 任意 CRS，只要**请求 CRS == 源 CRS** | 否 | 同名映射无需投影数学；CGCS2000（4490）/ 高斯-克吕格带图层裸机即可直出本投影瓦片与 GetMap |
| PROJ 后端 | 地理系：`EPSG:4490`（CGCS2000）、`4214`（西安 80）、`4258`；投影带：EPSG `4491..=4554`（CGCS2000/北京 54/西安 80 的 3°、6° 高斯-克吕格）、`4800..=4999`、`32648..=32660`（WGS84 UTM 48N–60N） | 是（跨 CRS 时） | 具体码能否建管线由 PROJ 决定；不在表内的码在协议层即被拒 |

- **写法宽容度**：短名 `EPSG:4490`、URN `urn:ogc:def:crs:EPSG::4490`、带版本
  `...:EPSG:6.6:4490`、GML 旧式 `urn:x-ogc:def:crs:EPSG:4490`、OGC def-URI
  `http://www.opengis.net/def/crs/EPSG/0/4490` 同等接受（QGIS/ArcGIS/owslib 常发 URN）。
- **轴序**：WMS 1.3.0 地理 CRS（4326/4490/…）按规范纬度在前，投影系与
  `CRS:84` 经度在前；`CRS:84` 即便被折叠到 4326 也不交换（判定看原始标识）。
- **CGCS2000 经纬度网格**：预设网格只有 `WebMercatorQuad`/`WorldCRS84Quad`，
  4490 经纬度切片需图层自带 `custom_grids`（示例见 `config/cog/wh4490.xml` 的
  `CGCS2000Geo`，与 ArcGIS CGCS2000 缓存方案逐瓦号对齐），经
  `/tileMatrixSets/{id}`、`/tiles/{layer}/{grid}/…`、`/tms/1.0.0/{layer}/{grid}/…`、
  WMTS 与 OGC API - Tiles 提供。
- **矢量侧**：PostGIS 图层除 `EPSG:3857` XYZ 外还有源投影接口
  `/mvt/{layer}/{srid}/{z}/{x}/{y}.pbf`（服务端不做 `ST_Transform`），按**源 CRS
  自身单位**建矩阵：经纬度源（4326/4490/4214/4258）为 `2^(z+1)×2^z` 度数矩阵，
  瓦号与预设网格 `WorldCRS84Quad`（4490 为 `CGCS2000Quad`）逐瓦一致，TileJSON 的
  `grid` 也就宣告这两个网格名；投影源（高斯-克吕格/UTM，如 `4543`）为 `2^z×2^z`
  米制矩阵，世界范围取 ±20037508.342789244 米，使各地面分辨率与 WebMercatorQuad
  可比。接口段只接受 `3857`/`EPSG:3857`/`WebMercatorQuad*`、本图层源 SRID
  （`{srid}` 或 `EPSG:{srid}`）与本图层可解析的网格 id，其余一律 400；行列号越出
  该 zoom 矩阵返回 404。TileJSON 按接口分别宣告 bounds/crs。
- **错误语义**：无法兑现的 CRS 一律 4xx（WMS 回 `InvalidCRS`，WMTS 回
  `InvalidParameterValue`，瓦片端点回 400），不会伪装成 500 让客户端重试。

实测（`pwsh -File tools/crs_probe.ps1`，本机 PROJ 8.2 + 6.27GB 五华区 CGCS2000 COG，
图层 `wh4490` 源 CRS `EPSG:4490`）：

| 请求 | `proj_loaded=true` | `proj_loaded=false` |
|---|---|---|
| WMS GetMap `CRS=EPSG:4490`（恒等） | 200，100871 B PNG | 200，100871 B PNG（同一张图） |
| WMS GetMap `CRS=EPSG:4326` / `CRS:84` / URN 写法（跨 CRS） | 200，100871 B | 400 |
| WMS GetMap `CRS=EPSG:3857`（跨 CRS） | 200，100801 B | 400 |
| WMS GetMap `CRS=EPSG:4543`（CGCS2000 3° CM 102E，米制 bbox） | 200，109877 B | 400 |
| `/tiles/wh4490/CGCS2000Geo/10/1608/368.png`（恒等网格） | 200，83241 B | 200，83241 B |
| `/tms/1.0.0/…/CGCS2000Geo/10/1608/655`（行翻转） | 200，83241 B（与 XYZ 同瓦同字节） | 200，83241 B |
| `/wmts?…TILEMATRIXSET=CGCS2000Geo…` | 200，83241 B | 200，83241 B |
| `/tiles/wh4490/10/803/437.png`（3857 网格跨 CRS） | 200，28221 B | **400**（修复前为 500） |
| 反向：源 `EPSG:4326` 图层被请求为 4490 / 4543 | 200，100871 / 109877 B（与 4490 源图层同字节） | 400 |
| 不支持的 CRS（`EPSG:2326`、`EPSG:32647`、`EPSG:9999`） | 400 `InvalidCRS` | 400 `InvalidCRS` |

4490 与 4326 在同一 bbox 下出图字节完全一致（CGCS2000 与 WGS84 椭球差异
在亚米级），而米制高斯-克吕格 bbox 由 PostGIS `ST_Transform` 反算核对，说明跨
CRS 链路落在正确地面范围。

## 实现要点

- **HTTP 框架**：axum **0.8.9** + tower-http 0.6.11（`Cargo.toml` 的 workspace 依赖统一版本）。路由参数语法用 axum 0.8 的花括号形式 `{param}` / 通配 `{*rest}`——**0.7 的 `:param` / `*rest` 在 matchit 0.8 下不是编译错误，而是注册路由时直接 panic**（`Path segments must not start with ':'`），即"编译通过 ≠ 能跑起来"。因此 `handlers::tests::routes_use_axum08_brace_syntax` 会扫描全部路由字面量，禁止旧语法回流。`Path<(A, B)>` 元组提取器的元素个数必须与路由的参数段数严格一致。中间件若要可选地读取 `ConnectInfo`，用 `Option<Extension<ConnectInfo<T>>>`——axum 0.8 移除了 `Option<T>` 的 blanket `FromRequestParts` 实现（改为 `OptionalFromRequestParts`）。
- **COG 解析（自研）**：经典 TIFF + BigTIFF（magic `II`/`MM`、42/43），IFD 链，跳过 `NewSubfileType` bit 2 之外的子文件；标签覆盖宽度/高度/位深/压缩/光度/每样本位数/预测器/平铺/`ModelPixelScale`/`ModelTiepoint`；`read_window_rgba` 按 overview 缩放因子选层、rayon 并行解码。
- **瓦片解码**：compression 1 / 5(LZW) / 8·32946(DEFLATE) / 50000(ZSTD)；predictor 2（8/16bit，16bit 分端序）、3（float 反字节差分+解交织）；1bit 位图展开；16bit 无符号 `>>8`、有符号重映射、float 钳制到 8bit。JPEG/WEBP 瓦片零填充并在响应中说明不支持。
- **渲染**：输出像素中心 → native 像素 → 选中 overview 层，两次重采样（nearest/bilinear），全出界返回透明，瓦片对齐窗口 +1px margin。
- **渲染投影快速路径**：请求 CRS 与源 CRS 相同（identity）且源 geotransform 为仿射时，output→native 像素映射整体双线性，由 4 个输出角点求解 + 画布中点投影校验（偏差 >0.05px 则回退逐像素）。512×256 GetMap 的 262,144 次逐点投影降为 5 次：**8.94ms → 3.11ms（2.9×，release micro-bench，`cargo test --release -p mapserver -- --nocapture pass1`）**；快速路径与逐像素参考逐点一致（单测断言 <1e-6）。
- **投影**：EPSG:4326 ↔ EPSG:3857 自写（无 proj），`MERC_MAX = 20037508.3427892`、`MAX_LAT = 85.0511287798066`；`CRS:84`/`OGC:CRS84` 归一化为 `EPSG:4326`，`900913`/`3785` 归一化为 `EPSG:3857`；CGCS2000（4490）与高斯-克吕格 / UTM 分带经可选 PROJ 后端互转，同名 CRS 走恒等路径不依赖 PROJ（见《坐标系支持》）。渲染侧只依赖 `gisserver::wms_common::reproject::Reprojector` trait，具体后端由服务层注入。
- **并发模型**：渲染/编码/文件 IO 一律走 `tokio::task::spawn_blocking`（tokio worker 不被阻塞，health 等轻请求零排队）；本地文件与 HTTP Range 源全部定位读（Windows `seek_read` / Unix `read_at`，不改共享 fd 游标，并发无竞态）；**远程源**的同步 HTTP 读取跑在专用 rayon 池（8 线程），慢源不会占满本地 COG 解码的全局 rayon 池。
- **缓存（foyer，`services/map-api/src/cache.rs` 的 `CacheHub` 统一收口；output 为 `HybridCache`，`server.disk_cache` 可选磁盘 L2 跨重启保留）**：五段缓存——① 输出瓦片（`output`，weigher 按字节计量，预算 `cache_mib`）；② 图层索引（`layers`，name → `Arc<LayerRuntime>`，替代 `AppState::layer` 的线性扫描，未命中名不做负缓存）；③ 元数据文档（`docs`，能力文档/OGC API 文档/TileJSON/style，按 `(服务,版本,base)` 等全量入 key，字节预算 32MiB）；④ 目录元数据（`catalog`，OGC API collections/tiled collections/advertised grids 单条目，构建时零 FeatureCollection clone）；⑤ 要素键值缓存（`features`，`(collectionId, featureId)` → 单要素 GeoJSON，替原整表 clone+线性扫描）。另保留解码瓦片缓存（`gisserver::storage::TileCache`，容量按**实际最大 tile_px** 估算，杜绝 512px 层的 4 倍内存超配）。
- **缓存刷新机制（三层）**：① **TTL**——`server.cache_ttl_secs`（默认 300s）从写入时刻起算（非空闲计时），数据集换盘后最多一个 TTL 窗口自动刷新；② **手动 purge**——`POST /api/cache/purge?scope=all|output|docs|catalog|features|layers`（`all` 同时清解码瓦片缓存），返回清理条目数；`GET /api/cache` 输出各缓存 entries/bytes/TTL/purge 计数；③ **周期维护**——`cache_stats_secs`（默认 60s）的 tokio 任务落一条统计日志（foyer 按访问驱逐，无需强制维护，0=关闭）。条目计数用 `entries()`（`clear()` 后同步归零）；`usage()` 为异步记账，仅作近似展示。
- **SLD 符号渲染（`crates/renderer/src/sld.rs` + `vector.rs`，零 XML 依赖）**：完整文档模型（NamedLayer → UserStyle → FeatureTypeStyle → Rule → Symbolizer），GeoServer 语义求值——所有匹配规则的符号器按画家序叠加（面填充→面描边→线→点→标注），`ElseFilter` 在同 FTS 无前置命中时生效；Filter 支持 6 种比较 / Between / Like（`*` `?` 转义）/ IsNull / And / Or / Not（数值与字符串自动判别；空间过滤与 `ogc:Function` 解析为不匹配并一次性告警）；Min/MaxScaleDenominator 按帧计算比例尺分母（0.28mm 标准像素）；PolygonSymbolizer（fill/opacity/stroke/width/dasharray）、LineSymbolizer（含虚线）、PointSymbolizer（circle/square/triangle/star/cross/x/arrow/hexagon/pentagon 九种 mark，fill+stroke 双通道）、TextSymbolizer（Label 属性/字面量 + Font 字号/粗体 + Halo 白晕 + AnchorPoint/Displacement，内置 5×7 位图字体，`font.rs`）。图层带同名 `.sld`（见 `Layer::sld`）即走 styled 路径，否则回退 legacy 平面样式（MapLibre JSON paint / 默认绿）；WMS GetMap 与瓦片路由同源。样例：`config/vector/places.sld`（pop 分类 + 名称标注 + ElseFilter + 虚线）。
- **GetLegendGraphic**：SLD 图层按 (rule, symbolizer) 逐行渲染符号；legacy 图层按几何类型渲染；栅格图层渲染调色板色带；支持 `layer`/`style`/`rule`/`format=image/png`/`width`/`height`；未传 `height` 时高度随样式行数自适应（`20 + 18×行数`，上限 512，多规则样式不会被压成 2px 行）。
- **MVT**：spec 2.1，extent 4096，ZigZag/varint 自编码，key/value 去重字典；编码前按瓦片 lon/lat bbox 做保守预过滤（bbox 不相交的要素直接跳过，不再全量 clone + 全量编码）。
- **PMTiles**：v3 127B header、Hilbert TileID（含 conformance 向量测试）、列式 varint 目录（5 列）、根目录/叶子目录二分查找、gzip 目录解码（上限 256MiB）、打包器（Hilbert 聚类、blob 去重、可选 gzip 重编码）；共享 fd 定位读（并发隔离有单测：8 线程 × 16 瓦片交叉校验）。
- **MBTiles**：SQLite（rusqlite bundled，不依赖系统 sqlite3.dll）；启动时只读打开校验 `tiles` 表并解析 `metadata`（format/bounds/minzoom/maxzoom）；只读连接池（每归档最多 8 条常连，`prepare_cached` 复用语句，出错连接不回收），读取在 `spawn_blocking` 内完成；TMS→XYZ 行翻转 `tile_row = (2^z - 1) - y`；矢量包 gzip 魔数透传 `Content-Encoding: gzip`。MBTiles 层自动获得 XYZ / TMS / WMTS / MVT 全部前端路由。
- **压缩策略**：`CompressionLayer` 只压缩文本类响应；image/*、`application/x-protobuf`、MVT、二进制 3D Tiles 分块跳过二次 gzip（tile 是流量主体，二次压缩纯耗 CPU）。
- **启动预检**：每个图层源路径先做存在性检查（远程 http(s) 除外），坏图层**一次列全**后按跳过告警处理、其余图层正常服务（2026-10-23 起为跳过策略，与 3D Tiles 注册表一致）；仅当所有图层都打不开时才启动失败——不再第一个坏路径就中断、其余问题半潜没。

## 性能优化指南

本项目的设计基调是「**瓦片优先、客户端渲染优先、服务端零拷贝直通优先**」。下面的章节按"选哪种服务 → 怎么配缓存 → 运行时为什么快 → 客户端怎么配合 → 怎么压测调优"组织，所有数字均来自 `实现要点` 与 `真实数据实测`。

### 1. 选服务的性能排序（调用方式决定延迟量级）

| 优先级 | 服务 / 调用方式 | 延迟量级 | 服务端做什么 | 何时用 |
|---|---|---|---|---|
| ⭐⭐⭐ | **MVT 矢量瓦片** `/mvt/{layer}/{z}/{x}/{y}.pbf` | 首片 1–3s（PostGIS 查询+编码），缓存命中 **16–19ms** | 客户端渲染，服务端只出 protobuf | 动态矢量（建筑/地块），要缩放跟手、样式可换 |
| ⭐⭐⭐ | **XYZ 栅格瓦片** `/tiles/{layer}/{z}/{x}/{y}.png` | **10–21ms/片** | 客户端贴图，服务端只解码该瓦片 | COG/栅格底图，要即时刷新 |
| ⭐⭐⭐ | **COG / PMTiles / MBTiles 直读** `/cog` `/pmtiles` `/mbtiles` | 206 部分读，按请求字节出 | **零解码**，HTTP Range 直通（PMTiles 还透传 gzip） | 大影像/归档，避免服务端整图解码 |
| ⭐⭐ | **WMTS / TMS**（走瓦片矩阵） | 同 XYZ，但带 KVP/REST 解析开销 | 复用瓦片渲染路径 | 需要标准 OGC 瓦片接口、老客户端 |
| ⚠️ 兜底 | **WMS GetMap** `/wms?REQUEST=GetMap` | 单帧 3–9ms（快速路径 2.9×），但**每请求服务端现渲** | 服务端按 bbox 重投影+重采样+编码整图 | 仅当客户端只能用 WMS、或单帧出图；**别拿它做底图缩放** |
| — | **WFS / OGC API - Features** | 内存过滤，秒级以下 | 内存 GeoJSON 查询（无 IO） | 要素查询/属性检索，非瓦片渲染 |
| — | **3D Tiles / terrain** `/3dtiles` `/terrain` | 文件 Range 直通 | 零解码，Cesium 直连 | 三维/地形 |

**关键认知**：早前实测中"COG、动态矢量瓦片都卡顿、缩放不刷新"，根因就是前端对**所有图层用了 WMS GetMap 服务端现渲**——每一次缩放/平移都要服务端重出整图，且缩放不刷新。修复后前端按图层自动选最快源（矢量→MVT、栅格→XYZ、其余→WMS 兜底），卡顿消失。所以**性能的第一杠杆是"调用方式"，不是"调参"**。

### 2. 缓存体系（性能的第二杠杆）

统一由 `services/map-api/src/cache.rs` 的 `CacheHub` 收口（foyer `HybridCache`），共**五段 + 一段解码缓存**：

| 缓存段 | key | 预算 | 作用 |
|---|---|---|---|
| `output` | 输出瓦片 | `server.cache_mib` | 渲染/编码结果，命中即免重渲（MVT 命中 16–19ms 即此） |
| `layers` | 图层名 → `Arc<LayerRuntime>` | — | 替代线性扫描，未命中名不做负缓存 |
| `docs` | 能力文档 / OGC 文档 / TileJSON / style | 32MiB | 元数据文档零克隆 |
| `catalog` | OGC collections / tiled collections | — | 构建时零 FeatureCollection clone |
| `features` | `(collection, featureId)` → 单要素 | — | 替整表 clone+线性扫描 |
| `decoded_tiles`（`gisserver::storage::TileCache`） | 解码后 RGBA | 按实际最大 `tile_px` 估算 | 杜绝 512px 层 4 倍内存超配 |

**三层刷新**：
- **TTL**：`server.cache_ttl_secs`（默认 300s），从写入时刻起算，数据集换盘后最多一个 TTL 窗口自动刷新；
- **手动 purge**：`POST /api/cache/purge?scope=all|output|docs|catalog|features|layers`（`all` 同时清解码缓存），返回清理条目数；`GET /api/cache` 看各段 entries/bytes/TTL；
- **周期维护**：`server.cache_stats_secs`（默认 60s）落统计日志（foyer 按访问驱逐，无需强制维护，0=关）。

**配置建议**：`cache_mib` 给足（如 512–2048，看机器内存）；矢量图层这种"首片慢、命中快"的场景，TTL 设长一点（如 3600）收益最大；`server.disk_cache` 可开可选磁盘 L2，跨重启保留 output 缓存。

### 3. 运行时为什么快（并发模型）

- **渲染/编码/文件 IO 一律 `spawn_blocking`**：tokio worker 不被阻塞，`/health` 等轻请求零排队。
- **本地文件与 HTTP Range 源全部定位读**（`seek_read` / `read_at`，不改共享 fd 游标）→ 并发无竞态，GB 级 COG/PMTiles 不整读进内存。
- **远程源同步 HTTP 读跑在专用 rayon 池（8 线程）**：慢源不会占满本地 COG 解码的全局 rayon 池（解码与远程取数隔离）。
- **渲染投影快速路径**：请求 CRS 与源 CRS 相同且 geotransform 仿射时，262,144 次逐点投影降为 5 次 → **8.94ms → 3.11ms（2.9×）**，与逐像素参考逐点一致（<1e-6）。
- **MVT 编码前 bbox 预过滤**：瓦片经纬度 bbox 不相交的要素直接跳过，不再全量 clone + 全量编码。

### 4. 数据源性能要点

- **COG**：`read_window_rgba` 按 overview 缩放因子选最粗可用层、rayon 并行解码；金字塔选择失效（曾修过的 `desired_factor` bug）会让全图请求永远解码 level0 全分辨率——务必确认 overview 链完整。
- **PostGIS**：按 DSN **共享连接池**，版本预检 PG≥11/PostGIS≥3.0；低 z 服务端 `ST_SimplifyPreserveTopology` 分级简化（`simplify`）；`from` 表达式发布 SQL/视图免落表；启动自检 GiST 索引、bounds/count 走统计估计（缺失回退精确扫描）。
- **矢量（GeoJSON）层**：整表常驻内存（`LayerRuntime::vector_fc`），WFS/OGC API 查询零 IO，`vector-query` 做 bbox 过滤/分页/编码（GML/GeoJSON/CSV）。
- **MBTiles**：SQLite 只读连接池（每归档 ≤8 常连，`prepare_cached` 复用语句），读取在 `spawn_blocking` 内完成。

### 5. 客户端侧怎么配合

- **自动选源**：`manager/ui/public/preview/viewer.html` 按图层探测最快源（MVT/XYZ/WMS 兜底），并把 `VectorTile` 的 `tileGrid` 显式设为 `createXYZ({tileSize:256})`（OpenLayers 默认 512 会与服务器 256 网格错位导致要素偏移/空白）。
- **ETag + 304**：所有瓦片响应带 FNV-1a `ETag` + `Cache-Control`，未变即 304，浏览器/CDN 二次命中零传输。
- **Range 206**：COG/PMTiles/3D Tiles 走 `Accept-Ranges: bytes`，客户端（QGIS/rasterio/Cesium）按需取字节，不全量下载。按 RFC 9110 §14.2 支持 `a-b`、`a-`、`-N`（suffix）三种形态，`end` 越界收敛到文件尾，起点≥长度回 **416 + `Content-Range: bytes */total`**；多区间或无法解析的 Range 一律忽略并回 200 全量（不报错、不重试死循环）。
- **压缩策略**：`CompressionLayer` 只压文本类响应；`image/*`、`application/x-protobuf`、MVT、二进制 3D Tiles **跳过二次 gzip**（瓦片是流量主体，二次压缩纯耗 CPU）。

### 6. 运维调优清单

```powershell
# 启动后先看缓存与内存
curl http://localhost:8080/api/cache            # 各段 entries/bytes/TTL
curl http://localhost:8080/health               # 探活
curl http://localhost:8080/ready                # 就绪（图层加载完）

# 数据集换盘后手动刷新（或等一个 TTL 窗口）
curl -X POST "http://localhost:8080/api/cache/purge?scope=all"

# HTTP 层压测（QPS / P50 / P99 / RSS）
python tools/bench_server.py --label before
```

调优优先级：**① 确认前端走 MVT/XYZ 而非 WMS → ② 给足 `cache_mib` 并设长 TTL → ③ 确认 COG overview 链完整、PostGIS 有 GiST 索引 → ④ 用 bench_server 测 P99，针对性扩 rayon/连接池。**

## 明确不支持 / 限制

- 坐标系统：内置仅 `EPSG:4326`/`CRS:84`/`EPSG:3857` 与「请求 CRS == 源 CRS」的
  恒等路径；CGCS2000（4490）/ 高斯-克吕格 / UTM 等其他 CRS 的**跨 CRS 互转**
  需要运行时提供 PROJ 后端（见《坐标系支持》），未提供时这类请求回 400。
  不支持任意 PROJ 码表之外的 CRS（如 `EPSG:2326`、不在接受区间的码）。
- COG 瓦片压缩不支持 JPEG(6/7)、WEBP(50001)（解码时零填充）；矢量源仅 GeoJSON（不做 FGB/GPKG/PostGIS）。
- 单层渲染：WMS GetMap 多图层合成未实现（图层须存在，渲染第一个图层）。
- SLD 边界：仅图层同名 `.sld` 文件（不支持每请求 `SLD=`/`SLD_BODY=`，WMS `STYLES` 仅 default）；ExternalGraphic 图标（无 PNG 解码器）、RasterSymbolizer ColorMap（栅格调色板走 JSON `Style`）、空间过滤、`ogc:Function`、`<Geometry>`、PerpendicularOffset、linecap/linejoin 精确控制、uom（metre）尺寸、文字斜体/自动避让暂不支持；详见 `docs/service-comparison-geoserver-2026-10-24.md`。
- 无鉴权/SSL 终结（建议置于反向代理之后）；线程模型为 axum/tokio + rayon 解码池。

## 测试与验证

```powershell
cargo test --workspace                                    # 单元测试（27 个套件）
cargo test --release -p mapserver -- --nocapture pass1     # 渲染投影 micro-bench
python tools/verify_fixes.py                              # 启动预检 + 压缩策略 + 全协议回归
python tools/wfs_ogcapi_probe.py                         # WFS + OGC API-Features/Tiles 实测（64 项）
python tools/capability_probe.py                          # 深度探针：@512 网格 / 3dtiles 穿越防护 / 错误语义 / 并发
pwsh -File tools/crs_probe.ps1 -Layer wh4490              # 坐标系能力实测（CGCS2000 / 高斯-克吕格 / 无 PROJ 降级）
python tools/bench_server.py --label before                # HTTP 层压测（QPS/P50/P99/RSS）
```

单测覆盖：COG 写入-回读、LZW/预测器、MVT（含 bbox 预过滤与瓦片经纬范围）、PMTiles Hilbert/打包/**并发查找隔离**、投影 round-trip、affine 快速路径 vs 逐像素参考、路由匹配。

`verify_fixes.py` 实测（release）：坏 catalog 启动失败且两个坏层一次列全（`data file not found`）；PNG/MVT 瓦片带 `Accept-Encoding: gzip` 不压缩、WMS caps XML 仍压缩；health/caps/TMS（含地理 grid）/XYZ/WMTS REST+KVP/MVT/TileJSON/GetMap/GetFeatureInfo/MBTiles/COG Range 206/ETag 304 全过。

### 真实数据实测（test_cog.tif，≈703MB）

用 `E:\data_km\testdata_jn24\test_cog.tif`（20221×13295、16bit×3 波段、DEFLATE、predictor=2、512 瓦片、7 级金字塔，CGCS2000 / 3-degree Gauss-Kruger zone 34 米制投影）实测：

- 服务器启动即完成 COG 元数据解析（IFD 链 + 1040 瓦片索引表），WMS GetMap 全幅出图 200（PNG 可见影像内容），GetFeatureInfo 采样返回真实灰度值（56–174），TMS/XYZ/WMTS/KVP 端点 200，COG Range 206。
- 实测修复的缺陷：
  1. **GetFeatureInfo 未处理 WMS 1.3.0 轴序**（BBOX=northing-first）→ 修复后采样正确；
  2. **16bit 渲染黑图**：`scale_to_u8` 固定 `>>8` 会把"伪 16bit"数据（真实值 0–255，仅少量溢出）渲染成全黑 → 改为按瓦片内高字节非零样本占比判定（>5% 用 `>>8`，否则取低字节）；
  3. **解码卡死**：`chunks().take().fold()` 统计最大值在 debug 构建大瓦片上极慢 → 改为两遍普通 `for` 循环；
  4. **overview 选择失效**：`desired_factor` 误用恒定像素分量求跨度，导致全图请求永远解码 level0 全分辨率 → 分量对调后正确选择金字塔层。
- 限制说明修正（原写“服务器仅支持 EPSG:4326/3857，需先 `gdalwarp` 重投影”）：
  该结论已过期。现版本对 CGCS2000 / 高斯-克吕格分带的支持见《坐标系支持》：
  同名 CRS 直接恒等直出，跨 CRS（↔ 4326/3857）由 PROJ 后端完成，无需预处理
  重投影（仅当目标机器不能部署 PROJ 时，才需要预先 `gdalwarp` 到 4326/3857）。

## 许可证 / License

| 组件 | 许可证 | 闭源商用 |
|---|---|---|
| **gisserver 核心库**（`../gisserver`） | MIT OR Apache-2.0 | ✅ 允许，无需授权 |
| **mapserver 服务端**（本项目） | AGPL-3.0-only + 商业授权例外 | ⚠️ 需购买商业授权（见下） |

### AGPL-3.0 与商业授权例外

mapserver 默认采用 **AGPL-3.0-only** 授权——这是一种「网络 copyleft」许可证：

- ✅ **免费场景**：内部使用、个人学习、开源项目集成 → 直接用 AGPL-3.0，无任何费用
- ⚠️ **商用 SaaS**：如果你把 mapserver 作为 SaaS 服务对外提供（如在线 GIS 平台、云端瓦片服务），AGPL-3.0 要求你向**所有网络用户**提供完整的服务端源代码
- ⚠️ **闭源集成**：如果你想把 mapserver 嵌入到自己的商业 GIS 产品中，且不希望公开衍生代码

上述两种场景需要购买**商业授权**（Commercial License Exception），购买后可：

1. **闭源使用**：无 copyleft 约束，无需公开任何代码
2. **无限分发**：可自由复制、修改、分发给客户、合作伙伴
3. **定制开发**：按需加企业级功能（集群部署、读写分离、监控面板等）
4. **技术支持**：优先 bug 修复、性能调优、架构咨询

> 商业授权详情见 [LICENSE 末尾](#commercial-license-exception) 的完整条款。

### 核心库 gisserver 的独立许可

核心库 `gisserver`（纯 Rust，无 GDAL 依赖的 COG/瓦片/MVT 引擎）采用 **MIT OR Apache-2.0** 双许可——这是**最宽松的开源许可**：

- ✅ 可闭源集成到任何商业产品（包括 C++ DLL、C# NuGet、Python wheel 等桥接层）
- ✅ 无需购买任何授权
- ✅ 无需保留衍生代码开源义务
- ✅ Apache-2.0 额外提供**明确的专利授权**（消除侵权风险）

### 商业合作 / 收购咨询

本项目的核心技术卖点（**纯 Rust 自研 COG/BigTIFF 解析、零 GDAL/tiff/mapbox 重型 C 绑定、高性能瓦片渲染与缓存**）可闭源授权或整体收购：

- **企业级 GIS 厂商**（如超图、中地数码、Esri 中国）→ 可将自研内核集成到商业 GIS 平台，替换旧 GDAL 依赖
- **云服务商**（如阿里云、腾讯云）→ 可作为 GIS 云服务的瓦片/OGC API 引擎
- **创业团队/个人开发者** → 可基于商业授权快速推出垂直 SaaS 产品
- **整体收购** → 核心库 + 服务端 + 管理后台全栈技术交接

**联系方式**：rex yan 853740877@qq.com

---

Copyright (c) 2026 rex yan 853740877@qq.com and contributors.