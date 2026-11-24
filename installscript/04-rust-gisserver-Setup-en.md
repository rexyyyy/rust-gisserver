# rust-gisserver_Setup Installation Guide (Windows)

*rust-gisserver Installation Documentation 3/4 · NSSM service · English · unified port 5432*

Prerequisite: manual 1/4 (PostgreSQL 16 + PostGIS 3.5) is complete and database `mapserver-manager` is initialized.

## Unified conventions

| Item | Value |
| --- | --- |
| Database port | 5432 (unified project-wide; `.env` and XML defaults agree, **no manual override needed**) |
| Database name | mapserver-manager (hyphen) |
| map-api port | 8080 |
| manager port | 6180 |
| Services | mapserve (map-api), map-manager (manager) |

## 1. Pre-install checklist

| # | Check | How to verify |
| --- | --- | --- |
| 1 | PostgreSQL 16 installed and running | `services.msc` → `postgresql-x64-16` Running |
| 2 | PostGIS enabled | psql: `SELECT postgis_full_version();` returns a version |
| 3 | DB `mapserver-manager` created and initialized | `SELECT count(*) FROM sys_user;` returns non-zero |
| 4 | Port 5432 reachable | `psql -U postgres -p 5432 -d mapserver-manager -c "select 1;"` |

If any check fails, go back to manual 1/4 first.

## 2. Install rust-gisserver_Setup.exe

Run `installscript\rust-gisserver_Setup.exe` as Administrator:

1. Welcome → Next.
2. Installation Directory: default `C:\Program Files\rust-gisserver`, recommended `D:\rust-gisserver`. Remember it as `<INSTALLDIR>`.
3. Ports: map-api default **8080**, manager default **6180**; change if taken and remember the new values.
4. DATA_ROOT: default `<INSTALLDIR>\data\`; fill in a full absolute path (e.g. `E:\gisdata`) if imagery/tiles live elsewhere.
5. NSSM service: check "Register as Windows service (auto-start)"; service names `mapserve` / `map-manager`, running as LocalSystem (data directory must be readable by it).
6. Next → Install → Finish.

The installer: ① writes install dir, ports and DATA_ROOT into `.env` and XML; ② registers both services via NSSM.

## 3. Post-install directory layout

```text
<INSTALLDIR>\
├─ .env                      ← MUST EDIT: DB password, JWT secret, DATA_ROOT
├─ config\
│  ├─ manager.xml            ← manager primary config
│  ├─ mapserver.xml          ← map-api primary config
│  └─ cog\ vector\ postgis\ arcgis\ 3dtiles\ ...  ← layer XMLs
├─ bin\                      ← manager.exe / mapserve.exe / PROJ DLLs
├─ web\                      ← frontend static files (served by manager)
├─ data\                     ← default data_root
├─ cache\foyer\              ← L2 disk cache
├─ share\proj\               ← PROJ data
├─ logs\                     ← runtime logs
├─ tools\                    ← nssm.exe etc.
└─ sql\                      ← DB init scripts (manual 1/4)
```

## 4. Post-install configuration (required)

Open `<INSTALLDIR>\.env` in Notepad and set at least:

```ini
# DB password: same as the postgres password chosen during install, both lines identical
MANAGER_DB_PASSWORD=<your postgres password>
MAPSERVER_DB_PASSWORD=<your postgres password>

# DB port: unified to 5432, no change needed
MANAGER_DB_PORT=5432
MAPSERVER_DB_PORT=5432

# JWT secret: production must be random (openssl rand -hex 32)
MANAGER_JWT_SECRET=<random long string>

# Data root: set only if data is not under the default data\
# DATA_ROOT=E:\gisdata
```

> **Port note**: the DSN templates in `manager.xml` / `mapserver.xml` are now `${MANAGER_DB_PORT:-5432}` and `.env` also says 5432 — they agree. If 5432 is occupied and you must change it, update `-Port`, both `.env` `*_DB_PORT` values, and the docker-compose mapping together.

### 4.1 Configure data_root

`data_root` only affects how map-api finds layer files; it has nothing to do with database tables. Priority: `DATA_ROOT` in `.env` > `${DATA_ROOT:-data}` in `mapserver.xml` (the `data` subfolder of the process working directory).

- Data under `<INSTALLDIR>\data\`: do nothing.
- Data elsewhere: append `DATA_ROOT=E:\gisdata` to `.env`, then restart the services.

## 5. Restart and verify

After editing `.env` or XML, restart services (Administrator PowerShell):

```powershell
net stop map-manager
net stop mapserve
net start map-manager
net start mapserve
sc query map-manager | findstr STATE    # expect STATE : 4 RUNNING
sc query mapserve   | findstr STATE
```

Open in a browser:

| Service | URL | Expected |
| --- | --- | --- |
| map-api health | http://127.0.0.1:8080/health | JSON `{"status":"ok",...}` |
| manager health | http://127.0.0.1:6180/manager-api/health | JSON response |
| Admin UI | http://127.0.0.1:6180/ | Login page (trailing slash) |

## 6. Common operations cheat sheet

```powershell
# Start / stop / status
net start map-manager ; net start mapserve
net stop  map-manager ; net stop  mapserve
sc query map-manager | findstr STATE

# Tail logs
Get-Content "<INSTALLDIR>\logs\manager.err.log" -Tail 50 -Wait
Get-Content "<INSTALLDIR>\logs\map-api.err.log" -Tail 50 -Wait

# NSSM
<INSTALLDIR>\tools\nssm.exe list
<INSTALLDIR>\tools\nssm.exe edit map-manager

# Firewall (admin)
New-NetFirewallRule -DisplayName "rust-gisserver Manager" -Direction Inbound -Protocol TCP -LocalPort 6180 -Action Allow
New-NetFirewallRule -DisplayName "rust-gisserver Map-API" -Direction Inbound -Protocol TCP -LocalPort 8080 -Action Allow
```

## 7. Uninstall

Run `<INSTALLDIR>\Uninstall.exe` or Control Panel → Programs and Features → rust-gisserver → Uninstall.

The uninstaller removes both Windows services and NSSM registry entries, and deletes program files. It **keeps** `config\`, `data\`, `cache\`, `logs\`, and `.env`. Database `mapserver-manager` is unaffected. Complete cleanup requires manually deleting the remaining directory.

## 8. FAQ

| Symptom | Cause | Fix |
| --- | --- | --- |
| Startup fails, log says `could not connect to server` | Wrong DB host/port in `.env` | Confirm 127.0.0.1 and 5432, then restart |
| Startup fails, `password authentication failed` | `*_DB_PASSWORD` does not match real password | Set both to the PostgreSQL password and restart |
| Blank page / 404 in browser | Wrong port or missing trailing slash | Use `http://127.0.0.1:6180/` and check actual ports |
| Changed `data_root` but layers still missing | No restart / wrong path | Ensure the `DATA_ROOT` line is active, restart services |
| Forgot JWT secret | Needs reset | Generate a new random value, restart; all tokens become invalid |
| Service exits seconds after start | exe crashed | Run the exe in a shell to see output; common cause: PROJ DLL missing — check `share\proj\` and `bin\proj_9.dll` |
| Admin UI opens but layer list is empty | Data directory not mounted correctly | Check `DATA_ROOT`, verify files with `Test-Path` |

---

When done: PostgreSQL 16 on port 5432, PostGIS enabled, `mapserver-manager` initialized, both services RUNNING. Open http://127.0.0.1:6180/ to reach the admin UI.
