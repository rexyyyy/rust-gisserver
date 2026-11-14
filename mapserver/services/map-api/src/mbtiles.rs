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
use gisserver::wms_common::Bbox;

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
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='tiles')",
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 生成唯一临时路径
    fn tmp_path(prefix: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}_{ts}.mbtiles"))
    }

    /// 创建 MBTiles metadata 表和测试行
    fn write_metadata(conn: &Connection, fmt: &str) {
        let sql = format!(
            "CREATE TABLE metadata (name TEXT, value TEXT);
             INSERT INTO metadata (name, value) VALUES
               ('name',    'test'),
               ('format',  '{fmt}'),
               ('bounds',  '-180,-85,180,85'),
               ('minzoom', '0'),
               ('maxzoom', '3');"
        );
        conn.execute_batch(&sql).unwrap();
    }

    /// 创建 tiles 表（单表形式，回归已有行为）
    fn create_single_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE tiles (
                 zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER,
                 tile_data BLOB NOT NULL);
             CREATE UNIQUE INDEX idx_tiles_zc ON tiles(zoom_level, tile_column, tile_row);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data) VALUES (?,?,?,?)",
            params![0i32, 0i32, 0i32, b"FAKE"],
        )
        .unwrap();
    }

    /// 创建双表稀疏存储 + tiles VIEW（WeMapGIS 等工具产物）
    fn create_view(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE images (
                 tile_id INTEGER PRIMARY KEY, tile_data BLOB NOT NULL);
             CREATE TABLE map (
                 zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_id INTEGER,
                 PRIMARY KEY(zoom_level, tile_column, tile_row));
             CREATE VIEW tiles AS
                 SELECT m.zoom_level, m.tile_column, m.tile_row, i.tile_data
                 FROM map m JOIN images i ON m.tile_id = i.tile_id;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images (tile_id, tile_data) VALUES (1, ?1)",
            params![b"FAKE"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO map (zoom_level, tile_column, tile_row, tile_id) VALUES (?,?,?,?)",
            params![0i32, 0i32, 0i32, 1i64],
        )
        .unwrap();
    }

    /// 没有 tiles 表也没有 tiles view → 应报 "no 'tiles' table"
    fn create_invalid(conn: &Connection) {
        write_metadata(conn, "png");
        // 故意不建 tiles 相关对象
    }

    // ────────────────────────────────────────────────────────────────────────
    // 核心修复回归：view 形式 tiles 表必须能识别
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn opens_mbtiles_with_view_tiles_schema() {
        let path = tmp_path("view");
        {
            let conn = Connection::open(&path).unwrap();
            write_metadata(&conn, "jpg");
            create_view(&conn);
        }
        let mb = MbtilesMeta::open(path.to_str().unwrap()).expect("view schema must open");
        assert_eq!(mb.format(), "jpg");
        assert_eq!(mb.content_type(), "image/jpeg");
        // 验证 tiles 查询也能通过 view 执行
        let tile = mb.read_tile(0, 0, 0).expect("tile read must succeed").expect("has tile");
        assert_eq!(tile.as_slice(), b"FAKE");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opens_mbtiles_with_single_table_schema() {
        let path = tmp_path("single");
        {
            let conn = Connection::open(&path).unwrap();
            write_metadata(&conn, "png");
            create_single_table(&conn);
        }
        let mb = MbtilesMeta::open(path.to_str().unwrap()).expect("single-table schema must open");
        assert_eq!(mb.format(), "png");
        assert_eq!(mb.content_type(), "image/png");
        let tile = mb.read_tile(0, 0, 0).expect("tile read must succeed").expect("has tile");
        assert_eq!(tile.as_slice(), b"FAKE");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_missing_tiles_object() {
        let path = tmp_path("nope");
        {
            let conn = Connection::open(&path).unwrap();
            create_invalid(&conn);
        }
        let err = MbtilesMeta::open(path.to_str().unwrap()).expect_err("missing tiles must fail");
        assert!(
            err.to_string().contains("no 'tiles' table"),
            "unexpected error: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn view_and_single_table_are_both_found_in_sqlite_master() {
        // 验证我们的 SQL 去掉 type 过滤后两种形式都被识别
        let check_sql = "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='tiles')";

        let p_view = tmp_path("sv");
        let p_single = tmp_path("st");

        // view
        {
            let c = Connection::open(&p_view).unwrap();
            create_view(&c);
            let ok: bool = c.query_row(check_sql, [], |r| r.get(0)).unwrap();
            assert!(ok, "view: sqlite_master 必须列出 tiles view");
        }
        // table
        {
            let c = Connection::open(&p_single).unwrap();
            create_single_table(&c);
            let ok: bool = c.query_row(check_sql, [], |r| r.get(0)).unwrap();
            assert!(ok, "single: sqlite_master 必须列出 tiles table");
        }

        std::fs::remove_file(&p_view).ok();
        std::fs::remove_file(&p_single).ok();
    }

    // ────────────────────────────────────────────────────────────────────────
    // format() / content_type() 工具方法
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn format_maps_to_correct_content_type() {
        for (fmt, ct) in [
            ("png", "image/png"),
            ("jpg", "image/jpeg"),
            ("jpeg", "image/jpeg"),
            ("webp", "image/webp"),
            ("pbf", "application/x-protobuf"),
            ("protobuf", "application/x-protobuf"),
            ("mvt", "application/x-protobuf"),
            ("", "image/png"), // 空 → 默认 png
            ("unknown", "image/png"), // 未知 → 回退 png
        ] {
            let path = tmp_path(fmt);
            {
                let conn = Connection::open(&path).unwrap();
                write_metadata(&conn, fmt);
                create_single_table(&conn);
            }
            let mb = MbtilesMeta::open(path.to_str().unwrap()).unwrap();
            assert_eq!(mb.content_type(), ct, "fmt={fmt}");
            std::fs::remove_file(&path).ok();
        }
    }

    // ────────────────────────────────────────────────────────────────────────
    // tile_out gzip 检测
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn tile_out_detects_gzip_for_vector() {
        let path = tmp_path("gz");
        {
            let conn = Connection::open(&path).unwrap();
            write_metadata(&conn, "pbf");
            create_single_table(&conn);
        }
        let mb = MbtilesMeta::open(path.to_str().unwrap()).unwrap();
        // 伪造 gzip magic bytes
        let gz = vec![0x1f, 0x8b, 0x00, 0x00, b'x', b'y', b'z'];
        let out = tile_out(&mb, gz);
        assert_eq!(out.content_type, "application/x-protobuf");
        assert_eq!(out.content_encoding.as_deref(), Some("gzip"));

        // 非 gzip pbf 不带 encoding
        let nogz = vec![0x50, 0x42, 0x46]; // "PBF"
        let out2 = tile_out(&mb, nogz);
        assert_eq!(out2.content_encoding, None);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tile_out_never_claims_gzip_for_raster() {
        let path = tmp_path("ngz");
        {
            let conn = Connection::open(&path).unwrap();
            write_metadata(&conn, "png");
            create_single_table(&conn);
        }
        let mb = MbtilesMeta::open(path.to_str().unwrap()).unwrap();
        // 即使 png tile 意外以 gzip magic 开头（极罕见），也不能标 gzip
        let fake = vec![0x1f, 0x8b, 0x00, 0x00, 0x89, 0x50, 0x4E, 0x47];
        let out = tile_out(&mb, fake);
        assert_eq!(out.content_type, "image/png");
        assert!(out.content_encoding.is_none(), "raster tiles must not be marked gzip");

        std::fs::remove_file(&path).ok();
    }
}
