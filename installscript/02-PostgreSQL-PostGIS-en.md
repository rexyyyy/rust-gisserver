# PostgreSQL 16 + PostGIS 3.5 Installation Guide (Windows)

*rust-gisserver Installation Documentation 1/4 · Windows x64 · English · unified port 5432*

This guide covers the database only. After finishing, continue with manual 3/4 (rust-gisserver Setup).

## Unified conventions (the only set used project-wide)

| Item | Value | Note |
| --- | --- | --- |
| Port | 5432 | PostgreSQL default, unified project-wide; do NOT use 5434 |
| Database | mapserver-manager | hyphen, not underscore |
| User | postgres | superuser |
| Encoding | UTF8 | do NOT set LC_COLLATE / LC_CTYPE; Windows does not know `en_US.UTF-8` |
| Extension | postgis only | topology / sfcgal / tiger_geocoder / fuzzystrmatch are not needed |
| One-shot script | sql\init_all.ps1 | create DB + enable extension + load system/map tables; stops on error |

## 1. Prerequisites

| Item | Requirement |
| --- | --- |
| OS | Windows 10 / Server 2016 or newer, x64 |
| Privilege | Local administrator (approve UAC) |
| Port | 5432 free (`netstat -ano \| findstr :5432`) |
| Disk | ~1.5 GB for PostgreSQL + PostGIS, plus data space |
| Installers | `postgresql-16.x-windows-x64.exe`, `postgis-bundle-pg16x64-setup-3.5.x-x64.exe` |

## 2. Install PostgreSQL 16

1. Run the installer as Administrator.
2. Installation Directory: e.g. `D:\PostgreSQL\16` (**no Chinese chars or spaces**; remember the full path).
3. Components: keep defaults; leave Data Directory at its default subfolder.
4. Superuser Password: set a strong password and remember it (referred to as `<your password>` below).
5. Port: confirm **5432**; do not change.
6. Locale: choose Default locale; do not force Chinese.
7. Last page: **uncheck** Stack Builder, then Finish.
8. Verify: `services.msc` shows `postgresql-x64-16` as Running.

## 3. Install PostGIS 3.5

Critical: PostGIS must be installed into the **exact same directory** as PostgreSQL, otherwise `CREATE EXTENSION postgis` fails with `could not open extension control file`.

1. Run `postgis-bundle-pg16x64-setup-*.exe` as Administrator.
2. Confirm Installation Directory matches the previous step (e.g. `D:\PostgreSQL\16`); never a subfolder.
3. Components: keep defaults (PostGIS, GDAL, PROJ, GEOS).
4. Create sample spatial database: choose **No**.
5. Finish.

## 4. One-shot database creation and initialization

The installer copies SQL scripts into `<INSTALLDIR>\sql\`. Open PowerShell (any working directory) and run:

```powershell
powershell -ExecutionPolicy Bypass -File "<INSTALLDIR>\sql\init_all.ps1" -Password "<your password>"
```

You may omit `-Password`; the script then prompts securely. If PostgreSQL is on an unusual drive, the script locates `psql.exe` via the registry and common drive paths automatically.

The script performs three steps, each with `-v ON_ERROR_STOP=1` and an exit-code check, so it **stops immediately on failure**:

| Step | Script | Purpose |
| --- | --- | --- |
| 1/3 | 01-init-database.sql | Create `mapserver-manager` (UTF8, no locale) + enable `postgis` extension |
| 2/3 | system.sql | System tables and seed data (menus, users, roles, dictionaries) |
| 3/3 | map.sql | Map and data tables |

> All three scripts are idempotent: fix the cause and re-run, no duplicate inserts.
> Pick one language baseline: `system.sql`+`map.sql` or `system.en_US.sql`+`map.en_US.sql` — never mix them.

## 5. Verify

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

At the psql prompt:

```sql
SELECT extname, extversion FROM pg_extension WHERE extname = 'postgis';  -- one row, e.g. 3.5.2
SELECT count(*) FROM sys_user;        -- non-zero
SELECT count(*) FROM sys_menu;        -- non-zero
SELECT count(*) FROM map_datasource;  -- table exists; rows may be 0
\q
```

A PostGIS version plus non-zero system-table counts means the database layer is ready.

## 6. Common errors

| Error | Cause | Fix |
| --- | --- | --- |
| `locale "en_US.UTF-8" does not exist` | POSIX locale written by hand | Use bundled `01-init-database.sql` (only `ENCODING 'UTF8' TEMPLATE template0`) |
| `could not open extension control file ... postgis.control` | PostGIS installed elsewhere | Reinstall PostGIS into the exact PostgreSQL directory (e.g. `D:\PostgreSQL\16`) |
| Script says "done" but service cannot connect | Old script ignored psql exit code (false success) | Use current `init_all.ps1`; it reports errors and stops |
| `psql.exe not found` | PostgreSQL not on PATH | Script checks registry/common drives; otherwise add `...\bin` to PATH |
| Password authentication failed | Wrong password or `.env` not synced | Pass `-Password`; set both `*_DB_PASSWORD` in `.env` to the same value |
| Garbled Chinese | Client encoding | `chcp 65001` or `$env:PGCLIENTENCODING="UTF8"` |
| Port 5432 in use | Another database installed | When changing port, update `-Port`, both `.env` `*_DB_PORT`, and docker-compose mapping together |

## 7. References

- PostgreSQL 16: https://www.postgresql.org/docs/16/
- PostGIS 3.5: https://postgis.net/docs/
- One-page quick guide: `installscript\安装速查.md` (if it conflicts with this manual, the quick guide wins)
