# rust-gisserver_Setup 安装指南（Windows）

> **目标读者**：刚完成 PostgreSQL + PostGIS 安装、数据库 `mapservemap-manager` 已初始化的用户
> **数据库端口**：**5432**（PostgreSQL 默认端口，与前一篇指南保持一致）
> **数据库名**：`mapservemap-manager`
> **安装包**：`installscript\rust-gisserver_Setup.exe`

---

## 目录

1. [前置条件](#1-前置条件)
2. [安装 rust-gisserver_Setup.exe](#2-安装-rust-gisserver_setupexe)
3. [安装后目录结构](#3-安装后目录结构)
4. [🔧 安装后配置（必做！）](#4-安装后配置必做)
5. [重启服务让配置生效](#5-重启服务让配置生效)
6. [验证服务是否正常](#6-验证服务是否正常)
7. [常用操作速查表](#7-常用操作速查表)
8. [卸载](#8-卸载)
9. [常见问题 FAQ](#9-常见问题-faq)
10. [配置文件对照表](#10-配置文件对照表)

---

## 1. 前置条件

开始安装前，以下项目必须**全部完成**：

| # | 检查项 | 验收方式 |
|---|---|---|
| 1 | PostgreSQL 16.7 已安装并运行 | `services.msc` → `postgresql-x64-16` 状态"正在运行" |
| 2 | PostGIS 3.5.2 已启用 | psql 执行 `SELECT postgis_full_version();` 返回 `3.5.2` |
| 3 | 数据库 `mapservemap-manager` 已创建 | psql 执行 `\l` 列表里能看到 |
| 4 | 数据库已初始化（system.sql + map.sql） | `SELECT count(*) FROM sys_user;` 返回非零 |
| 5 | 端口 5432 可用 | PostgreSQL 监听 5432 |

如果以上任何一项没通过，请**先完成前一篇** [PostgreSQL-PostGIS-Installation.zh-CN.md](./PostgreSQL-PostGIS-Installation.zh-CN.md)。

---

## 2. 安装 rust-gisserver_Setup.exe

以**管理员身份**双击 `installscript\rust-gisserver_Setup.exe`，按向导操作：

1. **欢迎页** → 下一步
2. **安装目录**（Installation Directory）：
   - 默认：`C:\Program Files\rust-gisserver`
   - 建议：`D:\rust-gisserver`（数据盘充足的位置）
   - ⚠️ 记下安装目录，下文统一用 `<安装目录>` 指代
3. **端口配置页**：
   - **map-api 端口**：默认 **8080**（地图引擎对外服务端口）
   - **manager 端口**：默认 **6180**（管理后台端口）
   - 若本机已有程序占用，可改端口（例如 8081 / 6181），记下改后的值
4. **数据根目录 DATA_ROOT**：
   - 默认：`<安装目录>\data\`（安装器自动创建）
   - 如果影像/瓦片/PMTiles/3D Tiles 等数据在其他盘（例如 `E:\gisdata\`），**现在就填完整绝对路径**
   - 后续改 DATA_ROOT 也可以（见第 4.2 节）
5. **NSSM 服务**：勾选 **"注册为 Windows 服务（开机自启）"**
   - 服务名：`mapserve`（map-api）和 `map-manager`（管理后台）
   - 以 `LocalSystem` 账户运行，数据目录需要对 LocalSystem 有读权限
6. 下一步 → 安装 → 完成

> 安装器会自动处理两件事：① 把安装目录、端口、DATA_ROOT 写入 `.env` 和 XML 配置；② 用 NSSM 把两个 exe 注册为 Windows 服务。

---

## 3. 安装后目录结构

```
<安装目录>\
├─ rust-gisserver_Setup.exe          ← 安装器副本
├─ .env                               ← ⭐ 必改！环境变量（DB 密码、JWT 密钥、端口）
├─ config\
│  ├─ manager.xml                     ← 管理后台主配置
│  ├─ mapserver.xml                   ← 地图引擎主配置
│  ├─ cog\ vector\ postgis\ arcgis\ 3dtiles\ ...  ← 各类图层 XML
│  └─ service-state.xml               ← 服务上下线状态
├─ bin\
│  ├─ manager.exe                     ← 管理后台可执行文件
│  └─ mapserve.exe                     ← 地图引擎可执行文件
├─ web\                               ← 前端静态文件（由 manager.exe 同源托管）
├─ data\                              ← ⭐ data_root 默认值，存放影像/瓦片数据
│  ├─ pmtiles\                        ← PMTiles 归档文件
│  ├─ mbtiles\                        ← MBTiles 瓦片包
│  ├─ cog\                            ← Cloud Optimized GeoTIFF
│  ├─ vector\                         ← GeoJSON / Shapefile
│  ├─ 3dtiles\                        ← 3D Tiles 服务
│  └─ osgb\                           ← OSGB 转 3D Tiles 源数据
├─ cache\
│  ├─ foyer\                          ← foyer L2 磁盘缓存（重启可复用）
│  └─ ...                             ← 其他缓存文件
├─ share\proj\                        ← PROJ 投影数据库
├─ logs\                              ← 运行日志
├─ tools\
│  └─ nssm.exe                        ← Windows 服务包装器
├─ svc_mapapi.cmd                     ← NSSM 承载的 map-api 启动脚本
├─ svc_manager.cmd                    ← NSSM 承载的 manager 启动脚本
└─ Uninstall.exe                      ← 卸载程序
```

---

## 4. 🔧 安装后配置（必做！）

安装器只写入了安装目录、端口和 DATA_ROOT 三项，但**数据库密码、JWT 密钥、数据库端口（默认 5434 → 实际 5432）** 必须你自己手动补齐。

### 4.1 修改 `.env` 数据库连接参数

用记事本或 VS Code 打开 `<安装目录>\.env`。**以下 4 项必须修改**：

```ini
# ============================================================
# ===== 必填：数据库密码 =====
# ============================================================
MANAGER_DB_PASSWORD=YourPassword123!      ← 改成你安装 PostgreSQL 时设置的 postgres 密码
MAPSERVER_DB_PASSWORD=YourPassword123!    ← 同上，两个密码一样

# ============================================================
# ===== 必填：数据库端口（默认是 5434，必须改成 5432）=====
# ============================================================
MANAGER_DB_PORT=5432                      ← 改成 5432（PostgreSQL 默认端口）
MAPSERVER_DB_PORT=5432                    ← 同上

# ============================================================
# ===== 如果数据库在另一台机器，取消注释并填写 =====
# ============================================================
# MANAGER_DB_HOST=192.168.1.100
# MAPSERVER_DB_HOST=192.168.1.100

# ============================================================
# ===== 必填：JWT 签名密钥（生产环境必须随机生成！）=====
# ============================================================
# 生成方法：PowerShell 执行
#   openssl rand -hex 32
#   (没装 openssl 则用 PowerShell 内置: -join ((48..57)+(97..122) | ForEach-Object {[char]$_} | Get-Random -Count 64)
MANAGER_JWT_SECRET=用openssl rand -hex 32生成的随机字符串   ← 替换，长度 64 字符（32 字节 hex）

# ============================================================
# ===== data_root：数据在其他盘时设置 =====
# ============================================================
# 未设置时，XML 里 ${DATA_ROOT:-data} 回退到相对路径 data
# 如果影像/瓦片数据在 E 盘：
# DATA_ROOT=E:\gisdata

# ============================================================
# ===== 以下为安装器已配置，按需修改 =====
# ============================================================
MAPSRV_HOST=127.0.0.1                    ← manager 回连 map-api 的目标地址（不是 bind 地址！）
MAPSRV_PORT=8080                         ← map-api 端口（安装器已写入）
MAPSERVER_MANAGER_URL=http://127.0.0.1:6180  ← manager 地址
GIS_ENCODING_REGION=zh-CN                ← 路径/文件名编码区域
```

> 🚨 **为什么 MANAGER_DB_PORT 和 MAPSERVER_DB_PORT 默认是 5434？**
> 因为项目的 manager.xml 和 mapserver.xml 里 DSN 模板写的是 `${MANAGER_DB_PORT:-5434}`（默认 5434）。但**前一篇指南我们用的是 PostgreSQL 默认端口 5432**，所以必须把 `.env` 里这两个端口显式改成 5432。

#### 真实 DSN 格式参考（帮助理解 env 变量怎么注入的）

**manager.xml 第 21 行**：
```xml
<database url="postgresql://postgres:${MANAGER_DB_PASSWORD}@${MANAGER_DB_HOST:-localhost}:${MANAGER_DB_PORT:-5434}/mapservemap-manager" pool_max="16"/>
```

**mapserver.xml 第 88 行**：
```xml
<admin dsn="postgresql://postgres:${MAPSERVER_DB_PASSWORD}@${MAPSERVER_DB_HOST:-localhost}:${MAPSERVER_DB_PORT:-5434}/mapservemap-manager"/>
```

当 `.env` 配好后，最终渲染出来的 DSN 类似：
```
postgresql://postgres:YourPassword123!@localhost:5432/mapservemap-manager
```

### 4.2 配置 data_root（数据根目录）

data_root 是 **map-api 找图层文件的根目录**，只影响 `mapserver.xml` 中 `source.path` 写相对路径时的解析起点。它的取值优先级：

1. **`.env` 中的 `DATA_ROOT`**（你手动设置）→ 最高优先级
2. **未设置时**：`${DATA_ROOT:-data}` 回退到 **进程启动目录下的 `data` 子目录**

#### 场景 A：数据就放在安装目录下（默认情况）

什么都不用做。安装器已经创建了 `<安装目录>\data\`，XML 里 `${DATA_ROOT:-data}` 会自动回退到这个目录。

#### 场景 B：数据在其他盘（推荐生产环境做法）

影像/瓦片/PMTiles/3D Tiles 等大文件不应该塞在 C 盘系统分区。假设你在 `E:\gisdata\` 下存放了全部数据：

1. 在 `.env` 末尾加一行：
   ```ini
   DATA_ROOT=E:\gisdata
   ```
2. 这样 `mapserver.xml` 里所有 `${DATA_ROOT:-data}` 都会展开成 `E:\gisdata`，包括：
   - `<server data_root="...">` → map-api 启动时的工作根
   - `<wuhua_buildings>${DATA_ROOT:-data}/pmtiles/wuhua_buildings.pmtiles</wuhua_buildings>` → 归档路径变成 `E:\gisdata\pmtiles\wuhua_buildings.pmtiles`
3. 确保 `E:\gisdata\` 目录结构与 XML 里引用的相对路径对应（比如有 `pmtiles\`、`mbtiles\`、`cog\` 子目录）

> 💡 **data_root 只影响 map-api 找图层文件，不影响 manager 的数据库表**。数据库表永远在 PostgreSQL 里，跟 data_root 无关。

#### 🚀 快速上手：把 data_root 指向 `E:\data\1`（实际测试数据）

如果你的影像/瓦片/PMTiles/3D Tiles 都在 `E:\data\1\` 下（里面已经有 `cog\`、`mbtiles\`、`pmtiles\`、`postgis\`、`3dtiles\` 等子目录），按下面做：

**1) 用记事本打开 `.env`**

位置：`D:\rust-gisserver\.env`

或者 PowerShell 命令（一行搞定）：
```powershell
notepad "D:\rust-gisserver\.env"
```

**2) 在文件末尾加一行**

```ini
DATA_ROOT=E:\data\1
```

完整示例 `.env`（已填好 `E:\data\1`）：
```ini
MANAGER_DB_HOST=127.0.0.1
MANAGER_DB_PORT=5434
MANAGER_DB_PASSWORD=123456
MANAGER_JWT_SECRET=local-dev-jwt-secret-20261119
MAPSRV_HOST=127.0.0.1
MAPSRV_PORT=8080
MAPSERVER_DB_HOST=127.0.0.1
MAPSERVER_DB_PORT=5434
MAPSERVER_DB_PASSWORD=123456
MAPSERVER_MANAGER_URL=http://127.0.0.1:6180
GIS_ENCODING_REGION=zh-CN

# ===== 这里新增一行 =====
DATA_ROOT=E:\data\1
```

**3) 重启两个 Windows 服务让配置生效**

**必须管理员 PowerShell / cmd**（非管理员会报 "Access is denied"）：
```powershell
net stop mapserve
net stop map-manager
net start mapserve
net start map-manager
```

或者用 NSSM（同样需要管理员）：
```powershell
D:\rust-gisserver\tools\nssm.exe restart mapserve
D:\rust-gisserver\tools\nssm.exe restart map-manager
```

**4) 验证生效**

map-api 启动后看日志：
```powershell
Get-Content "D:\rust-gisserver\logs\mapapi_out.log" -Tail 30
```

然后浏览器打开 **http://127.0.0.1:8080/** 或 **http://127.0.0.1:6180/**（manager 后台），数据源/图层列表里应该能看到来自 `E:\data\1\cog\`、`E:\data\1\mbtiles\` 等的图层。

**5) 出问题排障 checklist**

| 现象 | 原因 | 处理 |
|---|---|---|
| 图层列表空的 / 找不到文件 | `DATA_ROOT` 没生效或路径写错 | 确认 `.env` 里这行**没有**被 `#` 注释；路径用反斜杠 `\`；**重启服务** |
| mapapi_out.log 里报 `ENOENT` / `No such file or directory` | `E:\data\1\` 下缺对应子目录 | 检查 `E:\data\1\` 下是否有 XML 里引用的 `cog\` / `mbtiles\` / `pmtiles\` / `postgis\` 等目录 |
| 服务启动失败 | `.env` 格式错（中文引号 / 尾部多了空格） | 删掉 DATA_ROOT 行重新手打，不要复制粘贴富文本 |
| 没重启服务 | `.env` 只在服务启动时读一次 | 务必 stop → start |

### 4.3 其他可配置项（.env / XML 对照）

#### `.env` 里可配的全部项

| 变量 | 说明 | 默认值 |
|---|---|---|
| `MANAGER_DB_HOST` | PostgreSQL 主机 | `localhost`（由 `${MANAGER_DB_HOST:-localhost}` 注入） |
| `MANAGER_DB_PORT` | PostgreSQL 端口 | `5434`（**必须改成 5432**） |
| `MANAGER_DB_PASSWORD` | PostgreSQL 密码 | **必填，无默认** |
| `MANAGER_JWT_SECRET` | JWT 签名密钥 | **必填，无默认** |
| `MANAGER_DSN_KEY` | DSN 加密密钥（可选） | 空（留空时由 JWT secret 派生） |
| `MAPSERVER_DB_HOST` | 同上，给 map-api 用 | `localhost` |
| `MAPSERVER_DB_PORT` | 同上 | `5434`（**必须改成 5432**） |
| `MAPSERVER_DB_PASSWORD` | 同上 | **必填，无默认** |
| `MAPSRV_HOST` | manager 回连 map-api 的目标主机 | `127.0.0.1`（跨机部署改 IP） |
| `MAPSRV_PORT` | map-api 端口 | `8080`（安装器已写） |
| `MAPSERVER_MANAGER_URL` | map-api 探测 manager 健康检查的 URL | `http://127.0.0.1:6180` |
| `GIS_ENCODING_REGION` | 路径/文件名编码区域 | `zh-CN` |
| `DATA_ROOT` | 数据根目录 | 未设时回退 `<安装目录>\data\` |

#### `config/manager.xml` 里手动可调的项

```xml
<server addr="0.0.0.0:6180" ...>          <!-- 监听地址：想只监听本机改成 127.0.0.1:6180 -->
  <cors_origins>
    <item>http://localhost:5100</item>    <!-- 前端 vite 开发源，生产同源部署可删掉 -->
  </cors_origins>
</server>
<database url="..."/>                      <!-- DSN 走 .env 注入，不用手改 -->
<storage avatar_dir="storage/avatar"/>     <!-- 用户头像存储目录，生产建议绝对路径 -->
<mapsvc base_url="..."/>                   <!-- 读 ${MAPSRV_HOST}:${MAPSRV_PORT}，跨机部署改 .env -->
<security jwt_secret="...">               <!-- 读 ${MANAGER_JWT_SECRET}，生产密钥在 .env -->
```

#### `config/mapserver.xml` 里手动可调的项

```xml
<server host="0.0.0.0" port="8080" ...>
  <!-- host 默认 0.0.0.0（监听所有网卡），想只本机访问改成 127.0.0.1 -->
  <!-- port 由安装器已写入 -->
  <!-- cache_mib / foyer_cache_mib / disk_cache.capacity_mib：缓存预算，按需扩容 -->
</server>
<manager url="${MAPSERVER_MANAGER_URL:-...}"/>  <!-- 跨机部署改 .env -->
<admin dsn="..."/>                               <!-- 读 .env 注入 -->
<!--
<data_token>changeme</data_token>                <!-- 启用数据面鉴权时取消注释 -->
<s3 endpoint="..." region="auto" access_key="..." secret_key="..."/>  <!-- S3 存储 -->
-->
```

### 4.4 跨机部署（manager 和 map-api 在不同机器）

当 manager（管理后台）和 map-api（地图引擎）分别部署在**两台独立机器**上时：

**map-api 机器**（IP 例：`192.168.1.100`）：
```ini
# map-api 要让外部 manager 能连到它的服务端口
# ⚠️ 注意：MAPSRV_HOST 不是 bind 地址！bind 地址在 mapserver.xml 的 <server host="0.0.0.0">
MAPSRV_HOST=192.168.1.100   ← 自己这台机器的 IP（manager 会用这个地址回连）
MAPSRV_PORT=8080

# 数据库可以两边共用同一个 PostgreSQL（跨机 DB_HOST）
MAPSERVER_DB_HOST=192.168.1.50   ← PostgreSQL 所在机器 IP
MAPSERVER_DB_PORT=5432
MAPSERVER_DB_PASSWORD=YourPassword123!

# manager 地址（反向探测用，让 map-api 知道自己被谁管）
MAPSERVER_MANAGER_URL=http://192.168.1.101:6180
```

**manager 机器**（IP 例：`192.168.1.101`）：
```ini
# manager 要回连 map-api，填 map-api 那台机器的 IP
MAPSRV_HOST=192.168.1.100   ← map-api 机器 IP
MAPSRV_PORT=8080

# 数据库同上
MANAGER_DB_HOST=192.168.1.50
MANAGER_DB_PORT=5432
MANAGER_DB_PASSWORD=YourPassword123!

# manager 地址（自己）
MAPSERVER_MANAGER_URL=http://192.168.1.101:6180
```

**防火墙**：两台机器各自放行入站端口（manager 放行 6180，map-api 放行 8080，都放行 5432 的 PostgreSQL 连接）。

---

## 5. 重启服务让配置生效

`.env` 和 XML 配置改完后，**必须重启 Windows 服务**才能加载新配置。PowerShell 执行（需管理员权限）：

```powershell
# 停止两个服务
net stop map-manager
net stop mapserve

# 启动两个服务
net start map-manager
net start mapserve

# 查看服务状态
sc query map-manager
sc query mapserve
```

> ⚠️ 改 `.env` 需要**两个服务都重启**（manager 和 map-api 各读各自的 .env 变量）。
> 改 `config/manager.xml` 只需重启 `map-manager`。
> 改 `config/mapserver.xml` 只需重启 `mapserve`。
> 改图层 XML（`config/cog/*.xml` 等）**不用重启**，在管理后台点"热重载"即可。

---

## 6. 验证服务是否正常

### 6.1 检查服务状态

```powershell
sc query map-manager | findstr STATE
sc query mapserve | findstr STATE
```

预期输出：`STATE : 4 RUNNING`。

### 6.2 浏览器访问

| 服务 | URL | 预期 |
|---|---|---|
| map-api 健康检查 | http://127.0.0.1:8080/health | 返回 JSON `{"status":"ok",...}` |
| manager 健康检查 | http://127.0.0.1:6180/manager-api/health | 返回 JSON |
| 管理后台 UI | http://127.0.0.1:6180/ | 打开登录页 |

如果端口改过（安装时用了别的端口），换成你实际的端口号。

### 6.3 用 psql 确认数据库连接正常

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapservemap-manager"
```

```sql
-- 确认 manager 用的表都能读
SELECT count(*) FROM sys_user;
SELECT count(*) FROM map_datasource;
```

如果所有查询正常返回数字 → 连接没问题，服务启动失败一定是 `.env` 里密码或端口填错了。

---

## 7. 常用操作速查表

### 启停服务

```powershell
net stop map-manager
net stop mapserve
net start map-manager
net start mapserve
```

### 查看服务状态

```powershell
sc query map-manager
sc query mapserve
```

### 实时看日志

```powershell
Get-Content "<安装目录>\logs\manager.out.log" -Tail 50 -Wait
Get-Content "<安装目录>\logs\manager.err.log" -Tail 50 -Wait
Get-Content "<安装目录>\logs\map-api.out.log" -Tail 50 -Wait
Get-Content "<安装目录>\logs\map-api.err.log" -Tail 50 -Wait
```

### 防火墙放行（PowerShell 管理员）

```powershell
# manager 端口
New-NetFirewallRule -DisplayName "rust-gisserver Manager" -Direction Inbound `
    -Protocol TCP -LocalPort 6180 -Action Allow

# map-api 端口
New-NetFirewallRule -DisplayName "rust-gisserver Map-API" -Direction Inbound `
    -Protocol TCP -LocalPort 8080 -Action Allow
```

### NSSM 服务管理（高级）

```powershell
# 查看所有 NSSM 注册的服务
<安装目录>\tools\nssm.exe list

# 编辑服务启动参数（弹出 GUI）
<安装目录>\tools\nssm.exe edit map-manager
<安装目录>\tools\nssm.exe edit mapserve

# 移除服务（卸载时用）
<安装目录>\tools\nssm.exe remove map-manager confirm
<安装目录>\tools\nssm.exe remove mapserve confirm
```

### 切换手动启动模式

如果不想让它开机自启，或想让 NSSM 启动时有机会修改环境：

```powershell
sc config map-manager start= demand
sc config mapserve start= demand
```

---

## 8. 卸载

运行 `<安装目录>\Uninstall.exe` 或控制面板 → 程序和功能 → rust-gisserver → 卸载。

卸载程序会：
- 删除 Windows 服务（map-manager、mapserve）
- 删除 NSSM 注册表项
- 删除程序可执行文件

**⚠️ 保留以下目录**（数据安全，不会被删除）：
- `<安装目录>\config\` （所有 XML 配置）
- `<安装目录>\data\` （影像/瓦片数据）
- `<安装目录>\cache\` （磁盘缓存）
- `<安装目录>\logs\` （日志）
- `<安装目录>\.env` （环境变量）

如需**彻底清除**，卸载后手动删除剩余目录即可。数据库 `mapservemap-manager` 不受影响（不在安装目录内）。

---

## 9. 常见问题 FAQ

### Q1：启动失败，日志显示 "could not connect to server: No such file or directory"

`.env` 里的 `MANAGER_DB_HOST` / `MAPSERVER_DB_HOST` / `MANAGER_DB_PORT` / `MAPSERVER_DB_PORT` 有一个填错了。打开日志 `<安装目录>\logs\manager.err.log` 或 `map-api.err.log`，搜索 `FATAL` 或 `error connecting`。

**最常见的原因**：端口还是写着 5434 而 PostgreSQL 实际在 5432。改成 5432 再重启。

### Q2：启动失败，日志显示 "password authentication failed"

`MANAGER_DB_PASSWORD` 或 `MAPSERVER_DB_PASSWORD` 与 PostgreSQL 实际密码不匹配。确认你记得的 postgres 密码，改 `.env` 后重启。

### Q3：启动成功但浏览器访问空白/404

- 端口对不对？`.env` 里 `MAPSRV_PORT` 必须和 `config/mapserver.xml` 里 `<server port="...">` 一致
- 跨机部署时 `.env` 里的 `MAPSRV_HOST` 不能填 `0.0.0.0`（那是 bind 地址，回连要用真实 IP）
- manager UI 是同源托管的，URL 必须是 `http://<manager地址>:6180/`（末尾有斜杠），不是 `manager-api/` 路径

### Q4：data_root 改了但图层还是找不到文件

检查 3 点：

1. `.env` 里 `DATA_ROOT=E:\gisdata` 有没有加？（没加就是空值，XML 回退到相对路径 `data`）
2. 加了后有没有**重启 mapserve 服务**？（只有 NSSM 启动的服务会读 .env，手工 exe 启动读的是系统环境变量）
3. 绝对路径对不对？用 psql 或 PowerShell `Test-Path "E:\gisdata\pmtiles\..."` 验证文件真的在那个位置

### Q5：JWT secret 忘记了 / 需要重置

重新生成一个随机值覆盖 `.env` 里的 `MANAGER_JWT_SECRET`，重启 map-manager 服务。所有已签发的 token 立刻失效，用户需要重新登录——这是预期行为。

### Q6：Windows 服务启动后几秒钟自动退出

通常是 exe 启动报错然后退出了。

1. 直接在命令行运行 exe 看输出：
   ```powershell
   cd "<安装目录>\bin"
   $env:PATH += ";<安装目录>\bin"
   $env:PATH += ";<安装目录>\share\proj"
   $env:PROJ_LIB = "<安装目录>\share\proj"
   .\manager.exe
   .\mapserve.exe
   ```
2. 查看 `.err.log` 末尾的 FATAL 错误
3. 最常见：PROJ 运行库找不到 → 确保 `share\proj\` 目录在，或者把 `bin\proj_9.dll` 所在目录加入 PATH

### Q7：服务以 LocalSystem 运行时访问网络/共享路径失败

LocalSystem 账户对网络共享（UNC 路径 `\\server\share\`）和其他机器的 PostgreSQL 没有访问权限。两种解决方案：

- **推荐**：在安装器里选指定账户（installer 支持配置服务登录账户），或安装后用 `nssm.exe edit map-manager` / `nssm.exe edit mapserve` 改服务账户
- **临时**：所有网络路径映射为本地盘符（`net use E: \\server\share`），但 LocalSystem 也看不到用户的盘符映射

---

## 10. 配置文件对照表

### 10.1 `.env` vs XML 同名配置项

| 配置项 | `.env` | `config/manager.xml` | `config/mapserver.xml` | **哪个生效** |
|---|---|---|---|---|
| DB Host | `MANAGER_DB_HOST` / `MAPSERVER_DB_HOST` | （通过 `${MANAGER_DB_HOST:-localhost}` 注入 `<database url>`） | （通过 `${MAPSERVER_DB_HOST:-localhost}` 注入 `<admin dsn>`） | **.env 优先** |
| DB Port | `MANAGER_DB_PORT` / `MAPSERVER_DB_PORT` | `${MANAGER_DB_PORT:-5434}` | `${MAPSERVER_DB_PORT:-5434}` | **.env 优先**（⚠️ 默认 5434，必须改 5432） |
| DB Password | `MANAGER_DB_PASSWORD` / `MAPSERVER_DB_PASSWORD` | `${MANAGER_DB_PASSWORD}` | `${MAPSERVER_DB_PASSWORD}` | **.env 必填**（XML 里没默认值） |
| map-api 端口 | `MAPSRV_PORT` | — | `<server port="8080">` | **.env 优先**（XML 由安装器同步，但 .env 里没有 env 变量注入这一项，实际生效看启动命令传参；建议安装器同步的就别改了） |
| data_root | `DATA_ROOT` | — | `<server data_root="${DATA_ROOT:-data}">` 和 `<wuhua_buildings>${DATA_ROOT:-data}/pmtiles/...` | **.env 优先** |
| manager URL | `MAPSERVER_MANAGER_URL` | — | `<manager url="${MAPSERVER_MANAGER_URL:-...}">` | **.env 优先** |
| JWT Secret | `MANAGER_JWT_SECRET` | `<security jwt_secret="${MANAGER_JWT_SECRET}">` | — | **.env 必填** |
| map-api 主机（manager 回连） | `MAPSRV_HOST` | 注入 `<mapsvc base_url="http://${MAPSRV_HOST:-127.0.0.1}:${MAPSRV_PORT:-8080}">` | — | **.env 优先** |
| manager 监听地址 | — | `<server addr="0.0.0.0:6180">` | — | **只有 XML 可配**（安装器已写） |
| map-api 监听地址 | — | — | `<server host="0.0.0.0">` | **只有 XML 可配**（⚠️ 这是 bind 地址，不是 .env 的 MAPSRV_HOST！） |
| 缓存预算 | — | — | `cache_mib` / `foyer_cache_mib` / `disk_cache.capacity_mib` | **只有 XML 可配** |
| data_token | — | — | `<data_token>changeme</data_token>` | **只有 XML 可配**（启用数据面鉴权时取消注释） |
| S3 凭证 | — | — | `<s3 endpoint="..." region="auto" access_key="..." secret_key="...">` | **只有 XML 可配** |
| DSN 加密 | — | `<security dsn_cipher="des" dsn_key="${MANAGER_DSN_KEY}">` | — | **XML + .env 联合** |

### 10.2 一句话总结优先级

- **密码、密钥、DB 端口/主机、DATA_ROOT** → `.env` 必填 / 优先覆盖
- **监听地址（host/addr）、缓存预算、data_token、S3** → XML 手动改
- **端口**（map-api、manager）→ 安装器已同步到 XML，.env 里没注入，一般不用再改

---

## 完成

如果前两篇文档都按步骤走通了，现在 rust-gisserver 应该已经：
- ✅ PostgreSQL 16.7 运行在端口 5432
- ✅ PostGIS 3.5.2 已启用
- ✅ 数据库 `mapservemap-manager` 已初始化
- ✅ rust-gisserver_Setup 已安装并配置好 `.env`
- ✅ 两个服务都以 RUNNING 状态运行

打开浏览器访问 `http://127.0.0.1:6180/`，即可进入 rust-gisserver 管理后台。
