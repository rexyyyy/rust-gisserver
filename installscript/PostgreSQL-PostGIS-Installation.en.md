# PostgreSQL 16 + PostGIS 3.5 Installation Guide (Windows x64)

> **Fixed Versions**: PostgreSQL 16.7, PostGIS 3.5.2
> **Default Port**: 5432 (PostgreSQL's official default port)
> **Target Database**: `mapserver-manager` (hyphenated, not underscore)
> **Installers Location**: `installscript\数据库\`

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Install PostgreSQL 16.7](#2-install-postgresql-167)
3. [Verify PostgreSQL Login](#3-verify-postgresql-login)
4. [Install PostGIS 3.5.2](#4-install-postgis-352)
5. [Create Database + Enable PostGIS Extensions](#5-create-database--enable-postgis-extensions)
6. [Initialize Database — Run Project SQL Scripts](#6-initialize-database--run-project-sql-scripts)
7. [Verify Initialization Results](#7-verify-initialization-results)
8. [(Optional) Create Template Database template_postgis](#8-optional-create-template-database-template_postgis)
9. [FAQ](#9-faq)
10. [References](#10-references)

---

## 1. Prerequisites

| Item | Requirement |
|---|---|
| OS | Windows 10 / Windows Server 2016 or newer, x64 |
| Disk Space | ~1.5 GB for PostgreSQL + PostGIS; allocate extra room for the data drive |
| Permissions | Install with a **local Administrator account** (accept all UAC prompts) |
| Port | 5432 must be free (default port; see FAQ for conflicts) |

Installer files (`installscript\数据库\`):

- `postgresql-16.7-1-windows-x64.exe` — PostgreSQL 16.7 installer
- `postgis-bundle-pg16x64-setup-3.5.2-1.exe` — PostGIS 3.5.2 installer (includes bundled GDAL/PROJ runtime)

> ⚠️ This document uses **PostgreSQL's default port 5432** throughout. Every `psql` command and every DSN string below uses port `5432`.

---

## 2. Install PostgreSQL 16.7

1. Double-click `postgresql-16.7-1-windows-x64.exe` as **Administrator**.
2. **Installation Directory**:
   - Recommendation: `D:\PostgreSQL\16` (use any drive; just **write down the full path** — PostGIS must be installed to the same location)
   - The path should not contain special characters beyond spaces (no CJK characters).
3. **Select Components**: Leave all defaults checked (PostgreSQL Server, pgAdmin 4, Command Line Tools).
4. **Data Directory**: Keep the default (`data` subdirectory under the install directory). Do not change.
5. **Superuser Password**:
   - Set a strong password for the `postgres` superuser.
   - This document uses the placeholder `YourPassword123!`. **Replace it with your own password and remember it**.
6. **Port**: Confirm **5432** (the PostgreSQL official default). Do not change.
7. **Advanced Options → Locale**: Select **Default locale**. Do not force a Chinese locale (PostgreSQL internal encoding is UTF-8, independent of locale).
8. On the last page, **uncheck Stack Builder** (not required for this project) and click **Finish**.
9. Verify the service: press `Win+R`, type `services.msc`, confirm that `postgresql-x64-16` is listed and **Running**.

**Installation Record (please fill in)**:

| Item | Value |
|---|---|
| PostgreSQL install directory | e.g. `D:\PostgreSQL\16` |
| Data directory | e.g. `D:\PostgreSQL\16\data` |
| Superuser password | `YourPassword123!` (yours) |
| Port | **5432** |

> ⚠️ PostGIS requires this **exact same directory**. Save this table.

---

## 3. Verify PostgreSQL Login

Open **PowerShell** or **CMD**:

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432
```

- `-U postgres`: superuser name
- `-p 5432`: port (**5432, not 5434**)

Enter the password you set earlier (`YourPassword123!`) when prompted. Note that **no characters are echoed while typing** — this is normal. Press Enter when done.

Seeing the `postgres=#` prompt means login succeeded. Type `\q` and Enter to exit psql.

> You can also add `-W` to force a password prompt, or `--password YourPassword123!` in scripts (not recommended for production plaintext).

---

## 4. Install PostGIS 3.5.2

**Critical requirement**: PostGIS must be installed into the **exact same directory** as PostgreSQL (`D:\PostgreSQL\16`). If they differ, `CREATE EXTENSION postgis` will fail.

1. Double-click `postgis-bundle-pg16x64-setup-3.5.2-1.exe`.
2. The installer auto-detects the local PostgreSQL 16 instance. **Installation Directory must exactly match your PostgreSQL 16 install location**:
   - ✅ Correct: `D:\PostgreSQL\16` (same as in your record table)
   - ❌ Wrong: `D:\PostgreSQL\16\PostGIS` or any other path
   - If not auto-filled correctly, **manually enter the full path from your record table** and continue.
3. **Select Components**: Leave all defaults checked (PostGIS, GDAL, PROJ, GEOS, SQLite runtimes).
4. "Create sample spatial database": choose **No / Skip** (we'll create our own).
5. Finish installation.

---

## 5. Create Database + Enable PostGIS Extensions

Open PowerShell and start psql:

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432
```

At the `postgres=#` prompt, run each statement (press Enter after each):

```sql
-- 1. Create the project database (hyphenated name: mapserver-manager, not underscore)
CREATE DATABASE "mapserver-manager"
    WITH ENCODING 'UTF8'
         LC_COLLATE = 'en_US.UTF-8'
         LC_CTYPE   = 'en_US.UTF-8'
         TEMPLATE    = template0;

-- 2. Switch to the new database
\c "mapserver-manager"

-- 3. Enable PostGIS extensions
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS postgis_topology;
CREATE EXTENSION IF NOT EXISTS postgis_sfcgal;

-- 4. Enable fuzzy string match (project dependency)
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;

-- 5. Verify PostGIS version
SELECT postgis_full_version();

-- 6. Exit psql
\q
```

Expected output:

```
POSTGIS="3.5.2 64-bit" [RTTOPO][SFCGAL] ...
```

Seeing the version string (containing `POSTGIS="3.5.2"`) means success.

---

## 6. Initialize Database — Run Project SQL Scripts

The project ships initialization SQL scripts at:

- `D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\system.sql` — system tables (menus, users, permissions, dicts, etc.)
- `D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\map.sql` — map tables (datasources, layers, caches, etc.)

**Run order**: **system.sql first, then map.sql** (map.sql depends on tables created by system.sql).

Execute in PowerShell (paths are pre-filled to your local project root — copy-paste directly):

```powershell
# Change to PostgreSQL bin directory
cd /d "D:\PostgreSQL\16\bin"

# Run system.sql — system table initialization
psql -U postgres -p 5432 -d "mapserver-manager" `
     -f "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\system.sql"

# Run map.sql — map table initialization
psql -U postgres -p 5432 -d "mapserver-manager" `
     -f "D:\mycomputor\documents\gis-wt\thirdparty\rust-gisserver\manager\backend\sql\map.sql"
```

> You will be prompted for the postgres password (`YourPassword123!`) on each `psql` call. To run non-interactively, add `-W` to force a prompt.

Alternatively, run from inside psql:

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

```sql
-- inside psql
\i D:/mycomputor/documents/gis-wt/thirdparty/rust-gisserver/manager/backend/sql/system.sql
\i D:/mycomputor/documents/gis-wt/thirdparty/rust-gisserver/manager/backend/sql/map.sql
```

(Note: `\i` uses **forward slashes** `/` even on Windows.)

No error messages mean both scripts ran successfully.

---

## 7. Verify Initialization Results

Connect again and run the following SQL to confirm core tables and extensions are in place:

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

```sql
-- Check PostGIS extension
SELECT postgis_full_version();

-- Check system tables exist (each should return at least 1 row)
SELECT count(*) FROM sys_menu;
SELECT count(*) FROM sys_user;
SELECT count(*) FROM sys_dept;
SELECT count(*) FROM sys_role;

-- Check map tables exist
SELECT count(*) FROM map_datasource;
SELECT count(*) FROM map_layer;

-- List all created tables
\dt

-- Exit
\q
```

Expected outcomes:

- `postgis_full_version()` returns a version string (contains `3.5.2`)
- `sys_menu`, `sys_user`, and other system tables show non-zero row counts
- `map_datasource`, `map_layer`, and other map tables exist (row counts may be 0)
- `\dt` shows all initialized tables

---

## 8. (Optional) Create Template Database template_postgis

If you plan to create many new spatial databases, you can turn the already-extended `mapserver-manager` into a template:

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432
```

```sql
-- 1. Drop all other connections to mapserver-manager
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE datname = 'mapserver-manager' AND pid <> pg_backend_pid();

-- 2. Promote to template database
UPDATE pg_database SET datistemplate = TRUE WHERE datname = 'mapserver-manager';
VACUUM FREEZE "mapserver-manager";

-- Later, create new spatial DBs via:
-- CREATE DATABASE newgis TEMPLATE = "mapserver-manager";
```

This is optional and does not affect rust-gisserver.

---

## 9. FAQ

### Q1: Port 5432 is already in use

Find which process occupies 5432:

```powershell
netstat -ano | findstr :5432
```

If it's another PostgreSQL instance, **uninstall the old one first**. If you must run them side-by-side, change the PostgreSQL port to **5434** (or any free port) during installation, but then update **all** port references in this document — in Section 5, Section 6, Section 7, and later in the rust-gisserver `.env` file.

> This document uses **5432** throughout.

### Q2: Forgot the postgres password

Temporarily switch authentication to `trust` in pg_hba.conf:

1. Locate `D:\PostgreSQL\16\data\pg_hba.conf`
2. Change all `host` and `local` entries from `md5` / `scram-sha-256` to `trust`
3. Restart PostgreSQL: `net stop postgresql-x64-16 && net start postgresql-x64-16`
4. `psql -U postgres -p 5432` logs in without a password, then run `ALTER USER postgres WITH PASSWORD 'new-password';`
5. Restore `pg_hba.conf` to its original auth methods and restart the service

### Q3: `CREATE EXTENSION postgis` fails with "could not open extension control file"

Almost always means **PostGIS was installed into a different directory than PostgreSQL**. Uninstall PostGIS, reinstall it and **manually point the Installation Directory to your PostgreSQL 16 directory** (e.g. `D:\PostgreSQL\16`).

### Q4: Chinese characters are garbled when running SQL scripts

Set UTF-8 codepage and client encoding before running psql:

```powershell
chcp 65001
cd /d "D:\PostgreSQL\16\bin"
$env:PGCLIENTENCODING = "UTF8"
psql -U postgres -p 5432 -d "mapserver-manager" -f "D:\...\system.sql"
```

Or, inside psql, run `SET client_encoding = 'UTF8';` before `\i`.

### Q5: Remote machines cannot connect to this PostgreSQL

Check three things:

1. **pg_hba.conf**: append a line at the bottom: `host    all             all             0.0.0.0/0               scram-sha-256`
2. **postgresql.conf**: find `listen_addresses` and change it to `listen_addresses = '*'`
3. **Windows Firewall**: allow inbound TCP on port 5432

Restart PostgreSQL after all changes.

---

## 10. References

- PostgreSQL 16 official docs: https://www.postgresql.org/docs/16/
- PostGIS 3.5 official docs: https://postgis.net/docs/
- PostGIS download page: https://postgis.net/install/
- PostgreSQL Windows installers: https://www.postgresql.org/download/windows/

---

**Next document**: [rust-gisserver-Setup-Installation.en.md](./rust-gisserver-Setup-Installation.en.md) — after finishing this guide, proceed with installing the rust-gisserver service.
