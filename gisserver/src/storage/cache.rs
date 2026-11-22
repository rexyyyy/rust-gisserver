//! Bounded in-memory cache of decoded tiles (RGBA buffers), keyed by a
//! `(source_id, level_idx, tile_x, tile_y)` composite; runs on **foyer**
//! (in-memory cache, no moka) with a
//! hard memory cap and explicit purge (see `map-api`'s `CacheHub`).

use foyer::Cache as FoyerCache;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One decoded tile: RGBA, `tile_w * tile_h * 4` bytes.
pub type Tile = Arc<Vec<u8>>;

/// TTL wrapper (foyer's in-memory cache has no built-in TTL): the insert
/// instant travels with the value; expired entries are dropped on access.
#[derive(Clone)]
struct Timed {
    tile: Tile,
    at_ms: u64,
}

impl Timed {
    fn new(tile: Tile) -> Self {
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Timed { tile, at_ms }
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

/// Cache key: `source_id` distinguishes COGs, `level` is the IFD index,
/// `tx`/`ty` the tile column/row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileKey {
    pub source_id: u64,
    pub level: u32,
    pub tx: u32,
    pub ty: u32,
}

impl TileKey {
    pub fn new(source_id: u64, level: u32, tx: u32, ty: u32) -> Self {
        Self {
            source_id,
            level,
            tx,
            ty,
        }
    }
}

/// Thread-safe decoded-tile cache with a byte budget.
#[derive(Clone)]
pub struct TileCache {
    cache: FoyerCache<TileKey, Timed>,
    /// Approximate bytes per entry used for the cap.
    est_entry_bytes: u64,
    ttl: Option<Duration>,
}

impl TileCache {
    /// `capacity_mib` = hard memory cap (0 disables caching).
    /// `ttl` = per-entry time-to-live (`None` = entries only leave via
    /// capacity eviction). TTL is measured from insert, so a hot tile is
    /// re-decoded at most once per TTL window — deliberate: it bounds
    /// staleness after an underlying dataset is replaced on disk.
    pub fn new(capacity_mib: u64, est_entry_bytes: u64, ttl: Option<Duration>) -> TileCache {
        // Entry budget: foyer weights entries by 1, so the cache holds at
        // most `cap` decoded tiles — derived from `capacity_mib` /
        // `est_entry_bytes` like the moka era.
        let cap = if capacity_mib == 0 {
            1
        } else {
            ((capacity_mib * 1024 * 1024) / est_entry_bytes.max(1)).max(1)
        };
        TileCache {
            cache: foyer::CacheBuilder::new(cap as usize)
                .with_weighter(|_: &TileKey, _: &Timed| 1usize)
                .build(),
            est_entry_bytes: est_entry_bytes.max(1),
            ttl,
        }
    }

    pub fn get(&self, key: TileKey) -> Option<Tile> {
        match self.cache.get(&key) {
            Some(entry) => {
                if entry.expired(self.ttl) {
                    self.cache.remove(&key);
                    None
                } else {
                    Some(entry.tile.clone())
                }
            }
            None => None,
        }
    }

    pub fn insert(&self, key: TileKey, tile: Tile) {
        self.cache.insert(key, Timed::new(tile));
    }

    /// Drop every entry (cache refresh / data replaced on disk).
    pub fn purge(&self) {
        self.cache.clear();
    }

    /// No-op for foyer (eviction happens on access); kept so the periodic
    /// housekeeping call sites stay unchanged.
    pub fn run_pending_tasks(&self) {}

    pub fn estimated_bytes(&self) -> u64 {
        self.cache.usage() as u64 * self.est_entry_bytes
    }

    /// Entry count. foyer's `usage()` (bytes) is accounted asynchronously by
    /// its housekeeper and can lag behind `clear()`, so exact-count callers
    /// must use `entries()` (kept in sync synchronously by insert/remove).
    pub fn entry_count(&self) -> u64 {
        self.cache.entries() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_roundtrip() {
        let c = TileCache::new(1, 256 * 256 * 4, None);
        let k = TileKey::new(1, 0, 3, 4);
        assert!(c.get(k).is_none());
        c.insert(k, Arc::new(vec![0u8; 16]));
        assert!(c.get(k).is_some());
        assert_eq!(c.entry_count(), 1);
        c.purge();
        assert!(c.get(k).is_none());
        assert_eq!(c.entry_count(), 0);
    }

    #[test]
    fn ttl_entries_expire() {
        let c = TileCache::new(1, 1024, Some(Duration::from_millis(50)));
        let k = TileKey::new(1, 0, 0, 0);
        c.insert(k, Arc::new(vec![0u8; 16]));
        assert!(c.get(k).is_some());
        std::thread::sleep(Duration::from_millis(120));
        assert!(c.get(k).is_none(), "entry must be gone after TTL");
    }
}
