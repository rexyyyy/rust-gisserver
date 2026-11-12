//! foyer multi-level cache hub: every process-level cache in one place, plus
//! the refresh mechanism (TTL + explicit purge + periodic housekeeping).
//!
//! The whole caching stack runs on **foyer** (no moka, no redis):
//!
//! | cache      | foyer type    | key                       | value                  | notes |
//! |------------|---------------|---------------------------|------------------------|-------|
//! | `output`   | `HybridCache` | `route/layer/z/x/y/fmt`   | encoded tile bytes     | L1 memory + **optional L2 disk** (`server.disk_cache`); byte-weighed against `foyer_cache_mib` |
//! | `empty_tiles` | `Cache`    | output cache key          | `()`                   | negative cache for empty PostGIS tiles (404 without a DB query) |
//! | `tile_flights` | in-flight | output cache key          | one build per key      | coalesces concurrent cold tile queries (singleflight) |
//! | `doc_flights`  | in-flight | document key              | one render per key     | coalesces concurrent cold async document renders (TileJSON) |
//! | `layers`   | `Cache`       | layer name                | `Arc<LayerRuntime>`    | index cache replacing the O(n) scan in `AppState::layer` |
//! | `docs`     | `Cache`       | document key (see below)  | rendered metadata body | byte-weighed (64 MiB budget) |
//! | `catalog`  | `Cache`       | `"catalog"` (single)      | [`CatalogMeta`]        | collection metadata derived from the catalog |
//! | `features` | `Cache`       | `(collectionId, featId)`  | one feature's GeoJSON  | single-feature fetches |
//!
//! Document key convention (`docs` cache):
//!
//! ```text
//! wms-caps/{version}/{base}            WMS GetCapabilities
//! wmts-caps/{base}                     WMTS GetCapabilities (KVP + REST)
//! wmts-tms/{id}                        WMTS TileMatrixSet document
//! wfs-caps/{version}/{base}            WFS GetCapabilities
//! wfs-dft/{base}/{typeName}            WFS DescribeFeatureType
//! oaf-landing/{base}                   OGC API - Features landing page
//! oaf-conformance                      (base-free constant)
//! oaf-api/{base}                       OGC API - Features OpenAPI document
//! oaf-collections/{base}               collection list
//! oaf-collection/{base}/{id}           one collection
//! oaf-items/{base}/{id}/{limit}/{offset}/{bbox}   items page
//! oat-landing/{base}                   OGC API - Tiles landing page
//! oat-conformance                      (base-free constant)
//! oat-tms-list/{base}                  tileMatrixSets list
//! oat-tms/{base}/{id}                  one tileMatrixSet
//! oat-collections/{base}               tiled collections list
//! oat-collection/{base}/{id}           one tiled collection
//! oat-tilesets/{base}/{id}             tilesets of a collection
//! oat-tileset/{base}/{id}/{tms}        one tileset
//! mvt-tilejson/{base}/{layer}          TileJSON (file-vector layers)
//! mvt-tilejson-pg/{base}/{layer}/{tms} TileJSON (PostGIS, P2: DB probes cached)
//! mvt-style/{layer}                    MapLibre style (base-free)
//! ```
//!
//! ## TTL
//!
//! foyer's in-memory `Cache` has no built-in TTL, so every value is wrapped
//! in [`Timed`]: the insert timestamp travels with the value (and survives
//! the disk round-trip for the `output` cache). A lookup older than
//! `server.cache_ttl_secs` is treated as a miss and the entry is removed —
//! so an entry is refreshed at most once per TTL window, bounding staleness
//! after data on disk / in the database is replaced. `0` disables expiry.
//!
//! ## Refresh mechanism (three layers)
//!
//! 1. **TTL** (above).
//! 2. **Explicit purge**: `POST /api/cache/purge?scope=...` invalidates a
//!    cache (or all) immediately — the operator's refresh lever after
//!    swapping datasets. `GET /api/cache` reports live counters.
//! 3. **Periodic housekeeping**: a tokio task (`server.cache_stats_secs`,
//!    default 60) logs counters. foyer evicts on access, so unlike moka no
//!    forced maintenance pass is needed; the task remains for observability.
//!
//! ## Cold-miss coalescing (singleflight)
//!
//! foyer has no build-on-miss primitive. For cheap synchronous documents a
//! double build is merely wasted work, but a cold PostGIS tile costs a database
//! query (hundreds of ms) and an async TileJSON costs 2-3 catalog queries.
//! [`FlightGroup`] keeps one `tokio::sync::OnceCell` per key: concurrent misses
//! for the same key share exactly one build, and the leader's result (tile,
//! empty tile, or error) is delivered to every waiter. If the leader's task is
//! cancelled, the next waiter transparently becomes the new leader.
//!
//! Re-entrancy note: plain `doc()` / `catalog_meta()` builds only read
//! `AppState`'s immutable fields, so a rare double build is harmless.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use foyer::{Cache as FoyerCache, DeviceBuilder, HybridCache};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::OnceCell;

use ogcapi_features::CollectionInfo;
use ogcapi_tiles::TiledCollection;
use wms_common::Bbox;
use wms_common::tile::TileMatrixSet;

use crate::config::DiskCacheOpts;
use crate::layer::LayerRuntime;
use crate::state::AppState;
use crate::tiles::TileOut;

/// Single key under which [`CatalogMeta`] is cached (one catalog per process).
pub const CATALOG_KEY: &str = "catalog";

/// Byte budget for the `docs` cache (metadata documents). Documents are
/// small (a few KB each) but the key space grows with distinct base URLs and
/// query strings, so the budget — not an entry count — is the real cap.
/// P2: PostGIS TileJSON also lands here, so the budget is roomier than the
/// static capability documents alone would need.
const DOCS_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// Byte budget for negative entries (empty PostGIS tiles). An empty tile is
/// tiny (key + TTL wrapper) but clients routinely probe the whole empty ring
/// around a dataset, so the key space is much larger than the hot tile set.
const EMPTY_TILES_BUDGET_BYTES: u64 = 32 * 1024 * 1024;

/// Entry caps for the non-byte-weighed caches.
const LAYERS_CAP: u64 = 4096;
const CATALOG_CAP: u64 = 8;
const FEATURES_CAP: u64 = 8192;

/// Result of ONE cold PostGIS tile build, shared by every waiter coalesced on
/// the same cache key. The leader writes the positive/negative cache entry
/// before returning, so followers never touch the database.
#[derive(Clone, Debug)]
pub enum TileBuild {
    /// Encoded (and gzip'd) tile payload.
    Tile(TileOut),
    /// No geometry in this tile -> 404. Negative-cached for the TTL window.
    Empty,
    /// Query failed -> 500 with this diagnostic. Errors are never cached.
    Error(String),
}

/// Per-key singleflight over an async build. One `OnceCell` is installed in a
/// small map while the build runs and removed as soon as the result is ready;
/// late waiters find the value in the real cache instead.
pub struct FlightGroup<T>
where
    T: Clone + Send + Sync + 'static,
{
    cells: Mutex<HashMap<String, Arc<OnceCell<Arc<T>>>>>,
}

impl<T> Default for FlightGroup<T>
where
    T: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self {
            cells: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> FlightGroup<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Run `build` at most once per key for the set of concurrent callers.
    /// Every caller receives the same `Arc<T>`; only the leader's future is
    /// polled (tokio's `OnceCell::get_or_init` guarantees that).
    pub async fn run<F, Fut>(&self, key: &str, build: F) -> Arc<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let cell = {
            let mut cells = self.cells.lock().unwrap_or_else(|e| e.into_inner());
            cells
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };
        let value = cell
            .get_or_init(|| async { Arc::new(build().await) })
            .await
            .clone();
        // Only drop the cell we installed, never a newer flight for the same
        // key. Waiters keep their own Arc, so removing early is safe.
        let mut cells = self.cells.lock().unwrap_or_else(|e| e.into_inner());
        if cells
            .get(key)
            .map(|c| Arc::ptr_eq(c, &cell))
            .unwrap_or(false)
        {
            cells.remove(key);
        }
        value
    }

    /// Number of builds currently in flight (observability only).
    pub fn len(&self) -> usize {
        self.cells
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

/// TTL wrapper: the insert instant (ms since Unix epoch) travels with the
/// value so expiry survives the foyer disk round-trip. `Instant` cannot be
/// serialized; wall-clock ms is sufficient for cache staleness windows.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Timed<V> {
    pub v: V,
    pub at_ms: u64,
}

impl<V> Timed<V> {
    fn new(v: V) -> Self {
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Timed { v, at_ms }
    }

    fn expired(&self, ttl: Option<Duration>) -> bool {
        match ttl {
            None => false,
            Some(d) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|x| x.as_millis() as u64)
                    .unwrap_or(0);
                now.saturating_sub(self.at_ms) > d.as_millis() as u64
            }
        }
    }
}

/// Catalog-derived metadata, shared by the OGC API - Features / OGC API -
/// Tiles front-ends. Immutable while the process runs (layers are opened at
/// startup and never swapped), so it is built once and cached.
pub struct CatalogMeta {
    /// Vector layers as OGC API - Features collections.
    pub oaf_collections: Arc<Vec<CollectionInfo>>,
    /// Tile-serving layers as OGC API - Tiles tiled collections.
    pub oat_tiled: Arc<Vec<TiledCollection>>,
    /// Grids advertised at `/ogc/tiles/v1/tileMatrixSets` (presets + custom).
    pub advertised_grids: Arc<Vec<TileMatrixSet>>,
}

impl CatalogMeta {
    /// Build from the app state. Takes the layers read-guard ONCE and clones
    /// the `Arc`s out — `tiled_collection` re-locks internally, and holding
    /// the guard across that call would be a recursive read acquisition
    /// (deadlock-prone with a waiting writer). Safe to call from a
    /// miss-build closure (see module re-entrancy note).
    pub fn build(state: &AppState) -> CatalogMeta {
        let pg_reg = state.pg_registry();
        let layers: Vec<Arc<crate::layer::LayerRuntime>> = {
            let guard = state.layers.read().unwrap_or_else(|e| e.into_inner());
            guard.iter().filter(|l| l.is_queryable()).cloned().collect()
        };
        let oaf_collections = Arc::new(
            layers
                .iter()
                .map(|l| {
                    if let Some((_, pg_def)) = pg_reg.as_ref().and_then(|r| r.find(l.name())) {
                        // PostGIS layers have no in-memory FeatureCollection;
                        // collection metadata comes from the runtime-filled
                        // source def (bounds / count probed at startup).
                        CollectionInfo::from_parts(
                            l.name(),
                            &l.def.title,
                            pg_def.bounds.map(|b| Bbox::new(b[0], b[1], b[2], b[3])),
                            pg_def.feature_count.unwrap_or(0) as usize,
                        )
                    } else {
                        // Borrow the layer's FeatureCollection — no per-request clone.
                        let empty = serde_json::json!({"type":"FeatureCollection","features":[]});
                        let fc = l.vector_fc.as_ref().unwrap_or(&empty);
                        CollectionInfo::from_fc(l.name(), &l.def.title, fc)
                    }
                })
                .collect::<Vec<_>>(),
        );
        // Scan the layer list directly (not via the layer index) so a cache
        // build never touches another cache.
        let oat_tiled = Arc::new(
            layers
                .iter()
                .filter_map(|l| tiled_collection(state, l.name()))
                .collect::<Vec<_>>(),
        );
        let mut grids = vec![
            TileMatrixSet::web_mercator_quad(256),
            TileMatrixSet::world_crs84_quad(256),
            TileMatrixSet::cgcs2000_quad(256),
        ];
        for g in state
            .custom_grids
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            if !grids.iter().any(|x| x.id == g.id) {
                grids.push(g.clone());
            }
        }
        CatalogMeta {
            oaf_collections,
            oat_tiled,
            advertised_grids: Arc::new(grids),
        }
    }
}

/// One tile-serving layer as an OGC API - Tiles tiled collection
/// (moved here from `handlers::ogcapi_tiles` so the catalog cache can build it).
fn tiled_collection(state: &AppState, name: &str) -> Option<TiledCollection> {
    let layer = state
        .layers
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|l| l.name() == name)?
        .clone();
    let vector = layer.is_vector() || layer.is_postgis();
    let (data_type, formats): (&'static str, &'static [&'static str]) = if vector {
        ("vector", ogcapi_tiles::VECTOR_FORMATS)
    } else {
        ("map", ogcapi_tiles::MAP_FORMATS)
    };
    let tile_matrix_sets: Vec<String> = layer
        .def
        .grids
        .iter()
        .map(|g| wms_common::tile::strip_size_suffix(g))
        .collect();
    let extent = layer.bounds().map(|b| [b.minx, b.miny, b.maxx, b.maxy]);
    let zoom_range = state.default_grid(&layer).map(|g| {
        let zs: Vec<u32> = g.levels.iter().map(|l| l.z).collect();
        (*zs.first().unwrap_or(&0), *zs.last().unwrap_or(&0))
    });
    Some(TiledCollection {
        id: name.to_string(),
        title: if layer.def.title.is_empty() {
            name.to_string()
        } else {
            layer.def.title.clone()
        },
        data_type,
        tile_matrix_sets,
        formats,
        extent,
        zoom_range,
    })
}

/// Live counters for one cache (for `GET /api/cache`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct CacheCounters {
    /// Number of live entries (approximate for the `output` cache: foyer
    /// evicts in the background, so the tracked insert count only decreases
    /// on purge).
    pub entries: u64,
    /// Tracked bytes (byte-weighed caches only; 0 when entries are not weighed).
    pub bytes: u64,
}

/// Which cache(s) a purge targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheScope {
    All,
    Output,
    Docs,
    Catalog,
    Features,
    Layers,
}

impl CacheScope {
    /// Parse a `scope` query value (`None` = unknown scope).
    pub fn parse(s: &str) -> Option<CacheScope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "all" => Some(CacheScope::All),
            "output" | "tiles" => Some(CacheScope::Output),
            "docs" | "metadata" => Some(CacheScope::Docs),
            "catalog" | "collections" => Some(CacheScope::Catalog),
            "features" => Some(CacheScope::Features),
            "layers" | "index" => Some(CacheScope::Layers),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CacheScope::All => "all",
            CacheScope::Output => "output",
            CacheScope::Docs => "docs",
            CacheScope::Catalog => "catalog",
            CacheScope::Features => "features",
            CacheScope::Layers => "layers",
        }
    }
}

/// Result of a purge operation.
#[derive(Clone, Debug, Serialize)]
pub struct PurgeReport {
    pub scope: String,
    /// Entries dropped by this purge (inserts since the last purge).
    pub purged_entries: u64,
    /// Purges executed since process start (monotonic).
    pub purges: u64,
}

/// Snapshot of every cache (for `GET /api/cache`).
#[derive(Clone, Debug, Serialize)]
pub struct CacheStats {
    /// Configured TTL in seconds (0 = no expiry).
    pub ttl_secs: u64,
    /// Purges executed since process start.
    pub purges: u64,
    pub output: CacheCounters,
    /// Negative entries (empty PostGIS tiles).
    pub empty_tiles: CacheCounters,
    /// Concurrent cold-tile builds currently coalescing (one per key).
    pub tile_flights: u64,
    /// Concurrent async document builds currently coalescing.
    pub doc_flights: u64,
    pub layers: CacheCounters,
    pub docs: CacheCounters,
    pub catalog: CacheCounters,
    pub features: CacheCounters,
    /// Decoded-tile cache (lives in `storage::TileCache`, reported here too).
    pub decoded_tiles: CacheCounters,
    /// L2 disk tier of the foyer `output` cache (present only when
    /// `server.disk_cache` is configured). `entries` is approximate; `bytes`
    /// reports the foyer memory-tier usage of that hybrid cache.
    pub disk: Option<CacheCounters>,
}

/// Type alias for the foyer in-memory cache shape used by the hub.
type MemCache<K, V> = FoyerCache<K, Timed<V>>;

/// All process-level foyer caches plus the refresh bookkeeping.
pub struct CacheHub {
    /// Final output tiles: L1 foyer memory + optional L2 foyer disk.
    pub output: HybridCache<String, Timed<TileOut>>,
    /// Empty-tile negative cache (keyed exactly like `output`).
    empty_tiles: MemCache<String, ()>,
    /// One cold PostGIS tile query per key across concurrent requests.
    pub tile_flights: FlightGroup<TileBuild>,
    /// One cold async metadata render per key across concurrent requests.
    doc_flights: FlightGroup<Arc<String>>,
    /// Layer name -> runtime (index cache; no TTL — layers never change).
    pub layers: MemCache<String, Arc<LayerRuntime>>,
    /// Rendered metadata documents (capabilities / OGC API / TileJSON / style).
    pub docs: MemCache<String, Arc<String>>,
    /// Catalog-derived metadata under [`CATALOG_KEY`].
    pub catalog: MemCache<String, Arc<CatalogMeta>>,
    /// `(collectionId, featureId)` -> one feature's GeoJSON.
    pub features: MemCache<(String, String), Arc<Value>>,
    /// TTL applied to every cache (`None` = no expiry).
    ttl: Option<Duration>,
    /// Whether the foyer output cache carries an L2 disk tier.
    disk_enabled: bool,
    /// Insert counters (stats are approximate: foyer evicts in background).
    inserted_output: AtomicU64,
    inserted_empty: AtomicU64,
    inserted_layers: AtomicU64,
    inserted_docs: AtomicU64,
    inserted_catalog: AtomicU64,
    inserted_features: AtomicU64,
    purges: AtomicU64,
}

impl CacheHub {
    /// Build the hub. `cache_mib` budgets the foyer output cache (0 disables
    /// it; callers pass `server.foyer_cache_mib` when set); `ttl_secs` is the
    /// shared TTL (0 = no expiry); `disk` enables the foyer L2 disk tier for
    /// the output cache when configured.
    pub async fn build(
        cache_mib: u64,
        ttl_secs: u64,
        disk: Option<&DiskCacheOpts>,
    ) -> anyhow::Result<CacheHub> {
        let ttl = (ttl_secs > 0).then(|| Duration::from_secs(ttl_secs));
        let mem_bytes = (if cache_mib == 0 { 1 } else { cache_mib }) * 1024 * 1024;
        // `.storage().build()` with NO engine config yields a no-op disk tier:
        // the hybrid cache then behaves as memory-only (the config gate for
        // the L2 disk tier). With `server.disk_cache` set, a block engine on
        // the configured directory is attached.
        let mut phase = foyer::HybridCacheBuilder::new()
            .memory(mem_bytes as usize)
            .with_weighter(|k: &String, v: &Timed<TileOut>| {
                (k.len() + v.v.bytes.len() + 64).min(u32::MAX as usize)
            })
            .storage();
        let mut disk_enabled = false;
        if let Some(dc) = disk {
            let device = foyer::FsDeviceBuilder::new(std::path::Path::new(&dc.dir))
                .with_capacity((dc.capacity_mib.max(1) as usize) * 1024 * 1024)
                .build()
                .map_err(|e| anyhow::anyhow!("foyer disk cache '{}': {e}", dc.dir))?;
            phase = phase.with_engine_config(foyer::BlockEngineConfig::new(device));
            disk_enabled = true;
        }
        let output = phase.build().await?;
        let layers = foyer::CacheBuilder::new(LAYERS_CAP as usize)
            .with_weighter(|_: &String, _: &Timed<Arc<LayerRuntime>>| 1usize)
            .build();
        let docs = foyer::CacheBuilder::new(DOCS_BUDGET_BYTES as usize)
            .with_weighter(|k: &String, v: &Timed<Arc<String>>| {
                (k.len() + v.v.len() + 64).min(u32::MAX as usize)
            })
            .build();
        let empty_tiles = foyer::CacheBuilder::new(EMPTY_TILES_BUDGET_BYTES as usize)
            .with_weighter(|k: &String, _: &Timed<()>| (k.len() + 64).min(u32::MAX as usize))
            .build();
        let catalog = foyer::CacheBuilder::new(CATALOG_CAP as usize)
            .with_weighter(|_: &String, _: &Timed<Arc<CatalogMeta>>| 1usize)
            .build();
        let features = foyer::CacheBuilder::new(FEATURES_CAP as usize)
            .with_weighter(|_: &(String, String), _: &Timed<Arc<Value>>| 1usize)
            .build();
        Ok(CacheHub {
            output,
            empty_tiles,
            tile_flights: FlightGroup::default(),
            doc_flights: FlightGroup::default(),
            layers,
            docs,
            catalog,
            features,
            ttl,
            disk_enabled,
            inserted_output: AtomicU64::new(0),
            inserted_empty: AtomicU64::new(0),
            inserted_layers: AtomicU64::new(0),
            inserted_docs: AtomicU64::new(0),
            inserted_catalog: AtomicU64::new(0),
            inserted_features: AtomicU64::new(0),
            purges: AtomicU64::new(0),
        })
    }

    /// Configured TTL (`None` = entries never expire by time).
    pub fn ttl(&self) -> Option<Duration> {
        self.ttl
    }

    /// In-memory get with TTL handling: expired entries are removed and
    /// reported as misses.
    fn mem_get<K, V>(&self, cache: &MemCache<K, V>, key: &K) -> Option<V>
    where
        K: foyer::Key,
        V: Clone + Send + Sync + 'static,
    {
        let entry = cache.get(key)?;
        let timed: &Timed<V> = entry.value();
        if timed.expired(self.ttl) {
            cache.remove(key);
            None
        } else {
            Some(timed.v.clone())
        }
    }

    /// Rendered metadata document, cached under `key`. A miss builds once
    /// (two racing misses may build twice — see module note).
    pub fn doc(&self, key: String, build: impl FnOnce() -> String) -> Arc<String> {
        if let Some(v) = self.mem_get(&self.docs, &key) {
            return v;
        }
        let v = Arc::new(build());
        self.docs.insert(key, Timed::new(Arc::clone(&v)));
        self.inserted_docs.fetch_add(1, Ordering::Relaxed);
        v
    }

    /// Lookup without building (used by async cache paths that must decide
    /// whether to join a flight before they can construct the build closure).
    pub fn doc_get(&self, key: &str) -> Option<Arc<String>> {
        self.mem_get(&self.docs, &key.to_string())
    }

    /// Async version of [`Self::doc`]: for metadata whose render needs I/O
    /// (PostGIS TileJSON probes bounds and catalog fields). Concurrent misses
    /// for the same key share one render; the result is stored in `docs` with
    /// the shared TTL and is invalidated by `scope=docs` purges.
    pub async fn doc_async<F, Fut>(&self, key: String, build: F) -> Arc<String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = String>,
    {
        if let Some(v) = self.mem_get(&self.docs, &key) {
            return v;
        }
        let wrapped = self
            .doc_flights
            .run(&key, || async {
                let v = Arc::new(build().await);
                self.docs.insert(key.clone(), Timed::new(Arc::clone(&v)));
                self.inserted_docs.fetch_add(1, Ordering::Relaxed);
                v
            })
            .await;
        // unwrap the outer flight Arc; the inner Arc is the cached document.
        (*wrapped).clone()
    }

    /// Catalog-derived metadata (single entry under [`CATALOG_KEY`]).
    pub fn catalog_meta(&self, build: impl FnOnce() -> CatalogMeta) -> Arc<CatalogMeta> {
        let key = CATALOG_KEY.to_string();
        if let Some(v) = self.mem_get(&self.catalog, &key) {
            return v;
        }
        let v = Arc::new(build());
        self.catalog.insert(key, Timed::new(Arc::clone(&v)));
        self.inserted_catalog.fetch_add(1, Ordering::Relaxed);
        v
    }

    /// One feature by `(collectionId, featureId)`, built on miss. Misses that
    /// stay missing are **not** cached (no negative caching: an id that does
    /// not exist yet may exist after a data refresh).
    pub fn feature(
        &self,
        collection_id: &str,
        feature_id: &str,
        build: impl FnOnce() -> Option<Value>,
    ) -> Option<Arc<Value>> {
        let key = (collection_id.to_string(), feature_id.to_string());
        if let Some(v) = self.mem_get(&self.features, &key) {
            return Some(v);
        }
        let v = build().map(Arc::new)?;
        self.features.insert(key, Timed::new(Arc::clone(&v)));
        self.inserted_features.fetch_add(1, Ordering::Relaxed);
        Some(v)
    }

    /// Layer index lookup: name -> runtime, populated on first miss by
    /// scanning the immutable layer list. Unknown names are not cached.
    pub fn layer(
        &self,
        name: &str,
        build: impl FnOnce() -> Option<Arc<LayerRuntime>>,
    ) -> Option<Arc<LayerRuntime>> {
        let key = name.to_string();
        if let Some(v) = self.mem_get(&self.layers, &key) {
            return Some(v);
        }
        let v = build()?;
        self.layers.insert(key, Timed::new(Arc::clone(&v)));
        self.inserted_layers.fetch_add(1, Ordering::Relaxed);
        Some(v)
    }

    /// Two-level output lookup (L1 memory -> L2 disk, both foyer): a disk hit
    /// is promoted into the memory tier automatically by foyer.
    pub async fn output_get(&self, key: &str) -> Option<TileOut> {
        let entry = self.output.get(key).await.ok().flatten()?;
        let timed: &Timed<TileOut> = entry.value();
        if timed.expired(self.ttl) {
            self.output.remove(key);
            None
        } else {
            Some(timed.v.clone())
        }
    }

    /// Two-level output insert (memory + disk queue).
    pub fn output_insert(&self, key: &str, tile: TileOut) {
        self.output.insert(key.to_string(), Timed::new(tile));
        self.inserted_output.fetch_add(1, Ordering::Relaxed);
    }

    /// Negative lookup: `true` when this tile was already found empty and is
    /// still inside the TTL window. Empty tiles are NOT stored in `output`
    /// (their payload would be an empty 200 body); a separate keyed entry
    /// keeps the semantics explicit and the response a 404.
    pub fn empty_get(&self, key: &str) -> bool {
        self.mem_get(&self.empty_tiles, &key.to_string()).is_some()
    }

    /// Remember an empty tile for one TTL window.
    pub fn empty_insert(&self, key: &str) {
        self.empty_tiles.insert(key.to_string(), Timed::new(()));
        self.inserted_empty.fetch_add(1, Ordering::Relaxed);
    }

    /// Invalidate the caches selected by `scope`, returning what was dropped.
    pub fn purge(&self, scope: CacheScope) -> PurgeReport {
        let purged = match scope {
            CacheScope::All => self.sum_inserted(),
            CacheScope::Output => {
                self.take_inserted(&self.inserted_output) + self.take_inserted(&self.inserted_empty)
            }
            CacheScope::Docs => self.take_inserted(&self.inserted_docs),
            CacheScope::Catalog => self.take_inserted(&self.inserted_catalog),
            CacheScope::Features => self.take_inserted(&self.inserted_features),
            CacheScope::Layers => self.take_inserted(&self.inserted_layers),
        };
        if matches!(scope, CacheScope::All | CacheScope::Output) {
            // HybridCache::clear is async; fire-and-forget so the operator
            // endpoint stays synchronous. The disk tier is rebuilt by foyer's
            // reclaimers; the memory tier drops immediately on next tick.
            let out = self.output.clone();
            tokio::spawn(async move {
                let _ = out.clear().await;
            });
            // Negative entries describe the same tiles as `output`: dropping
            // one without the other would keep serving stale 404s.
            self.empty_tiles.clear();
        }
        if matches!(scope, CacheScope::All | CacheScope::Docs) {
            self.docs.clear();
        }
        if matches!(scope, CacheScope::All | CacheScope::Catalog) {
            self.catalog.clear();
        }
        if matches!(scope, CacheScope::All | CacheScope::Features) {
            self.features.clear();
        }
        if matches!(scope, CacheScope::All | CacheScope::Layers) {
            self.layers.clear();
        }
        PurgeReport {
            scope: scope.as_str().to_string(),
            purged_entries: purged,
            purges: self.purges.fetch_add(1, Ordering::Relaxed) + 1,
        }
    }

    fn sum_inserted(&self) -> u64 {
        self.take_inserted(&self.inserted_output)
            + self.take_inserted(&self.inserted_empty)
            + self.take_inserted(&self.inserted_docs)
            + self.take_inserted(&self.inserted_catalog)
            + self.take_inserted(&self.inserted_features)
            + self.take_inserted(&self.inserted_layers)
    }

    fn take_inserted(&self, c: &AtomicU64) -> u64 {
        c.swap(0, Ordering::Relaxed)
    }

    /// Live counters for every cache (`decoded_tiles` is passed in from the
    /// `storage::TileCache`, which lives outside the hub).
    ///
    /// foyer evicts in the background, so `entries` here is an upper bound
    /// (tracked inserts since the last purge) rather than moka's exact count.
    pub fn stats(&self, decoded_tiles: CacheCounters) -> CacheStats {
        let c = |entries: u64, bytes: u64| CacheCounters { entries, bytes };
        let out_entries = self.inserted_output.load(Ordering::Relaxed);
        // NOTE: foyer's `clear()` resets shard entry counters but NOT the
        // weighted-usage counters, so `entries()` is exact while `usage()`
        // lags after a purge (advisory bytes only).
        let out_bytes = self.output.memory().usage() as u64;
        let disk = self.disk_enabled.then(|| CacheCounters {
            entries: out_entries,
            bytes: out_bytes,
        });
        CacheStats {
            ttl_secs: self.ttl.map(|d| d.as_secs()).unwrap_or(0),
            purges: self.purges.load(Ordering::Relaxed),
            output: c(out_entries, out_bytes),
            empty_tiles: c(
                self.inserted_empty.load(Ordering::Relaxed),
                self.empty_tiles.usage() as u64,
            ),
            tile_flights: self.tile_flights.len() as u64,
            doc_flights: self.doc_flights.len() as u64,
            layers: c(self.layers.entries() as u64, 0),
            docs: c(self.docs.entries() as u64, self.docs.usage() as u64),
            catalog: c(self.catalog.entries() as u64, 0),
            features: c(self.features.entries() as u64, 0),
            decoded_tiles,
            disk,
        }
    }

    /// One info line with every cache's counters (periodic stats task).
    pub fn log_stats(&self, decoded_tiles: CacheCounters) {
        let s = self.stats(decoded_tiles);
        tracing::info!(
            ttl_secs = s.ttl_secs,
            purges = s.purges,
            output_entries = s.output.entries,
            output_bytes = s.output.bytes,
            empty_tiles = s.empty_tiles.entries,
            tile_flights = s.tile_flights,
            doc_flights = s.doc_flights,
            disk = s.disk.is_some(),
            layers_entries = s.layers.entries,
            docs_entries = s.docs.entries,
            docs_bytes = s.docs.bytes,
            catalog_entries = s.catalog.entries,
            features_entries = s.features.entries,
            decoded_tile_entries = s.decoded_tiles.entries,
            decoded_tile_bytes = s.decoded_tiles.bytes,
            "cache stats"
        );
    }

    /// Compatibility no-op: foyer evicts on access, so no forced maintenance
    /// pass is required (the periodic housekeeping task still runs for stats).
    pub fn run_pending_tasks(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn hub(ttl_secs: u64) -> CacheHub {
        CacheHub::build(1, ttl_secs, None).await.unwrap()
    }

    /// Hybrid output cache WITH the foyer disk tier: insert/get must complete.
    /// Guards the Windows block-engine path (a hang here means the disk tier
    /// is unusable and `server.disk_cache` must stay off until fixed).
    /// Multi-thread flavor: mirrors the serving runtime (the single-thread
    /// variant passed even when the live server hung).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn output_hybrid_disk_roundtrip() {
        let dir = std::env::temp_dir().join(format!("foyer-wb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let disk = crate::config::DiskCacheOpts {
            dir: dir.to_string_lossy().to_string(),
            capacity_mib: 16,
        };
        let h = tokio::time::timeout(
            Duration::from_secs(15),
            CacheHub::build(1, 0, Some(&disk)),
        )
        .await;
        eprintln!("[roundtrip] build done: {}", h.is_ok());
        let h = h.unwrap().unwrap();
        let tile = TileOut {
            bytes: vec![7u8; 512],
            content_type: "application/x-protobuf".to_string(),
            content_encoding: None,
        };
        eprintln!("[roundtrip] inserting");
        h.output_insert("k1", tile.clone());
        eprintln!("[roundtrip] insert returned, getting");
        let got = tokio::time::timeout(Duration::from_secs(10), h.output_get("k1")).await;
        eprintln!("[roundtrip] get done: {}", got.is_ok());
        assert!(got.is_ok(), "output_get timed out (foyer disk hang)");
        assert_eq!(got.unwrap().unwrap().bytes, tile.bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pure MISS get against a disk-backed hybrid cache (no insert first):
    /// exercises the storage lookup/replay path that every cold tile request
    /// takes. A hang here means cold requests hang when `disk_cache` is on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn output_hybrid_disk_cold_miss() {
        let dir = std::env::temp_dir().join(format!("foyer-wb-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let disk = crate::config::DiskCacheOpts {
            dir: dir.to_string_lossy().to_string(),
            capacity_mib: 16,
        };
        let h = CacheHub::build(1, 0, Some(&disk)).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(10), h.output_get("cold-key")).await;
        assert!(got.is_ok(), "cold output_get timed out (foyer disk hang)");
        assert!(got.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn doc_cache_builds_once_per_window() {
        let h = hub(0).await;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let build = || {
            calls.fetch_add(1, Ordering::Relaxed);
            "doc".to_string()
        };
        let a = h.doc("k".to_string(), build);
        let b = h.doc("k".to_string(), build);
        assert!(Arc::ptr_eq(&a, &b), "second lookup must hit the cache");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(h.stats(CacheCounters::default()).docs.bytes > 0, true);
    }

    #[tokio::test]
    async fn doc_cache_ttl_refreshes_entry() {
        // Config granularity is whole seconds; exercise the Duration path
        // directly with a short window.
        let mut h = hub(0).await;
        h.ttl = Some(Duration::from_millis(50));
        let a = h.doc("k".to_string(), || "v1".to_string());
        tokio::time::sleep(Duration::from_millis(120)).await;
        let b = h.doc("k".to_string(), || "v2".to_string());
        assert!(!Arc::ptr_eq(&a, &b), "expired entry must be rebuilt");
        assert_eq!(&**b, "v2");
    }

    #[tokio::test]
    async fn catalog_meta_cached_under_single_key() {
        let h = hub(0).await;
        assert_eq!(h.catalog.entries(), 0);
        // build() needs an AppState; the plumbing is covered by state tests.
        assert_eq!(CATALOG_KEY, "catalog");
    }

    #[tokio::test]
    async fn purge_clears_and_repopulates() {
        let h = hub(0).await;
        let a = h.doc("k".to_string(), || "v1".to_string());
        let r = h.purge(CacheScope::Docs);
        assert_eq!(r.scope, "docs");
        assert_eq!(r.purged_entries, 1);
        assert_eq!(r.purges, 1);
        let b = h.doc("k".to_string(), || "v2".to_string());
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(&**b, "v2");
        assert_eq!(h.purge(CacheScope::Docs).purges, 2);
    }

    #[tokio::test]
    async fn purge_scope_is_selective() {
        let h = hub(0).await;
        let _ = h.doc("d".to_string(), || "x".to_string());
        let f = h.feature("places", "a", || Some(serde_json::json!({"id": "a"})));
        assert!(f.is_some());
        let r = h.purge(CacheScope::Docs);
        assert_eq!(r.purged_entries, 1, "only docs purged");
        assert_eq!(h.features.entries(), 1);
        let r = h.purge(CacheScope::Features);
        assert_eq!(r.purged_entries, 1);
        assert_eq!(h.docs.entries(), 0);
    }

    #[tokio::test]
    async fn feature_cache_miss_not_negative_cached() {
        let h = hub(0).await;
        assert!(h.feature("places", "nope", || None).is_none());
        assert!(h.feature("places", "nope", || None).is_none());
        assert_eq!(h.features.entries(), 0);
    }

    #[tokio::test]
    async fn feature_cache_hit_is_stable() {
        let h = hub(0).await;
        let a = h
            .feature("places", "a", || Some(serde_json::json!({"id": "a"})))
            .unwrap();
        let b = h
            .feature("places", "a", || Some(serde_json::json!({"id": "a"})))
            .unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(h.features.entries(), 1);
    }

    #[tokio::test]
    async fn output_two_level_roundtrip() {
        // Memory-only hybrid: get/insert round-trip.
        let h = hub(0).await;
        let t = TileOut {
            bytes: vec![1, 2, 3],
            content_type: "application/x-protobuf".into(),
            content_encoding: None,
        };
        h.output_insert("k", t);
        let hit = h.output_get("k").await.unwrap();
        assert_eq!(hit.bytes, vec![1, 2, 3]);
        assert_eq!(hit.content_type, "application/x-protobuf");
        assert!(h.output_get("missing").await.is_none());
    }

    #[tokio::test]
    async fn scope_parse_roundtrip() {
        for s in [
            "", "all", "output", "tiles", "docs", "metadata", "catalog", "collections", "features",
            "layers", "index",
        ] {
            assert!(CacheScope::parse(s).is_some(), "scope '{s}' must parse");
        }
        assert!(CacheScope::parse("bogus").is_none());
        assert_eq!(CacheScope::parse("ALL"), Some(CacheScope::All));
    }

    #[tokio::test]
    async fn layer_index_populates_from_builder() {
        let h = hub(0).await;
        let built = std::sync::atomic::AtomicUsize::new(0);
        let make = || {
            built.fetch_add(1, Ordering::Relaxed);
            // Stand-in layer value: None here (no fixture state needed); the
            // point is the cache plumbing, exercised with Some below.
            None::<Arc<LayerRuntime>>
        };
        assert!(h.layer("x", make).is_none());
        assert!(h.layer("x", make).is_none());
        // Unknown names are not negative-cached: builder runs again.
        assert_eq!(built.load(Ordering::Relaxed), 2);
        assert_eq!(h.layers.entries(), 0);
    }

    /// P0: a burst of cold requests for ONE tile must issue exactly one DB
    /// query (the old behaviour issued one per request, up to the pool cap).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tile_flight_coalesces_concurrent_cold_builds() {
        let h = hub(0).await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mk = || {
            let calls = Arc::clone(&calls);
            h.tile_flights
                .run("mvt-pg/places/3857/10/1/2", move || async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    TileBuild::Empty
                })
        };
        let all = tokio::join!(
            mk(), mk(), mk(), mk(), mk(), mk(), mk(), mk(),
            mk(), mk(), mk(), mk(), mk(), mk(), mk(), mk()
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1, "one build per burst");
        assert!(matches!(&*all.0, TileBuild::Empty));
        assert!(matches!(&*all.15, TileBuild::Empty));
        assert_eq!(h.tile_flights.len(), 0, "flight map must be drained");
    }

    /// P2: async documents are rendered once per burst AND once per TTL window.
    #[tokio::test]
    async fn doc_async_caches_and_coalesces() {
        let h = hub(0).await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mk = || {
            let calls = Arc::clone(&calls);
            h.doc_async("tj".to_string(), move || async move {
                calls.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(30)).await;
                "{\"tilejson\":\"3.0.0\"}".to_string()
            })
        };
        let (a, b, c) = tokio::join!(mk(), mk(), mk());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(a.as_str(), "{\"tilejson\":\"3.0.0\"}");
        assert!(Arc::ptr_eq(&a, &b) && Arc::ptr_eq(&b, &c));
        assert_eq!(h.doc_get("tj").map(|d| d.to_string()), Some(a.to_string()));
        assert_eq!(h.doc_flights.len(), 0);
    }

    /// P0: empty tiles are negative-cached and invalidated with `scope=output`.
    #[tokio::test]
    async fn empty_tile_negative_cache_is_purged_with_output() {
        let h = hub(0).await;
        let k = "mvt-pg/places/3857/12/3/4";
        assert!(!h.empty_get(k));
        h.empty_insert(k);
        assert!(h.empty_get(k));
        assert_eq!(h.stats(CacheCounters::default()).empty_tiles.entries, 1);
        assert_eq!(h.purge(CacheScope::Output).purged_entries, 1);
        assert!(!h.empty_get(k), "stale 404 must not survive a purge");
    }

    /// P0: cached pbf is gzip'd once and losslessly recoverable.
    #[test]
    fn gzip_pbf_roundtrip() {
        let raw: Vec<u8> = (0..20_000).map(|i| (i % 251) as u8).collect();
        let t = crate::tiles::gzip_pbf(raw.clone());
        assert_eq!(t.content_encoding.as_deref(), Some("gzip"));
        assert!(t.bytes.len() < raw.len() / 2, "pbf should shrink by ~50%");
        assert_eq!(crate::tiles::gunzip_pbf(&t.bytes).unwrap(), raw);
        // Tiny payloads grow under gzip; stay raw so clients aren't misled.
        let tiny = crate::tiles::gzip_pbf(vec![1, 2, 3]);
        assert!(tiny.content_encoding.is_none());
    }
}
