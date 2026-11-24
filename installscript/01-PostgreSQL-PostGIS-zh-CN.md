# PostgreSQL 16 + PostGIS 3.5 安装指南（Windows）

*rust-gisserver 安装文档 1/4 · Windows x64 · 简体中文 · 统一口径 5432*

本指南只讲"装数据库"这一件事。装完后接着看第 3 份《rust-gisserver_Setup 安装指南》。

## 统一口径（全项目只认这一套）

| 项 | 值 | 说明 |
| --- | --- | --- |
| 端口 | 5432 | PostgreSQL 默认端口，全项目统一，不要改成 5434 |
| 数据库名 | mapserver-manager | 短横线，不是下划线 |
| 用户 | postgres | 超级用户 |
| 编码 | UTF8 | 不写 LC_COLLATE / LC_CTYPE，Windows 不认 `en_US.UTF-8` |
| 扩展 | 只装 postgis 一个 | 不需要 topology / sfcgal / tiger_geocoder / fuzzystrmatch |
| 一键脚本 | sql\init_all.ps1 | 建库 + 启用扩展 + 灌 system/map 表，失败即停 |

## 1. 前置条件

| 项 | 要求 |
| --- | --- |
| 操作系统 | Windows 10 / Server 2016 及以上，x64 |
| 权限 | 本地管理员账户（UAC 弹窗点"是"） |
| 端口 | 5432 未被占用（`netstat -ano \| findstr :5432` 检查） |
| 磁盘 | PostgreSQL + PostGIS 约 1.5 GB，另留数据空间 |
| 安装包 | `postgresql-16.x-windows-x64.exe`、`postgis-bundle-pg16x64-setup-3.5.x-x64.exe` |

## 2. 安装 PostgreSQL 16

1. 以管理员身份运行安装程序。
2. Installation Directory：建议 `D:\PostgreSQL\16`（**无中文无空格**，务必记住完整路径）。
3. Components：默认全选；Data Directory 保持默认子目录。
4. Superuser Password：设一个强密码并牢记，下文用 `<你的密码>` 指代。
5. Port：确认 **5432**，不要改。
6. Locale：选 Default locale，不要强制选中文。
7. 最后一页**取消** Stack Builder，Finish。
8. 验证：`services.msc` 里 `postgresql-x64-16` 状态为"正在运行"。

## 3. 安装 PostGIS 3.5

关键：PostGIS 必须装到与 PostgreSQL **完全相同**的目录，否则 `CREATE EXTENSION postgis` 会报 `could not open extension control file`。

1. 以管理员身份运行 `postgis-bundle-pg16x64-setup-*.exe`。
2. Installation Directory 手动确认与上一步一致（如 `D:\PostgreSQL\16`），不要装到子目录。
3. Components：默认全选（PostGIS、GDAL、PROJ、GEOS 等）。
4. Create sample spatial database：选 **No**。
5. Finish。

## 4. 一键建库与初始化

安装包复制的 SQL 在 `<安装目录>\sql\`。打开 PowerShell（任意目录均可），执行：

```powershell
powershell -ExecutionPolicy Bypass -File "<安装目录>\sql\init_all.ps1" -Password "<你的密码>"
```

也可不带 `-Password`，脚本会交互提示输入（不回显）。若 PostgreSQL 不在默认盘符，脚本会自动查注册表与各盘常见目录定位 `psql.exe`。

脚本依次做三件事，每步都带 `-v ON_ERROR_STOP=1` 并检查退出码，**失败立即停止**：

| 步骤 | 脚本 | 作用 |
| --- | --- | --- |
| 1/3 | 01-init-database.sql | 建 `mapserver-manager` 库（UTF8，不指定 locale）+ 启用 `postgis` 扩展 |
| 2/3 | system.sql | 系统表 + 初始化数据（菜单、用户、角色、字典） |
| 3/3 | map.sql | 地图与数据表 |

> 三个脚本都幂等：失败修好后直接重跑即可，不会重复插入数据。
> 中文 / 英文初始数据二选一：`system.sql`+`map.sql` 或 `system.en_US.sql`+`map.en_US.sql`，**不要混跑**。

## 5. 验证

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

在 psql 提示符下：

```sql
SELECT extname, extversion FROM pg_extension WHERE extname = 'postgis';  -- 返回 1 行，如 3.5.2
SELECT count(*) FROM sys_user;        -- 非 0
SELECT count(*) FROM sys_menu;        -- 非 0
SELECT count(*) FROM map_datasource;  -- 表存在即可，可为 0
\q
```

看到 PostGIS 版本号 + 系统表有数据行数，数据库这层就算通过了。

## 6. 常见报错

| 报错 | 原因 | 处理 |
| --- | --- | --- |
| `locale "en_US.UTF-8" does not exist` | 手写了 POSIX locale | 用自带 `01-init-database.sql`，里面只写 `ENCODING 'UTF8' TEMPLATE template0` |
| `could not open extension control file ... postgis.control` | PostGIS 没装到同一目录 | 卸载 PostGIS 重装，Installation Directory 选 `D:\PostgreSQL\16` |
| 脚本打印"完成"但服务连不上库 | 旧脚本不查 psql 退出码（假成功） | 用当前 `init_all.ps1`，失败会明确报错并停止 |
| `psql.exe not found` | PATH 没有 PostgreSQL | 脚本自动查注册表/常见盘符；仍找不到则把 `...\bin` 加 PATH |
| 密码认证失败 | 传错密码或 `.env` 没同步 | 脚本加 `-Password`；装完把 `.env` 两个 `*_DB_PASSWORD` 改成同一个 |
| 中文乱码 | 客户端编码 | `chcp 65001` 或 `$env:PGCLIENTENCODING="UTF8"` |
| 5432 被占用 | 本机已有别的数据库 | 换端口时 `-Port`、`.env` 两个 `*_DB_PORT`、docker-compose 映射一起改 |

## 7. 参考

- PostgreSQL 16：https://www.postgresql.org/docs/16/
- PostGIS 3.5：https://postgis.net/docs/
- 一页纸速查：`installscript\安装速查.md`（若与本文冲突，以速查为准）
