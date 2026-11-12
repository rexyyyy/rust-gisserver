# 一键部署（开发计划 P0-1 · 第一阶段）

三仓数据底座的容器化编排入口。`compose` 拉起 **PostGIS + map-api（发布平面）+ manager（管理平面）**，
gis-srv（能力底座 / MCP）以 profile 占位、默认不启动。

## 架构与端口

| 服务      | 镜像 / 构建               | 宿主端口 | 说明                        |
|-----------|---------------------------|----------|-----------------------------|
| postgis   | `postgis/postgis:16-3.4`  | 5434→5432| 单库 `mapserver_manager`    |
| map-api   | `../../r-mapserver`       | 8080     | WMS/WMTS/WFS/MVT/COG/PMTiles… |
| manager   | `..`（r-manager）         | 6180     | 管理平面 + 前端静态资源     |

容器网络内互访用服务名：manager→`http://map-api:8080`，两服务→`postgis:5432`。

## 快速开始

```powershell
cd r-manager/deploy
copy .env.example .env       # 必填 MAPSERVER_DB_PASSWORD / MANAGER_JWT_SECRET
docker compose up -d --build
```

- 前端：<http://localhost:6180/>
- 服务健康：<http://localhost:8080/health>
- WMS 能力：<http://localhost:8080/wms?service=WMS&request=GetCapabilities>

首次启动 PostGIS 会自动执行 `../backend/sql/*.sql` 建表。

## 密钥去哪了（P0-3）

镜像与仓库 **都不含明文口令**。`config/mapserver/*.xml`、`config/manager/*.xml`
是带 `${VAR}` 的模板；各容器 `docker-entrypoint.sh` 用 `envsubst` 按 `.env`
（compose 自动加载、已 gitignore）渲染到运行期 `/app/config`。
`.env` 只存在于部署机，轮换口令改它即可，无需重 build。

## 数据与图层（P0-4 预告）

- 数据卷：`./data` 只读挂到 map-api 的 `/app/data`。
- 默认零图层：镜像内不烘焙 `config/`，起来即可演示 health/capabilities/admin。
- 挂真实图层：把 `cog/`、`vector/` 等图层 xml 放进 `./config/mapserver/`
  （entrypoint 会一并渲染），图层配置里的数据路径改用 `/app/data/...`
  而非机器绝对路径（`E:\...`），即完成「去机器化 data_root」。

## 常用命令

```powershell
docker compose ps                 # 状态
docker compose logs -f map-api    # 跟日志
docker compose down               # 停止（保留数据卷）
docker compose down -v            # 停止并删卷（清空 DB/缓存）
```
