# mapserve 安装说明（Windows）

## 一、前置条件

- **PostgreSQL 14+ 已安装并启用 PostGIS 扩展**（默认端口 5434，数据库 `mapserver_manager` 已由安装包初始化脚本创建则免）
- **Windows 10 / Server 2016 x64**
- 数据目录所在磁盘对 `LocalSystem` 有读权限（服务默认以 LocalSystem 运行）

## 二、安装

1. 双击 `mapserve_Setup.exe`（需管理员权限）
2. 选择安装目录（默认 `C:\Program Files\mapserve`）
3. 配置页填写：
   - **map-api 端口**（默认 8080）
   - **map-manager 端口**（默认 6180）
   - **数据根目录 DATA_ROOT**（默认 `E:\data`）
   - 勾选「注册为 Windows 服务」→ NSSM 开机自启
4. 完成安装；安装器会自动写入端口/数据目录到配置文件

安装后目录结构：

```
mapserve/
├─ .env                       ← 环境变量（DB 口令 / JWT 密钥 / DATA_ROOT / MAPSRV_PORT）
├─ config/
│  ├─ mapserver/mapserver.xml ← 地图引擎主配置（端口 / data_root / 令牌）
│  ├─ manager/manager.xml     ← 管理端主配置（端口 / DB / JWT / CORS）
│  ├─ cog/ arcgis/ postgis/ 3dtiles/  ← 各类图层 XML
│  └─ service-state.xml       ← 服务上下线状态
├─ map-api/mapserve.exe
├─ manager/manager.exe
├─ manager/ui/dist/           ← 前端静态文件（由 manager.exe 同源托管）
├─ logs/  data/  cache/  storage/
├─ tools/nssm.exe             ← Windows 服务包装器
├─ svc_mapapi.cmd / svc_manager.cmd ← NSSM 承载的启动脚本
└─ start_*.bat / stop_all.bat ← 手动启动脚本（未注册服务时用）
```

### 安装器自动写入的位置（**不用手改**）

配置页填的三个值（MapPort / MgrPort / DataRoot）由 `installer-assets/apply_config.ps1` 一次性同步到以下四处，重复执行幂等：

| # | 文件 | 被改的字段 |
|---|---|---|
| 1 | `config/mapserver/mapserver.xml` | `<server ... port="NNNN">` |
| 2 | `config/manager/manager.xml` | `<server addr="0.0.0.0:NNNN">` |
| 3 | `.env` | `MAPSRV_PORT=` / `DATA_ROOT=` |
| 4 | `start_mapapi.bat` / `start_manager.bat` | `set DATA_ROOT=...` |

> 后续想改端口/数据目录：**推荐重装时改**；已装环境需要手工改时，以上 4 处必须同时改，否则会出现「`.env` 与 bat 里 DATA_ROOT 不一致」的错乱。`MAPSRV_HOST` 不在同步范围内，跨机部署时手动改 `.env` 即可。

## 三、**安装完成后必改的配置文件**

### 1. `.env`（**必改**，改完重启服务生效）

```
MANAGER_DB_HOST=localhost
MANAGER_DB_PORT=5434
MANAGER_DB_USER=postgres
MANAGER_DB_PASSWORD=123456          ← 改成实际 PG 口令
MANAGER_DB_NAME=mapserver_manager

MANAGER_JWT_SECRET=325346b44985...  ← 必须换掉（随便起个 64 位十六进制串）

MAPSRV_HOST=127.0.0.1                ← map-manager 回连 map-api 的客户端 URL（不是 bind 地址，见下节）
MAPSRV_PORT=8080                    ← 同上，安装器已按配置页写入
DATA_ROOT=E:\data                   ← 安装器已按配置页写入，一般不用动
```

> **⚠ 别把 `MAPSRV_HOST` 当绑定地址**：
> - **map-api 的实际 bind**：`config/mapserver/mapserver.xml` 里 `<server host="0.0.0.0" ...>`，默认已监听所有网卡，外部机器可直接访问。
> - **`.env` 的 `MAPSRV_HOST`**：只被 `manager.xml` 的 `<mapsvc base_url="http://${MAPSRV_HOST:-127.0.0.1}:${MAPSRV_PORT:-8080}"/>` 消费，用于 map-manager 作为**客户端**回连 map-api。同机部署保持 `127.0.0.1`；跨机部署改填 map-api 所在机器的实际 IP 或主机名，**不能填 `0.0.0.0`**（Windows 上会当 loopback 处理，Linux 上直接失败）。

### 2. `config/mapserver/mapserver.xml`（**按需**，启用鉴权 / 改绑定时改）

- `<server host="0.0.0.0" port="8080" data_root="${DATA_ROOT}">`：端口与数据目录安装器已写入；bind 默认全网卡
  - 想只监听本机 → `host="127.0.0.1"`；想只走某张网卡 → 填该网卡 IP
- **要开启数据面鉴权**：取消注释 `<data_token>changeme</data_token>` 并改为实际口令
- **要开启管理面鉴权**：`<admin token="...">` 内填主令牌；后续 map-manager 生成的 API Key 会自动加入共享库 `sys_api_key` 一起生效

### 3. `config/manager/manager.xml`（**一般不用动**）

- `<server addr="0.0.0.0:6180">`：安装器已写入；如需只监听本机改成 `127.0.0.1:6180`
- `<database url="postgresql://...">`：口令走 `${MANAGER_DB_PASSWORD}`，只需改 `.env`
- `<mapsvc base_url="http://${MAPSRV_HOST}:${MAPSRV_PORT}"/>`：读取 `.env`，跨机部署时改 `.env` 即可
- `<cors_origins>`：生产同源部署可留空或删掉 vite 开发条目

## 四、启动 / 停止

- **注册了服务**：开机自启；手动控制
  ```
  net stop mapserve & net stop map-manager
  net start mapserve  & net start map-manager
  ```
- **未注册服务**：双击 `start_mapapi.bat` / `start_manager.bat`；`stop_all.bat` 停止

## 五、访问

- 地图服务：`http://127.0.0.1:8080/`
- 管理后台：`http://127.0.0.1:6180/`（默认账号由初始化脚本决定）

## 六、改配置后如何生效

| 改的文件 | 需要重启 |
|---|---|
| `.env` | mapserve + map-manager 两个服务都重启 |
| `config/mapserver/mapserver.xml` | mapserve |
| `config/manager/manager.xml` | map-manager |
| `config/{cog,postgis,arcgis,3dtiles}/*.xml` 图层 | 不用重启，管理后台点「热重载」即可 |

## 七、卸载

控制面板 → 卸载 `mapserve`；或运行 `$INSTDIR\Uninstall.exe`。
自动删除程序文件与服务，**保留** `config/` `data/` `logs/` `.env`。
