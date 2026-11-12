//! Application state and shared resources.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use pg_source::{PgPool, PgRegistry, PgSourceDef};
use serde::Serialize;
use storage::TileCache;
use wms_common::layer::{Layer, SourceKind};
use wms_common::tile::TileMatrixSet;

use crate::admin::access_log::AccessLog;
use crate::admin::registry::{Registry, ServiceScan};
use crate::admin::usage::Usage;
use crate::cache::{CacheCounters, CacheHub, CacheScope, CacheStats, CatalogMeta, PurgeReport};
use crate::config::{CatalogBackend, GridDef, ServerConfig};
use crate::layer::LayerRuntime;
use crate::terrain::TerrainRegistry;
use crate::tilesets::TilesetRegistry;

/// Per-DSN connection cap for PostGIS pools.
pub const PG_POOL_MAX: usize = 8;

/// The source-type label of a layer definition (`cog` / `vector` / ...).
pub fn source_kind(s: &SourceKind) -> &'static str {
    match s {
        SourceKind::Cog { .. } => "cog",
        SourceKind::Vector { .. } => "vector",
        SourceKind::Pmtiles { .. } => "pmtiles",
        SourceKind::Mbtiles { .. } => "mbtiles",
        SourceKind::Arcgis { .. } => "arcgis",
        SourceKind::Postgis(_) => "postgis",
    }
}

/// Human-readable data location of a layer source (registry / admin UI).
pub fn describe_source(s: &SourceKind) -> String {
    match s {
        SourceKind::Cog { path } => format!("cog:{path}"),
        SourceKind::Vector { path } => format!("vector:{path}"),
        SourceKind::Pmtiles { path } => format!("pmtiles:{path}"),
        SourceKind::Mbtiles { path } => format!("mbtiles:{path}"),
        SourceKind::Arcgis { path } => format!("arcgis:{path}"),
        SourceKind::Postgis(pg) => format!("postgis:{}/{}", pg.schema, pg.table),
    }
}

/// One withheld layer in a refresh report (registry state or open failure).
#[derive(Clone, Debug, Serialize)]
pub struct RefreshSkip {
    pub name: String,
    pub reason: String,
}

/// Result of [`AppState::refresh_catalog`] (also the `/admin/reload` body).
#[derive(Clone, Debug, Serialize)]
pub struct RefreshReport {
    pub registry_mode: String,
    pub served: Vec<String>,
    pub tilesets: Vec<String>,
    pub skipped: Vec<RefreshSkip>,
    pub warnings: Vec<String>,
}

/// 管理面状态（运维控制面）。
///
/// 与数据面状态（`layers` / `cache` / `pg` 等）显式分离：数据面 handler 若要
/// 触碰登记簿 / 计量 / 访问日志 / 样式库，必须经由 `AppState::admin`，编译期
/// 即隔离，避免数据面代码误用管理面字段。
pub struct AdminState {
    /// Service registry (PG `sys_service` or file sidecar). Always present —
    /// files mode is the zero-config default.
    pub registry: Registry,
    /// Request usage counters (see `admin::usage`).
    pub usage: Usage,
    /// Access-log audit ring (see `admin::access_log`).
    pub access_log: AccessLog,
    /// SLD 样式库目录（`<config_path 父目录>/styles`），供 `/admin/styles*`。
    pub styles_dir: PathBuf,
}

/// Shared server state (mounted via `axum::extract::State`).
///
/// 字段分区：
/// - **全局配置**：`config` / `config_path`（数据面与管理面共享的只读配置）。
/// - **数据面（serving plane）**：`layers` / `custom_grids` / `tilesets` /
///   `terrains` / `tile_cache` / `cache` / `base_url` / `pg` —— 瓦片与要素服务
///   运行时直接读取，全部 `RwLock` 包装以支持 `/admin/reload` 热替换。
/// - **管理面（admin plane）**：`admin: AdminState` —— 登记簿 / 计量 / 访问
///   日志 / 样式库，经 `state.admin` 访问，数据面不应直接触碰。
pub struct AppState {
    /// 全局配置（数据面与管理面共享）。
    pub config: ServerConfig,
    /// Path the config was loaded from (`/admin/reload` re-reads it).
    pub config_path: PathBuf,
    /// DB 目录后端连接池（`catalog.backend = "db"` 时为 `Some`）：读取共享库
    /// `sys_published_layer.service_doc`。`files` 后端为 `None`。
    pub catalog_pool: RwLock<Option<Arc<PgPool>>>,
    /// 当前目录图层快照（files 来自磁盘重扫、db 来自共享库），供管理面列表读取；
    /// 由 `refresh_catalog` 在每次热重载/周期重扫后刷新。
    pub catalog_layers: RwLock<Vec<Layer>>,
    // ── 数据面（serving plane）：瓦片 / 要素服务运行时状态 ──
    /// Layers wrapped in `Arc` so handlers can hand a cheap reference to the
    /// blocking pool (`spawn_blocking` requires `'static` closures).
    pub layers: RwLock<Vec<Arc<LayerRuntime>>>,
    /// Resolved custom grids (from `config.grids`); swapped on reload.
    pub custom_grids: RwLock<BTreeMap<String, TileMatrixSet>>,
    /// 3D Tiles services loaded from `<catalog.root>/3dtiles` (one XML = one service).
    pub tilesets: RwLock<TilesetRegistry>,
    /// Quantized-Mesh 地形服务（`<catalog.root>/terrain/*.terrainpack`）。
    pub terrains: RwLock<TerrainRegistry>,
    /// Decoded COG-tile cache (RGBA).
    pub tile_cache: TileCache,
    /// All moka caches (output tiles / layer index / metadata docs / catalog
    /// metadata / feature lookups) plus the refresh controls.
    pub cache: CacheHub,
    /// Base URL advertised in capabilities (config override or per-request host).
    pub base_url: String,
    /// PostGIS pool registry. `Some` **only** when at least one layer declares
    /// `source.type: postgis` — the single switch that enables every PG code
    /// path (dynamic vector tiles, WMS/WFS/OAF queries). Built by
    /// [`build_pg_registry`] before `AppState::new`; hot-swapped on reload.
    pub pg: RwLock<Option<Arc<PgRegistry>>>,
    /// 共享库 API Key 缓存（r-manager `sys_api_key`）：主令牌之外的附加认可凭证。
    /// 后台任务定时刷新，校验只读内存，见 [`crate::apikeys`]。
    pub api_keys: crate::apikeys::ApiKeyCache,
    // ── 管理面（admin plane）：经 `state.admin` 访问，数据面不应直接触碰 ──
    pub admin: AdminState,
}

/// Build the PostGIS pool registry from the configured layers.
///
/// * `Ok(None)` when no layer uses `source.type: postgis` (feature off).
/// * Layers that leave `properties` empty get their non-geometry columns
///   discovered from `information_schema` once at startup.
pub async fn build_pg_registry(
    config: &ServerConfig,
) -> anyhow::Result<Option<Arc<PgRegistry>>> {
    let sources: Vec<(String, PgSourceDef)> = config
        .layers
        .iter()
        .filter_map(|l| match &l.source {
            SourceKind::Postgis(def) => Some((l.name.clone(), def.clone())),
            _ => None,
        })
        .collect();
    if sources.is_empty() {
        tracing::info!("no postgis layers configured; PG support disabled");
        return Ok(None);
    }
    // Property discovery for defs that left `properties` empty. A temporary
    // pool per DSN probes the catalog; PgRegistry::build then owns the real
    // (shared) pools. Pool creation is verified-but-lazy, so this is cheap.
    let mut discovered: BTreeMap<String, Arc<PgPool>> = BTreeMap::new();
    let mut filled: Vec<(String, PgSourceDef)> = Vec::with_capacity(sources.len());
    for (name, mut def) in sources {
        // Config sanity (from-expression / simplify knobs): fail fast — a bad
        // FROM expression is a deploy-time bug, not a per-request one.
        def.validate()
            .map_err(|e| anyhow::anyhow!("postgis layer '{name}': {e}"))?;
        if def.properties.is_empty() {
            let pool = match discovered.get(&def.dsn) {
                Some(p) => Arc::clone(p),
                None => {
                    let p = Arc::new(
                        PgPool::new(&def.dsn, PG_POOL_MAX)
                            .await
                            .map_err(|e| anyhow::anyhow!("postgis {name}: {e}"))?,
                    );
                    discovered.insert(def.dsn.clone(), Arc::clone(&p));
                    p
                }
            };
            // SQL/view layers cannot be probed via information_schema; their
            // columns come from the row description of the FROM expression.
            let discovered_props = if let Some(f) = &def.from {
                match pg_source::sql::discover_from_fields(&pool, f, &def.geometry_column).await {
                    Ok(cols) if cols.is_empty() => {
                        tracing::warn!(
                            layer = %name,
                            from = %f,
                            "postgis property discovery found no non-geometry columns"
                        );
                        Some(Vec::new())
                    }
                    Ok(cols) => {
                        tracing::info!(
                            layer = %name,
                            from = %f,
                            columns = ?cols.iter().map(|(c, _)| c).collect::<Vec<_>>(),
                            "postgis properties discovered (sql/view layer)"
                        );
                        Some(cols.into_iter().map(|(c, _)| c).collect())
                    }
                    Err(e) => {
                        tracing::warn!(layer = %name, "postgis property discovery failed: {e}");
                        None
                    }
                }
            } else {
                match pg_source::sql::discover_columns(
                    &pool,
                    &def.schema,
                    &def.table,
                    &def.geometry_column,
                )
                .await
                {
                    Ok(cols) if cols.is_empty() => {
                        tracing::warn!(
                            layer = %name,
                            table = %def.table,
                            "postgis property discovery found no non-geometry columns"
                        );
                        Some(Vec::new())
                    }
                    Ok(cols) => {
                        tracing::info!(
                            layer = %name,
                            table = %def.table,
                            columns = ?cols,
                            "postgis properties discovered"
                        );
                        Some(cols)
                    }
                    Err(e) => {
                        tracing::warn!(layer = %name, "postgis property discovery failed: {e}");
                        None
                    }
                }
            };
            if let Some(cols) = discovered_props {
                def.properties = cols;
            }
        }
        // Bounds (EPSG:4326) + feature count probe for capabilities / OGC API
        // collection metadata. Uses the same temp pool as discovery; failures
        // are non-fatal (metadata stays unset, tile serving is unaffected).
        let probe_pool = match discovered.get(&def.dsn) {
            Some(p) => Arc::clone(p),
            None => {
                let p = Arc::new(
                    PgPool::new(&def.dsn, PG_POOL_MAX)
                        .await
                        .map_err(|e| anyhow::anyhow!("postgis {name}: {e}"))?,
                );
                discovered.insert(def.dsn.clone(), Arc::clone(&p));
                p
            }
        };
        // Spatial-index pre-flight (table sources only): a missing GiST index
        // turns every tile/feature query into a sequential scan — say so once,
        // loudly, at startup instead of in p99 latency later.
        if def.from.is_none() {
            match pg_source::sql::query_count(
                &probe_pool,
                &pg_source::sql::gist_index_sql(&def),
                &[],
            )
            .await
            {
                Ok(0) => tracing::warn!(
                    layer = %name,
                    table = %def.table,
                    "no GiST index on the geometry column; tile/feature queries will \
                     sequential-scan (CREATE INDEX ... USING GIST (geom))"
                ),
                Ok(_) => {}
                Err(e) => tracing::debug!(layer = %name, "gist index check failed: {e}"),
            }
        }
        if def.bounds.is_none() {
            let mut bounds: Option<[f64; 4]> = None;
            // Fast path (P3): planner statistics via ST_EstimatedExtent —
            // no full-table scan. Table sources only; NULL stats fall back.
            if def.from.is_none() {
                match pg_source::sql::query_bounds(
                    &probe_pool,
                    &pg_source::sql::bounds_sql_estimated(&def),
                )
                .await
                {
                    Ok(Some(b)) => bounds = Some(b),
                    Ok(None) => {
                        tracing::debug!(layer = %name, "no extent statistics; exact bounds probe")
                    }
                    Err(e) => tracing::debug!(layer = %name, "estimated bounds failed: {e}"),
                }
            }
            if bounds.is_none() {
                match pg_source::sql::query_bounds(&probe_pool, &pg_source::sql::bounds_sql(&def))
                    .await
                {
                    Ok(Some(b)) => bounds = Some(b),
                    _ => tracing::warn!(layer = %name, "postgis bounds probe failed"),
                }
            }
            def.bounds = bounds;
        }
        // Count: estimated first (reltuples), exact `count(*)` on fallback —
        // for SQL/view layers always exact (no single stats entry).
        let mut count: Option<i64> = None;
        if def.from.is_none() {
            match pg_source::sql::query_count(
                &probe_pool,
                &pg_source::sql::count_sql_estimated(&def),
                &[],
            )
            .await
            {
                Ok(c) if c > 0 => count = Some(c),
                _ => tracing::debug!(layer = %name, "no row statistics; exact count probe"),
            }
        }
        let count = match count {
            Some(c) => c,
            None => match pg_source::sql::query_count(
                &probe_pool,
                &pg_source::sql::count_all_sql(&def),
                &[],
            )
            .await
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(layer = %name, "postgis count probe failed: {e}");
                    0
                }
            },
        };
        def.feature_count = Some(count as u64);
        filled.push((name, def));
    }
    let registry = PgRegistry::build(filled, PG_POOL_MAX)
        .await
        .map_err(|e| anyhow::anyhow!("postgis registry: {e}"))?;
    match &registry {
        Some(r) => tracing::info!(layers = r.len(), "postgis support enabled"),
        None => tracing::info!("no postgis layers configured; PG support disabled"),
    }
    Ok(registry)
}

impl AppState {
    /// Test/compat constructor: files-mode registry, default config path.
    pub async fn new(config: ServerConfig, pg: Option<Arc<PgRegistry>>) -> anyhow::Result<AppState> {
        let registry = Registry::open_files(&config);
        Self::new_managed(config, pg, registry, None, PathBuf::from("config/mapserver.xml")).await
    }

    /// Async: the foyer output cache (and its optional disk tier) is built
    /// with an awaited builder. `registry` persists service management
    /// states (PG `sys_service` or file sidecar); services whose persisted
    /// state is stopped/paused/deleted are withheld at startup.
    pub async fn new_managed(
        config: ServerConfig,
        pg: Option<Arc<PgRegistry>>,
        registry: Registry,
        catalog_pool: Option<Arc<PgPool>>,
        config_path: PathBuf,
    ) -> anyhow::Result<AppState> {
        let cache_mib = config.server.cache_mib;
        // Decoded-tile cache capacity must assume the LARGEST configured tile
        // pixel size: a 512px layer stores 512*512*4 = 1 MiB per entry, not the
        // 256 KiB the old hardcode assumed (a 4x memory overshoot past
        // `cache_mib` on large-tile layers).
        let max_tile_px = config
            .layers
            .iter()
            .map(|l| l.tile_px.max(1))
            .max()
            .unwrap_or(256);
        // Decoded tiles expire on the shared TTL so a dataset replaced on disk
        // is re-decoded at most one TTL window later.
        let ttl = (config.server.cache_ttl_secs > 0)
            .then(|| std::time::Duration::from_secs(config.server.cache_ttl_secs));
        let tile_cache = TileCache::new(
            cache_mib,
            max_tile_px as u64 * max_tile_px as u64 * 4,
            ttl,
        );
        // Output cache is budgeted by BYTES (weigher), not entry count: a
        // 256px PNG tile is ~50-300 KB, so a plain entry-count capacity of 512
        // would evict after ~512 tiles (~50 MB at best). `foyer_cache_mib`
        // (default: `cache_mib`) budgets ONLY the foyer L1 tier; the decoded
        // RGBA tile cache above keeps its own `cache_mib` budget. Cached pbf
        // tiles are gzip'd once on the cold path, so this budget holds roughly
        // twice as many vector tiles as before.
        let foyer_mib = config.server.foyer_cache_mib.unwrap_or(cache_mib);
        let cache = CacheHub::build(
            foyer_mib,
            config.server.cache_ttl_secs,
            config.server.disk_cache.as_ref(),
        )
        .await?;
        tracing::info!(
            foyer_l1_mib = foyer_mib,
            decoded_cache_mib = cache_mib,
            foyer_l2_mib = config.server.disk_cache.as_ref().map(|d| d.capacity_mib).unwrap_or(0),
            "cache budgets (foyer output L1/L2 + decoded tiles)"
        );
        let custom_grids = config
            .grids
            .iter()
            .map(|(id, g)| (id.clone(), g.to_tms(id)))
            .collect();
        // Open every layer, COLLECTING all failures. A broken layer (typo'd
        // drive letter, moved dataset) is skipped with a warning so the rest of
        // the catalog still serves — same policy as the 3D Tiles registry below.
        // Startup only aborts when NOTHING could be opened: a server with zero
        // layers answers capabilities but fails every request, which is worse
        // for callers than an explicit startup error.
        // Persisted management states (registry): layers/tilesets whose state
        // is stopped/paused/deleted are not opened at all.
        let states = registry.fetch_states().await.unwrap_or_else(|e| {
            tracing::warn!("registry states unavailable ({e}); serving everything");
            BTreeMap::new()
        });
        let state_off = |name: &str| -> bool {
            states.get(name).map(|s| crate::admin::registry::is_off(s)).unwrap_or(false)
        };
        let mut state_skipped: Vec<String> = Vec::new();

        let mut layers = Vec::new();
        let mut open_errors: Vec<String> = Vec::new();
        let configured = config.layers.len();
        let s3_cfg = config.server.s3.as_ref().map(|c| c.to_storage());
        for (i, def) in config.layers.clone().into_iter().enumerate() {
            let name = def.name.clone();
            if state_off(&name) {
                state_skipped.push(name);
                continue;
            }
            match LayerRuntime::open(def, None, i as u64 + 1, s3_cfg.as_ref()) {
                Ok(rt) => layers.push(Arc::new(rt)),
                Err(e) => open_errors.push(format!("  layer '{name}': {e}")),
            }
        }
        if !state_skipped.is_empty() {
            tracing::info!(
                skipped = state_skipped.len(),
                names = ?state_skipped,
                "layers withheld by registry state"
            );
        }
        if !open_errors.is_empty() {
            for e in &open_errors {
                tracing::warn!("layer open failed, skipped:{e}");
            }
            if layers.is_empty() && configured > 0 {
                anyhow::bail!(
                    "all {configured} configured layer(s) failed to open, nothing to serve:\n{}",
                    open_errors.join("\n")
                );
            }
            tracing::warn!(
                skipped = open_errors.len(),
                served = layers.len(),
                "layers skipped due to open errors; service continues"
            );
        }
        // PROJ backend (optional, environment concern): resolution order is
        // explicit env vars PROJ_DLL / PROJ_DATA, then run-directory defaults
        // (`proj_9.dll` in the working directory, `<workdir>/share/proj`).
        // Missing/failed load only disables arbitrary-CRS reprojection.
        projection::init_proj(None, None);
        tracing::info!(
            proj_loaded = projection::proj().is_some(),
            "projection backend (PROJ_DLL/PROJ_DATA env or run-dir defaults)"
        );
        let base_url = config
            .server
            .public_url
            .clone()
            .unwrap_or_else(|| format!("http://{}:{}", config.server.host, config.server.port));
        // 3D Tiles 服务：扫描 <catalog.root>/3dtiles（一个 XML = 一个服务），失败仅告警。
        let tilesets_dir = config.tilesets_dir();
        let (mut tilesets, tileset_errors) = TilesetRegistry::load_dir(&tilesets_dir);
        for e in &tileset_errors {
            tracing::warn!("{e}");
        }
        // 登记簿状态过滤：停止/暂停/删除的 3D Tiles 服务不进服务目录。
        let ts_before = tilesets.len();
        tilesets.retain_by(|name| !state_off(name));
        if ts_before != tilesets.len() {
            tracing::info!(
                withheld = ts_before - tilesets.len(),
                "3D Tiles services withheld by registry state"
            );
        }
        tracing::info!(
            dir = %tilesets_dir.display(),
            count = tilesets.len(),
            "3D Tiles services loaded"
        );
        // Quantized-Mesh 地形服务：扫描 <catalog.root>/terrain/*.terrainpack。
        let terrain_dir = config.terrain_dir();
        let (terrains, terrain_errors) = TerrainRegistry::load_dir(&terrain_dir);
        for e in &terrain_errors {
            tracing::warn!("{e}");
        }
        tracing::info!(
            dir = %terrain_dir.display(),
            count = terrains.len(),
            "quantized-mesh terrain services loaded"
        );
        // SLD 样式库目录：配置所在目录下的 `styles/`（config/styles）。
        let styles_dir = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("styles");
        if !styles_dir.exists() {
            let _ = std::fs::create_dir_all(&styles_dir);
        }
        // db 后端：目录快照随 config 走（启动时已由 main 从共享库载入）。
        // 后续每次 refresh_catalog 会用库里的最新定义替换它。
        let catalog_layers = if config.catalog.backend == CatalogBackend::Db {
            config.layers.clone()
        } else {
            Vec::new()
        };
        Ok(AppState {
            config,
            config_path,
            catalog_pool: RwLock::new(catalog_pool),
            catalog_layers: RwLock::new(catalog_layers),
            layers: RwLock::new(layers),
            custom_grids: RwLock::new(custom_grids),
            tilesets: RwLock::new(tilesets),
            terrains: RwLock::new(terrains),
            tile_cache,
            cache,
            base_url,
            pg: RwLock::new(pg),
            api_keys: crate::apikeys::ApiKeyCache::default(),
            admin: AdminState {
                registry,
                usage: Usage::default(),
                access_log: AccessLog::new(4000),
                styles_dir,
            },
        })
    }

    /// Layer lookup by name, backed by the moka layer index
    /// (`CacheHub::layers`): O(1) after the first hit instead of a linear scan
    /// of the layer list. Unknown names are not negative-cached, so a layer
    /// added by a future hot-reload becomes visible immediately.
    pub fn layer(&self, name: &str) -> Option<Arc<LayerRuntime>> {
        self.cache.layer(name, || {
            self.layers
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|l| l.name() == name)
                .map(Arc::clone)
        })
    }

    /// Alias of [`Self::layer`] (kept for call sites that read better with the
    /// explicit name; both return the cached `Arc`).
    pub fn layer_arc(&self, name: &str) -> Option<Arc<LayerRuntime>> {
        self.layer(name)
    }

    /// Catalog-derived metadata (OGC API collections / grids), built once.
    pub fn catalog_meta(&self) -> Arc<CatalogMeta> {
        self.cache.catalog_meta(|| CatalogMeta::build(self))
    }

    /// Purge caches (operator refresh lever). `scope=all` also drops the
    /// decoded-tile cache; the layer index repopulates lazily on next lookup.
    pub fn purge_caches(&self, scope: CacheScope) -> PurgeReport {
        let report = self.cache.purge(scope);
        if scope == CacheScope::All {
            self.tile_cache.purge();
        }
        report
    }

    /// Live counters for every cache (decoded tiles included).
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats(CacheCounters {
            entries: self.tile_cache.entry_count(),
            bytes: self.tile_cache.estimated_bytes(),
        })
    }

    /// Force cache housekeeping (periodic stats task).
    pub fn cache_housekeeping(&self) {
        self.cache.run_pending_tasks();
        self.tile_cache.run_pending_tasks();
    }

    /// One periodic tick: log a stats line, then run housekeeping so
    /// TTL-expired entries are evicted even on an idle cache.
    pub fn cache_tick(&self) {
        let decoded = CacheCounters {
            entries: self.tile_cache.entry_count(),
            bytes: self.tile_cache.estimated_bytes(),
        };
        self.cache.log_stats(decoded);
        self.cache_housekeeping();
    }

    pub fn layer_names(&self) -> Vec<String> {
        self.layers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|l| l.name().to_string())
            .collect()
    }

    /// Served 3D Tiles service names (for /admin/services). Only *available*
    /// services count — a registered-but-data-missing service is visible in
    /// the catalog but reports `serving = false`.
    pub fn tileset_names(&self) -> Vec<String> {
        self.tilesets
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .list()
            .into_iter()
            .filter(|s| s.available)
            .map(|s| s.name.clone())
            .collect()
    }

    /// Current PostGIS registry handle (cheap `Arc` clone).
    pub fn pg_registry(&self) -> Option<Arc<PgRegistry>> {
        self.pg
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 重读主配置文件（不含 catalog 目录合并），供热重载与管理面复用。
    fn read_config_file(&self) -> anyhow::Result<ServerConfig> {
        let text = std::fs::read_to_string(&self.config_path)
            .map_err(|e| anyhow::anyhow!("read config {}: {e}", self.config_path.display()))?;
        let value = crate::xmlcfg::parse_to_value(&text)
            .map_err(|e| anyhow::anyhow!("parse config: {e}"))?;
        serde_json::from_value(value).map_err(|e| anyhow::anyhow!("parse config: {e}"))
    }

    /// 管理面用的「当前」图层定义。
    ///
    /// `config.layers` 是启动快照（启动时已被写成「内联 + 目录」的合并结果），
    /// 因此改名/硬删后的旧条目会一直残留在其中，而新建的文件型图层又不在里面；
    /// 直接拿它出管理列表就会出现「幽灵服务」与「新建服务看不见」。
    ///
    /// db 后端：目录真理源在共享库 `sys_published_layer.service_doc`，磁盘目录
    /// 不再是真相，这里返回库快照（由 `refresh_catalog` 每次刷新）。
    pub fn current_layers(&self) -> Vec<Layer> {
        if self.config.catalog.backend == CatalogBackend::Db {
            return self
                .catalog_layers
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
        }
        match self.read_config_file() {
            Ok(cfg) => cfg.collect_layers().0,
            Err(e) => {
                tracing::warn!("重读配置失败，管理面退回启动快照：{e}");
                self.config.collect_layers().0
            }
        }
    }

    /// Catalog scan for the admin surface: layers + 3D Tiles services as
    /// found on disk (config snapshot + tileset dir scan).
    pub fn catalog_scan(&self) -> Vec<ServiceScan> {
        // 从磁盘实时扫描 catalog 目录 + 内联 `layers`（`collect_layers` 会
        // 读盘），确保新建/回收（文件仍在盘）的服务都能出现在管理列表中，
        // 而不仅依赖启动时的 `config` 快照。
        let layers = self.current_layers();
        let mut out: Vec<ServiceScan> = layers
            .iter()
            .map(|l| ServiceScan {
                name: l.name.clone(),
                kind: source_kind(&l.source).to_string(),
                title: l.title.clone(),
                source: describe_source(&l.source),
            })
            .collect();
        let (tilesets, _) = TilesetRegistry::load_dir(&self.config.tilesets_dir());
        for s in tilesets.list() {
            out.push(ServiceScan {
                name: s.name.clone(),
                kind: "3dtiles".to_string(),
                title: s.title.clone(),
                source: s.root.display().to_string(),
            });
        }
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.name.cmp(&b.name)));
        out
    }

    /// DB 目录后端：从共享库重载图层定义。
    async fn load_db_catalog(
        &self,
    ) -> anyhow::Result<(Vec<Layer>, BTreeMap<String, GridDef>, Vec<String>)> {
        let pool = self
            .catalog_pool
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| anyhow::anyhow!("catalog.backend=db 但未配置目录连接池"))?;
        crate::db_catalog::load_catalog(&pool).await
    }

    /// Hot reload: re-read the config from disk, re-scan the catalog, sync the
    /// registry (upsert scan, never touching states), rebuild the serving sets
    /// filtered by persisted states and purge all caches.
    ///
    /// `reread=false` 只表示「这是一次状态热应用」：读盘失败时退回内存快照继续
    /// 生效，而不是让请求失败；图层清单本身始终按磁盘重建，否则启动后新建/改名的
    /// 服务会在任意一次状态变更后被挤出在服集合。
    pub async fn refresh_catalog(&self, reread: bool) -> anyhow::Result<RefreshReport> {
        let mut warnings: Vec<String> = Vec::new();
        let cfg: ServerConfig = match self.read_config_file() {
            Ok(mut c) => {
                if c.catalog.backend == CatalogBackend::Db {
                    // db 后端：图层定义来自共享库，磁盘目录不再扫描。
                    match self.load_db_catalog().await {
                        Ok((layers, extra_grids, warns)) => {
                            warnings.extend(warns);
                            c.grids.extend(extra_grids);
                            c.layers = layers;
                        }
                        Err(e) if reread => return Err(e),
                        Err(e) => {
                            warnings.push(format!("目录库重读失败，按内存快照重建：{e}"));
                            c.layers = self.config.layers.clone();
                        }
                    }
                } else {
                    let (layers, extra_grids, warns) = c.collect_layers();
                    warnings.extend(warns);
                    c.grids.extend(extra_grids);
                    c.layers = layers;
                }
                c
            }
            Err(e) if reread => return Err(e),
            Err(e) => {
                warnings.push(format!("重读配置失败，按内存快照重建：{e}"));
                self.config.clone()
            }
        };

        // 3D Tiles scan + registry states.
        let (mut tilesets, terrs) = TilesetRegistry::load_dir(&cfg.tilesets_dir());
        warnings.extend(terrs);
        self.admin.registry
            .upsert_services(&self.scan_records(&cfg, &tilesets))
            .await
            .unwrap_or_else(|e| warnings.push(format!("registry upsert: {e}")));
        let states = self
            .admin
            .registry
            .fetch_states()
            .await
            .unwrap_or_else(|e| {
                warnings.push(format!("registry states: {e}"));
                BTreeMap::new()
            });
        let state_off = |name: &str| -> bool {
            states
                .get(name)
                .map(|s| crate::admin::registry::is_off(s))
                .unwrap_or(false)
        };

        // Rebuild layer runtimes for the kept set.
        let mut served: Vec<Arc<LayerRuntime>> = Vec::new();
        let mut skipped: Vec<RefreshSkip> = Vec::new();
        let s3_cfg = cfg.server.s3.as_ref().map(|c| c.to_storage());
        for (i, def) in cfg.layers.iter().enumerate() {
            let name = &def.name;
            if state_off(name) {
                skipped.push(RefreshSkip {
                    name: name.clone(),
                    reason: states.get(name).cloned().unwrap_or_default(),
                });
                continue;
            }
            match LayerRuntime::open(def.clone(), None, i as u64 + 1, s3_cfg.as_ref()) {
                Ok(rt) => served.push(Arc::new(rt)),
                Err(e) => skipped.push(RefreshSkip {
                    name: name.clone(),
                    reason: format!("open failed: {e}"),
                }),
            }
        }
        let ts_before = tilesets.len();
        tilesets.retain_by(|name| !state_off(name));
        let tileset_names: Vec<String> =
            tilesets.list().into_iter().map(|s| s.name.clone()).collect();
        let _ = ts_before;

        // PostGIS registry: keep the existing pools when they already cover
        // every served postgis layer; rebuild only when the set grew.
        let kept_postgis: Vec<String> = served
            .iter()
            .filter(|l| l.is_postgis())
            .map(|l| l.name().to_string())
            .collect();
        let pg = match self.pg_registry() {
            Some(r) if kept_postgis.iter().all(|n| r.find(n).is_some()) => Some(r),
            _ => {
                let mut pg_cfg = cfg.clone();
                pg_cfg.layers
                    .retain(|l| served.iter().any(|s| s.name() == l.name));
                build_pg_registry(&pg_cfg).await?
            }
        };

        // Custom grids: the runtime authority (config.grids may have been
        // re-read; config itself stays the startup snapshot).
        let grids: BTreeMap<String, TileMatrixSet> = cfg
            .grids
            .iter()
            .map(|(id, g)| (id.clone(), g.to_tms(id)))
            .collect();

        // Swap everything, then drop stale caches (layer index etc.).
        let served_names: Vec<String> =
            served.iter().map(|l| l.name().to_string()).collect();
        *self.layers.write().unwrap_or_else(|e| e.into_inner()) = served;
        *self.tilesets.write().unwrap_or_else(|e| e.into_inner()) = tilesets;
        // db 后端：管理面快照同步为库里的最新定义（current_layers 读它）。
        if cfg.catalog.backend == CatalogBackend::Db {
            *self.catalog_layers.write().unwrap_or_else(|e| e.into_inner()) = cfg.layers.clone();
        }
        // 地形服务一并重扫（新放 .terrainpack 后 /admin/reload 即生效）。
        let (terrains, terrain_errs) = TerrainRegistry::load_dir(&cfg.terrain_dir());
        for e in &terrain_errs {
            warnings.push(e.clone());
        }
        *self.terrains.write().unwrap_or_else(|e| e.into_inner()) = terrains;
        *self.custom_grids.write().unwrap_or_else(|e| e.into_inner()) = grids;
        *self.pg.write().unwrap_or_else(|e| e.into_inner()) = pg;
        self.purge_caches(CacheScope::All);

        let report = RefreshReport {
            registry_mode: self.admin.registry.mode().to_string(),
            served: served_names,
            tilesets: tileset_names,
            skipped,
            warnings,
        };
        tracing::info!(
            served = report.served.len(),
            tilesets = report.tilesets.len(),
            skipped = report.skipped.len(),
            reread,
            "catalog refreshed"
        );
        Ok(report)
    }

    /// Scan records for the registry (layers + tilesets of a config).
    fn scan_records(&self, cfg: &ServerConfig, tilesets: &TilesetRegistry) -> Vec<ServiceScan> {
        let mut out: Vec<ServiceScan> = cfg
            .layers
            .iter()
            .map(|l| ServiceScan {
                name: l.name.clone(),
                kind: source_kind(&l.source).to_string(),
                title: l.title.clone(),
                source: describe_source(&l.source),
            })
            .collect();
        for s in tilesets.list() {
            out.push(ServiceScan {
                name: s.name.clone(),
                kind: "3dtiles".to_string(),
                title: s.title.clone(),
                source: s.root.display().to_string(),
            });
        }
        out
    }

    /// Resolve the TileMatrixSet a layer publishes under `grid_id`.
    /// Authority order: the swapped-in custom grid map (hot-reloadable) —
    /// `config.grids` is only its startup snapshot — then the preset grids.
    pub fn grid_for_layer(&self, layer: &LayerRuntime, grid_id: &str) -> Option<TileMatrixSet> {
        let custom = self.custom_grids.read().unwrap_or_else(|e| e.into_inner());
        if let Some(g) = custom.get(grid_id) {
            return Some(g.clone());
        }
        // 容忍能力文档宣告的“剥离尺寸后缀”写法（如 `wh_custom_grid` ↔ 配置里的
        // 完整 id `wh_custom_grid_4522`）：`strip_size_suffix` 会把投影带码（4522）
        // 当尺寸后缀剥掉，客户端（尤其 OL WMTS 按 caps 回发）只会带剥离 id。这里在
        // 图层声明的网格里按同一剥离规则反查完整 key 再取，避免 WMTS GetTile 误判无网格。
        let want = wms_common::tile::strip_size_suffix(grid_id).to_ascii_lowercase();
        for declared in &layer.def.grids {
            if wms_common::tile::strip_size_suffix(declared).to_ascii_lowercase() == want {
                if let Some(g) = custom.get(declared) {
                    return Some(g.clone());
                }
            }
        }
        drop(custom);
        TileMatrixSet::resolve(grid_id, layer.def.tile_px, &BTreeMap::new())
    }

    /// The layer's first published grid (the default for TMS/XYZ).
    pub fn default_grid(&self, layer: &LayerRuntime) -> Option<TileMatrixSet> {
        let id = self.config.default_grid_id(&layer.def);
        self.grid_for_layer(layer, &id)
    }

    /// Resolve a grid by id **only when the layer advertises it** (matches one
    /// of `layer.def.grids`, size-suffix / case insensitive). Lets XYZ/TMS serve
    /// the geographic `WorldCRS84Quad` grid the way WMTS already does, without
    /// silently rasterizing grids a layer never declared.
    pub fn advertised_grid(&self, layer: &LayerRuntime, grid_id: &str) -> Option<TileMatrixSet> {
        let want = wms_common::tile::strip_size_suffix(grid_id).to_ascii_lowercase();
        let declared = layer
            .def
            .grids
            .iter()
            .any(|g| wms_common::tile::strip_size_suffix(g).to_ascii_lowercase() == want);
        if declared {
            self.grid_for_layer(layer, grid_id)
        } else {
            None
        }
    }

    /// All grids the layer publishes (for capabilities).
    pub fn grids_for_layer(&self, layer: &LayerRuntime) -> Vec<TileMatrixSet> {
        layer
            .def
            .grids
            .iter()
            .filter_map(|g| self.grid_for_layer(layer, g))
            .collect()
    }
}
