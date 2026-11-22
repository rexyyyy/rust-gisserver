# 架构说明

rust-gisserver 是一套地图服务管理平台，采用单仓多组件（monorepo）结构：核心库、地图引擎、管理控制台与构建安装脚本分目录维护。

## 1. 仓库分层

```text
rust-gisserver/
├── gisserver/      核心库：唯一发布到 crates.io 的包（单包 + features 门控）
├── mapserver/      地图引擎 workspace：services/map-api（AGPL-3.0-only）
├── manager/        管理控制台 workspace：backend（Rust）+ ui（Vue 3）
├── build/          构建、安装、卸载脚本与编译产物目录
├── installscript/  数据库初始化 SQL 与安装辅助脚本
└── run-env/        运行时安装目录：config / web / 日志 / 可执行文件
```

- `gisserver`、`mapserver`、`manager` 版本号统一为 `0.8.x`。
- `mapserver` 与 `manager` 通过 path 依赖引用 `gisserver`；发布时 Cargo 自动剥离 path，改用 crates.io 版本。

## 2. gisserver 核心库

`gisserver` 是单包结构，功能域以 `src/<module>` 组织，并通过 Cargo features 按需编译，避免引入无关依赖。

| 模块 | 职责 |
| --- | --- |
| `wms_common` | 公共基础类型：Bbox、CRS、Layer、Style、TileMatrixSet |
| `wms_protocol` | WMS/WMTS 请求解析与能力文档生成 |
| `wfs_protocol` | WFS 协议响应类型 |
| `ogcapi_features` / `ogcapi_tiles` | OGC API Features / Tiles |
| `encoding` | DBF/属性编码检测（`encoding_rs`，可选 Windows ANSI 回退） |
| `projection` | 内置 4326↔3857 转换，可选 PROJ 后端 |
| `storage` | RangeSource 抽象与瓦片/元数据缓存 |
| `cog` | COG/BigTIFF 解析、解码与写入 |
| `renderer` | 重采样、调色板、PNG 编码与矢量栅格化 |
| `mvt` | MVT 2.1 编码器 |
| `pmtiles` | PMTiles 归档读取与打包 |
| `arcgis_cache` | ArcGIS 缓存（TPK/Bundle）解析 |
| `pg_source` | PostGIS 数据源与连接池 |
| `vector_query` | 矢量要素查询与过滤 |

Feature 约定：

- `default`：`encoding`、`mvt`、`projection`、`vector-query`
- `full`：启用全部功能域，供 `mapserver` 使用
- 各模块均有独立 feature，按需组合，保持核心零 FFI（PROJ 可选）

## 3. mapserver 地图引擎

`mapserver` 是 Cargo workspace，当前仅包含 `services/map-api` 一个 HTTP 服务。

- 路由：`handlers/`（协议端点）与 `admin/`（管理接口）
- 配置：XML 驱动，`config/mapserver.xml` 为入口，`config/<type>/*.xml` 定义图层
- 服务类型：WMS、WMTS、TMS、XYZ、MVT、MBTiles、PMTiles、COG、3D Tiles、ArcGIS 缓存、WFS、OGC API
- 缓存：内存缓存 + 磁盘 L2 两层结构，支持 TTL 与按区清理
- 热重载：`POST /admin/reload` 重新加载配置，无需重启进程
- 坐标参考：内置常用 CRS 转换，自定义坐标系通过自定义 grid 与 LOD 配置实现

## 4. manager 管理控制台

`manager` 为 Cargo workspace（`backend`）+ 前端工程（`ui`）。

**backend（axum + tokio-postgres）**

- 登录鉴权、验证码、会话与权限校验
- 用户 / 角色 / 菜单 / 部门 / 岗位管理
- 数据服务：工作空间、数据源、数据图层、发布账本
- 地图服务：服务列表与重载、SLD 样式库、MVT 样式、缓存、引擎、API Key、访问日志
- PostGIS 数据源：连接测试、表/几何列发现、SHP 导入（同步 + SSE 进度）
- 安全：DSN 落库加密（DES / SM4）、错误信息密码掩码、SQL 标识符转义
- i18n：响应中间件按字典翻译 JSON 字符串，非 JSON 响应自动跳过

**ui（Vue 3 + Element Plus + OpenLayers）**

- 用户管理、数据服务、地图服务、系统监控四大板块
- 动态路由与菜单由后端下发，支持中英文实时切换
- MVT 预览采用纯客户端样式渲染，SLD 仅作用于 WMS 栅格出图与图例

## 5. 关键数据流

### 栅格瓦片（COG，TMS/WMTS/XYZ 共用）

1. 路由把 `(grid, z, col, row)` 换算为请求 CRS 的 BBOX；
2. `renderer` 计算输出像素中心与源像素映射，选择合适 overview；
3. `cog::read_window_rgba` 并行解码瓦片窗口，命中缓存则直接复用；
4. 应用调色板与 nodata 透明规则，编码 PNG 并返回 `ETag/304`。

### 矢量瓦片（MVT）

1. 读取 GeoJSON / PostGIS 要素；
2. 坐标转换到 Web Mercator，再换算为 extent 4096 的瓦片局部坐标；
3. 按 MVT 2.1 编码几何、属性字典与 feature 元信息；
4. 客户端（OpenLayers）按后端下发的样式 JSON 动态渲染。

### PMTiles / MBTiles

- PMTiles：解析 127 字节头与目录，按 TileID 二分定位；归档可通过 Range 直出；
- MBTiles：SQLite 只读打开，读取 metadata 与瓦片数据。

### PostGIS

- 连接信息加密存储，发布时优先使用发现阶段返回的 schema、table、geometry_column；
- 几何列 SRID 为 0 时回退到用户选择的 CRS，解析失败直接报错；
- 瓦片查询使用参数化 SQL，标识符统一双引号转义。

## 6. 配置与运行时

- **双配置目录**：`mapserver/config` 为源码侧配置，`run-env/config` 为运行中服务实际读取的配置；修改后需同步两处。
- **热重载**：图层或服务配置变更后调用 `/admin/reload` 即可生效。
- **数据库**：PostgreSQL（PostGIS 扩展），初始化脚本自动启用 `postgis` 与 `fuzzystrmatch`。
- **部署**：`build/script/build_all.bat` 编译，`install.bat` 安装到 `run-env`，`uninstall.bat` 卸载。

## 7. 版本与发布

- 核心库 `gisserver` 发布至 crates.io，当前版本 `0.8.1`；
- 仓库地址：<https://github.com/rexyyyy/rust-gisserver>；
- `mapserver`、`manager` 作为应用组件随仓库发布，不单独上传 crates.io。
