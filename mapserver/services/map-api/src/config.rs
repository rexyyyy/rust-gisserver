//! Server configuration (`config/mapserver.xml`).
//!
//! `mapserver.xml` holds **server-wide options only** (listen address,
//! cache, catalog root). Everything layer-related lives in the layer
//! configuration files under type sub-directories of `catalog.root`:
//! custom grid definitions go into a layer file's `custom_grids:` key, and
//! 3D Tiles services are discovered at `<catalog.root>/3dtiles/*.xml`.
//!
//! All configuration files are **XML**. They are read through the
//! [`crate::xmlcfg`] bridge, which turns XML into a `serde_json::Value` that
//! the existing serde structs (`Layer`, `GridDef`, `ServerConfig`, …)
//! deserialize from unchanged.
//!
//! ```text
//! config/
//! ├─ mapserver.xml   server-wide options (listen/cache/catalog)
//! ├─ cog/             raster layers   (cog/*.xml, one layer per file)
//! ├─ vector/          vector layers   (vector/*.xml)
//! ├─ pmtiles/         PMTiles layers  (pmtiles/*.xml)
//! ├─ mbtiles/         MBTiles layers  (mbtiles/*.xml, served at
//! │                   /mbtiles/{name}/{z}/{x}/{y} + XYZ/TMS/WMTS/MVT)
//! ├─ postgis/         PostGIS dynamic vector layers (postgis/*.xml: MVT +
//! │                   WMS/WFS/OGC API, one connection pool per DSN)
//! ├─ arcgis/          ArcGIS server tile caches (arcgis/*.xml: read-through of
//! │                   a pre-cut compact cache v1/v2; the layer's grid comes
//! │                   from the cache's own Conf.xml, see `arcgis-cache`)
//! ├─ terrain/         quantized-mesh terrain packs (*.terrainpack)
//! ├─ mvt/             (reserved; MVT output is derived from vector/pmtiles)
//! ├─ osgb/            (reserved; OSGB lands as 3D Tiles via the converter)
//! ├─ 3dtiles/         3D Tiles services (one XML = one service)
//! └─ styles/          style XML files
//! ```
//!
//! Each layer file is a single XML document with the same shape as the
//! legacy inline `layers:` entries, plus an optional `custom_grids:` map:
//!
//! ```xml
//! <!-- config/cog/world.xml -->
//! <layer name="world" title="World gradient COG" src_crs="EPSG:4326" tile_px="256">
//!   <source type="cog" path="fixtures/sample.tif"/>
//!   <grids>
//!     <item>WebMercatorQuad</item>
//!     <item>WorldCRS84Quad</item>
//!   </grids>
//!   <!-- custom_grids: optional, grid defs referenced by grids: -->
//!   <custom_grids>
//!     <item id="mygrid" crs="EPSG:4326" tile_px="256">
//!       <origin><item>-180</item><item>90</item></origin>
//!       <extent><item>-180</item><item>-90</item><item>180</item><item>90</item></extent>
//!       <resolutions><item>0.703125</item></resolutions>
//!       <!-- optional: per-rung LevelID ladder + source scale denominators
//!            (written for arcgis caches; absent = index is the zoom, OGC rule) -->
//!       <levels><item>0</item></levels>
//!       <scales><item>295829355.454566</item></scales>
//!     </item>
//!   </custom_grids>
//! </layer>
//! ```
//!
//! The `source.type` must match the directory name; mismatches are skipped
//! with a warning. Layer names must be unique — first one wins, duplicates
//! are warned and skipped.
//!
//! PROJ runtime assets (`proj_9.dll`, `share/proj/proj.db`) are an
//! **environment** concern, not server config: resolution order is explicit
//! env vars `PROJ_DLL` / `PROJ_DATA`, then run-directory defaults
//! (`proj_9.dll` in the working directory, `<workdir>/share/proj`).
//!
//! `catalog.backend` prepares for a future database-backed layer registry:
//! `files` (default, implemented) reads the directories above; `db` is a
//! reserved variant that currently falls back to `files` with a warning.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use gisserver::wms_common::layer::Layer;
use gisserver::wms_common::tile::{TileMatrixSet, TmLevel};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerOpts {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Decoded-tile cache cap in MiB (0 disables). Also the default budget
    /// for the foyer output cache unless `foyer_cache_mib` overrides it.
    #[serde(default = "default_cache_mib")]
    pub cache_mib: u64,
    /// Dedicated budget for the foyer output cache (L1 memory) in MiB.
    /// Absent = follow `cache_mib`. Split out because dynamic PostGIS MVT
    /// tiles are far larger and far more numerous than decoded raster tiles:
    /// raising this does NOT inflate the RGBA decoded-tile cache.
    #[serde(default)]
    pub foyer_cache_mib: Option<u64>,
    /// TTL in seconds for the foyer caches (output tiles, decoded tiles,
    /// metadata documents, catalog metadata, feature lookups). Measured from
    /// insert, so an entry is refreshed at most once per window. `0` = no
    /// expiry (entries only leave via capacity eviction / explicit purge).
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,
    /// Seconds between cache statistics log lines (0 disables). The same task
    /// also forces moka housekeeping, so TTL-expired entries are evicted
    /// promptly even on an idle cache.
    #[serde(default = "default_cache_stats_secs")]
    pub cache_stats_secs: u64,
    /// Public base URL for capabilities documents (overrides request host).
    #[serde(default)]
    pub public_url: Option<String>,
    /// Optional L2 disk cache (foyer hybrid cache). Absent = disabled: the
    /// output cache is then foyer memory-only. When present, dynamic tile
    /// output is cached through a two-level lookup: foyer memory tier ->
    /// foyer disk tier (survives restarts).
    #[serde(default)]
    pub disk_cache: Option<DiskCacheOpts>,
    /// Global data root for manager-created services: UI-added services may
    /// only reference files under this directory (paths are stored relative).
    /// Absent = no data-root restriction (manager shows the whole drive).
    #[serde(default)]
    pub data_root: Option<String>,
    /// Seconds between periodic catalog re-scans (0 = disabled, default).
    ///
    /// Cluster convergence: each node re-reads the shared XML catalog files,
    /// so a layer added/edited through another node's admin API becomes
    /// serving here without a manual reload. Single-node deployments don't
    /// need it (the admin API hot-reloads locally already). Re-reading
    /// re-opens data sources and purges caches — keep the interval in the
    /// tens-of-seconds range.
    #[serde(default = "default_catalog_scan_secs")]
    pub catalog_scan_secs: u64,
    /// Global S3 credentials for `s3://bucket/key` layer sources. Absent =
    /// `s3://` sources are rejected with a configuration error. Path-style
    /// addressing: works with MinIO / RustFS / AWS / S3-compatible clouds.
    #[serde(default)]
    pub s3: Option<S3Opts>,
    /// 数据面鉴权令牌（OGC 瓦片 / 要素服务）。配置后，所有数据面请求
    /// （/wms /wfs /wmts /tiles /mvt /pmtiles /mbtiles /cog /3dtiles /terrain
    /// 等）必须携带令牌，否则返回 401；令牌支持三种携带方式：请求头
    /// `X-Data-Token`、 `Authorization: Bearer <token>`、查询参数 `?token=`
    /// （便于浏览器 / viewer 把令牌直接拼在瓦片 URL 上）。未配置则不设防
    /// （零配置向后兼容，与 `admin.token` 的「安全带」定位一致——它是令牌
    /// 护栏而非完整鉴权体系，角色 / ACL 交给网关或 manager）。
    #[serde(default)]
    pub data_token: Option<String>,
}

/// Global S3 credentials (`server.s3` section).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct S3Opts {
    /// Full endpoint including scheme, e.g. `http://172.17.11.6:9000`.
    pub endpoint: String,
    /// SigV4 signing region. MinIO / RustFS: any value (`auto` conventional);
    /// AWS: the bucket's region (e.g. `cn-north-1`).
    #[serde(default = "default_s3_region")]
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

impl S3Opts {
    /// Convert into the storage-layer signing config.
    pub fn to_storage(&self) -> gisserver::storage::S3Config {
        gisserver::storage::S3Config {
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            access_key: self.access_key.clone(),
            secret_key: self.secret_key.clone(),
        }
    }
}

fn default_s3_region() -> String {
    "auto".to_string()
}

fn default_catalog_scan_secs() -> u64 {
    0
}

/// Configuration of the foyer-backed L2 disk cache (`server.disk_cache`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiskCacheOpts {
    /// Directory holding the disk cache files (created on startup; survives
    /// restarts and is re-indexed by foyer on boot).
    pub dir: String,
    /// Disk budget in MiB (eviction: FIFO-style region reclaim by foyer).
    #[serde(default = "default_disk_capacity_mib")]
    pub capacity_mib: u64,
}

fn default_disk_capacity_mib() -> u64 {
    4096
}


impl Default for ServerOpts {
    fn default() -> Self {
        ServerOpts {
            host: default_host(),
            port: default_port(),
            cache_mib: default_cache_mib(),
            foyer_cache_mib: None,
            cache_ttl_secs: default_cache_ttl_secs(),
            cache_stats_secs: default_cache_stats_secs(),
            public_url: None,
            disk_cache: None,
            data_root: None,
            catalog_scan_secs: default_catalog_scan_secs(),
            s3: None,
            data_token: None,
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_cache_mib() -> u64 {
    1024
}

fn default_cache_ttl_secs() -> u64 {
    300
}

fn default_cache_stats_secs() -> u64 {
    60
}

/// Admin / manager integration options (`admin:` section, all optional).
/// Everything defaults off: without a DSN the server runs exactly as before
/// (files catalog, file-sidecar service states, in-memory usage history).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AdminOpts {
    /// Registry DSN (PostgreSQL). Absent = file sidecar
    /// (`<catalog.root>/service-state.json`); unreachable = same fallback.
    #[serde(default)]
    pub dsn: Option<String>,
    /// Optional shared secret. When set, `/admin/*` and `/api/usage` require
    /// the `X-Admin-Token` header.
    #[serde(default)]
    pub token: Option<String>,
    /// Usage flush interval in seconds (0 disables the flush task; counters
    /// stay in memory and `/api/usage` reports "since restart").
    #[serde(default = "default_usage_flush_secs")]
    pub usage_flush_secs: u64,
}

impl Default for AdminOpts {
    fn default() -> Self {
        AdminOpts {
            dsn: None,
            token: None,
            usage_flush_secs: default_usage_flush_secs(),
        }
    }
}

fn default_usage_flush_secs() -> u64 {
    30
}

/// Layer-catalog source: where layer configurations come from.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CatalogBackend {
    /// One XML file per layer under `<root>/<type>/` (implemented).
    Files,
    /// Database-backed layer registry (reserved, not implemented yet).
    Db,
}

impl Default for CatalogBackend {
    fn default() -> Self {
        CatalogBackend::Files
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogOpts {
    /// Root directory holding per-type layer sub-directories
    /// (`cog/`, `vector/`, `pmtiles/`, `3dtiles/`, ...). Relative to the
    /// server process working directory.
    #[serde(default = "default_catalog_root")]
    pub root: String,
    /// Layer registry backend: `files` (default) or `db` (reserved).
    #[serde(default)]
    pub backend: CatalogBackend,
    /// Database DSN for `backend: db` (reserved; unused for now).
    #[serde(default)]
    pub dsn: Option<String>,
}

impl Default for CatalogOpts {
    fn default() -> Self {
        CatalogOpts {
            root: default_catalog_root(),
            backend: CatalogBackend::Files,
            dsn: None,
        }
    }
}

fn default_catalog_root() -> String {
    "config".to_string()
}

impl CatalogOpts {
    /// Layer source type served from this sub-directory (reserved dirs yield
    /// `None` — they exist for organization / future work only).
    pub(crate) fn dir_source_type(dir: &str) -> Option<&'static str> {
        match dir {
            "cog" => Some("cog"),
            "vector" => Some("vector"),
            "pmtiles" => Some("pmtiles"),
            "mbtiles" => Some("mbtiles"),
            "arcgis" => Some("arcgis"),
            "postgis" => Some("postgis"),
            _ => None, // mvt / osgb / 3dtiles / styles: reserved
        }
    }
}

/// 一个图层 XML 文档的解析结果。
pub struct ParsedLayerDoc {
    /// 图层本体（已剥离 `custom_grids`）。
    pub layer: Layer,
    /// 该文档自带的自定义网格定义。
    pub grids: BTreeMap<String, GridDef>,
    /// `custom_grids` 解析失败时的非致命提示（不影响图层本体）。
    pub grids_warning: Option<String>,
}

/// 解析单个图层 XML 文档 → 图层 + 其自带网格。
///
/// `files` 后端（目录扫描）与 `catalog-import`（XML → 库迁移）共用这一个入口，
/// 因此两条路径对同一份 XML 的理解**逐字节一致**——这是 db 后端能等价替换
/// files 后端的前提。
pub fn parse_layer_doc(text: &str) -> Result<ParsedLayerDoc, String> {
    // 先按原始 JSON value 解析：图层文件可能带一个可选的 `custom_grids` 映射
    // （图层 `grids:` 引用的网格定义），它不属于 Layer 结构本身。
    let mut val: serde_json::Value = crate::xmlcfg::parse_to_value(text)?;
    let mut grids: BTreeMap<String, GridDef> = BTreeMap::new();
    let mut grids_warning: Option<String> = None;
    if let Some(g) = val.get("custom_grids").cloned() {
        match serde_json::from_value::<BTreeMap<String, GridDef>>(g) {
            Ok(defs) => grids = defs,
            Err(e) => grids_warning = Some(format!("custom_grids: {e}")),
        }
    }
    if let Some(m) = val.as_object_mut() {
        m.remove("custom_grids");
    }
    let layer: Layer = serde_json::from_value(val).map_err(|e| e.to_string())?;
    Ok(ParsedLayerDoc {
        layer,
        grids,
        grids_warning,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub server: ServerOpts,
    /// Admin / manager integration (service registry + usage metering).
    #[serde(default)]
    pub admin: AdminOpts,
    /// Legacy inline layers (still honored; merged with catalog files).
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Named standalone PMTiles archives exposed at `/pmtiles/{name}`.
    #[serde(default)]
    pub archives: BTreeMap<String, String>,
    /// Layer catalog: where per-layer configurations are loaded from.
    #[serde(default)]
    pub catalog: CatalogOpts,
    /// Custom TileMatrixSets (referenced by layers' `grids:` and served at
    /// `/tileMatrixSets/:id`). Populated at startup from the `custom_grids:`
    /// key of **layer config files**; a legacy inline `grids:` map in
    /// mapserver.xml is still honored for backward compatibility.
    #[serde(default)]
    pub grids: BTreeMap<String, GridDef>,
}

/// A config-defined custom TileMatrixSet: explicit CRS + top-left origin +
/// full extent + tile size + a resolution ladder.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GridDef {
    pub crs: String,
    /// Top-left corner `[x, y]` in CRS units.
    pub origin: [f64; 2],
    pub extent: [f64; 4],
    #[serde(default = "default_tile_px")]
    pub tile_px: u32,
    /// CRS units per pixel, z0..zN.
    pub resolutions: Vec<f64>,
    /// Zoom id per rung, when it is not the array index (an ArcGIS cache cut
    /// from L06 upward). Absent = `0..resolutions.len()-1`.
    #[serde(default)]
    pub levels: Vec<u32>,
    /// Scale denominator per rung as the **source** declares it (ArcGIS
    /// `LODInfo/Scale`). Absent/zero = derived from `resolutions` with the OGC
    /// 0.28 mm rule, which differs from ArcGIS' 0.0254/DPI by ~5.8 %.
    #[serde(default)]
    pub scales: Vec<f64>,
}

fn default_tile_px() -> u32 {
    256
}

impl GridDef {
    pub fn to_tms(&self, id: &str) -> TileMatrixSet {
        let [minx, miny, maxx, maxy] = self.extent;
        let (w, h) = (maxx - minx, maxy - miny);
        let levels = self
            .resolutions
            .iter()
            .enumerate()
            .map(|(i, &r)| {
                let span = self.tile_px as f64 * r;
                let z = self.levels.get(i).copied().unwrap_or(i as u32);
                TmLevel {
                    z,
                    resolution: r,
                    matrix_w: ((w / span).ceil() as u32).max(1),
                    matrix_h: ((h / span).ceil() as u32).max(1),
                    scale_denominator: self.scales.get(i).copied().filter(|s| *s > 0.0),
                }
            })
            .collect();
        TileMatrixSet {
            id: id.to_string(),
            crs: gisserver::wms_common::crs::normalize(&self.crs),
            origin_x: self.origin[0],
            origin_y: self.origin[1],
            tile_w: self.tile_px,
            tile_h: self.tile_px,
            levels,
        }
    }
}

impl ServerConfig {
    /// 3D Tiles 服务配置目录：`<catalog.root>/3dtiles`（属图层侧配置，
    /// 不再是 server 级选项）。目录下每个 `*.xml` 定义一个服务，
    /// 经 `/3dtiles/{name}/...` 提供服务；相对路径相对服务进程工作目录。
    pub fn tilesets_dir(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.catalog.root).join("3dtiles")
    }

    /// 地形量化网格服务目录：`<catalog.root>/terrain`（一个 .terrainpack = 一个服务，
    /// 由 gis-wt 底座 G10b 编码产出，见 docs/11-地形量化网格编码设计.md）。
    pub fn terrain_dir(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.catalog.root).join("terrain")
    }

    /// Assemble the effective layer list: legacy inline `layers:` entries +
    /// one-file-per-layer XML documents from `<catalog.root>/<type>/`.
    ///
    /// Layer files may carry a `custom_grids:` map (id -> grid definition);
    /// those are merged into the returned map (first definition wins,
    /// duplicates are warned) so the definitions live next to the layers
    /// that reference them.
    ///
    /// Returns the merged layer list, the collected custom grid definitions
    /// and non-fatal warnings (type mismatches, duplicate names, reserved
    /// dirs with files). `backend: db` 读取共享库 `sys_published_layer.service_doc`
    /// （见 `crate::db_catalog`），不再扫描文件目录。
    pub fn collect_layers(
        &self,
    ) -> (Vec<Layer>, BTreeMap<String, GridDef>, Vec<String>) {
        let mut warnings = Vec::new();
        let mut extra_grids: BTreeMap<String, GridDef> = BTreeMap::new();
        let mut layers: Vec<Layer> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> =
            self.layers.iter().map(|l| l.name.clone()).collect();
        // db 目录后端：图层定义已由 catalog_pool 从共享库
        //（sys_published_layer.service_doc）载入 config.layers（启动见 main.rs
        // 的 load_db_catalog_into、热重载见 state::refresh_catalog），此处直接
        // 返回内联层，不再扫描文件目录。custom_grids 已并入 config.grids。
        if self.catalog.backend == CatalogBackend::Db {
            let mut layers = self.layers.clone();
            let mut grids: BTreeMap<String, GridDef> = BTreeMap::new();
            self.inject_arcgis_grids(&mut layers, &mut grids, &mut warnings);
            return (layers, grids, warnings);
        }
        layers.extend(self.layers.iter().cloned());

        let root = std::path::Path::new(&self.catalog.root);
        let mut type_dirs: Vec<std::path::PathBuf> = Vec::new();
        match std::fs::read_dir(root) {
            Ok(entries) => {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        type_dirs.push(p);
                    }
                }
                type_dirs.sort();
            }
            Err(e) => warnings.push(format!("catalog root {}: {e}", root.display())),
        }

        for dir in type_dirs {
            let dir_name = dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let want_type = match CatalogOpts::dir_source_type(&dir_name) {
                Some(t) => t,
                None => {
                    // Reserved directory (mbtiles/mvt/osgb/...): surface files
                    // as warnings so nobody wonders why they are not served.
                    let n = std::fs::read_dir(&dir)
                        .map(|it| {
                            it.flatten()
                                .filter(|f| {
                                    let n = f.file_name().to_string_lossy().to_lowercase();
                                    n.ends_with(".xml")
                                })
                                .count()
                        })
                        .unwrap_or(0);
                    if n > 0 {
                        warnings.push(format!(
                            "catalog dir '{dir_name}' is reserved; {n} layer file(s) ignored"
                        ));
                    }
                    continue;
                }
            };
            let files = match std::fs::read_dir(&dir) {
                Ok(it) => {
                    let mut fs: Vec<std::path::PathBuf> = it
                        .flatten()
                        .map(|f| f.path())
                        .filter(|p| {
                            matches!(
                                p.extension().and_then(|e| e.to_str()),
                                Some("xml")
                            )
                        })
                        .collect();
                    fs.sort();
                    fs
                }
                Err(e) => {
                    warnings.push(format!("read {}: {e}", dir.display()));
                    continue;
                }
            };
            for f in files {
                let text = match std::fs::read_to_string(&f) {
                    Ok(t) => t,
                    Err(e) => {
                        warnings.push(format!("read {}: {e}", f.display()));
                        continue;
                    }
                };
                // 与 catalog-import 共用同一个解析入口（见 parse_layer_doc）。
                let parsed = match parse_layer_doc(&text) {
                    Ok(p) => p,
                    Err(e) => {
                        warnings.push(format!("parse {}: {e}", f.display()));
                        continue;
                    }
                };
                if let Some(w) = parsed.grids_warning {
                    warnings.push(format!("{}: {}", f.display(), w));
                }
                for (id, def) in parsed.grids {
                    if self.grids.contains_key(&id) || extra_grids.contains_key(&id) {
                        warnings.push(format!(
                            "{}: duplicate custom grid '{}' skipped",
                            f.display(),
                            id
                        ));
                    } else {
                        extra_grids.insert(id, def);
                    }
                }
                let mut layer = parsed.layer;
                let got_type = match &layer.source {
                    gisserver::wms_common::layer::SourceKind::Cog { .. } => "cog",
                    gisserver::wms_common::layer::SourceKind::Vector { .. } => "vector",
                    gisserver::wms_common::layer::SourceKind::Pmtiles { .. } => "pmtiles",
                    gisserver::wms_common::layer::SourceKind::Mbtiles { .. } => "mbtiles",
                    gisserver::wms_common::layer::SourceKind::Arcgis { .. } => "arcgis",
                    gisserver::wms_common::layer::SourceKind::Postgis(_) => "postgis",
                };
                if got_type != want_type {
                    warnings.push(format!(
                        "{}: source type '{got_type}' does not match directory '{want_type}'",
                        f.display()
                    ));
                    continue;
                }
                if !seen.insert(layer.name.clone()) {
                    warnings.push(format!(
                        "{}: duplicate layer name '{}' skipped",
                        f.display(),
                        layer.name
                    ));
                    continue;
                }
                // Vector layers: auto-discover a colocated `<config-stem>.sld`
                // sibling unless the file already names one via `sld:`.
                if layer.sld.is_none() && want_type == "vector" {
                    let sld = f.with_extension("sld");
                    if sld.exists() {
                        layer.sld = Some(sld.to_string_lossy().replace('\\', "/"));
                    }
                }
                layers.push(layer);
            }
        }
        self.inject_arcgis_grids(&mut layers, &mut extra_grids, &mut warnings);
        (layers, extra_grids, warnings)
    }

    /// ArcGIS Tile Package（`.tpk`）的解压根目录。与 foyer L2 磁盘缓存
    /// 同级：取 `disk_cache.dir`（如 `cache/foyer`）的父目录下的 `tpk/`，
    /// 即 `cache/tpk`；未配磁盘缓存时退回相对工作目录的 `cache/tpk`。
    /// 解压落点与源包位置无关，故只读/大容量源盘不会被污染。
    pub fn tpk_cache_root(&self) -> std::path::PathBuf {
        if let Some(dc) = &self.server.disk_cache {
            if let Some(parent) = std::path::Path::new(&dc.dir).parent() {
                return parent.join("tpk");
            }
        }
        std::path::Path::new("cache").join("tpk")
    }

    /// ArcGIS 缓存图层的网格以 `Conf.xml` 为准：为每个 `source.type=arcgis`
    /// 的图层推导一份 custom grid（LOD 分辨率 + 源比例尺 + `LevelID` 阶梯），
    /// 注册进 `extra_grids`，并把图层 `grids:` 指向它。
    ///
    /// 为什么在此覆盖而非尊重图层文件的声明：预设网格（WebMercatorQuad /
    /// WorldCRS84Quad）与缓存自身的 LOD 阶梯不等价（z0 分辨率 0.703125 对
    /// 0.70391441568404511），用它取瓦片会整体错位而**不报错**——只有数据自己
    /// 的原点/分辨率/范围能算对 bundle 槽位。files 与 db 两条目录后端都经过
    /// [`collect_layers`]，因此两种后端的寻址行为一致。
    fn inject_arcgis_grids(
        &self,
        layers: &mut [Layer],
        extra_grids: &mut BTreeMap<String, GridDef>,
        warnings: &mut Vec<String>,
    ) {
        for layer in layers.iter_mut() {
            let path = match &layer.source {
                gisserver::wms_common::layer::SourceKind::Arcgis { path } => path.clone(),
                _ => continue,
            };
            let cache = match gisserver::arcgis_cache::ArcgisCache::open(std::path::Path::new(&path)) {
                Ok(c) => c,
                Err(e) => {
                    // 不在此摘掉图层：打开时 `LayerRuntime::open` 会用同一条
                    // 错误把它记进 refresh 报告的 skipped 里。
                    warnings.push(format!("layer '{}': arcgis cache {path} unusable ({e})", layer.name));
                    continue;
                }
            };
            let gid = format!("arcgis_{}", layer.name);
            let spec = match cache.grid_spec(&gid) {
                Ok(s) => s,
                Err(e) => {
                    warnings.push(format!("layer '{}': {e}", layer.name));
                    continue;
                }
            };
            // 图层文件自带同名 custom_grids 时以文件为准（运维手工收紧密度的
            // 唯一合法入口），但要校验它是否仍与当前 Conf.xml 一致。
            let declared = self.grids.get(&gid).or_else(|| extra_grids.get(&gid));
            match declared {
                Some(d) => {
                    if d.resolutions != spec.resolutions || d.origin != spec.origin {
                        warnings.push(format!(
                            "layer '{}': configured grid '{gid}' differs from Conf.xml (cache re-cut? re-import the layer file)",
                            layer.name
                        ));
                    }
                }
                None => {
                    extra_grids.insert(
                        gid.clone(),
                        GridDef {
                            crs: spec.crs.clone(),
                            origin: spec.origin,
                            extent: spec.extent,
                            tile_px: spec.tile_px,
                            resolutions: spec.resolutions.clone(),
                            levels: spec.levels.clone(),
                            scales: spec.scales.clone(),
                        },
                    );
                }
            }
            let src = gisserver::wms_common::crs::normalize(&spec.crs);
            if gisserver::wms_common::crs::normalize(&layer.src_crs) != src {
                // 缺省值就是 EPSG:4326（没写 src_crs），不必告警。
                if layer.src_crs != gisserver::wms_common::crs::EPSG_4326 {
                    warnings.push(format!(
                        "layer '{}': src_crs '{}' overridden by the cache CRS '{src}'",
                        layer.name, layer.src_crs
                    ));
                }
                layer.src_crs = src;
            }
            layer.tile_px = spec.tile_px;
            if layer.extent.is_none() {
                layer.extent = cache.bounds();
            }
            if layer.grids.first().map(|g| g.as_str()) != Some(gid.as_str()) {
                // `grids:` 缺省时 default_grids() 给出 WebMercatorQuad，
                // 这是预期路径，不告警；显式声明了其它网格才提醒。
                if !(layer.grids.len() == 1 && layer.grids[0] == "WebMercatorQuad") {
                    warnings.push(format!(
                        "layer '{}': grids {:?} replaced by the cache-derived grid '{gid}'",
                        layer.name, layer.grids
                    ));
                }
                layer.grids = vec![gid];
            }
        }
    }

    /// Resolve a grid id for a layer: layer preset / custom / id-suffixed preset.
    pub fn resolve_grid(&self, id: &str, tile_px: u32) -> Option<TileMatrixSet> {
        if let Some(g) = self.grids.get(id) {
            return Some(g.to_tms(id));
        }
        TileMatrixSet::resolve(id, tile_px, &BTreeMap::new())
    }

    /// The grid id that a layer publishes on by default (first of `grids:`).
    pub fn default_grid_id(&self, layer: &Layer) -> String {
        layer.grids.first().cloned().unwrap_or_else(|| "WebMercatorQuad".to_string())
    }
}
