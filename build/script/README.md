# rust-gisserver 编译与安装说明（Windows）

本文件说明在 Windows 上编译、安装并运行 rust-gisserver（mapserver + manager）所需的
**环境、路径、步骤、注意点、产物与安装清单**。所有脚本统一放在 `build\script\`，
编译产物统一落在 `build\target\`，安装目录默认 `..\..\..\rmap_target`。

---

## 1. 目录与路径约定

```
rust-gisserver/                 <- 仓库根（下文称 <repo>）
├─ mapserver/                   <- map-api 服务（Rust）
│  └─ config/                   <- 服务端配置（mapserver.xml、env/、mbtiles/…）
├─ manager/                     <- 管理后台
│  ├─ backend/                  <- manager 服务（Rust，cargo workspace 成员）
│  │  └─ config/                <- 后台配置（manager.xml、manager.dev.xml、manager.probe.xml）
│  └─ ui/                       <- 前端（Vue + Vite），构建后产物 ui/dist
├─ gisserver/                   <- 核心库（crate）
├─ build/
│  ├─ script/                   <- ★ 本目录：所有 bat / ps1 / 清单都在此
│  │  ├─ build_all.bat          <- 编译 mapserver + manager（可选前端）
│  │  ├─ install.bat            <- 收集 exe + PROJ 运行期资产到安装目录
│  │  ├─ uninstall.bat          <- 卸载
│  │  ├─ fix_staticdir.ps1      <- 安装时把 manager.xml 的 static_dir 改写为 web
│  │  └─ PROJ_RUNTIME_MANIFEST.csv  <- PROJ 运行期 DLL / 数据清单（不入库，仅记录）
│  └─ target/                   <- ★ 编译产物（CARGO_TARGET_DIR 重定向到此）
│     └─ release\
│        ├─ map-api.exe
│        └─ manager.exe
└─ (上层) gis-wt\thirdparty\rmap_target   <- ★ 默认安装目录（<repo> 往上 3 级）
```

| 项 | 路径 | 说明 |
|----|------|------|
| 脚本目录 | `<repo>\build\script\` | 本文件所在目录，`%~dp0` |
| 编译产物 | `<repo>\build\target\release\` | 由 `build_all.bat` 设 `CARGO_TARGET_DIR` 重定向 |
| 默认安装目录 | `<repo>\..\..\..\rmap_target` | 即 `<gis-wt>\thirdparty\rmap_target` |
| PROJ 资产候选源 | `<gis-wt>\build`、`<gis-wt>\dist`、`<gis-wt>\thirdparty\GDAL3.13\dll` | 逐级查找 proj_9.dll / proj.db |
| 前端源码 | `<repo>\manager\ui\` | 需 `pnpm build` 产出 `ui\dist` |
| 服务端配置源 | `<repo>\mapserver\config` | 拷贝到 `<安装目录>\config` |
| 后台配置源 | `<repo>\manager\backend\config` | 拷贝到 `<安装目录>\config\manager` |

> `.gitignore`：`build/target/`、`mapserver/target/`、`manager/target/` 均被忽略；
> PROJ 运行期 DLL（`proj_9.dll` 等）与 `share/` 数据**不入库**，仅以 `PROJ_RUNTIME_MANIFEST.csv` 记录清单。

---

## 2. 编译所需环境

| 组件 | 要求 | 备注 |
|------|------|------|
| 操作系统 | Windows 10 / 11 x64 | |
| Rust 工具链 | rustup stable，host = `x86_64-pc-windows-msvc` | 需 `cargo` / `rustc` 在 PATH |
| MSVC 编译工具 | Visual Studio 2019/2022 Build Tools 或 Community/Professional | 提供 `cl.exe` / `link.exe`；`build_all.bat` 会 `call vcvars64.bat` 注入环境 |
| Windows SDK | 含 `kernel32.lib` 等导入库与头文件（本机 `10.0.26100.0`） | `LIB`/`INCLUDE` 必须含 `Windows Kits\10\Lib\<ver>\um\x64`、`ucrt\x64` 及对应 `Include`；**vcvars 有时漏注入 SDK 路径，见 §4 LNK1181** |
| Node.js + pnpm | 可选，仅构建前端时需要 | `build_all.bat frontend` 或手动 `pnpm install && pnpm build` |
| PROJ 运行期资产 | `proj_9.dll` + 9 个传递依赖 + `proj.db` | **不在 git 内**，须从 `gis-wt\build` / `dist` / `GDAL3.13\dll` 置备（详见 §6 与清单 CSV） |
| VC++ 运行时 | VC++ 2015-2022 可再发行包 | `MSVCP140` / `VCRUNTIME140` / `VCRUNTIME140_1`。目标机已装则可运行；未随包分发时安装会 WARN（见 §4） |
| PostgreSQL | 可选（manager 后台用） | 缺省可用文件侧边（file-sidecar）模式运行 |

### 2.1 Windows SDK 路径（本机实测）
- SDK 根：`C:\Program Files (x86)\Windows Kits\10\`
- 本机版本：`10.0.26100.0`
- 需加入 `LIB`：`...\Lib\10.0.26100.0\um\x64`、`...\Lib\10.0.26100.0\ucrt\x64`
- 需加入 `INCLUDE`：`...\Include\10.0.26100.0\um`、`...\Include\10.0.26100.0\shared`、`...\Include\10.0.26100.0\ucrt`
- `build_all.bat` 会在 vcvars 未注入时**自动探测最新含 `kernel32.lib` 的 SDK 版本**并追加，无需手改。

---

## 3. 编译与运行步骤

> 所有命令在 `build\script\` 目录下以 **PowerShell 或 cmd 直接运行 `.bat`**（不要用 Git Bash 调 `cmd /c`，
> 沙箱里会把路径里的 `\r` 当回车转义导致失败）。

### 3.1 编译
```powershell
cd <repo>\build\script
.\build_all.bat            # 仅编译 mapserver + manager
# 或同时构建前端：
.\build_all.bat frontend   # 额外在 manager/ui 跑 pnpm/npm install + build
```
- `build_all.bat` 设 `CARGO_TARGET_DIR=<repo>\build\target`，产物落在 `build\target\release\`。
- 内部依次：`mapserver`（`cargo build --release -p mapserver`）、`manager`（`cargo build --release` in `manager/backend`）。
- 成功标志：`build\target\release\map-api.exe` 与 `manager.exe` 生成。

### 3.2 安装
```powershell
cd <repo>\build\script
.\install.bat                       # 默认装到 ..\..\..\rmap_target
# 或指定目录：
.\install.bat D:\some\path\rmap_target
```
安装流程（install.bat 内部 6 步）：
1. 建目录结构（bin/config/data/cache/logs/share/proj）。
2. 拷贝 `map-api.exe`、`manager.exe` → `bin\`。
3. 拷贝 PROJ：`proj_9.dll` + 9 个传递依赖 DLL → `bin\`（按 §1 候选源逐级查找）；`proj.db` → `share\proj\`。
4. 拷贝配置：`mapserver\config` → `config\`；`manager\backend\config` → `config\manager\`，并复制一份 `manager.xml` 到 `config\` 根；随后调用 `fix_staticdir.ps1` 把 `static_dir` 改写为 `web`。
5. 拷贝前端：`manager\ui\dist` → `web\`（未构建则 WARN，UI 会 404）。
6. 生成 `.env.example`、`env\` 预设、`start-mapserver.bat`、`start-manager.bat`。

### 3.3 配置
```powershell
cd <安装目录>
copy .env.example .env
notepad .env
```
关键变量（见 `.env.example`）。判据：`xmlcfg.rs::expand_env` 对 `${VAR}` 未设置时展开为**空串**，
`${VAR:-默认}` 未设置时取默认值——所以无默认值的占位符就是必需项。

| 级别 | 变量 | 读取位置 | 未设置后果 |
| --- | --- | --- | --- |
| **必需** | `MANAGER_DB_PASSWORD` | `config/manager/manager.xml` 的 `<database url>` | DSN 口令为空，manager 连不上库 |
| **必需** | `MANAGER_JWT_SECRET` | 同上 `<security jwt_secret>` | 签名密钥为空串，登录态不可用 |
| **必需** | `MAPSERVER_DB_PASSWORD` | `config/mapserver.xml` 的 `<admin dsn>` | `/admin/*` 管理面连库失败 |
| 可选 | `MANAGER_DB_HOST` / `MANAGER_DB_PORT` | manager DSN | 默认 `localhost` / `5434` |
| 可选 | `MAPSERVER_DB_HOST` / `MAPSERVER_DB_PORT` | map-api admin DSN | 默认 `localhost` / `5434` |
| 可选 | `MAPSRV_HOST` / `MAPSRV_PORT` | manager 反代 map-api | 默认 `127.0.0.1` / `8080` |
| 可选 | `MAPSERVER_MANAGER_URL` | map-api 回连 manager（跨机部署） | 默认 `http://127.0.0.1:6180` |
| 可选 | `GIS_ENCODING_REGION` / `GIS_ENCODING_LOCALE` | `gisserver::encoding` | 默认 `zh-CN` |
| 可选 | `DATA_ROOT` | `<server data_root>` 及 `archives` 路径前缀 | **默认 `data`**，即启动工作目录下的 `data\`（安装脚本已创建）；仅当数据放在外置盘时才需要设，例如 `DATA_ROOT=E:\data\1` |
| 可选 | `TESTDATA_ROOT` / `SHPPGSQL_BIN` | e2e 数据根 / shp2pgsql 路径 | 不影响正常服务 |

- `data_root` 写的是 `${DATA_ROOT:-data}`：不设变量即以相对路径 `data` 生效。`start-*.bat`
  先 `cd /d` 到安装目录，所以它等价于 `<安装目录>\data`；源码开发从 `mapserver\` 启动时
  等价于 `mapserver\data`（该目录留了 `.gitkeep`，真实数据已被 `.gitignore` 挡住）。
  路径守卫会 `canonicalize`，目录必须存在——用安装脚本或保留 `.gitkeep` 即可。
- `PROJ_DLL` / `PROJ_DATA` / `PROJ_LIB` 由 `start-*.bat` 自动设置，无需手填。
- **已失效、代码不再读取**：`MAPSERVER_PORT`（端口取 `config/mapserver.xml` 的 `<server port>` 或 CLI `--listen`）、
  `MAPSERVER_DATA_DIR`、`MAPSERVER_CACHE_DIR`、`API_KEY`，别再补这几个。

### 3.4 启动
```powershell
cd <安装目录>
.\start-mapserver.bat   # map-api，端口 8080
.\start-manager.bat     # manager，端口 6180
```
- `start-*.bat` 会先 `cd /d` 到脚本所在目录、`set PROJ_LIB=`（清空，见 §4）、`set PROJ_DLL`、`set PROJ_DATA`，再启动 exe。
- manager 后台固定监听 `0.0.0.0:6180`（见 `config\manager.xml` 的 `addr`），`base_api=/manager-api`。

### 3.5 注册为 Windows 服务（可选，NSSM）
```powershell
nssm install map-api  "<安装目录>\bin\map-api.exe"
nssm install manager "<安装目录>\bin\manager.exe"
nssm set map-api AppDirectory "<安装目录>"
nssm set manager AppDirectory "<安装目录>"
nssm start map-api
nssm start manager
```
**必须设 `AppDirectory`**：`data_root` 默认是相对路径 `data`，且路径守卫会
`canonicalize`。NSSM 不设时工作目录是 exe 所在的 `bin\`，`data` 会解析成
`bin\data`（不存在）并让管理面报 500 `data_root 不可访问`。同理，任何自定义
启动方式（systemd / 计划任务）都要把工作目录设为安装目录，或显式设
`DATA_ROOT` 为绝对路径。

---

## 4. 注意事项 / 已知坑

### 4.1 LNK1181：找不到 `kernel32.lib`
- **现象**：`link.exe` 报 `LNK1181: cannot open file 'kernel32.lib'`。
- **原因**：`LIB` 里没有 Windows SDK 的 `um\x64` / `ucrt\x64` 导入库（vcvars 偶尔漏注入）。
- **处理**：`build_all.bat` 已内置兜底——自动探测最新含 `kernel32.lib` 的 SDK 版本并追加到 `LIB`/`INCLUDE`。若仍失败，确认 Windows SDK 已安装（§2.1 路径）。

### 4.2 `PROJ_LIB` 必须清空，`PROJ_DATA` 指向自带 `proj.db`
- **原因**：机器级 `PROJ_LIB` 可能指向 PostGIS 自带的 `proj.db`，其 layout 版本过旧，会导致 PROJ 初始化失败：
  `DATABASE.LAYOUT.VERSION.MINOR = 2 whereas a number >= 5 is expected`。
- **处理**：`start-mapserver.bat` / `start-manager.bat` 里均 `set "PROJ_LIB="`，并 `set "PROJ_DATA=<安装目录>\share\proj"`，让 PROJ 只用随包自带的 `proj.db`。**不要**把机器级 `PROJ_LIB` 传进去。

### 4.3 `static_dir` 必须改写为 `web`
- **现象**：源码 `manager.xml` 里 `static_dir="manager/ui/dist"`（或 `../ui/dist`），指向开发态前端路径。
- **处理**：安装时由 `fix_staticdir.ps1` 把两个 `manager.xml`（`config\manager.xml`、`config\manager\manager.xml`）的 `static_dir` 统一改写为 `web`，对应安装后的 `web\` 目录。
- **注意**：若安装后手动改了配置，需保证 `static_dir="web"`，否则 manager 静态资源 404。改写采用独立 `.ps1` 而非内联 PowerShell，是为了规避 `cmd` 在括号块内展开路径与双引号的引号地狱。

### 4.4 前端未构建 → `web\` 为空 → UI 404
- `install.bat` 仅拷贝已存在的 `manager\ui\dist`。若没构建，安装只 WARN，`web\` 缺失或为空，浏览器打开管理后台会 404。
- **处理**：先 `build_all.bat frontend`，或到 `manager\ui` 手动 `pnpm install && pnpm build`，重跑 `install.bat`。

### 4.5 三个 VC++ 运行时 WARN 是预期的
- 安装时 `MSVCP140.dll` / `VCRUNTIME140.dll` / `VCRUNTIME140_1.dll` 三个会 WARN（它们来自 VC++ 可再发行包，不由 PROJ 源提供）。
- **不影响运行**：只要目标机装了 **VC++ 2015-2022 可再发行包** 即可。如需随包分发，把这 3 个 DLL 放到上述 PROJ 候选源目录之一，`install.bat` 会自动收集进 `bin\`。

### 4.6 PROJ 资产不入库
- `proj_9.dll` 等 10 个 DLL 与 `proj.db` **不进 git**（见 `.gitignore`）。换机器/干净检出时，必须从 `gis-wt\build` / `dist` / `GDAL3.13\dll` 置备，否则 `bin\` 缺 `proj_9.dll`、PROJ 坐标变换不可用（`/health` 返回 `proj:false`）。

### 4.7 `manager.xml` 注释乱码（上游源问题，非安装引入）
- 源 `manager\backend\config\manager.xml` 的 XML 声明为 `encoding="UTF-8"`，但注释字节实为 GBK，被读取时显示乱码。
- 这是**上游源文件本身的编码不一致**，安装拷贝/改写不改变配置值（server addr、static_dir、db url 均为 ASCII/合法），不影响解析与运行。如需根治，应修正上游源文件的编码声明或注释文本。

---

## 5. 编译最终生成的文件

经 `build_all.bat` 后，产物统一在 `<repo>\build\target\release\`：

| 文件 | 来源 crate | 说明 |
|------|-----------|------|
| `map-api.exe` | `mapserver` | 地图/瓦片服务主程序 |
| `manager.exe` | `manager/backend` | 管理后台服务 |

> 其余各 crate 的 `.rlib` / 中间产物也落在 `build\target\`（不再散落 `mapserver/target`、`manager/target`）。
> `build/target/`、`mapserver/target/`、`manager/target/` 均在 `.gitignore` 中。

---

## 6. 安装目录文件清单

默认安装到 `..\..\..\rmap_target`（即 `<gis-wt>\thirdparty\rmap_target`）。实测结构：

```
rmap_target/
├─ bin\
│  ├─ map-api.exe              # 主服务
│  ├─ manager.exe              # 管理后台
│  ├─ proj_9.dll               # PROJ 9 主库（坐标变换引擎）
│  ├─ sqlite3.dll              # proj.db 底层存储
│  ├─ libcurl.dll              # 网络功能
│  ├─ tiff.dll                 # GeoTIFF/栅格
│  ├─ zlib.dll                 # 压缩
│  ├─ libssh2.dll              # SSH/SFTP
│  ├─ brotlidec.dll            # brotli 解码
│  ├─ brotlicommon.dll         # brotli 公共
│  ├─ jpeg62.dll               # JPEG 解码
│  ├─ libcrypto-3-x64.dll      # OpenSSL
│  └─ (可选) MSVCP140 / VCRUNTIME140 / VCRUNTIME140_1   # VC++ 运行时，随包分发时才有
├─ share\proj\
│  └─ proj.db                  # PROJ 坐标系数据库（LAYOUT.MINOR=6，PROJ 9 要求 >=5）
├─ web\                        # 前端静态资源（177 个文件，来自 manager/ui/dist）
├─ config\
│  ├─ manager.xml              # 根配置（static_dir="web"）
│  ├─ mapserver.xml            # map-api 配置
│  ├─ manager\                 # 后台配置（manager.xml / manager.dev.xml / manager.probe.xml）
│  ├─ env\                     # .env.zh-CN / .env.zh-TW / .env.en / .env.ko + README
│  ├─ mbtiles\ mvt\ osgb\ pmtiles\ vector\   # 各模块 README / SLD 样式
├─ data\                       # 数据目录（TPK / MBTiles 等）
├─ cache\                      # 响应缓存
├─ logs\                       # 日志
├─ .env.example                # 配置模板
├─ start-mapserver.bat         # 启动脚本（清 PROJ_LIB、设 PROJ_DATA）
└─ start-manager.bat           # 启动脚本
```

### 6.1 PROJ 运行期依赖（详细，对应 `PROJ_RUNTIME_MANIFEST.csv`）

| 文件 | 是否必需 | 作用 |
|------|---------|------|
| `proj_9.dll` | 是 | PROJ 9 主库，坐标变换核心 |
| `sqlite3.dll` | 是 | proj.db 读取 / 网格索引底层存储 |
| `libcurl.dll` | 是 | PROJ 网络数据源（proj-network） |
| `tiff.dll` | 是 | GeoTIFF / 栅格支持（传递依赖） |
| `zlib.dll` | 是 | 压缩（libcurl / tiff 依赖） |
| `libssh2.dll` | 是 | SFTP/SSH 传输（libcurl 依赖） |
| `brotlidec.dll` | 是 | brotli 解码（libcurl 依赖） |
| `brotlicommon.dll` | 是 | brotli 公共代码 |
| `jpeg62.dll` | 是 | JPEG 解码（tiff 依赖） |
| `libcrypto-3-x64.dll` | 是 | OpenSSL（libcurl / SSH 依赖） |
| `proj.db` | 是 | PROJ 坐标系数据库，置 `share\proj\proj.db` |
| `MSVCP140.dll` | 是（运行时） | VC++ 运行时，来自可再发行包（非 PROJ 源） |
| `VCRUNTIME140.dll` | 是（运行时） | 同上 |
| `VCRUNTIME140_1.dll` | 是（运行时） | 同上 |

> 即：**`proj_9.dll` + 9 个传递依赖 = 10 个 PROJ 相关 DLL**，加上 `proj.db`；
> 另 3 个 VC++ 运行时预期以 WARN 形式跳过（除非随包置备）。

---

## 7. 卸载

```powershell
cd <repo>\build\script
.\uninstall.bat                 # 默认卸 ..\..\..\rmap_target
# 或指定：
.\uninstall.bat D:\some\path\rmap_target
```
会先 `taskkill` 停止 `map-api.exe` / `manager.exe`，确认后 `rmdir /S /Q` 删除整个安装目录。

---

## 8. 故障排查

| 现象 | 可能原因 | 处理 |
|------|---------|------|
| `LNK1181 kernel32.lib` | SDK 导入库不在 `LIB` | §4.1；`build_all.bat` 已兜底探测 SDK |
| PROJ 加载失败（`/health` 返回 `proj:false`） | `bin\` 缺 `proj_9.dll` 或 `share\proj\proj.db` 缺失 | 从 PROJ 候选源置备（§1、§6.1）；确认不在干净检出下遗漏 |
| `DATABASE.LAYOUT.VERSION.MINOR = 2 whereas ... >= 5` | 继承了机器级旧 `PROJ_LIB` | §4.2；用 `start-*.bat` 启动（已清 `PROJ_LIB`） |
| 管理后台 UI 404 | `web\` 为空（前端未构建） | §4.4；构建前端后重跑 `install.bat` |
| 静态资源 404 / 路径错 | `static_dir` 未改为 `web` | §4.3；确认 `config\manager.xml` 的 `static_dir="web"` |
| 启动报缺少 `MSVCP140.dll` 等 | 目标机没装 VC++ 可再发行包 | 安装 VC++ 2015-2022 可再发行包，或把 3 个 DLL 置备到 PROJ 候选源随包分发 |
| 端口被占用 | 8080 / 6180 被占 | 释放端口，或修改 `.env` / `manager.xml` 的 `addr` 后重装 |
