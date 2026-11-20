# PostgreSQL 16 + PostGIS 3.5 安装指南（Windows x64）

> **版本固定**：PostgreSQL 16.7、PostGIS 3.5.2
> **默认端口**：5432（PostgreSQL 官方默认端口）
> **目标数据库**：`mapserver-manager`（短横线）
> **安装包位置**：`installscript\数据库\`

---

## 目录

1. [前置条件](#1-前置条件)
2. [安装 PostgreSQL 16.7](#2-安装-postgresql-167)
3. [验证 PostgreSQL 登录](#3-验证-postgresql-登录)
4. [安装 PostGIS 3.5.2](#4-安装-postgis-352)
5. [建立数据库 + 启用 PostGIS 扩展](#5-建立数据库--启用-postgis-扩展)
6. [初始化数据库 — 执行项目 SQL 脚本](#6-初始化数据库--执行项目-sql-脚本)
7. [验证初始化结果](#7-验证初始化结果)
8. [（可选）创建模板库 template_postgis](#8-可选创建模板库-template_postgis)
9. [常见问题 FAQ](#9-常见问题-faq)
10. [参考链接](#10-参考链接)

---

## 1. 前置条件

| 项目 | 要求 |
|---|---|
| 操作系统 | Windows 10 / Windows Server 2016 及以上，x64 |
| 磁盘空间 | PostgreSQL + PostGIS 约占 1.5 GB；数据盘预留足够空间 |
| 权限 | 安装时使用**本地管理员账户**（UAC 弹窗点"是"） |
| 端口 | 5432 未被占用（默认端口；若已被占用，见 FAQ 端口冲突） |

安装包清单（`installscript\数据库\`）：

- `postgresql-16.7-1-windows-x64.exe` — PostgreSQL 16.7 安装程序
- `postgis-bundle-pg16x64-setup-3.5.2-1.exe` — PostGIS 3.5.2 安装程序（已含 GDAL/PROJ 运行库）

> ⚠️ 本文全程使用 **PostgreSQL 默认端口 5432**。下文所有 psql 命令和 DSN 字符串中的端口号均为 `5432`。

---

## 2. 安装 PostgreSQL 16.7

1. 以**管理员身份**双击 `postgresql-16.7-1-windows-x64.exe`。
2. **Installation Directory**（安装目录）：
   - 建议：`D:\PostgreSQL\16`（换盘或默认均可，但**必须记下完整路径**，PostGIS 要装到同一位置）
   - 路径不要包含中文、空格以外的特殊字符。
3. **Select Components**（组件）：保持默认全选（PostgreSQL Server、pgAdmin 4、Command Line Tools）。
4. **Data Directory**（数据目录）：保持默认（安装目录下的 `data` 子目录），不要改动。
5. **Superuser Password**（超级用户密码）：
   - 为 `postgres` 用户设置一个强密码。
   - 本文示例使用占位符 `YourPassword123!`，**实际使用时请替换为你自己的密码，并牢记**。
6. **Port**：确认是 **5432**（默认值，PostgreSQL 官方标准端口），不要改动。
7. **Advanced Options → Locale**：选择 **Default locale**，不要强制选中文（PostgreSQL 内部编码 UTF-8 不依赖 locale）。
8. 最后一页 **取消勾选 Stack Builder**（本项目不需要），点击 **Finish** 完成安装。
9. 验证服务：按 `Win+R`，输入 `services.msc` 回车，确认存在 `postgresql-x64-16` 服务且状态为**正在运行**。

**安装登记表（请填写）**：

| 项目 | 填写值 |
|---|---|
| PostgreSQL 安装目录 | 例：`D:\PostgreSQL\16` |
| 数据目录 | 例：`D:\PostgreSQL\16\data` |
| 超级用户密码 | `YourPassword123!`（你自己的） |
| 端口 | **5432** |

> ⚠️ PostGIS 安装时需要填入**完全相同的安装目录**，务必保存好。

---

## 3. 验证 PostgreSQL 登录

打开 **PowerShell** 或 **CMD**：

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432
```

- `-U postgres`：超级用户名
- `-p 5432`：端口号（**5432，不是 5434**）

首次连接会提示输入密码，输入你在安装时设置的 `YourPassword123!`（注意：输入时**不显示任何字符**，这是正常行为，输完直接回车）。

成功看到 `postgres=#` 提示符即登录成功。输入 `\q` 回车可退出 psql。

> 也可以直接加 `-W` 参数强制弹密码提示，或加 `--password YourPassword123!` 在脚本里使用（不推荐生产环境明文）。

---

## 4. 安装 PostGIS 3.5.2

**关键要求**：PostGIS 必须安装到与 PostgreSQL **完全相同的目录**（`D:\PostgreSQL\16`），否则 `CREATE EXTENSION postgis` 会报错。

1. 双击 `postgis-bundle-pg16x64-setup-3.5.2-1.exe`。
2. 安装器会自动探测本机 PostgreSQL 16 实例。**Installation Directory 必须与 PostgreSQL 16 安装位置一致**：
   - ✅ 正确：`D:\PostgreSQL\16`（与上节登记表一致）
   - ❌ 错误：`D:\PostgreSQL\16\PostGIS` 或其他任何不同路径
   - 若安装器未自动填对，**手动改成你登记表里的完整路径**再继续。
3. **Select Components**（组件）：保持默认全选（PostGIS、GDAL、PROJ、GEOS、SQLite 等运行库）。
4. 是否创建示例空间数据库（Create sample spatial database）：选 **No / Skip**（我们后续自己建）。
5. 完成安装。

---

## 5. 建立数据库 + 启用 PostGIS 扩展

打开 PowerShell，进入 psql：

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432
```

在 `postgres=#` 提示符下依次执行（每行回车）：

```sql
-- 1. 创建项目数据库（短横线名：mapserver-manager，不是下划线）
CREATE DATABASE "mapserver-manager"
    WITH ENCODING 'UTF8'
         LC_COLLATE = 'en_US.UTF-8'
         LC_CTYPE   = 'en_US.UTF-8'
         TEMPLATE    = template0;

-- 2. 切换到该数据库
\c "mapserver-manager"

-- 3. 启用 PostGIS 扩展
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS postgis_topology;
CREATE EXTENSION IF NOT EXISTS postgis_sfcgal;

-- 4. 启用 fuzzy string match（项目依赖）
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;

-- 5. 验证 PostGIS 版本
SELECT postgis_full_version();

-- 6. 退出 psql
\q
```

预期输出类似：

```
POSTGIS="3.5.2 64-bit" [RTTOPO][SFCGAL] ...
```

能看到版本号（含 `POSTGIS="3.5.2"`）即为成功。

---

## 6. 初始化数据库 — 执行项目 SQL 脚本

项目自带了初始化 SQL 脚本，位于：

- `D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\system.sql` — 系统表（菜单、用户、权限、字典等）
- `D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\map.sql` — 地图表（数据源、图层、缓存等）

**执行顺序**：**先 system.sql，再 map.sql**（map.sql 依赖 system.sql 建立的表结构）。

在 PowerShell 中依次执行（路径**已写死为你本机项目路径**，可直接 copy-paste）：

```powershell
# 切换到 PostgreSQL bin 目录
cd /d "D:\PostgreSQL\16\bin"

# 执行 system.sql —— 系统表初始化
psql -U postgres -p 5432 -d "mapserver-manager" `
     -f "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\system.sql"

# 执行 map.sql —— 地图表初始化
psql -U postgres -p 5432 -d "mapserver-manager" `
     -f "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\map.sql"
```

> 首次运行会提示输入 postgres 密码（即你之前设置的 `YourPassword123!`）。如果想一次性跑，可用 `-W` 参数强制提示。

也可以在 psql 交互模式内执行：

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

```sql
-- 在 psql 里
\i D:/mycomputor/documents/gis-wt/thirdparty/rust-gisserver/manager/backend/sql/system.sql
\i D:/mycomputor/documents/gis-wt/thirdparty/rust-gisserver/manager/backend/sql/map.sql
```

（注意 psql 的 `\i` 命令使用**正斜杠** `/`，即使在 Windows 下）

两条脚本都显示 `CommandLine ended with empty input`（或无报错）即执行成功。

---

## 7. 验证初始化结果

再次连接数据库执行以下 SQL，确认核心表和扩展都到位了：

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

```sql
-- 检查 PostGIS 扩展
SELECT postgis_full_version();

-- 检查系统表是否存在（应该返回 1 行）
SELECT count(*) FROM sys_menu;
SELECT count(*) FROM sys_user;
SELECT count(*) FROM sys_dept;
SELECT count(*) FROM sys_role;

-- 检查地图表是否存在
SELECT count(*) FROM map_datasource;
SELECT count(*) FROM map_layer;

-- 列出所有已创建的表
\dt

-- 退出
\q
```

预期：

- `postgis_full_version()` 返回版本号（含 `3.5.2`）
- `sys_menu`、`sys_user` 等系统表都有数据行数（非 0）
- `map_datasource`、`map_layer` 等地图表存在（行数可以为 0）
- `\dt` 能看到所有初始化出来的表

---

## 8. （可选）创建模板库 template_postgis

如果将来要频繁创建新的空间数据库，可以把当前 `mapserver-manager`（已启用 postgis 扩展的数据库）克隆为模板：

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432
```

```sql
-- 1. 先断开所有到 mapserver-manager 的连接
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE datname = 'mapserver-manager' AND pid <> pg_backend_pid();

-- 2. 改为模板数据库
UPDATE pg_database SET datistemplate = TRUE WHERE datname = 'mapserver-manager';
VACUUM FREEZE "mapserver-manager";

-- 以后建新房空间库就可以用：
-- CREATE DATABASE newgis TEMPLATE = "mapserver-manager";
```

这是可选操作，不影响 rust-gisserver 运行。

---

## 9. 常见问题 FAQ

### Q1：5432 端口被占用怎么办？

用以下命令查看哪个进程占用了 5432：

```powershell
netstat -ano | findstr :5432
```

如果是另一个 PostgreSQL 实例，建议**先卸载旧的**再装；如果必须并存，安装时 PostgreSQL 端口改成 **5434**（或其他空闲端口），但**全文所有端口号也要同步改成 5434**（包括第 5 节建库前、第 6 节执行 SQL、第 7 节验证，以及后续 rust-gisserver 的 `.env` 里）。

> 本文档全文以 **5432** 为准。

### Q2：忘记 postgres 密码怎么办？

在 pg_hba.conf 里临时把认证方式改为 `trust`：

1. 找到 `D:\PostgreSQL\16\data\pg_hba.conf`
2. 把所有 `host` 和 `local` 行的认证方法从 `md5`/`scram-sha-256` 改为 `trust`
3. 重启 PostgreSQL 服务（`net stop postgresql-x64-16 && net start postgresql-x64-16`）
4. `psql -U postgres -p 5432` 无需密码即可登录，然后执行 `ALTER USER postgres WITH PASSWORD '新密码';`
5. 改回 `pg_hba.conf` 的认证方法，重启服务

### Q3：`CREATE EXTENSION postgis` 报错 "could not open extension control file"

几乎肯定是 **PostGIS 装到了与 PostgreSQL 不同的目录**。卸载 PostGIS，重新安装时将 Installation Directory 手动改为 PostgreSQL 16 的安装目录（如 `D:\PostgreSQL\16`）。

### Q4：执行 SQL 脚本时中文乱码

PowerShell 执行 psql 前先切到 UTF-8 代码页：

```powershell
chcp 65001
cd /d "D:\PostgreSQL\16\bin"
$env:PGCLIENTENCODING = "UTF8"
psql -U postgres -p 5432 -d "mapserver-manager" -f "D:\...\system.sql"
```

或直接在 psql 交互里执行 `SET client_encoding = 'UTF8';` 后再 `\i`。

### Q5：远程机器连不上本 PostgreSQL

检查以下三点：

1. **pg_hba.conf**：在文件末尾加一行 `host    all             all             0.0.0.0/0               scram-sha-256`
2. **postgresql.conf**：找到 `listen_addresses`，改为 `listen_addresses = '*'`
3. **Windows 防火墙**：入站规则放行 TCP 5432

修改后重启 PostgreSQL 服务生效。

---

## 10. 参考链接

- PostgreSQL 16 官方文档：https://www.postgresql.org/docs/16/
- PostGIS 3.5 官方文档：https://postgis.net/docs/
- PostGIS 官方下载页：https://postgis.net/install/
- PostgreSQL Windows 安装包：https://www.postgresql.org/download/windows/

---

**下一篇文档**：[rust-gisserver-Setup-Installation.zh-CN.md](./rust-gisserver-Setup-Installation.zh-CN.md) — 安装完本指南后，接着安装 rust-gisserver 服务端。
