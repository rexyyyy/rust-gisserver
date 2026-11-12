//! PMTiles v3 reader: opens an archive, caches the header + root directory +
//! metadata, and looks up a tile's bytes by (z,x,y) with positioned reads.

use super::codec::{deserialize_directory, gunzip, read_header};
use super::{zxy_to_tileid, Entry, Header, PmResult};
use std::fs::File;
use std::path::Path;

/// A read-only PMTiles archive. The file handle is shared and read with
/// **positioned** reads (`storage::read_exact_at`: Windows `seek_read`, Unix
/// `read_at`), so concurrent tile lookups never serialize on a cursor and
/// cannot observe each other's data.
pub struct PmtilesReader {
    file: File,
    file_len: u64,
    header: Header,
    root: Vec<Entry>,
    metadata: String,
}

/// Read `len` bytes at `offset`, rejecting any range outside `[0, file_len)`
/// (offset/len come from an untrusted archive; a crafted huge len must not
/// reach an over-large allocation).
fn read_at(file: &File, file_len: u64, offset: u64, len: u64) -> PmResult<Vec<u8>> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| format!("pmtiles: read {offset}+{len} overflows"))?;
    if end > file_len {
        return Err(format!("pmtiles: read {offset}+{len} exceeds file length {file_len}"));
    }
    let mut buf = vec![0u8; len as usize];
    storage::read_exact_at(file, &mut buf, offset).map_err(|e| format!("read: {e}"))?;
    Ok(buf)
}

impl PmtilesReader {
    pub fn open(path: &Path) -> PmResult<PmtilesReader> {
        let f = File::open(path).map_err(|e| format!("open {path:?}: {e}"))?;
        let file_len = f.metadata().map_err(|e| format!("stat {path:?}: {e}"))?.len();
        let head_bytes = read_at(&f, file_len, 0, 127)?;
        let header = read_header(&head_bytes)?;
        let root = deserialize_directory(&read_at(
            &f,
            file_len,
            header.root_dir_offset,
            header.root_dir_length,
        )?)?;
        let metadata = if header.metadata_length > 0 {
            String::from_utf8(gunzip(&read_at(
                &f,
                file_len,
                header.metadata_offset,
                header.metadata_length,
            )?)?)
            .map_err(|e| format!("metadata utf8: {e}"))?
        } else {
            String::new()
        };
        Ok(PmtilesReader {
            file: f,
            file_len,
            header,
            root,
            metadata,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn metadata(&self) -> &str {
        &self.metadata
    }

    /// The metadata JSON as a `serde_json::Value`, when parseable.
    pub fn metadata_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.metadata).ok()
    }

    /// The stored blob one tile entry points at (not decompressed).
    pub fn entry_blob(&self, e: &Entry) -> PmResult<Vec<u8>> {
        let off = self
            .header
            .tile_data_offset
            .checked_add(e.offset)
            .ok_or_else(|| "pmtiles: offset overflow".to_string())?;
        read_at(&self.file, self.file_len, off, e.length)
    }
    /// The archive's declared tile type (MVT / PNG / ...).
    pub fn tile_type(&self) -> u8 {
        self.header.tile_type
    }

    /// The tile compression scheme the archive declares.
    pub fn tile_compression(&self) -> u8 {
        self.header.tile_compression
    }

    /// Look up one tile; returns `Ok(None)` when the archive has no such tile.
    pub fn get(&self, z: u32, x: u32, y: u32) -> PmResult<Option<Vec<u8>>> {
        let tile_id = zxy_to_tileid(z, x, y);
        let entry = self.find_entry(&self.root, tile_id)?;
        match entry {
            None => Ok(None),
            Some(e) if e.run_length == 0 => {
                // Leaf-directory pointer: read the leaf dir, then search it.
                let leaf = deserialize_directory(&read_at(
                    &self.file,
                    self.file_len,
                    self.header.leaf_dirs_offset + e.offset,
                    e.length,
                )?)?;
                match self.find_entry(&leaf, tile_id)? {
                    None => Ok(None),
                    Some(le) => self.entry_blob(&le).map(Some),
                }
            }
            Some(e) => self.entry_blob(&e).map(Some),
        }
    }

    /// Binary-search a sorted directory for the entry covering `tile_id`
    /// (entries cover `[tile_id, tile_id + run_length)`; `run_length == 0`
    /// entries are leaf pointers covering exactly one id).
    fn find_entry(&self, dir: &[Entry], tile_id: u64) -> PmResult<Option<Entry>> {
        let mut lo = 0usize;
        let mut hi = dir.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if dir[mid].tile_id <= tile_id {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return Ok(None);
        }
        let e = dir[lo - 1];
        if tile_id >= e.tile_id && tile_id < e.tile_id + e.run_length.max(1) {
            Ok(Some(e))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack;
    use std::sync::Arc;

    #[test]
    fn round_trips_a_packed_archive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.pmtiles");
        // Three tiles at z=1.
        let tiles = vec![
            ((1u32, 0u32, 0u32), b"tile-a".to_vec()),
            ((1u32, 0u32, 1u32), b"tile-b".to_vec()),
            ((1u32, 1u32, 1u32), b"tile-c".to_vec()),
        ];
        pack::pack_tiles(
            &p,
            &tiles,
            crate::tile_type::MVT,
            crate::compression::NONE,
            "test",
        )
        .unwrap();
        let r = PmtilesReader::open(&p).unwrap();
        assert_eq!(r.tile_type(), crate::tile_type::MVT);
        assert_eq!(r.get(1, 0, 0).unwrap().unwrap(), b"tile-a");
        assert_eq!(r.get(1, 0, 1).unwrap().unwrap(), b"tile-b");
        assert_eq!(r.get(1, 1, 1).unwrap().unwrap(), b"tile-c");
        assert!(r.get(1, 1, 0).unwrap().is_none());
        assert!(r.get(2, 0, 0).unwrap().is_none());
    }

    /// The reader shares one fd across threads with positioned reads; every
    /// concurrent lookup must return exactly its own tile's bytes (the old
    /// Mutex+seek path could interleave and return another tile's data).
    #[test]
    fn concurrent_lookups_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.pmtiles");
        let tiles: Vec<((u32, u32, u32), Vec<u8>)> = (0..16u32)
            .map(|i| ((2u32, i % 4, i / 4), vec![b'a' + i as u8; 64]))
            .collect();
        pack::pack_tiles(&p, &tiles, crate::tile_type::MVT, crate::compression::NONE, "t")
            .unwrap();
        let r = std::sync::Arc::new(PmtilesReader::open(&p).unwrap());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let r = Arc::clone(&r);
            handles.push(std::thread::spawn(move || {
                for _ in 0..200 {
                    for i in 0..16u32 {
                        let got = r.get(2, i % 4, i / 4).unwrap().unwrap();
                        assert_eq!(got, vec![b'a' + i as u8; 64]);
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("worker panicked");
        }
    }
}
