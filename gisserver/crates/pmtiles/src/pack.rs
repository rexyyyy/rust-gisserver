//! Minimal PMTiles packer: packs `(z, x, y) -> bytes` tiles into a clustered
//! v3 archive with a single root directory (no leaf directories). Used by
//! tests and by the `map-api pmtiles-pack` CLI.

use super::codec::{serialize_directory, write_header};
use super::{zxy_to_tileid, Entry, Header, PmResult};
use std::path::Path;

/// Pack tiles into a `.pmtiles` archive. `tiles` is a slice of
/// `((z, x, y), bytes)`; the archive is clustered (tiles stored in TileID
/// order), `internal_compression = gzip`, `tile_compression` as given.
pub fn pack_tiles(
    path: &Path,
    tiles: &[((u32, u32, u32), Vec<u8>)],
    tile_type: u8,
    tile_compression: u8,
    name: &str,
) -> PmResult<()> {
    if tiles.is_empty() {
        return Err("pack: no tiles".into());
    }

    // Sort by TileID (Hilbert order) so the archive is clustered.
    let mut sorted: Vec<((u32, u32, u32), Vec<u8>)> = tiles.to_vec();
    sorted.sort_by_key(|(zxy, _)| zxy_to_tileid(zxy.0, zxy.1, zxy.2));

    // Tile blobs: dedupe consecutive identical payloads via run-length entries.
    let mut tile_data: Vec<u8> = Vec::new();
    let mut entries: Vec<Entry> = Vec::with_capacity(sorted.len());
    let mut offsets: std::collections::HashMap<Vec<u8>, u64> = std::collections::HashMap::new();

    for ((z, x, y), blob) in &sorted {
        let id = zxy_to_tileid(*z, *x, *y);
        let (offset, length) = match offsets.get(blob) {
            Some(&o) => (o, blob.len() as u64),
            None => {
                let o = tile_data.len() as u64;
                tile_data.extend_from_slice(blob);
                offsets.insert(blob.clone(), o);
                (o, blob.len() as u64)
            }
        };
        entries.push(Entry {
            tile_id: id,
            offset,
            length,
            run_length: 1,
        });
    }

    // Compress the tile payloads if requested: re-encode each unique payload
    // and remap entry offsets/lengths.
    if tile_compression == super::compression::GZIP {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        // Recover unique payloads by old offset.
        let mut by_old: std::collections::BTreeMap<u64, Vec<u8>> = std::collections::BTreeMap::new();
        for e in &entries {
            by_old.entry(e.offset).or_insert_with(|| {
                tile_data[e.offset as usize..(e.offset + e.length) as usize].to_vec()
            });
        }
        let mut remap: std::collections::HashMap<u64, (u64, u64)> = std::collections::HashMap::new();
        let mut final_tile_data = Vec::new();
        for (old, payload) in &by_old {
            let mut enc = GzEncoder::new(Vec::new(), Compression::default());
            enc.write_all(payload).map_err(|e| format!("gzip tile: {e}"))?;
            let gz = enc.finish().map_err(|e| format!("gzip finish: {e}"))?;
            let new = final_tile_data.len() as u64;
            final_tile_data.extend_from_slice(&gz);
            remap.insert(*old, (new, gz.len() as u64));
        }
        for e in entries.iter_mut() {
            let (new_off, new_len) = remap[&e.offset];
            e.offset = new_off;
            e.length = new_len;
        }
        tile_data = final_tile_data;
    }

    // Layout: header(127) | root dir (gzip) | metadata (gzip) | tile data.
    let root_dir = serialize_directory(&entries);
    let metadata = serde_json::json!({
        "name": name,
        "generator": "r-mapserver pmtiles-pack",
        "format": if tile_type == super::tile_type::MVT { "pbf" } else { "png" }
    });
    let metadata_json = serde_json::to_vec(&metadata).map_err(|e| format!("metadata: {e}"))?;
    let metadata_gz = super::codec::gzip(&metadata_json);

    let header_len: u64 = 127;
    let root_off = header_len;
    let metadata_off = root_off + root_dir.len() as u64;
    let tile_off = metadata_off + metadata_gz.len() as u64;

    let header = Header {
        root_dir_offset: root_off,
        root_dir_length: root_dir.len() as u64,
        metadata_offset: metadata_off,
        metadata_length: metadata_gz.len() as u64,
        leaf_dirs_offset: 0,
        leaf_dirs_length: 0,
        tile_data_offset: tile_off,
        tile_data_length: tile_data.len() as u64,
        num_addressed_tiles: entries.len() as u64,
        num_tile_entries: entries.len() as u64,
        num_tile_contents: entries.iter().map(|e| e.run_length).sum::<u64>(),
        clustered: 1,
        internal_compression: super::compression::GZIP,
        tile_compression,
        tile_type,
        min_zoom: sorted.iter().map(|((z, _, _), _)| *z).min().unwrap_or(0) as u8,
        max_zoom: sorted.iter().map(|((z, _, _), _)| *z).max().unwrap_or(0) as u8,
        min_lon_e7: 0,
        min_lat_e7: 0,
        max_lon_e7: 0,
        max_lat_e7: 0,
        center_zoom: 0,
        center_lon_e7: 0,
        center_lat_e7: 0,
    };

    let mut out = Vec::with_capacity(127 + root_dir.len() + metadata_gz.len() + tile_data.len());
    out.extend_from_slice(&write_header(&header));
    out.extend_from_slice(&root_dir);
    out.extend_from_slice(&metadata_gz);
    out.extend_from_slice(&tile_data);
    std::fs::write(path, &out).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(())
}
