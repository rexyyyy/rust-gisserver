# 一键部署（开发计划 P0-1 · 第一阶段）

数据底座容器化编排。`compose` 拉起 **PostGIS + map-api（发布平面）+ manager（管理平面）**，
gis-srv（能力底座 / MCP）以 profile 占位、默认不启动。

**按语言分目录**：本目录（`deploy/`）只放公共资产（config 模板、`.env.example`、
validate / start 脚本、文档）；两个语言目录是各自独立的 compose 入口：

| 目录               | compose 项目名       | DB 首次初始化                                    |
|--------------------|----------------------|--------------------------------------------------|
| `../deploy_zh-cn/` | `gis-data-base-zh`   | `system.sql → map.sql`（中文基线）               |
| `../deploy_en/`    | `gis-data-base-en`   | `system.en_US.sql → map.en_US.sql`（英文基线）   |

## 快速开始

```powershell
cd manager/deploy_zh-cn          # 英语环境用 cd manager/deploy_en
copy ..\deploy\.env.example .env # 必填 MANAGER_JWT_SECRET；其余项均有默认值
docker compose up -d --build
```

或用一键脚本（读所选目录的 `.env` → up → 等健康 → 跑 validate.ps1）：

```powershell
pwsh -File ..\deploy\start.ps1 -Dir ..\deploy_zh-cn
pwsh -File ..\deploy\start.ps1 -Dir ..\deploy_en
```

- 前端：<http://localhost:6180/>
- 服务健康：<http://localhost:8080/health>
- WMS 能力：<http://localhost:8080/wms?service=WMS&request=GetCapabilities>

## 语言切换语义

- 两个语言目录是**独立 compose 项目**（pgdata / 缓存卷互不共享）：切换语言 = 另一
  目录直接 `up`，不需要 `down -v`，数据互不覆盖。
- 端口（6180 / 8080 / MAPSERVER_DB_PORT 默认 15434）两套相同，**不能同时运行**。
- DB 种子语言只在**首次建库**时写入；运行期接口 msg 由 manager 后端按请求头
  `X-Language` 动态 zh→en，与库种子无关。英文库上中文请求的 DB 文本（菜单/字典）
  会显示英文（中间件无 en→zh 路径），故英文目录面向纯英语环境。
- 标签类列宽已统一 64（nick_name / notice_title），中英文种子都放得下。

## 架构与端口

| 服务      | 镜像 / 构建                          | 宿主端口 | 说明                        |
|-----------|--------------------------------------|----------|-----------------------------|
| postgis   | `postgis/postgis:16-3.4`             | MAPSERVER_DB_PORT→5432 | 单库 `mapserver-manager`（默认 5434，宿主被占时在语言目录 .env 改） |
| map-api   | 仓库根上下文 + `mapserver/Dockerfile` | 8080     | WMS/WMTS/WFS/MVT/COG/PMTiles… |
| manager   | `..`（manager/，backend + ui）        | 6180     | 管理平面 + 前端静态资源     |

容器网络内互访用服务名：manager→`http:// mapserve:8080`，两服务→`postgis:5432`。

## 环境变量（用户可配项）

| 变量                    | 默认值      | 说明                                                                 |
|-------------------------|-------------|----------------------------------------------------------------------|
| `MANAGER_JWT_SECRET`    | （必填）    | JWT 签名密钥，compose `:?` 强校验，未设置拒绝启动                    |
| `POSTGRES_USER`         | `postgres`  | PG 超级用户名；注入 DB 初始化与两服务 DSN（envsubst 渲染 config 模板）|
| `MAPSERVER_DB_PASSWORD` | `123456`    | PG 口令（默认 = 本机开发口径；共享环境务必改强口令）                 |
| `MAPSERVER_DB_PORT`     | `5434`      | PostGIS 暴露到宿主机的端口                                           |
| `GISDATA_DIR`           | `./gisdata` | gisdata 挂载点（宿主目录）：**只挂到 map-api** 的 `/app/data`（只读）；manager 不挂数据卷，所有数据操作经 map-api 接口执行。默认工作区 ws1 的 `data_root` 即此目录；Windows 外置盘路径请用正斜杠 `E:/data/gisdata` |

## 密钥去哪了（P0-3）

镜像与仓库 **都不含明文口令**。`config/mapserver/*.xml`、`config/manager/*.xml`
是带 `${VAR}` 的模板；各容器 `docker-entrypoint.sh` 用 `envsubst` 按 `.env`
（compose 自动加载、已 gitignore）渲染到运行期 `/app/config`。
`.env` 只存在于部署机的语言目录里，轮换口令改它即可，无需重 build。

## 数据与图层（gisdata 挂载点）

- 数据挂载：`GISDATA_DIR`（默认语言目录下 `./gisdata`）**只挂到 map-api** 的
  `/app/data`（只读）。manager 不挂数据卷：目录浏览、Shapefile 导入、网格解析等
  数据操作全部代理到 map-api 执行。默认工作区 ws1 的 `data_root='/app/data/'`
  在 DB 首次初始化时写入（dbinit 尾部 UPDATE）。
- 默认零图层：镜像内不烘焙 `config/`，起来即可演示 health/capabilities/admin。
- 挂真实图层：把 `cog/`、`vector/` 等图层 xml 放进 `config/mapserver/`
  （entrypoint 会一并渲染），图层配置里的数据路径改用 `/app/data/...`
  而非机器绝对路径（`E:\...`），即完成「去机器化 data_root」。

## 常用命令（在语言目录下执行）

```powershell
docker compose ps                 # 状态
docker compose logs -f map-api    # 跟日志
docker compose down               # 停止（保留数据卷）
docker compose down -v            # 停止并删卷（清空 DB/缓存，下次 up 重新初始化）
```
