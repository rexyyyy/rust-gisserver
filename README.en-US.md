# rust-gisserver

|                            |                 |
| -------------------------- | --------------- |
| [English](README.en-US.md) | [中文](README.md) |

> **First things first: rust-gisserver serves a very wide range of service types.** One engine provides WMS 1.1.1/1.3.0, WMTS 1.0.0, TMS/XYZ, MVT vector tiles (including dynamic PostGIS vectors), WFS, OGC API - Features, OGC API - Tiles, 3D Tiles/terrain pass-through, plus direct reads of COG / PMTiles / MBTiles / ArcGIS compact caches (`.tpk`/`.tpkx`). It also handles CGCS2000, Gauss-Kruger 3°/6° zones and UTM with identity-path output and optional PROJ reprojection.

**rust-gisserver** is built in Rust: **low runtime footprint, high performance and high concurrency**. It is a new benchmark for open GIS servers, designed for domestic coordinate systems (CGCS2000 / Gauss-Kruger) and large-scale concurrency. Core byte-level paths (TIFF/COG parsing, tile decoding, MVT encoding, PMTiles archive I/O, projection) are implemented from public format specifications, without heavy GDAL C bindings. PROJ is an optional runtime backend and the server falls back to built-in projection paths when it is absent.

> **Four pillars of advancement (migration-friendly):**
> ① **Smooth migration of traditional tile formats** — XYZ/TMS directories, MBTiles and PMTiles mount directly, **no re-tiling, no format conversion**;
> ② **ArcGIS compact caches read as-is** — Server cache directories, `.tpk` (compact v1), `.tpkx` (compact v2) and gis-wt `.webpdle` (WTC v1) through one read path;
> ③ **Direct serving of modern COG** — byte-range HTTP Range delivery, **no tiling pipeline, no preprocessing**;
> ④ **High-performance dynamic MVT from PostGIS** — vector tiles generated live from spatial tables/views, **change the DB, change the map**.
> See [Advanced data compatibility and migration](#advanced-data-compatibility-and-migration).

## Three components

```text
rust-gisserver/
├── gisserver/   # Core library: single package, feature-gated modules, publishable to crates.io
├── mapserver/   # Map server: self-contained tile/imagery/feature engine (binary mapserve)
├── manager/     # Admin console: Rust backend + Vite UI (catalog/publish/logs/styles)
├── docs/        # Architecture and per-service publishing guides
└── installscript/ # PostgreSQL/PostGIS and installer assets, SQL baselines
```

| Component     | Role          | Description                                                                                                                                        | License                              |
| ------------- | ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| **gisserver** | Core library  | Pure Rust data/protocol layer: COG/BigTIFF, tile rendering, MVT, PMTiles/MBTiles, WMS/WFS/OGC API, projection, PostGIS sources, path encoding      | MIT OR Apache-2.0                    |
| **mapserver** | Map server    | Ready-to-run HTTP services: WMS / WMTS / TMS / XYZ / MVT / PMTiles / MBTiles / COG / WFS / OGC API / 3D Tiles; five-stage cache + optional disk L2 | AGPL-3.0-only + commercial exception |
| **manager**   | Admin console | Web console: service catalog, publishing wizard (CRS/bbox validation), runtime monitoring, styles, cache and audit logs                            | AGPL-3.0-only + commercial exception |

> Licensing: **gisserver** is dual-licensed MIT OR Apache-2.0 and may be closed-source/commercial without a license fee. **mapserver** and **manager** are AGPL-3.0-only with a **Commercial License Exception**: internal use, personal study and open-source integration are free; SaaS or closed-source commercial integration requires a commercial license (see the end of [mapserver/LICENSE](mapserver/LICENSE) / [manager/LICENSE](manager/LICENSE), contact rex yan 853740877@qq.com).

## Why it is fast

- **No heavy dependencies**: pure Rust, no GDAL/tiff/mapbox C bindings; PROJ is optional and the default build has zero FFI / zero unsafe.
- **Call style first**: MVT cache hits 16–19 ms, XYZ raster tiles 10–21 ms/tile, archives/COG/3D Tiles use zero-decode HTTP Range pass-through; WMS GetMap is a fallback, not the basemap zoom path.
- **Five-stage cache**: output tiles, layer index, metadata documents, catalog metadata and feature key/value, unified in `CacheHub`, with TTL, manual purge and optional disk L2.
- **Concurrency**: rendering/encoding/blocking IO all run in `spawn_blocking`; tokio workers only schedule. Local files and HTTP Range sources use positional reads (`read_at`/`seek_read`) with no shared-fd cursor races; remote sources are isolated in a dedicated rayon pool.
- **Render fast path**: when request CRS == source CRS and the geotransform is affine, 262,144 per-pixel projections for a 512×256 GetMap drop to 5 (2.9× in a release micro-benchmark).

See [mapserver/README.md](mapserver/README.md) for measurements and implementation notes.

---

## Advanced data compatibility and migration

In one sentence: **your existing tile assets can be served without re-tiling, conversion or GDAL; new data goes straight to COG reads and dynamic PostGIS MVT — zero preprocessing end to end.** The four pillars in detail:

### 1) Traditional tile formats: smooth migration, no re-tiling

| Existing asset | Migration path | Services gained immediately |
| --- | --- | --- |
| `z/x/y.{png,jpg,pbf}` directory trees | Drop into a layer directory and discover | XYZ / TMS / WMTS (top-origin / bottom-origin y-flip automatic) |
| MBTiles (SQLite tile packages) | Read-only direct access, pooled connections + `prepare_cached` | All front ends: XYZ / TMS / WMTS / MVT |
| PMTiles v3 archives | Whole-archive HTTP Range delivery | `/pmtiles/{name}` and `pmtiles://` clients |
| Static `z/x/y` tile directories | Optionally pack into PMTiles | Hilbert clustering, blob dedup, gzip re-encoding |

MBTiles supports both PNG/JPEG/WebP raster and gzipped pbf vector packages with automatic TMS row flipping; publishing one archive yields every tile front end — no per-protocol re-tiling.

### 2) ArcGIS compact caches: four forms, one read path

Historical ArcGIS assets are **read as-is** — no unpack-and-retile, no format conversion:

| Form | Carrier | Byte layout | Grid config |
| --- | --- | --- | --- |
| ArcGIS Server cache directory | Directory | compact v1 / v2 | `Conf.xml` + `conf.cdi` |
| `.tpk` tile package | ZIP | compact v1 (separate `.bundlx`, column-major) | `Conf.xml` + `conf.cdi` |
| `.tpkx` (Pro Compact Tile Package) | ZIP | compact v2 (embedded index, row-major) | `root.json` |
| gis-wt `.webpdle` (WTC v1, native) | Directory | WTC v1 sparse index | `Conf.xml` + `conf.cdi` |

- **One logic**: all four normalize their grid scheme into `gisserver::arcgis_cache::definition::CacheDefinition`; differences are limited to directory layout and tile byte layout, so there is a single tile-reading implementation.
- **Definition reuse**: keyed by a content fingerprint (CRS, origin, tile size, DPI, LODs, PacketSize, tile root). One grid can back datasets with different extents and still share a record.
- **One-click import**: `POST /admin/import-tpk` performs unpack → cache-root promotion → atomic publish into `data_root/arcgis_tpk/`; a temp directory plus atomic rename leaves no half-finished output, and re-importing the same name overwrites with rollback.
- **Verified samples**: `road.tpk` (EPSG:4490, compact v1, L00–L08) and `bai.tpkx` (EPSG:4522, compact v2, L00–L04) are both recognized as `arcgis` by `/admin/discover` and publishable after import.

Format details: [docs/arcgis-cache-formats.md](docs/arcgis-cache-formats.md). Publishing steps: [ArcGIS cache guide](docs/services/arcgis-cache.en-US.md).

### 3) Modern COG: direct serving, no tiling pipeline

- Cloud Optimized GeoTIFF read directly from local paths or HTTP(S) / S3 pre-signed URLs via **HTTP Range 206** byte requests; QGIS / rasterio can open it remotely and GB-scale imagery is never loaded whole into memory.
- In-house TIFF / BigTIFF parser (IFD chains, overview-pyramid selection, rayon parallel decoding), **without GDAL or other heavy C bindings**.
- One COG serves WMS / WMTS / TMS / XYZ simultaneously, avoiding the long "tile → import → publish" pipeline; `/cog/{layer}` offers a zero-decode pass-through endpoint.
- Compatible with CGCS2000, Gauss-Kruger 3°/6° zones and UTM; identity output in the source CRS, optional PROJ backend for cross-CRS transforms.

### 4) High-performance dynamic MVT tiles from PostGIS

- **Change the DB, change the map**: generate MVT vector tiles live from PostGIS tables, views or SQL expressions — no pre-tiling, no cache-directory refresh.
- **Dual CRS endpoints**: `/{z}/{x}/{y}.pbf` (EPSG:3857) and `/{srid}/{z}/{x}/{y}.pbf` (source CRS units, e.g. 4543 metres or 4326 degrees, with no server-side `ST_Transform`); TileJSON declares bounds/crs/grid per endpoint.
- **Built for concurrency**: connection pools shared per DSN, PG≥11 / PostGIS≥3.0 version preflight, GiST index self-check at startup; bbox pre-filtering before encoding, `ST_SimplifyPreserveTopology` at low zooms; output-cache hits around 16–25 ms with optional disk L2 surviving restarts.
- **Publish without new tables**: `from` expressions publish SQL/view layers directly; bounds/count use statistics estimates with automatic fallback to exact scans.
- **Legacy vectors take the same road**: Shapefiles go through the admin "Import SHP" action (`shp2pgsql`, reading EPSG from `.prj` for CGCS2000 / WGS84). FileGDB is read by **Esri's official open-source FileGDB API** ([Esri/file-geodatabase-api](https://github.com/Esri/file-geodatabase-api), Apache-2.0, cross-platform C++, currently 1.5.5) — schemas, features, attributes and spatial queries are read directly from `.gdb` and written straight into PostGIS, **with no GDAL conversion or intermediate files**. Both become `postgis_table` and can publish dynamic MVT / WFS / OAF. The FileGDB API is linked only in the import path, so the core library stays free of heavy C bindings.

Publishing and tuning: [MVT guide](docs/services/mvt.en-US.md) and [architecture notes](docs/architecture.md).

> **Why this is more advanced**: traditional GIS servers treat format conversion + pyramid tiling + import as prerequisites to publishing, and any data change reruns the whole chain. rust-gisserver implements the byte layer in-house and faces existing assets (traditional tiles, ArcGIS compact caches) and modern standards (COG, dynamic PostGIS MVT) head-on: **zero re-tiling to migrate, zero preprocessing for new data, publish-and-serve**. Combined with pure-Rust zero-heavy-dependency design, the five-stage cache, positional reads and the `spawn_blocking` concurrency model, it delivers compatibility and performance together for domestic CRS and high-concurrency scenarios.

---

## Data formats and directory mapping

The system organizes catalogs by source kind: `cog/`, `vector/`, `pmtiles/`, `mbtiles/`, `postgis/`, `3dtiles/`, `terrain/` and `arcgis/`; `mvt/`, `osgb/` and `styles/` are reserved/auxiliary directories. A local test dataset on the author's machine (`E:\data\1`, with a per-format breakdown in its `README.md` §1.1) covers the assets below as migration test cases:

| Format / asset | Support | Publishable services | Sample location (`E:\data\1`) |
| --- | --- | --- | --- |
| COG / BigTIFF (`.tif`) | ✅ native | WMS / WMTS / XYZ | `cog/` (real EPSG:4326 imagery + fixture) |
| GeoJSON | ✅ native (→ `vector/`) | MVT / WMS / WMTS / XYZ / WFS / OAF | `geojson/`, `recon/` |
| MBTiles (raster / vector) | ✅ native | raster → WMTS/XYZ; vector → MVT | `mbtiles/`, `recon/` |
| PMTiles (raster / vector) | ✅ native | raster → WMTS/XYZ; vector → MVT | `pmtiles/` |
| ArcGIS Server cache (compact v1 / v2) | ✅ native (→ `arcgis/`) | WMTS / XYZ | `arcgis_cache/` (3×v1 + 1×v2) |
| `.tpk` / `.tpkx` compact tile packages | ✅ native (→ `arcgis/`) | WMTS / XYZ after import | `arcgis_tpk/`, `arcgis_cache/` |
| 3D Tiles (`tileset.json` + `.b3dm`) | ✅ native | `/3dtiles` pass-through | `3dtiles/` (recon / kmczc / kmczc_full) |
| Quantized-Mesh (`.terrainpack`) | ✅ native pass-through | `/terrain/{name}/layer.json` | `terrain/` |
| PostGIS spatial tables / views | ✅ native | **Dynamic MVT** / WMS / WFS / OAF | `postgis/` (pg_dump + view export) |
| Static `z/x/y` tile trees | ⚠️ pack first | WMTS/XYZ after packing to PMTiles/MBTiles | `xyz-tms/` (sample pending) |
| Raw OSGB photogrammetry | ⚠️ convert first | Publish as 3D Tiles | `osgb/` (sample pending) |
| **Shapefile (`.shp`)** | ✅ import into PostGIS | **Dynamic MVT** / WMS / WFS / OAF; admin "Import SHP" → `postgis_table` | `shapefile/` |
| **FileGDB (`.gdb`)** | ✅ import into PostGIS | **Dynamic MVT** / WMS / WFS / OAF; Esri FileGDB API reads `.gdb` directly → writes straight into PostGIS | Business gdb pending; `Status.gdb` in caches is a state store, not a data source |
| Static MVT tile directories | ⚠️ discover only | **Not publishable** (no SourceKind) | `mvt/` (reserved) |
| Esri `.mmpk` | ❌ unsupported | Extract the inner COG first | `cog/cog.mmpk` |

> The authoritative matrix is `data_type_caps` behind `GET /admin/capabilities` (it describes **direct** publishing from static data sources). Shapefile / FileGDB follow the "import into PostGIS → dynamic MVT" migration path and join the matrix as `postgis_table` once loaded; FileGDB reading uses Esri's official open-source API (Apache-2.0), not a GDAL conversion.

---

## Install on Linux

### 1) Docker (recommended)

Deployment entries are split by language × platform. On Linux use:

| Directory                     | Compose project          | First-run DB seed        |
| ----------------------------- | ------------------------ | ------------------------ |
| `manager/deploy_zh-cn/linux/` | `gis-data-base-zh-linux` | `system.sql` → `map.sql` |
| `manager/deploy_en/linux/`    | `gis-data-base-en-linux` | `.en_US` seeds           |

Both projects use the same ports (6180 / 8080 / `MAPSERVER_DB_PORT`, default 5434), so **they cannot run at the same time**. Volumes are isolated per project, so switching language does not need `down -v`.

```bash
cd manager/deploy_zh-cn/linux          # English: manager/deploy_en/linux
cp ../../deploy/.env.example .env
# Edit .env: at minimum set MANAGER_JWT_SECRET (openssl rand -hex 32) and change MAPSERVER_DB_PASSWORD
bash ../../deploy/start.sh --dir .     # up --build → wait for healthy PostGIS → validate
```

Or manually:

```bash
docker compose up -d --build
docker compose ps
docker compose logs -f map-api
```

Open: admin console <http://localhost:6180/>, map API health <http://localhost:8080/health>, WMS capabilities <http://localhost:8080/wms?service=WMS\&request=GetCapabilities>.

`GISDATA_DIR` (default `./gisdata`) is mounted read-only at map-api `/app/data`. Put layer XML files in the deploy config template `config/mapserver/` and reference data as `/app/data/...`. See [manager/deploy/README.md](manager/deploy/README.md).

### 2) Build from source

Requirements: Rust 1.80+ (validated on 1.98.1), build-essential, pkg-config, PostgreSQL client; PROJ runtime libraries for cross-CRS transforms.

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl libproj-dev proj-bin libsqlite3-0 postgresql-client

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"

cargo build --release --manifest-path mapserver/Cargo.toml
cargo build --release --manifest-path manager/backend/Cargo.toml

cd manager/ui
corepack enable
pnpm install --frozen-lockfile
pnpm run build:prod
cd ../../..
```

Initialize PostgreSQL 16 + PostGIS (the database name contains a hyphen and needs double quotes):

```bash
psql -h localhost -p 5434 -U postgres -c 'CREATE DATABASE "mapserver-manager"'
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/system.sql
psql -h localhost -p 5434 -U postgres -d mapserver-manager -f manager/backend/sql/map.sql
# English baseline: use system.en_US.sql / map.en_US.sql instead; never run both sets
```

Run from the repository root so that `static_dir="manager/ui/dist"` and relative `data_root` resolve correctly:

```bash
export MANAGER_DB_PASSWORD='<your password>'
export MANAGER_JWT_SECRET='<random long secret>'
export MAPSERVER_DB_PASSWORD='<your password>'
export PROJ_DATA=/usr/share/proj
# Optional: export DATA_ROOT=/srv/gisdata

./mapserver/target/release/mapserve serve --config mapserver/config/mapserver.xml    # :8080
./manager/backend/target/release/manager --config manager/backend/config/manager.xml  # :6180
```

> Install PROJ explicitly (`libproj-dev proj-bin libsqlite3-0`) in containers and source builds. Without it the service still starts, but cross-CRS transforms (4490/45xx/UTM) fail. Check the `proj` field of `/health`.

PostgreSQL guides: [中文](docs/postgresql16-postgis-linux-install.zh-CN.md) / [English](docs/postgresql16-postgis-linux-install.en-US.md).

---

## Install on Windows

### 1) Build and install

Requirements: Windows 10/11 x64, Rust MSVC toolchain, VS 2019/2022 Build Tools, optional Node.js + pnpm.

```powershell
cd <repo>\build\script
.\build_all.bat            # mapserver + manager
.\build_all.bat frontend   # also build the UI (recommended; otherwise admin UI is 404)
.\install.bat              # default: <repo>\..\..\..\rmap_target
# or: .\install.bat D:\gis\rmap_target
```

The install directory contains `bin\` (`mapserve.exe`, `manager.exe`, PROJ DLLs), `config\`, `web\`, `data\`, `cache\`, `logs\`, `share\proj\`, `start-mapserver.bat` and `start-manager.bat`. Copy `.env.example` to `.env` and set at least `MANAGER_DB_PASSWORD`, `MANAGER_JWT_SECRET` and `MAPSERVER_DB_PASSWORD`. Full paths and known issues: [build/script/README.md](build/script/README.md).

### 2) Run as a Windows service (NSSM)

```powershell
nssm install map-api "<install dir>\bin\mapserve.exe"
nssm install manager  "<install dir>\bin\manager.exe"
nssm set map-api AppDirectory "<install dir>"
nssm set manager  AppDirectory "<install dir>"
nssm start map-api
nssm start manager
```

**Always set `AppDirectory`**: `data_root` defaults to the relative path `data`. Without it NSSM uses `bin\` as the working directory and the admin plane reports `data_root` inaccessible. The same rule applies to any custom launcher (systemd / Task Scheduler).

### 3) Start from the command line

```powershell
cd <install dir>
.\start-mapserver.bat   # map-api on :8080; clears PROJ_LIB and sets PROJ_DATA
.\start-manager.bat     # manager on :6180
# or manually:
.\bin\mapserve.exe serve --config config\mapserver.xml
.\bin\manager.exe --config config\manager.xml
```

Ports are not read from `.env`: map-api uses `<server port>` in `config/mapserver.xml` or `--listen`, manager uses `<server addr>` in `config/manager.xml`. After changing the map-api port, update `<mapsvc base_url>` in `manager.xml`. See [mapserver/install/windows/README.md](mapserver/install/windows/README.md).

---

## System architecture

```text
HTTP (axum 0.8 / tokio)     mapserver/services/map-api
  ├─ handlers/  WMS · WMTS · TMS · XYZ · MVT · PMTiles · MBTiles · COG · WFS · OAF · OAT · 3DTiles · health
  ├─ admin/     discovery/publish wizard · service registry · styles · access log · cache · TPK import
  ├─ state.rs   AppState (layer runtime, five-stage cache, base URL, manager mode)
  └─ config.rs  XML → serde structs → TileMatrixSet

Protocol layer              gisserver::wms_protocol / wfs_protocol / ogcapi_features / ogcapi_tiles
Data layer                  gisserver::projection / storage / cog / renderer / mvt / pmtiles / pg_source / arcgis_cache
Admin plane                 manager/backend (axum) + manager/ui (Vue 3 + Vite + Element Plus)
```

- map-api is a pure map engine and **does not host the UI**; `<manager>` only performs a short health probe to choose `managed` / `standalone` mode.
- Layer configuration is XML, one layer per file, under `<catalog.root>/<kind>/`. The admin plane writes the same location with syntax validation, `.xml.bak` backup and reload rollback.
- `RangeSource` is the single read seam: local files, HTTP(S) objects and S3 pre-signed URLs all use positional reads.


Layering, data flows, concurrency and cache conventions: [docs/architecture.md](docs/architecture.md).

---

## Publishing each service type

Recommended flow: **workspace → data-source Store → discover data layer → publishing wizard (select service types) → published-service preview / copy endpoints**. Under the hood manager calls map-api `/admin/services` to write layer XML and hot-reload.

**Migration and dynamic data first**: existing XYZ/TMS tile directories, MBTiles/PMTiles archives and ArcGIS compact caches mount and publish directly ([migration capabilities](#1-traditional-tile-formats-smooth-migration-no-re-tiling)); for new data, prefer COG reads (raster) and dynamic PostGIS MVT (vector) — change the DB, change the map, no pre-tiling.

| Service | Publishable sources | Details | Operation guide |
|---|---|---|---|
| **WMS** 1.1.1/1.3.0 | COG, GeoJSON, PostGIS tables | [WMS guide](docs/services/wms.en-US.md) | [Steps](docs/services/wms.en-US.md#operation-guide-admin-ui) |
| **WMTS** 1.0.0 | COG, GeoJSON, raster PMTiles/MBTiles, ArcGIS cache | [WMTS guide](docs/services/wmts.en-US.md) | [Steps](docs/services/wmts.en-US.md#operation-guide-admin-ui) |
| **XYZ / TMS** | Same as WMTS; legacy `z/x/y` tile directories migrate with no re-tiling | [XYZ/TMS guide](docs/services/xyz-tms.en-US.md) | [Steps](docs/services/xyz-tms.en-US.md#operation-guide-admin-ui) |
| **MVT** vector tiles | **Dynamic from PostGIS**, GeoJSON, vector PMTiles/MBTiles | [MVT guide](docs/services/mvt.en-US.md) | [Steps](docs/services/mvt.en-US.md#operation-guide-admin-ui) |
| **WFS** 2.0/1.1/1.0 | GeoJSON, PostGIS tables | [WFS guide](docs/services/wfs.en-US.md) | [Steps](docs/services/wfs.en-US.md#operation-guide-admin-ui) |
| **OGC API - Features** | GeoJSON, PostGIS tables | [OAF guide](docs/services/oaf.en-US.md) | [Steps](docs/services/oaf.en-US.md#operation-guide-admin-ui) |
| **OGC API - Tiles** | COG, GeoJSON, raster archives, ArcGIS cache | [OAT guide](docs/services/oat.en-US.md) | [Steps](docs/services/oat.en-US.md#operation-guide-admin-ui) |
| **3D Tiles** | `tileset.json` directories (convert OSGB first) | [3D Tiles guide](docs/services/3dtiles.en-US.md) | [Steps](docs/services/3dtiles.en-US.md#operation-guide-admin-ui) |
| **ArcGIS cache** | `Conf.xml` cache dirs, `.tpk`, `.tpkx` | [ArcGIS cache guide](docs/services/arcgis-cache.en-US.md) | [Steps](docs/services/arcgis-cache.en-US.md#operation-guide-admin-ui) |

> Static Shapefile / FileGDB directories cannot be published directly: Shapefiles via the admin "Import SHP" action (`shp2pgsql`), FileGDB via Esri's official FileGDB API reading `.gdb` directly and writing straight into PostGIS. Both then publish as **dynamic MVT** / WMS / WFS / OAF under `postgis_table`. Static MVT tile directories remain discover-only; plain PostGIS tables without a geometry column are data-query only. The authoritative matrix is `GET /admin/capabilities`.

All guides: [中文](docs/services/README.zh-CN.md) / [English](docs/services/README.en-US.md). Protocol matrix, request samples and limits: [mapserver/README.md](mapserver/README.md).

---

## Next steps

Roadmap directions, without a fixed schedule:

1. **Portable deployments (P1)**: store runtime-published `source.path` as `${DATA_ROOT}`-relative paths and resolve them against `data_root` at serve time, for both file and DB backends. See [manager/deploy/data-root.md](manager/deploy/data-root.md).
2. **More data sources**: Shapefile / FileGDB publish dynamic MVT via PostGIS import (FileGDB through Esri's official open-source FileGDB API, read-and-write directly); next up is wiring in that FileGDB reader, plus GeoPackage, stronger ArcGIS-cache publishing, and COG JPEG/WEBP tile decoding and writing.
3. **Security and governance**: data-plane auth/RBAC, TLS termination, clustered deployment, monitoring and alerting. The current version should sit behind a reverse proxy.
4. **OGC completeness**: WMS multi-layer composition, broader SLD symbol/filter support, more conformance classes and automated tests.
5. **Performance**: parallelize resampling/palette loops, add an output cache for WMS GetMap, make rayon and MBTiles pool sizes configurable.
6. **Delivery experience**: Linux one-click installer and systemd unit, plus continued bilingual docs and sample data.

---

## License

- `gisserver/` (core library): **MIT OR Apache-2.0**, see [gisserver/LICENSE](gisserver/LICENSE)
- `mapserver/` and `manager/`: **AGPL-3.0-only** with a commercial license exception, see each `LICENSE`; third-party notices in [mapserver/THIRD-PARTY-NOTICES.md](mapserver/THIRD-PARTY-NOTICES.md)

Copyright (c) 2026 rex yan 853740877@qq.com and contributors.
