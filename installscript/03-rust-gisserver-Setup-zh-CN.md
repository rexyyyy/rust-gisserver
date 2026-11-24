# rust-gisserver_Setup 安装指南（Windows）

*rust-gisserver 安装文档 3/4 · NSSM 服务 · 简体中文 · 统一口径 5432*

前置：已完成第 1 份《PostgreSQL 16 + PostGIS 3.5 安装指南》，数据库 `mapserver-manager` 已初始化。

## 统一口径

| 项 | 值 |
| --- | --- |
| 数据库端口 | 5432（全项目统一，`.env` 与 XML 默认值一致，**无需再手工改端口**） |
| 数据库名 | mapserver-manager（短横线） |
| map-api 端口 | 8080 |
| manager 端口 | 6180 |
| 服务名 | mapserve（map-api）、map-manager（manager） |

## 1. 前置检查

| # | 检查项 | 验收方式 |
| --- | --- | --- |
| 1 | PostgreSQL 16 已安装并运行 | `services.msc` → `postgresql-x64-16` 为"正在运行" |
| 2 | PostGIS 已启用 | psql 执行 `SELECT postgis_full_version();` 返回版本号 |
| 3 | 库 `mapserver-manager` 已建并初始化 | `SELECT count(*) FROM sys_user;` 返回非 0 |
| 4 | 端口 5432 可连 | `psql -U postgres -p 5432 -d mapserver-manager -c "select 1;"` |

任一项不通过，先回到第 1 份指南完成数据库准备。

## 2. 安装 rust-gisserver_Setup.exe

以管理员身份双击 `installscript\rust-gisserver_Setup.exe`：

1. 欢迎页 → 下一步。
2. Installation Directory：默认 `C:\Program Files\rust-gisserver`，建议 `D:\rust-gisserver`。记下路径，下文称 `<安装目录>`。
3. 端口配置：map-api 默认 **8080**、manager 默认 **6180**；被占用可改并记住新值。
4. DATA_ROOT：默认 `<安装目录>\data\`；影像/瓦片在其他盘就填完整绝对路径（如 `E:\gisdata`）。
5. NSSM 服务：勾选"注册为 Windows 服务（开机自启）"，服务名 `mapserve` / `map-manager`，以 LocalSystem 运行（数据目录须对其可读）。
6. 下一步 → 安装 → 完成。

安装器自动：① 把安装目录、端口、DATA_ROOT 写入 `.env` 与 XML；② 用 NSSM 注册两个 Windows 服务。

## 3. 安装后目录结构

```text
<安装目录>\
├─ .env                      ← 必改：DB 密码、JWT 密钥、DATA_ROOT
├─ config\
│  ├─ manager.xml            ← manager 主配置
│  ├─ mapserver.xml          ← map-api 主配置
│  └─ cog\ vector\ postgis\ arcgis\ 3dtiles\ ...  ← 图层 XML
├─ bin\                      ← manager.exe / mapserve.exe / PROJ DLL
├─ web\                      ← 前端静态文件（manager 同源托管）
├─ data\                     ← data_root 默认值
├─ cache\foyer\              ← L2 磁盘缓存
├─ share\proj\               ← PROJ 投影数据库
├─ logs\                     ← 运行日志
├─ tools\                    ← nssm.exe 等
└─ sql\                      ← 数据库初始化脚本（第 1 份指南用）
```

## 4. 安装后配置（必做）

用记事本打开 `<安装目录>\.env`，至少设置：

```ini
# 数据库密码：与安装 PostgreSQL 时的 postgres 密码一致，两项相同
MANAGER_DB_PASSWORD=<你的postgres密码>
MAPSERVER_DB_PASSWORD=<你的postgres密码>

# 数据库端口：已统一 5432，无需改动
MANAGER_DB_PORT=5432
MAPSERVER_DB_PORT=5432

# JWT 密钥：生产环境必须随机生成（openssl rand -hex 32）
MANAGER_JWT_SECRET=<随机长串>

# 数据根目录：数据不在默认 data\ 时设置
# DATA_ROOT=E:\gisdata
```

> **端口说明**：`manager.xml` / `mapserver.xml` 的 DSN 模板现为 `${MANAGER_DB_PORT:-5432}`，`.env` 里也写 5432，两边一致。若宿主 5432 被占用需换端口，把 `-Port`、`.env` 两个 `*_DB_PORT`、docker-compose 映射三处一起改。

### 4.1 配置 data_root

`data_root` 只影响 map-api 找图层文件，与数据库表无关。优先级：`.env` 的 `DATA_ROOT` > `mapserver.xml` 的 `${DATA_ROOT:-data}`（进程启动目录下的 `data` 子目录）。

- 数据在安装目录 `data\` 下：什么都不用做。
- 数据在其他盘：在 `.env` 末尾加 `DATA_ROOT=E:\gisdata`，重启服务。

## 5. 重启服务并验证

改完 `.env` 或 XML 后需重启服务（管理员 PowerShell）：

```powershell
net stop map-manager
net stop mapserve
net start map-manager
net start mapserve
sc query map-manager | findstr STATE    # 期望 STATE : 4 RUNNING
sc query mapserve   | findstr STATE
```

浏览器访问：

| 服务 | 地址 | 期望 |
| --- | --- | --- |
| map-api 健康 | http://127.0.0.1:8080/health | JSON `{"status":"ok",...}` |
| manager 健康 | http://127.0.0.1:6180/manager-api/health | JSON 响应 |
| 管理后台 | http://127.0.0.1:6180/ | 登录页（末尾斜杠） |

## 6. 常用操作速查

```powershell
# 服务启停 / 状态
net start map-manager ; net start mapserve
net stop  map-manager ; net stop  mapserve
sc query map-manager | findstr STATE

# 日志跟踪
Get-Content "<安装目录>\logs\manager.err.log" -Tail 50 -Wait
Get-Content "<安装目录>\logs\map-api.err.log" -Tail 50 -Wait

# NSSM 管理
<安装目录>\tools\nssm.exe list
<安装目录>\tools\nssm.exe edit map-manager

# 防火墙放行（管理员）
New-NetFirewallRule -DisplayName "rust-gisserver Manager" -Direction Inbound -Protocol TCP -LocalPort 6180 -Action Allow
New-NetFirewallRule -DisplayName "rust-gisserver Map-API" -Direction Inbound -Protocol TCP -LocalPort 8080 -Action Allow
```

## 7. 卸载

运行 `<安装目录>\Uninstall.exe` 或 控制面板 → 程序和功能 → rust-gisserver → 卸载。

卸载器移除 Windows 服务与 NSSM 注册表项、删除程序文件；**保留** `config\`、`data\`、`cache\`、`logs\`、`.env`。数据库 `mapserver-manager` 不受影响。彻底清理需手工删除残留目录。

## 8. 常见问题

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| 启动失败，日志 `could not connect to server` | `.env` 里 DB 主机/端口错误 | 确认为 127.0.0.1 与 5432，改后重启 |
| 启动失败，日志 `password authentication failed` | `.env` 两个 `*_DB_PASSWORD` 与实际密码不符 | 改成安装 PostgreSQL 时的密码，重启 |
| 浏览器空白 / 404 | 端口不对或未带末尾斜杠 | 用 `http://127.0.0.1:6180/`，核对实际配置端口 |
| `data_root` 改了但图层找不到 | 未重启服务 / 路径写错 | 确认 `.env` 的 `DATA_ROOT` 行有效，重启服务 |
| JWT secret 忘记 | 需重置 | 重新生成随机值覆盖 `MANAGER_JWT_SECRET`，重启，所有 token 失效 |
| 服务启动几秒后退出 | exe 启动报错 | 命令行直接运行 exe 看输出；常见 PROJ DLL 找不到，检查 `share\proj\` 与 `bin\proj_9.dll` |
| 管理后台能开但图层列表空 | 数据目录未挂对 | 检查 `DATA_ROOT`，用 `Test-Path` 验证实际文件存在 |

---

完成以上步骤后：PostgreSQL 16 运行在 5432、PostGIS 已启用、`mapserver-manager` 已初始化、两个服务 RUNNING。打开 http://127.0.0.1:6180/ 即可进入管理后台。
