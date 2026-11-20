# rust-gisserver_Setup Installation Guide (Windows)

> **Target Audience**: Users who have just finished installing PostgreSQL + PostGIS and initialized the `mapserver-manager` database.
> **Database Port**: **5432** (PostgreSQL default, matches the previous PostgreSQL guide)
> **Database Name**: `mapserver-manager`
> **Installer**: `installscript\rust-gisserver_Setup.exe`

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Install rust-gisserver_Setup.exe](#2-install-rust-gisserver_setupexe)
3. [Post-Installation Directory Layout](#3-post-installation-directory-layout)
4. [🔧 Post-Installation Configuration (MANDATORY!)](#4--post-installation-configuration-mandatory)
5. [Restart Services to Apply Config](#5-restart-services-to-apply-config)
6. [Verify Services Are Running Correctly](#6-verify-services-are-running-correctly)
7. [Cheat Sheet: Common Operations](#7-cheat-sheet-common-operations)
8. [Uninstall](#8-uninstall)
9. [FAQ](#9-faq)
10. [Configuration Reference Table](#10-configuration-reference-table)

---

## 1. Prerequisites

Before starting the installer, **all** of the following must be done:

| # | Check | How to Verify |
|---|---|---|
| 1 | PostgreSQL 16.7 is installed and running | `services.msc` → `postgresql-x64-16` shows "Running" |
| 2 | PostGIS 3.5.2 is enabled | In psql: `SELECT postgis_full_version();` returns `3.5.2` |
| 3 | Database `mapserver-manager` created | In psql: `\l` lists it |
| 4 | Database initialized (system.sql + map.sql) | In psql: `SELECT count(*) FROM sys_user;` returns non-zero |
| 5 | Port 5432 is listening | PostgreSQL binds to 5432 |

If any item fails, **go back and complete** [PostgreSQL-PostGIS-Installation.en.md](./PostgreSQL-PostGIS-Installation.en.md).

---

## 2. Install rust-gisserver_Setup.exe

Run `installscript\rust-gisserver_Setup.exe` as **Administrator**. Follow the wizard:

1. **Welcome** → Next
2. **Installation Directory**:
   - Default: `C:\Program Files\rust-gisserver`
   - Recommendation: `D:\rust-gisserver` (a data drive with room)
   - ⚠️ Write this path down — `<INSTALLDIR>` is used throughout this document
3. **Port Configuration**:
   - **map-api port**: default **8080** (public map service)
   - **manager port**: default **6180** (admin backend)
   - Change if those ports are already taken (e.g. 8081 / 6181). Remember what you picked.
4. **DATA_ROOT (data root directory)**:
   - Default: `<INSTALLDIR>\data\` (created by installer automatically)
   - If your imagery/tiles/PMTiles/3D Tiles live on another drive (e.g. `E:\gisdata\`), **fill in the full absolute path now**
   - You can also change it later (see Section 4.2)
5. **NSSM Service**: Check **"Register as Windows service (auto-start on boot)"**
   - Service names: `r-mapserver` (map-api) and `r-manager` (admin backend)
   - Runs as `LocalSystem`. The data directory must be readable by this account.
6. Next → Install → Finish

> The installer handles two things automatically: ① writes install dir, ports, and DATA_ROOT into `.env` and XML configs; ② registers both exes as Windows services via NSSM.

---

## 3. Post-Installation Directory Layout

```
<INSTALLDIR>\
├─ rust-gisserver_Setup.exe          ← installer copy
├─ .env                               ← ⭐ MUST EDIT! (DB password, JWT secret, ports)
├─ config\
│  ├─ manager.xml                     ← admin backend primary config
│  ├─ mapserver.xml                   ← map engine primary config
│  ├─ cog\ vector\ postgis\ arcgis\ 3dtiles\ ...  ← per-layer XMLs
│  └─ service-state.xml               ← service online/offline state
├─ bin\
│  ├─ manager.exe                     ← admin backend executable
│  └─ mapserve.exe                     ← map engine executable
├─ web\                               ← frontend static files (served by manager.exe)
├─ data\                              ← ⭐ default data_root, stores imagery/tiles
│  ├─ pmtiles\                        ← PMTiles archives
│  ├─ mbtiles\                        ← MBTiles tile bundles
│  ├─ cog\                            ← Cloud Optimized GeoTIFFs
│  ├─ vector\                         ← GeoJSON / Shapefiles
│  ├─ 3dtiles\                        ← 3D Tiles services
│  └─ osgb\                           ← OSGB → 3D Tiles source
├─ cache\
│  ├─ foyer\                          ← foyer L2 disk cache (survives restart)
│  └─ ...                             ← other cache files
├─ share\proj\                        ← PROJ projection database
├─ logs\                              ← runtime logs
├─ tools\
│  └─ nssm.exe                        ← Windows service wrapper
├─ svc_mapapi.cmd                     ← NSSM-hosted map-api startup script
├─ svc_manager.cmd                    ← NSSM-hosted manager startup script
└─ Uninstall.exe                      ← uninstaller
```

---

## 4. 🔧 Post-Installation Configuration (MANDATORY!)

The installer only wrote install dir, ports, and DATA_ROOT. You **must** manually fill in the following: database password, JWT secret, and (most importantly) change the default DB port from **5434 → 5432**.

### 4.1 Edit `.env` — Database Connection

Open `<INSTALLDIR>\.env` with Notepad or VS Code. **These 4 items must be changed**:

```ini
# ============================================================
# ===== REQUIRED: database password =====
# ============================================================
MANAGER_DB_PASSWORD=YourPassword123!      ← replace with the postgres password you set during PostgreSQL install
MAPSERVER_DB_PASSWORD=YourPassword123!    ← same value for both

# ============================================================
# ===== REQUIRED: database port (default is 5434, change to 5432!) =====
# ============================================================
MANAGER_DB_PORT=5432                      ← PostgreSQL default port, NOT 5434
MAPSERVER_DB_PORT=5432                    ← same here

# ============================================================
# ===== If PostgreSQL is on another machine, uncomment and fill in =====
# ============================================================
# MANAGER_DB_HOST=192.168.1.100
# MAPSERVER_DB_HOST=192.168.1.100

# ============================================================
# ===== REQUIRED: JWT signing secret (PRODUCTION MUST RANDOMLY GENERATE!) =====
# ============================================================
# Generate in PowerShell:
#   openssl rand -hex 32
#   (if openssl missing: -join ((48..57)+(97..122) | ForEach-Object {[char]$_} | Get-Random -Count 64)
MANAGER_JWT_SECRET=replace-with-the-random-value   ← 64 hex characters (32 bytes)

# ============================================================
# ===== data_root: set when data lives on a different drive =====
# ============================================================
# If not set, ${DATA_ROOT:-data} in XML falls back to relative path 'data'
# Example — imagery/tiles on E: drive:
# DATA_ROOT=E:\gisdata

# ============================================================
# ===== Below were set by the installer, change only if needed =====
# ============================================================
MAPSRV_HOST=127.0.0.1                    ← where manager calls back to map-api (NOT the bind address!)
MAPSRV_PORT=8080                         ← map-api port (written by installer)
MAPSERVER_MANAGER_URL=http://127.0.0.1:6180  ← manager URL (map-api probes this for health)
GIS_ENCODING_REGION=zh-CN                ← path/filename encoding region
```

> 🚨 **Why do MANAGER_DB_PORT and MAPSERVER_DB_PORT default to 5434?**
> Because `manager.xml` and `mapserver.xml` both use `${MANAGER_DB_PORT:-5434}` (default 5434) in their DSN templates. But in the previous PostgreSQL guide we used **5432** (PostgreSQL's official default). So you **must** explicitly override both ports to 5432 in `.env`.

#### Real DSN format reference (how env vars are injected)

**manager.xml line 21**:
```xml
<database url="postgresql://postgres:${MANAGER_DB_PASSWORD}@${MANAGER_DB_HOST:-localhost}:${MANAGER_DB_PORT:-5434}/mapserver-manager" pool_max="16"/>
```

**mapserver.xml line 88**:
```xml
<admin dsn="postgresql://postgres:${MAPSERVER_DB_PASSWORD}@${MAPSERVER_DB_HOST:-localhost}:${MAPSERVER_DB_PORT:-5434}/mapserver-manager"/>
```

After `.env` is filled, the rendered DSN looks like:
```
postgresql://postgres:YourPassword123!@localhost:5432/mapserver-manager
```

### 4.2 Configure data_root

data_root is the **root directory where map-api looks for layer files**. It affects only how `source.path` values resolve in layer XMLs — it has **no effect** on the manager's database tables.

Priority order:

1. **`DATA_ROOT` in `.env`** (you set it) → highest priority
2. **Not set**: `${DATA_ROOT:-data}` falls back to the `data` subdirectory next to where the process was started from

#### Scenario A: data stays inside the install directory (default)

Do nothing. The installer already created `<INSTALLDIR>\data\`, and `${DATA_ROOT:-data}` resolves to it automatically.

#### Scenario B: data lives on another drive (recommended for production)

Large imagery/tiles/PMTiles/3D Tiles should NOT live on the C: system drive. Suppose all data is at `E:\gisdata\`:

1. Append a line to `.env`:
   ```ini
   DATA_ROOT=E:\gisdata
   ```
2. Now every `${DATA_ROOT:-data}` in `mapserver.xml` expands to `E:\gisdata`, including:
   - `<server data_root="...">` → working root for map-api
   - `<wuhua_buildings>${DATA_ROOT:-data}/pmtiles/wuhua_buildings.pmtiles</wuhua_buildings>` → resolves to `E:\gisdata\pmtiles\wuhua_buildings.pmtiles`
3. Make sure `E:\gisdata\` has subdirectories matching what your layer XMLs reference (e.g. `pmtiles\`, `mbtiles\`, `cog\`)

> 💡 **data_root only affects map-api's file lookup, not PostgreSQL tables.** Database tables always live in PostgreSQL, completely independent of data_root.

#### 🚀 Quick Start — Point data_root to `E:\data\1` (Real Test Data)

If your imagery / tiles / PMTiles / 3D Tiles live under `E:\data\1\` (with `cog\`, `mbtiles\`, `pmtiles\`, `postgis\`, `3dtiles\` subdirs already there), do this:

**1) Open `.env` with Notepad**

Location: `D:\rust-gisserver\.env`

One-line PowerShell shortcut:
```powershell
notepad "D:\rust-gisserver\.env"
```

**2) Append one line at the end**

```ini
DATA_ROOT=E:\data\1
```

Full `.env` with `E:\data\1` filled in:
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

# ===== Add this line =====
DATA_ROOT=E:\data\1
```

**3) Restart both Windows services (Admin shell required)**

Non-admin PowerShell will report "Access is denied". Use an **elevated** PowerShell / cmd:
```powershell
net stop map-serve
net stop map-manager
net start map-serve
net start map-manager
```

Or via NSSM (also needs admin):
```powershell
D:\rust-gisserver\tools\nssm.exe restart map-serve
D:\rust-gisserver\tools\nssm.exe restart map-manager
```

**4) Verify**

Check the map-api log after restart:
```powershell
Get-Content "D:\rust-gisserver\logs\mapapi_out.log" -Tail 30
```

Then open **http://127.0.0.1:8080/** (map-api) or **http://127.0.0.1:6180/** (manager). Your catalog should now list layers found under `E:\data\1\cog\`, `E:\data\1\mbtiles\`, etc.

**5) Troubleshooting cheat-sheet**

| Symptom | Cause | Fix |
|---|---|---|
| Empty layer list / "file not found" | `DATA_ROOT` not applied or wrong path | Confirm no leading `#` in front of the line; use backslashes; **restart services** |
| `ENOENT` / `No such file or directory` in mapapi_out.log | Missing subdirs under `E:\data\1\` | Check that XML-referenced subdirs exist (`cog\`, `mbtiles\`, `pmtiles\`, `postgis\`...) |
| Service won't start | Malformed `.env` (smart quotes / trailing spaces) | Delete the DATA_ROOT line and re-type it by hand |
| Changed `.env` but nothing happened | `.env` is only read at startup | You **must** stop → start |

### 4.3 Other Configurable Items (.env / XML Cross-Reference)

#### All `.env` variables

| Variable | Description | Default |
|---|---|---|
| `MANAGER_DB_HOST` | PostgreSQL host | `localhost` (via `${MANAGER_DB_HOST:-localhost}`) |
| `MANAGER_DB_PORT` | PostgreSQL port | `5434` (**must change to 5432**) |
| `MANAGER_DB_PASSWORD` | PostgreSQL password | **required, no default** |
| `MANAGER_JWT_SECRET` | JWT signing secret | **required, no default** |
| `MANAGER_DSN_KEY` | DSN encryption key (optional) | empty (derived from JWT secret when blank) |
| `MAPSERVER_DB_HOST` | Same, for map-api | `localhost` |
| `MAPSERVER_DB_PORT` | Same | `5434` (**must change to 5432**) |
| `MAPSERVER_DB_PASSWORD` | Same | **required, no default** |
| `MAPSRV_HOST` | What manager uses to call map-api | `127.0.0.1` (change IP for cross-machine deploy) |
| `MAPSRV_PORT` | map-api port | `8080` (written by installer) |
| `MAPSERVER_MANAGER_URL` | map-api probes this URL for manager health | `http://127.0.0.1:6180` |
| `GIS_ENCODING_REGION` | path/filename encoding region | `zh-CN` |
| `DATA_ROOT` | data root directory | not set → falls back `<INSTALLDIR>\data\` |

#### Manual edits in `config/manager.xml`

```xml
<server addr="0.0.0.0:6180" ...>          <!-- bind address: change to 127.0.0.1:6180 to localhost-only -->
  <cors_origins>
    <item>http://localhost:5100</item>    <!-- vite dev origin; remove for same-origin production -->
  </cors_origins>
</server>
<database url="..."/>                      <!-- DSN injected via .env, no manual edit needed -->
<storage avatar_dir="storage/avatar"/>     <!-- user avatar storage; use absolute path in production -->
<mapsvc base_url="..."/>                   <!-- reads ${MAPSRV_HOST}:${MAPSRV_PORT}; change .env for cross-machine -->
<security jwt_secret="...">               <!-- reads ${MANAGER_JWT_SECRET}; put production key in .env -->
```

#### Manual edits in `config/mapserver.xml`

```xml
<server host="0.0.0.0" port="8080" ...>
  <!-- host defaults to 0.0.0.0 (all NICs); change to 127.0.0.1 for localhost-only -->
  <!-- port written by installer -->
  <!-- cache_mib / foyer_cache_mib / disk_cache.capacity_mib: cache budget, scale for production -->
</server>
<manager url="${MAPSERVER_MANAGER_URL:-...}"/>  <!-- cross-machine: change .env -->
<admin dsn="..."/>                               <!-- injected via .env, no manual edit -->
<!--
<data_token>changeme</data_token>                <!-- uncomment to enable data-plane auth -->
<s3 endpoint="..." region="auto" access_key="..." secret_key="..."/>  <!-- S3 storage -->
-->
```

### 4.4 Cross-Machine Deployment (manager and map-api on separate boxes)

When manager (admin backend) and map-api (map engine) run on **two separate machines**:

**map-api machine** (IP example: `192.168.1.100`):
```ini
# ⚠️ MAPSRV_HOST is NOT the bind address! Bind address is <server host="0.0.0.0"> in mapserver.xml
MAPSRV_HOST=192.168.1.100   ← this machine's IP (manager will call back to this)
MAPSRV_PORT=8080

# Both services can share the same PostgreSQL (use DB_HOST for cross-machine DB)
MAPSERVER_DB_HOST=192.168.1.50   ← PostgreSQL machine IP
MAPSERVER_DB_PORT=5432
MAPSERVER_DB_PASSWORD=YourPassword123!

# Manager URL (health probe so map-api knows who manages it)
MAPSERVER_MANAGER_URL=http://192.168.1.101:6180
```

**manager machine** (IP example: `192.168.1.101`):
```ini
# Manager calls back to map-api — fill in the map-api machine's IP
MAPSRV_HOST=192.168.1.100   ← map-api machine IP
MAPSRV_PORT=8080

# Same PostgreSQL
MANAGER_DB_HOST=192.168.1.50
MANAGER_DB_PORT=5432
MANAGER_DB_PASSWORD=YourPassword123!

# Manager's own URL
MAPSERVER_MANAGER_URL=http://192.168.1.101:6180
```

**Firewall**: On each machine, allow inbound TCP for its own service ports (manager → 6180, map-api → 8080, both → PostgreSQL 5432 for DB connections).

---

## 5. Restart Services to Apply Config

After editing `.env` or XML configs, **restart the Windows services** to reload. Run in PowerShell (as Administrator):

```powershell
# Stop both services
net stop r-manager
net stop r-mapserver

# Start both services
net start r-manager
net start r-mapserver

# Check status
sc query r-manager
sc query r-mapserver
```

> ⚠️ Editing `.env` requires **both** services to be restarted (manager and map-api each read their own subset of env vars).
> Editing `config/manager.xml` only needs `r-manager` restarted.
> Editing `config/mapserver.xml` only needs `r-mapserver` restarted.
> Editing per-layer XMLs (`config/cog/*.xml`, etc.) does **NOT** need a restart — use "Hot Reload" in the admin UI.

---

## 6. Verify Services Are Running Correctly

### 6.1 Service Status Check

```powershell
sc query r-manager | findstr STATE
sc query r-mapserver | findstr STATE
```

Expected output: `STATE : 4 RUNNING`.

### 6.2 Browser Access

| Service | URL | Expected |
|---|---|---|
| map-api health check | http://127.0.0.1:8080/health | JSON `{"status":"ok",...}` |
| manager health check | http://127.0.0.1:6180/manager-api/health | JSON response |
| Admin UI | http://127.0.0.1:6180/ | Login page |

If you changed ports during install, use the actual ports you configured.

### 6.3 Confirm DB Connectivity via psql

```powershell
cd /d "D:\PostgreSQL\16\bin"
psql -U postgres -p 5432 -d "mapserver-manager"
```

```sql
-- Confirm manager tables are readable
SELECT count(*) FROM sys_user;
SELECT count(*) FROM map_datasource;
```

If every query returns a number cleanly → the DB connection works. Service startup failures always trace back to wrong password or wrong port in `.env`.

---

## 7. Cheat Sheet: Common Operations

### Start / Stop Services

```powershell
net stop r-manager
net stop r-mapserver
net start r-manager
net start r-mapserver
```

### Check Service Status

```powershell
sc query r-manager
sc query r-mapserver
```

### Real-time Log Tail

```powershell
Get-Content "<INSTALLDIR>\logs\manager.out.log" -Tail 50 -Wait
Get-Content "<INSTALLDIR>\logs\manager.err.log" -Tail 50 -Wait
Get-Content "<INSTALLDIR>\logs\map-api.out.log" -Tail 50 -Wait
Get-Content "<INSTALLDIR>\logs\map-api.err.log" -Tail 50 -Wait
```

### Firewall Rule (PowerShell, Admin)

```powershell
# Manager port
New-NetFirewallRule -DisplayName "rust-gisserver Manager" -Direction Inbound `
    -Protocol TCP -LocalPort 6180 -Action Allow

# map-api port
New-NetFirewallRule -DisplayName "rust-gisserver Map-API" -Direction Inbound `
    -Protocol TCP -LocalPort 8080 -Action Allow
```

### NSSM Service Management (advanced)

```powershell
# List all NSSM-registered services
<INSTALLDIR>\tools\nssm.exe list

# Edit service startup parameters (opens GUI)
<INSTALLDIR>\tools\nssm.exe edit r-manager
<INSTALLDIR>\tools\nssm.exe edit r-mapserver

# Remove services (used during uninstall)
<INSTALLDIR>\tools\nssm.exe remove r-manager confirm
<INSTALLDIR>\tools\nssm.exe remove r-mapserver confirm
```

### Switch to Manual Startup

To disable auto-start and start services manually when needed:

```powershell
sc config r-manager start= demand
sc config r-mapserver start= demand
```

---

## 8. Uninstall

Run `<INSTALLDIR>\Uninstall.exe` or Control Panel → Programs and Features → rust-gisserver → Uninstall.

The uninstaller:
- Removes Windows services (`r-manager`, `r-mapserver`)
- Removes NSSM registry entries
- Deletes program executables

**⚠️ These are KEPT** (safe data, never deleted automatically):
- `<INSTALLDIR>\config\` (all XML configs)
- `<INSTALLDIR>\data\` (imagery/tiles data)
- `<INSTALLDIR>\cache\` (disk cache)
- `<INSTALLDIR>\logs\` (logs)
- `<INSTALLDIR>\.env` (env vars)

For **complete cleanup**, manually delete the remaining directory after uninstall. The PostgreSQL database `mapserver-manager` is unaffected (it lives outside the install directory).

---

## 9. FAQ

### Q1: Startup fails, log says "could not connect to server: No such file or directory"

One of `MANAGER_DB_HOST`, `MAPSERVER_DB_HOST`, `MANAGER_DB_PORT`, `MAPSERVER_DB_PORT` in `.env` is wrong. Check `<INSTALLDIR>\logs\manager.err.log` or `map-api.err.log` for `FATAL` or `error connecting`.

**Most common cause**: port is still 5434 while PostgreSQL listens on 5432. Change to 5432 and restart.

### Q2: Startup fails, log says "password authentication failed"

`MANAGER_DB_PASSWORD` or `MAPSERVER_DB_PASSWORD` doesn't match the real PostgreSQL password. Fix `.env` and restart.

### Q3: Service starts but browser shows blank page / 404

- Is the port correct? `MAPSRV_PORT` in `.env` must match `<server port="...">` in `config/mapserver.xml`
- For cross-machine deploy, `MAPSRV_HOST` in `.env` CANNOT be `0.0.0.0` (that's the bind address). Use the actual IP for call-back.
- Manager UI is served at `http://<manager-address>:6180/` (trailing slash), NOT at the `manager-api/` path

### Q4: Changed data_root but layers still can't find files

Check 3 things:

1. Is `DATA_ROOT=E:\gisdata` actually in `.env`? (If missing, XML falls back to relative path `data`)
2. After adding it, did you **restart the r-mapserver service**? (Services read `.env` at startup only)
3. Is the absolute path correct? Verify with PowerShell `Test-Path "E:\gisdata\pmtiles\..."`

### Q5: Forgot JWT secret / need to reset

Generate a new random value, overwrite `MANAGER_JWT_SECRET` in `.env`, restart `r-manager`. All previously issued tokens immediately become invalid — users will need to log in again. This is expected.

### Q6: Service exits a few seconds after starting

The exe crashed on startup. Diagnose directly:

1. Run the exe manually from PowerShell:
   ```powershell
   cd "<INSTALLDIR>\bin"
   $env:PATH += ";<INSTALLDIR>\bin"
   $env:PATH += ";<INSTALLDIR>\share\proj"
   $env:PROJ_LIB = "<INSTALLDIR>\share\proj"
   .\manager.exe
   .\mapserve.exe
   ```
2. Check `.err.log` for the FATAL line
3. Most common: PROJ runtime not found → ensure `share\proj\` exists, or add the dir containing `proj_9.dll` to PATH

### Q7: Service running as LocalSystem fails to access network / UNC paths

LocalSystem has no permissions to reach network shares (`\\server\share\`) or other machines' PostgreSQL. Two solutions:

- **Recommended**: Use `nssm.exe edit r-manager` / `nssm.exe edit r-mapserver` to change the service account to a domain or local user account
- **Workaround**: Map all network paths to drive letters (`net use E: \\server\share`), but LocalSystem won't see user-mapped drives

---

## 10. Configuration Reference Table

### 10.1 `.env` vs XML: Same-Named Settings

| Setting | `.env` | `config/manager.xml` | `config/mapserver.xml` | **Which Wins** |
|---|---|---|---|---|
| DB Host | `MANAGER_DB_HOST` / `MAPSERVER_DB_HOST` | injected into `<database url>` via `${MANAGER_DB_HOST:-localhost}` | injected into `<admin dsn>` via `${MAPSERVER_DB_HOST:-localhost}` | **.env takes precedence** |
| DB Port | `MANAGER_DB_PORT` / `MAPSERVER_DB_PORT` | `${MANAGER_DB_PORT:-5434}` | `${MAPSERVER_DB_PORT:-5434}` | **.env takes precedence** (⚠️ default 5434, MUST change to 5432) |
| DB Password | `MANAGER_DB_PASSWORD` / `MAPSERVER_DB_PASSWORD` | `${MANAGER_DB_PASSWORD}` | `${MAPSERVER_DB_PASSWORD}` | **.env REQUIRED** (no XML default) |
| map-api port | `MAPSRV_PORT` | — | `<server port="8080">` | Installer syncs both; `.env` not injected here. Don't change after install unless you also update XML. |
| data_root | `DATA_ROOT` | — | `<server data_root="${DATA_ROOT:-data}">` and `<wuhua_buildings>${DATA_ROOT:-data}/pmtiles/...` | **.env takes precedence** |
| manager URL | `MAPSERVER_MANAGER_URL` | — | `<manager url="${MAPSERVER_MANAGER_URL:-...}">` | **.env takes precedence** |
| JWT Secret | `MANAGER_JWT_SECRET` | `<security jwt_secret="${MANAGER_JWT_SECRET}">` | — | **.env REQUIRED** |
| map-api host (manager call-back) | `MAPSRV_HOST` | injected into `<mapsvc base_url="http://${MAPSRV_HOST:-127.0.0.1}:${MAPSRV_PORT:-8080}">` | — | **.env takes precedence** |
| manager bind address | — | `<server addr="0.0.0.0:6180">` | — | **XML only** (written by installer) |
| map-api bind address | — | — | `<server host="0.0.0.0">` | **XML only** (⚠️ this is the bind address, NOT `.env`'s MAPSRV_HOST!) |
| Cache budget | — | — | `cache_mib` / `foyer_cache_mib` / `disk_cache.capacity_mib` | **XML only** |
| data_token | — | — | `<data_token>changeme</data_token>` | **XML only** (uncomment to enable data-plane auth) |
| S3 credentials | — | — | `<s3 endpoint="..." region="auto" access_key="..." secret_key="...">` | **XML only** |
| DSN encryption | — | `<security dsn_cipher="des" dsn_key="${MANAGER_DSN_KEY}">` | — | **XML + .env combined** |

### 10.2 Quick Priority Summary

- **Passwords, secrets, DB port/host, DATA_ROOT** → `.env` required / takes precedence
- **Bind addresses (host/addr), cache budget, data_token, S3** → XML manual edit only
- **Ports** (map-api, manager) → installer synced to XML; no `.env` injection path, don't change unless you know what you're doing

---

## Done

If you followed both PostgreSQL and setup guides correctly, rust-gisserver should now be:
- ✅ PostgreSQL 16.7 running on port 5432
- ✅ PostGIS 3.5.2 enabled
- ✅ Database `mapserver-manager` initialized
- ✅ rust-gisserver_Setup installed with `.env` filled in
- ✅ Both services in RUNNING state

Open http://127.0.0.1:6180/ in your browser to access the rust-gisserver admin UI.
