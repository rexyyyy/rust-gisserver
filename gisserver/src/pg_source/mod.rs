//! PostGIS data source for rust-gisserver.
//!
//! This crate is the **only** new code needed to serve a PostGIS table
//! through rust-gisserver: it owns the connection pool, the SQL templates
//! (built from the documented `ST_AsMVT` / `ST_AsMVTGeom` semantics),
//! and the OGC-Filter → SQL-WHERE translation used to push WFS/OGC API
//! filters down to the database.
//!
//! Runtime enablement is config-driven: [`PgRegistry`](crate::pg_source::PgRegistry) is
//! built by the service **only** when at least one layer declares
//! `source.type: postgis`. With no such layer, no pool is ever created and
//! none of the PostGIS code paths run.

pub mod filter;
pub mod grid;
pub mod pool;
pub mod sql;

pub use pool::{PgPool, PgPoolError};

// PgSourceDef / SimplifyOpts live in wms_common (config struct, zero runtime deps)
// so that the `postgis` feature stays truly optional; re-exported here to keep
// existing `pg_source::PgSourceDef` call sites unchanged.
pub use crate::wms_common::pg_source_def::{PgSourceDef, SimplifyOpts, DEFAULT_EXTENT, DEFAULT_BUFFER};

use std::collections::BTreeMap;
use std::sync::Arc;

/// A pool plus every source that uses it.
struct PoolEntry {
    pool: Arc<PgPool>,
    /// `(layer name, def)` pairs served by this pool.
    sources: Vec<(String, PgSourceDef)>,
}

/// Registry of PostGIS pools, built once at startup from the configured
/// layers. Present (`Some`) only when at least one layer uses
/// `source.type: postgis` — that is the single switch that enables all
/// PostGIS functionality.
#[derive(Default)]
pub struct PgRegistry {
    entries: Vec<PoolEntry>,
}

impl PgRegistry {
    /// Build a registry from `(layer name, def)` pairs. `max_size` is the
    /// per-pool connection cap. Returns `None` when the list is empty
    /// (nothing configured → feature disabled).
    pub async fn build(sources: Vec<(String, PgSourceDef)>, max_size: usize) -> Result<Option<Arc<PgRegistry>>, PgPoolError> {
        if sources.is_empty() {
            return Ok(None);
        }
        // Group by DSN so one pool is shared by all layers on a database.
        let mut by_dsn: BTreeMap<String, Vec<(String, PgSourceDef)>> = BTreeMap::new();
        for (name, def) in sources {
            by_dsn.entry(def.pool_key()).or_default().push((name, def));
        }
        let mut entries = Vec::with_capacity(by_dsn.len());
        for (dsn, group) in by_dsn {
            let pool = PgPool::new(&dsn, max_size).await?;
            entries.push(PoolEntry {
                pool: Arc::new(pool),
                sources: group,
            });
        }
        Ok(Some(Arc::new(PgRegistry { entries })))
    }

    /// Look up the pool + def for a layer name.
    pub fn find(&self, layer_name: &str) -> Option<(Arc<PgPool>, &PgSourceDef)> {
        for e in &self.entries {
            for (name, def) in &e.sources {
                if name == layer_name {
                    return Some((Arc::clone(&e.pool), def));
                }
            }
        }
        None
    }

    /// All configured `(layer, def, pool)` triples (capabilities documents).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PgSourceDef, &PgPool)> {
        self.entries.iter().flat_map(|e| {
            e.sources
                .iter()
                .map(move |(name, def)| (name.as_str(), def, e.pool.as_ref()))
        })
    }

    /// Number of configured PostGIS layers.
    pub fn len(&self) -> usize {
        self.entries.iter().map(|e| e.sources.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
