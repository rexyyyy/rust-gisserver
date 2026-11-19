# 一键部署（开发计划 P0-1 · 第一阶段）

数据底座容器化编排。`compose` 拉起 **PostGIS + map-api（发布平面）+ manager（管理平面）**，
gis-srv（能力底座 / MCP）以 profile 占位、默认不启动。

**按语言 × 平台分目录**：本目录（`deploy/`）只放公共资产（config 模板、`.env.example`、
一键脚本 `.ps1`/`.sh`、文档）。真正的 compose 入口有 **4 个**，语言决定是否种中/英基线，
平台决定用哪套一键脚本；两者交叉，各自独立 compose 项目、卷互不共享：

| 目录                       | 平台   | compose 项目名           | DB 首次初始化                                   |
|----------------------------|--------|--------------------------|-------------------------------------------------|
| `../deploy_zh-cn/win/`     | Windows| `gis-data-base-zh-win`   | `system.sql → map.sql`（中文基线）              |
| `../deploy_zh-cn/linux/`   | Linux  | `gis-data-base-zh-linux` | `system.sql → map.sql`（中文基线）              |
| `../deploy_en/win/`        | Windows| `gis-data-base-en-win`   | `system.en_US.sql → map.en_US.sql`（英文基线）  |
| `../deploy_en/linux/`      | Linux  | `gis-data-base-en-linux` | `system.en_US.sql → map.en_US.sql`（英文基线）  |

四个目录的镜像与服务定义完全一致，差别仅在：① 首建库种子语言（语言级 `dbinit/`）；
② 一键脚本平台（`.ps1` / `.sh`）；③ compose 项目名（隔离卷）。路径都用正斜杠，
Windows 与 Linux 的 compose 文件本身可通用，分目录是为了各自独立的 `.env`、`gisdata/`
和卷，切换语言/平台不会互相覆盖。

## 快速开始

Linux（中文、在项目根目录执行）：

```bash
cd manager/deploy_zh-cn/linux
cp ../../deploy/.env.example .env    # 必填 MANAGER_JWT_SECRET；其余项均有默认值
docker compose up -d --build
```

Windows（中文）：

```powershell
cd manager/deploy_zh-cn/win
copy ..\..\deploy\.env.example .env
docker compose up -d --build
```

换英文库把路径里的 `deploy_zh-cn` 改成 `deploy_en`；换平台把 `linux` 与 `win` 对调。

或用一键脚本（读所选目录的 `.env` → up → 等健康 → 自检），脚本统一放在公共 `deploy/`：

```bash
# Linux：--dir 相对 deploy/ 解析，指向平台目录
bash deploy/start.sh --dir deploy_zh-cn/linux
bash deploy/start.sh --dir deploy_en/linux
bash deploy/start.sh --dir deploy_zh-cn/linux --no-build       # 不重建镜像
```

```powershell
# Windows：-Dir 相对 deploy/ 解析，指向平台目录
pwsh -File deploy\start.ps1 -Dir ..\deploy_zh-cn\win
pwsh -File deploy\start.ps1 -Dir ..\deploy_en\win
```

- 前端：<http://localhost:6180/>
- 服务健康：<http://localhost:8080/health>
- WMS 能力：<http://localhost:8080/wms?service=WMS&request=GetCapabilities>

## 切换语义（语言 × 平台）

- 四个目录是**独立 compose 项目**（pgdata / 缓存卷互不共享）：切换语言或平台 = 到另一
  目录直接 `up`，不需要 `down -v`，数据互不覆盖。
- 端口（6180 / 8080 / MAPSERVER_DB_PORT 默认 5434）四套全部相同，**不能同时运行**。
- DB 种子语言只在**首次建库**时写入；运行期接口 msg 由 manager 后端按请求头
  `X-Language` 动态 zh→en，与库种子无关。英文库上中文请求的 DB 文本（菜单/字典）
  会显示英文（中间件无 en→zh 路径），故英文目录面向纯英语环境。
- 标签类列宽已统一 64（nick_name / notice_title），中英文种子都放得下。

## 架构与端口

| 服务      | 镜像 / 构建                          | 宿主端口 | 说明                        |
|-----------|--------------------------------------|----------|-----------------------------|
| postgis   | `postgis/postgis:16-3.4`             | MAPSERVER_DB_PORT→5432 | 单库 `mapserver-manager`（默认 5434，宿主被占时在平台目录 .env 改） |
| map-api   | 仓库根上下文 + `mapserver/Dockerfile` | 8080     | WMS/WMTS/WFS/MVT/COG/PMTiles… |
| manager   | `manager/` 上下文 + `Dockerfile`      | 6180     | 管理平面 + 前端静态资源     |

容器网络内互访用服务名：manager→`http://map-api:8080`，两服务→`postgis:5432`。

## Linux 构建说明

两个 Dockerfile 构建的目标镜像都是 `debian:bookworm-slim`（Linux 原生），Windows 与 Linux
宿主机**共用同一份**，无需为 Linux 另写。要点：

- **Rust 工具链**：用 USTC 镜像源（`RUSTUP_DIST_SERVER` / `RUSTUP_UPDATE_ROOT`）装并钉在
  1.98.1，绕开国内拉 `rust:1-slim-book` manifest 被 DENIED 的问题；只装 minimal profile，
  依赖里 rusqlite 走 bundled、ureq/tokio-postgres 无 TLS，故只需 C 工具链、无需 OpenSSL。
- **前端**：`node:22-slim` + pnpm 12，`pnpm install --frozen-lockfile`（`pnpm-lock.yaml` 已入库）。
- **配置模板**：entrypoint 用 `envsubst`（gettext-base）按环境变量渲染；`sed -i 's/\r$//'`
  归一化 CRLF，因此源码在 Windows 上编辑过也不会破坏 Linux 容器内的 shebang。
- **构建上下文**：四个平台 compose 的 `map-api` 用仓库根（含 `gisserver` path 依赖，见根
  `.dockerignore`），`manager` 用 `manager/`（见 `manager/.dockerignore`）。相对路径已按平台
  子目录深度调整（`../../..` / `../..`），在任一平台目录 `docker compose config` 可校验。
- **shell 脚本行尾**：根目录 `.gitattributes` 强制 `*.sh text eol=lf`，确保 `start.sh` /
  `validate.sh` 在 Windows 克隆后仍是 LF，拷到 Linux 直接可执行。
- **PROJ（跨 CRS 重投影）**：map-api 镜像内 `apt-get install libproj25 proj-data`，
  并把 `libproj.so.25` 软链到固定路径 `/usr/local/lib/libproj.so`，设
  `PROJ_DLL=/usr/local/lib/libproj.so`、`PROJ_DATA=/usr/share/proj`；entrypoint 启动时
  `unset PROJ_LIB`（旧变量名，常被系统 PostGIS 的旧 proj.db 占用）。
  manager 镜像不装 PROJ（后端不依赖 gisserver，无投影需求）。

## PROJ 与 proj.db：怎么保证装上了

**为什么必须装**：PROJ 是运行期 `dlopen` 的可选后端，不参与链接。缺失时 map-api
**照样能启动**，但只有内置的 EPSG:4326 ↔ EPSG:3857 与「源 CRS == 请求 CRS」的恒等
变换可用；CGCS2000（4490）、高斯-克吕格分带（45xx）、UTM 等跨 CRS 互转会返回
`unsupported CRS ... needs the PROJ backend`。这类故障表现为「服务正常但某些图层
请求失败」，不查日志很难归因。

**镜像里的 PROJ**（`mapserver/Dockerfile` runtime 阶段）：

| 项 | 值 |
|----|----|
| 运行时库 | Debian `libproj25` → `/usr/lib/<triplet>/libproj.so.25` |
| 软链 | `/usr/local/lib/libproj.so`（代码按裸名 `libproj.so` 查找；Debian 运行时包**不含**裸名软链，那是 `-dev` 包的事） |
| 数据 | Debian `proj-data` → `/usr/share/proj/proj.db` |
| 环境变量 | `PROJ_DLL=/usr/local/lib/libproj.so`、`PROJ_DATA=/usr/share/proj` |

版本说明：Debian bookworm 为 PROJ 9.1.1，`proj-data` 与 `libproj25` 同源同版本，
不会出现 Windows 安装包那种 `proj.db` LAYOUT 版本错配（Windows 侧是 PROJ 9.6.2 +
自带 proj.db，要求 `DATABASE.LAYOUT.VERSION.MINOR ≥ 5`）。

**怎么验证装上了**（三层，从快到慢）：

1. **看 `/health`** —— map-api 的健康端点直接返回 `proj` 布尔字段：

   ```bash
   curl -s http://localhost:8080/health
   # {"status":"ok","proj":true,"mode":"standalone","manager":null}
   ```

   `proj:true` = 库已加载；`proj:false` = 没装成功。

2. **看容器日志** —— entrypoint 启动时会打印 PROJ 自检结果：

   ```
   [entrypoint] PROJ lib: /usr/local/lib/libproj.so
   [entrypoint] PROJ data: /usr/share/proj/proj.db
   ```
   缺任一项会打 `NOT found` 到 stderr。

3. **看部署自检** —— `validate.sh` / `validate.ps1` 已内置 PROJ 检查项，输出
   `PROJ 后端 PASS/WARN`；`WARN` 表示 `proj=false`，跨 CRS 请求会失败。

   ```bash
   bash ../../deploy/validate.sh --dir .
   ```

**注意**：`proj:true` 只代表**库加载成功**，不代表 `proj.db` 版本匹配、转换一定成功
（Windows 侧踩过 LAYOUT 错配的坑）。真正的判据是 `proj_create_crs_to_crs` 能否成功，
详见 `mapserver/THIRD-PARTY-NOTICES.md` §2.2。实测跨 CRS 转换：

```bash
curl -s "http://localhost:8080/wms?service=WMS&version=1.3.0&request=GetCapabilities"
```
返回的 `CRS` 列表里应能看到 `EPSG:4490` / `EPSG:45xx` 且实际请求不报
`unsupported CRS`。

**装不上时的排查**：`docker compose logs map-api | grep -i proj`。若提示
`dlopen failed`，多半是 `PROJ_DLL` 指向的路径不存在（检查架构 triplet 是否为
`x86_64-linux-gnu` 或 `aarch64-linux-gnu`）；若库在但转换报 proj.db 版本错配，检查
是否被外部注入了 `PROJ_LIB`（entrypoint 已 `unset`，但 compose 里显式传环境变量会
覆盖 entrypoint 的 unset）。

## 环境变量（用户可配项）

| 变量                    | 默认值      | 说明                                                                 |
|-------------------------|-------------|----------------------------------------------------------------------|
| `MANAGER_JWT_SECRET`    | （必填）    | JWT 签名密钥，compose `:?` 强校验，未设置拒绝启动                    |
| `POSTGRES_USER`         | `postgres`  | PG 超级用户名；注入 DB 初始化与两服务 DSN（envsubst 渲染 config 模板）|
| `MAPSERVER_DB_PASSWORD` | `123456`    | PG 口令（默认 = 本机开发口径；共享环境务必改强口令）                 |
| `MAPSERVER_DB_PORT`     | `5434`      | PostGIS 暴露到宿主机的端口                                           |
| `GISDATA_DIR`           | `./gisdata` | gisdata 挂载点（宿主目录）：挂到 map-api（只读）与 manager（可写）的 `/app/data`，默认工作区 ws1 的 `data_root` 即此目录；Windows 外置盘路径请用正斜杠 `E:/data/gisdata` |

## 密钥去哪了（P0-3）

镜像与仓库 **都不含明文口令**。`config/mapserver/*.xml`、`config/manager/*.xml`
是带 `${VAR}` 的模板；各容器 `docker-entrypoint.sh` 用 `envsubst` 按 `.env`
（compose 自动加载、已 gitignore）渲染到运行期 `/app/config`。
`.env` 只存在于部署机的平台目录里（如 `deploy_zh-cn/linux/.env`，已 gitignore），轮换口令改它即可，无需重 build。

## 数据与图层（gisdata 挂载点）

- 数据挂载：`GISDATA_DIR`（默认平台目录下 `./gisdata`）挂到 map-api 的 `/app/data`
  （只读）与 manager 的 `/app/data`（可写）。默认工作区 ws1 的 `data_root='/app/data/'`
  在 DB 首次初始化时写入（dbinit 尾部 UPDATE），管理界面保存工作区时
  `ensure_std_subdirs` 会在其下自动建 14 个标准子目录（arcgis_cache/arcgis_tpk/cog/…）。
- 默认零图层：镜像内不烘焙 `config/`，起来即可演示 health/capabilities/admin。
- 挂真实图层：把 `cog/`、`vector/` 等图层 xml 放进 `config/mapserver/`
  （entrypoint 会一并渲染），图层配置里的数据路径改用 `/app/data/...`
  而非机器绝对路径（`E:\...`），即完成「去机器化 data_root」。

## 常用命令（在所选平台目录下执行）

```bash
docker compose ps                 # 状态
docker compose logs -f map-api    # 跟日志
docker compose down               # 停止（保留数据卷）
docker compose down -v            # 停止并删卷（清空 DB/缓存，下次 up 重新初始化）
```

自检脚本 `deploy/validate.sh`（Linux）/ `validate.ps1`（Windows）可单独运行，端口与数据目录
探测项对齐；`start.sh` / `start.ps1` 在 up 之后会自动调用它。

## 打包到另一台机器部署

镜像不在镜像仓库，**两台机器各自现场构建**。用打包脚本产出一份源码 bundle：

```bash
python manager/tools/package_deploy.py            # -> dist/rust-gisserver-deploy-<时间戳>.zip
python manager/tools/package_deploy.py --platform linux   # 只带 linux 平台入口
```

产物约 **3.4 MB**（495 个文件），已排除 `target/`、`node_modules/`、`dist*/`、`.git/`、
`2026-11-12/` 等本地产物。关键约束与脚本已处理的部分：

- **保持仓库根布局**：mapserver 的核心库是 path 依赖 `../gisserver`，compose 的 build
  context 依赖相对路径（`../../..` = 仓库根、`../..` = manager/），解压时不要拆散层级。
- **`.dockerignore` 一并打包**：根目录那份是 map-api 构建上下文的白名单，
  `manager/.dockerignore` 排除前端产物与本地数据，缺了会把近 1 GB `node_modules` 送进 daemon。
- **shell 脚本在包内强制 LF**：Windows 工作区受 `core.autocrlf` 影响是 CRLF，脚本写入 zip 时
  统一转 LF，否则拷到 Linux 报 `^M: bad interpreter`。
- **`.env` 不入包**（明文密钥），只带 `.env.example` 模板；目标机自行复制填写。
- **Cargo.lock 已带**（`gisserver/` 与 `mapserver/` 各一份），依赖版本固定。

包内 `DEPLOY.md` 是给目标机看的部署步骤（解压 → 选平台目录 → 配 `.env` → 启动 → 验收）。
目标机需 Docker Desktop（Linux 容器模式）+ 外网（拉 crates / npm 包 / PostGIS 镜像）。
