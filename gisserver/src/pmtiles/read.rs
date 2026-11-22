//! PMTiles v3 archive reader.
//!
//! Layout recap (PMTiles v3 public spec): a fixed 127-byte header at offset 0
//! is followed by four sections in order — root directory, metadata (a gzipped
//! JSON document), leaf directories, and tile data. Directories are lists of
//! entries; an entry either covers a run of consecutive TileIDs (`run_length >
//! 0`) or points into the leaf-directory section (`run_length == 0`).
//!
//! The reader opens the file once, keeps the header and root directory in
//! memory, and serves every later access through positioned reads on the
//! shared file descriptor. Concurrent lookups therefore never fight over a
//! seek cursor and each caller only ever observes the bytes it asked for.

use super::codec::{deserialize_directory_in, gunzip, read_header};
use super::{zxy_to_tileid, Entry, Header, PmResult};
use std::fs::File;
use std::path::Path;

/// Size of the fixed v3 header, in bytes.
const HEADER_BYTES: u64 = 127;

/// A byte range inside the archive, checked before any allocation happens.
struct Range {
    offset: u64,
    length: u64,
}

impl Range {
    /// Reject ranges that fall outside `[0, file_len)` or overflow `u64`.
    /// Offsets and lengths come from the archive itself, which is untrusted
    /// input: a hostile header must not be able to trigger a huge allocation.
    fn checked(offset: u64, length: u64, file_len: u64) -> PmResult<Range> {
        let end = offset.checked_add(length).ok_or_else(|| {
            format!("pmtiles: section at {offset} with length {length} overflows u64")
        })?;
        if end > file_len {
            return Err(format!(
                "pmtiles: section [{offset}, {end}) extends past the end of the {file_len}-byte archive"
            ));
        }
        Ok(Range { offset, length })
    }

    fn read_within(&self, file: &File) -> PmResult<Vec<u8>> {
        let mut buf = vec![0u8; self.length as usize];
        crate::storage::read_exact_at(file, &mut buf, self.offset)
            .map_err(|e| format!("pmtiles: positioned read at {} failed: {e}", self.offset))?;
        Ok(buf)
    }
}

/// Cached view of one PMTiles archive on disk.
pub struct PmtilesReader {
    file: File,
    file_len: u64,
    header: Header,
    root: Vec<Entry>,
    metadata: String,
}

impl PmtilesReader {
    /// Open `path` and validate + cache the header, root directory and
    /// metadata JSON. Tile bytes themselves are read lazily.
    pub fn open(path: &Path) -> PmResult<PmtilesReader> {
        let file = File::open(path).map_err(|e| format!("pmtiles: cannot open {path:?}: {e}"))?;
        let file_len = file
            .metadata()
            .map_err(|e| format!("pmtiles: cannot stat {path:?}: {e}"))?
            .len();
        if file_len < HEADER_BYTES {
            return Err(format!(
                "pmtiles: {path:?} is {file_len} bytes, shorter than the {HEADER_BYTES}-byte v3 header"
            ));
        }

        let head = Range::checked(0, HEADER_BYTES, file_len)?.read_within(&file)?;
        let header = read_header(&head)?;

        let root = deserialize_directory_in(
            &Range::checked(header.root_dir_offset, header.root_dir_length, file_len)?
                .read_within(&file)?,
            header.internal_compression,
        )?;

        let metadata = if header.metadata_length == 0 {
            String::new()
        } else {
            let raw = Range::checked(header.metadata_offset, header.metadata_length, file_len)?
                .read_within(&file)?;
            let inflated = gunzip(&raw).map_err(|e| format!("pmtiles: metadata gunzip: {e}"))?;
            String::from_utf8(inflated)
                .map_err(|e| format!("pmtiles: metadata is not valid UTF-8: {e}"))?
        };

        Ok(PmtilesReader {
            metadata,
            root,
            header,
            file_len,
            file,
        })
    }

    /// The decoded 127-byte v3 header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The raw metadata JSON document (may be empty).
    pub fn metadata(&self) -> &str {
        &self.metadata
    }

    /// The metadata JSON parsed into a `serde_json::Value`, if it parses.
    pub fn metadata_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.metadata).ok()
    }

    /// Declared tile content type (see [`crate::pmtiles::tile_type`]).
    pub fn tile_type(&self) -> u8 {
        self.header.tile_type
    }

    /// Declared tile payload compression (see [`crate::pmtiles::compression`]).
    pub fn tile_compression(&self) -> u8 {
        self.header.tile_compression
    }

    /// Raw bytes of the tile-data blob one entry points at. The blob is
    /// returned as stored — tile compression is *not* removed here.
    pub fn entry_blob(&self, e: &Entry) -> PmResult<Vec<u8>> {
        let offset = self
            .header
            .tile_data_offset
            .checked_add(e.offset)
            .ok_or_else(|| "pmtiles: tile offset overflows the archive".to_string())?;
        Range::checked(offset, e.length, self.file_len)?.read_within(&self.file)
    }

    /// Fetch one tile by (z,x,y). `Ok(None)` means the archive simply does
    /// not contain that tile.
    pub fn get(&self, z: u32, x: u32, y: u32) -> PmResult<Option<Vec<u8>>> {
        let id = zxy_to_tileid(z, x, y);
        let entry = match locate(&self.root, id) {
            Some(e) => e,
            None => return Ok(None),
        };
        if entry.run_length > 0 {
            return self.entry_blob(&entry).map(Some);
        }
        // `run_length == 0` is a leaf-directory pointer. PMTiles v3 defines
        // exactly one indirection level: root -> leaf -> tile.
        let leaf_base = self.header.leaf_dirs_offset.checked_add(entry.offset).ok_or_else(
            || "pmtiles: leaf directory offset overflows the archive".to_string(),
        )?;
        let leaf = deserialize_directory_in(
            &Range::checked(leaf_base, entry.length, self.file_len)?.read_within(&self.file)?,
            self.header.internal_compression,
        )?;
        match locate(&leaf, id) {
            Some(leaf_entry) => self.entry_blob(&leaf_entry).map(Some),
            None => Ok(None),
        }
    }
}

/// Find the directory entry that covers `id`, i.e. the last entry with
/// `tile_id <= id` whose run `[tile_id, tile_id + max(run_length, 1))`
/// contains it. Entries must be sorted by `tile_id`, as the format requires.
fn locate(dir: &[Entry], id: u64) -> Option<Entry> {
    // Index of the first entry strictly greater than `id`; the candidate is
    // the entry just before it.
    let idx = dir.partition_point(|e| e.tile_id <= id);
    if idx == 0 {
        return None;
    }
    let e = dir[idx - 1];
    // A leaf pointer (`run_length == 0`) covers exactly one TileID.
    let run = e.run_length.max(1);
    if id < e.tile_id + run {
        Some(e)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pmtiles::{compression, pack, tile_type};
    use std::sync::Arc;

    /// Pack three z=1 tiles, reopen the archive and read each one back; also
    /// confirm header accessors and that missing tiles resolve to `None`.
    #[test]
    fn pack_then_read_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("roundtrip.pmtiles");
        let tiles = vec![
            ((1u32, 0u32, 0u32), b"alpha".to_vec()),
            ((1u32, 0u32, 1u32), b"beta".to_vec()),
            ((1u32, 1u32, 1u32), b"gamma".to_vec()),
        ];
        pack::pack_tiles(&path, &tiles, tile_type::MVT, compression::NONE, "spec").unwrap();

        let reader = PmtilesReader::open(&path).unwrap();
        assert_eq!(reader.tile_type(), tile_type::MVT);
        assert_eq!(reader.tile_compression(), compression::NONE);
        assert!(reader.metadata_json().is_some());

        for ((z, x, y), body) in &tiles {
            assert_eq!(reader.get(*z, *x, *y).unwrap().unwrap(), body.as_slice());
        }
        // Present at z=1, absent one level down and off the Hilbert curve.
        assert!(reader.get(1, 1, 0).unwrap().is_none());
        assert!(reader.get(2, 0, 0).unwrap().is_none());
    }

    /// One descriptor, many threads: every concurrent `get` must observe its
    /// own tile's bytes. Positioned reads (seek_read / read_at) make this
    /// deterministic, unlike a shared seek cursor guarded by a Mutex.
    #[test]
    fn threads_never_cross_contaminate_tiles() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("threads.pmtiles");
        let tiles: Vec<((u32, u32, u32), Vec<u8>)> = (0..16u32)
            .map(|i| {
                let fill = (0x20 + i) as u8; // distinct printable byte per tile
                ((2u32, i % 4, i / 4), vec![fill; 96])
            })
            .collect();
        pack::pack_tiles(&path, &tiles, tile_type::MVT, compression::NONE, "t").unwrap();

        let reader = Arc::new(PmtilesReader::open(&path).unwrap());
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let reader = Arc::clone(&reader);
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        for i in 0..16u32 {
                            let want = vec![(0x20 + i) as u8; 96];
                            let got = reader.get(2, i % 4, i / 4).unwrap().unwrap();
                            assert_eq!(got, want, "thread read tile {} bytes", i);
                        }
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().expect("reader thread panicked");
        }
    }
}
