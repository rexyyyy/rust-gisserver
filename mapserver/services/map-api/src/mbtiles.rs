//! MBTiles (SQLite tile package) reader.
//!
//! An `.mbtiles` file is a SQLite database with a `metadata` table (name/value
//! key pairs) and a `tiles` table (`zoom_level`, `tile_column`, `tile_row`,
//! `tile_data`). Tiles are stored in the **TMS** row scheme (row 0 at the
//! bottom), whereas our HTTP route uses **XYZ** addressing (row 0 at the top),
//! so the incoming `y` is flipped internally: `tile_row = (2^z - 1) - y`.
//!
//! The SQLite connections are pooled inside `MbtilesMeta` (up to
//! [`POOL_CAP`] read-only connections): per-request open/close of the archive
//! file is avoided, and every checkout/return happens inside
//! `spawn_blocking`, so no `Connection` is ever shared across threads and the
//! blocking SQLite I/O stays off the async runtime. `MbtilesMeta` itself holds
//! only the resolved path, the parsed `metadata` map and the pool (all
//! `Send + Sync`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags};

use crate::tiles::TileOut;
use wms_common::Bbox;

/// Maximum number of read-only connections kept warm per archive.
const POOL_CAP: usize = 8;

/// An opened MBTiles archive: its path, the parsed `metadata` table, and a
/// small pool of read-only connections.
#[derive(Debug)]
pub struct MbtilesMeta {
    pub path: PathBuf,
    pub metadata: BTreeMap<String, String>,
    /// Idle read-only connections. `SQLITE_OPEN_NO_MUTEX` is safe because the
    /// pool guarantees single-threaded use per connection.
    pool: Mutex<Vec<Connection>>,
}

fn open_readonly(path: &Path) -> anyhow::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| anyhow::anyhow!("open mbtiles {}: {e}", path.display()))
}

impl MbtilesMeta {
    /// Open read-only, verify the `tiles` table exists and load `metadata`.
    pub fn open(path: &str) -> anyhow::Result<MbtilesMeta> {
        let p = Path::new(path);
        let conn = open_readonly(p)?;
        let tiles_ok: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tiles')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !tiles_ok {
            return Err(anyhow::anyhow!("mbtiles {}: no 'tiles' table", path));
        }
        let mut metadata = BTreeMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT name, value FROM metadata") {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    metadata.insert(row.0, row.1);
                }
            }
        }
        Ok(MbtilesMeta {
            path: p.to_path_buf(),
            metadata,
            pool: Mutex::new(Vec::new()),
        })
    }

    /// Tile format declared in `metadata` (lowercased: `png`/`jpg`/`webp`/`pbf`).
    pub fn format(&self) -> String {
        self.metadata
            .get("format")
            .cloned()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    /// Content-Type for the declared format (defaults to PNG, the MBTiles spec
    /// default when `format` is absent).
    pub fn content_type(&self) -> &'static str {
        match self.format().as_str() {
            "pbf" | "protobuf" | "mvt" => "application/x-protobuf",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        }
    }

    /// Coverage from the `metadata` table: `bounds` is `west,south,east,north`
    /// (EPSG:4326) per the MBTiles spec; `minzoom`/`maxzoom` clamp the zoom
    /// range. Missing keys yield `None` (callers fall back to layer config).
    pub fn bounds(&self) -> Option<Bbox> {
        let b = self.metadata.get("bounds")?;
        let parts: Vec<f64> = b
            .split(',')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        if parts.len() == 4 && parts[0] < parts[2] && parts[1] < parts[3] {
            Some(Bbox::new(parts[0], parts[1], parts[2], parts[3]))
        } else {
            None
        }
    }

    /// Zoom range from the `metadata` table (`minzoom`/`maxzoom`).
    pub fn zoom_range(&self) -> Option<(u32, u32)> {
        let min = self.metadata.get("minzoom").and_then(|v| v.trim().parse().ok())?;
        let max = self.metadata.get("maxzoom").and_then(|v| v.trim().parse().ok())?;
        Some((min, max))
    }

    /// Read one tile by XYZ address. `None` when the tile is absent. The XYZ
    /// `y` is flipped to the MBTiles TMS row before the query.
    ///
    /// Uses a pooled read-only connection with a cached prepared statement;
    /// the connection returns to the pool on success (a failed query leaves
    /// the connection discarded rather than recycled in a bad state).
    pub fn read_tile(&self, z: u32, x: u32, y_xyz: u32) -> anyhow::Result<Option<Vec<u8>>> {
        // TMS row = (2^z - 1) - y; guard the shift so an out-of-range y just
        // misses (returns None) rather than wrapping.
        let Some(row) = (1u64 << z)
            .checked_sub(1)
            .and_then(|m| m.checked_sub(y_xyz as u64))
        else {
            return Ok(None);
        };
        // Checkout: reuse an idle connection when available, else open one.
        let mut conn = match self.pool.lock() {
            Ok(mut pool) => pool.pop(),
            Err(_) => None,
        };
        let owned = conn.is_none();
        if conn.is_none() {
            conn = Some(open_readonly(&self.path)?);
        }
        let conn = conn.expect("connection present");
        let result = (|| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT tile_data FROM tiles \
                     WHERE zoom_level=?1 AND tile_column=?2 AND tile_row=?3",
                )
                .map_err(|e| anyhow::anyhow!("prepare tile query: {e}"))?;
            match stmt.query_row(
                rusqlite::params![z as i64, x as i64, row as i64],
                |r| r.get::<_, Vec<u8>>(0),
            ) {
                Ok(b) => Ok(Some(b)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow::anyhow!("read tile {z}/{x}/{y_xyz}: {e}")),
            }
        })();
        // Return healthy connections to the pool; drop the rest.
        if result.is_ok() && !owned {
            if let Ok(mut pool) = self.pool.lock() {
                if pool.len() < POOL_CAP {
                    pool.push(conn);
                }
            }
        }
        result
    }

    /// Metadata as JSON (path / format / content_type + the raw metadata map),
    /// so operators can inspect bounds/minzoom/maxzoom without opening the DB.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "file": self.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
            "format": self.format(),
            "content_type": self.content_type(),
            "bounds": self.bounds().map(|b| vec![b.minx, b.miny, b.maxx, b.maxy]),
            "zoom_range": self.zoom_range(),
            "metadata": self.metadata,
        })
    }
}

/// Build a [`TileOut`] for raw MBTiles tile bytes.
///
/// Content-Type comes from the `metadata` `format` key. Vector MBTiles often
/// store gzip-compressed pbf; the payload is passed through with
/// `Content-Encoding: gzip`. The gzip magic is detected rather than trusting
/// metadata so uncompressed archives are not mislabeled.
pub fn tile_out(mb: &MbtilesMeta, bytes: Vec<u8>) -> TileOut {
    let ct = mb.content_type();
    let gzipped = bytes.starts_with(&[0x1f, 0x8b]);
    TileOut {
        bytes,
        content_type: ct.to_string(),
        content_encoding: if gzipped && ct == "application/x-protobuf" {
            Some("gzip".to_string())
        } else {
            None
        },
    }
}
