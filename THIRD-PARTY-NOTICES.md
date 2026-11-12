# r-mapserver 第三方许可证通知

生成日期：2026-11-11 · 依据：安装包产物实测 PE 导入表解析（`scripts/_pe_tree.py`）+ 运行时模块快照。

**适用范围**：r-mapserver 本仓（`map-api`、`manager`）及其 Windows 安装包（`thirdparty/r-mapserver-install/`）。本仓代码采用 **MIT License**（全文见 `LICENSE`）。主仓 gis-wt 为 AGPL-3.0-only，其 `THIRD-PARTY-NOTICES.md` 不覆盖本仓，见该文件 §「适用范围」。

## 1. 本项目

- 许可证：**MIT License**，全文见 `LICENSE`。
- 版权声明：`Copyright (c) 2026 rex yang <13556388@qq.com> and contributors.`
- 商业友好：本仓代码可闭源、可商用，与主仓 gis-wt 的 AGPLv3 义务相互独立。

## 2. 运行时二进制依赖

r-mapserver 为**纯 Rust 实现**，不使用 GDAL / gis_core / libpq。`map-api.exe` 与 `manager.exe` 静态链接 Rust 运行时，`rusqlite` 启用 `bundled` 特性（libsqlite3 静态编入），`tokio-postgres` 为纯 Rust 实现。

唯一的动态二进制依赖是 **PROJ 投影库**（`crates/projection/src/proj.rs` 通过 `LoadLibraryExW` + `GetProcAddress` 手动加载），用于 WMS/WMTS 动态重投影。PROJ 在运行期按需加载，缺失时 map-api 仍可启动，仅重投影不可用。

| DLL | 版本 | 许可证（SPDX） | 角色 |
| :--- | :--- | :--- | :--- |
| `proj_9.dll` | 9.6.2 | MIT | PROJ 主体（WMS/WMTS 重投影） |
| `sqlite3.dll` | 3.53.4 | blessing（Public Domain） | PROJ 读取 `proj.db` |
| `libcurl.dll` | 8.21.0 | curl（MIT-like） | PROJ 网络坐标服务 |
| `tiff.dll` | 4.7.0 | libtiff（BSD-3-Clause-like） | PROJ 栅格网格 |
| `jpeg62.dll` | 2.62.0 (libjpeg-turbo) | IJG + Zlib 双许可 | libtiff 的 JPEG 编解码 |
| `zlib.dll` | 1.3.2 | Zlib | 通用压缩 |
| `libssh2.dll` | 1.11.1 | BSD-3-Clause | libcurl 的 SSH 传输后端 |
| `libcrypto-3-x64.dll` | 3.6.4 | Apache-2.0 | OpenSSL 3.x（libssh2 的加密后端） |
| `brotlidec.dll` / `brotlicommon.dll` | 1.1.0 | MIT | Brotli 解码（libcurl 传输编码） |
| `MSVCP140.dll` / `VCRUNTIME140.dll` / `VCRUNTIME140_1.dll` | 14.38.33135.0 | Microsoft Visual C++ Runtime（MICROSOFT VISUAL C++ RUNTIME 有限使用条款） | C++ 运行时 |

### 2.1 部署布局约束（重要）

- `proj_9.dll` 及其 12 个传递依赖**必须位于同一目录**。`LoadLibraryExW(LOAD_WITH_ALTERED_SEARCH_PATH)` 仅在 `proj_9.dll` 自身目录内解析其导入项，依赖散落会导致 `LoadLibraryExW` 返回 NULL。
- 安装包布局：`map-api\share\*.dll`（PROJ 全套）+ `map-api\share\proj\proj.db`（数据）；`map-api\VCRUNTIME140.dll` 与 `manager\VCRUNTIME140.dll` 与各自 exe 同级（exe 自身的一层导入）。
- `map-api.exe` 与 `manager.exe` 的一层导入仅含 `VCRUNTIME140.dll` 与系统 DLL；`api-ms-win-crt-*` 为 Windows API Set 转发桩，由系统提供。

### 2.2 PROJ 数据目录（proj.db）

`proj.db` 为 SQLite 数据库，其 `metadata` 表中的 `DATABASE.LAYOUT.VERSION.MINOR` 必须满足所配 PROJ 版本的要求：

| PROJ 版本 | 要求 LAYOUT.MINOR | 本项目自带 proj.db |
| :--- | :--- | :--- |
| 9.6.2（`proj_9.dll`） | ≥ 5 | **5** ✓ 匹配 |

**部署要点**：服务启动脚本必须显式设置 `PROJ_DATA` 指向自带的 `share\proj`，并**清除 `PROJ_LIB`**。Windows 机器上常见的 `PROJ_LIB=C:\Program Files\PostgreSQL\16\share\contrib\postgis-3.5\proj` 指向 PostGIS 附带的旧版 `proj.db`（LAYOUT.MINOR=2），会导致 PROJ 报错中止：

```
proj.db contains DATABASE.LAYOUT.VERSION.MINOR = 2 whereas a number >= 5 is expected. It comes from another PROJ installation.
```

`PROJ_DATA` 优先级高于 `PROJ_LIB`；实测在保留系统级 `PROJ_LIB` 的情况下，只要 `PROJ_DATA` 正确指向自带 `proj.db`，坐标转换仍正常。

**判定提示**：服务日志中的 `proj_loaded=true` 仅代表 DLL 加载成功，**不代表数据库可用**——LAYOUT 版本错配时该标志仍为 true。真正的判据是 `proj_create_crs_to_crs` 是否成功。

### 2.3 交付义务

- 全部组件均为宽松许可（MIT / BSD / Apache-2.0 / Zlib / Public Domain），随附 DLL 与本文件即可满足声明义务。
- **OpenSSL 3.x（Apache-2.0）**：须保留其 `NOTICE` 与许可声明；本项目未修改 OpenSSL 源码。
- **Microsoft Visual C++ Runtime**：按 Microsoft Visual C++ Runtime 的有限使用条款再分发，随包附带的 `VCRUNTIME140*.dll` / `MSVCP140.dll` 属该条款允许的应用本地部署（app-local deployment），不得修改。
- 若将来替换或新增 PROJ 传递依赖，须同步更新本文件与 `thirdparty/licenses/`。

## 3. 许可证原文索引

| 许可证 | 文本位置 |
| :--- | :--- |
| MIT | 主仓 `thirdparty/licenses/MIT.txt` |
| BSD-3-Clause | 主仓 `thirdparty/licenses/BSD-3-Clause.txt` |
| Apache-2.0（OpenSSL 等） | 主仓 `thirdparty/licenses/Apache-2.0.txt` |
| Zlib | 主仓 `thirdparty/licenses/Zlib.txt` |
| IJG + Zlib（libjpeg-turbo） | 主仓 `thirdparty/install/share/doc/libjpeg-turbo/LICENSE.md` |
| curl | https://curl.se/docs/copyright.html |
| Microsoft Visual C++ Runtime | https://learn.microsoft.com/en-us/visualstudio/vs/redist/ |

## 4. 源码参考与授权链条（2026-11-12 补充）

`crates/pmtiles` 依据 PMTiles v3 公开格式规范实现：127 字节头、五列 ULEB128 目录页、offset sentinel、Hilbert TileID 排序、gzip 段。字段布局、编码规则与曲线排序属公开事实与算法思想，不受著作权保护。

| 项目 | 许可证 | 用途 | 是否复制源码 |
| :--- | :--- | :--- | :--- |
| **RustGIS** `https://github.com/opengeos/RustGIS`（`crates/tiles/src/pmtiles.rs`） | **Apache-2.0**（顶层 `LICENSE`、`[workspace.package] license = "Apache-2.0"`） | 2026-11-12 对 header 字段偏移、目录列顺序、象限顺序做**格式行为交叉验证** | **否**。实测本仓 `crates/pmtiles/src/*.rs` 与该文件共同行最高 **0.4%**，未复制其源码 |

因未复制源码，Apache-2.0 §4 的源码归属义务严格来说不触发；此处保留项目名与许可证标识，作为事实致谢与尽调追溯依据。该参考实现经本地全量比对，与受核查的 MPL-2.0 上游源码相似度最高 0.9%，确认为独立实现，其 Apache-2.0 声明可信。

过程证据（比对原始输出、重写前后相似度、功能验证与法理边界）见 `docs/PMTiles净室重写证据链-2026-11-12.md`，属内部留档，不进发布包。

## 5. 变更记录

- 2026-11-11：首次生成。随 Windows 安装包补齐 PROJ 全套二进制依赖（12 个传递依赖 DLL），并明确 `PROJ_DATA` / `PROJ_LIB` 部署约束。
- 2026-11-12：新增 §4 源码参考与授权链条（PMTiles v3 公开格式规范实现与 Apache-2.0 参考实现的事实致谢）。
