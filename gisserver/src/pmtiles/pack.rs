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
        "generator": "rust-gisserver pmtiles-pack",
        "format": if tile_type == super::tile_type::MVT { "pbf" } else { "png" }
    });
    let metadata_json = serde_json::to_vec(&metadata).map_err(|e| format!("metadata: {e}"))?;
    let metadata_gz = super::codec::gzip(&metadata_json);

    let header_len: u64 = 127;
    let root_off = header_len;
    let metadata_off = root_off + root_dir.len() as u64;
    let tile_off = metadata_off + metadata_gz.len() as u64;

    // Geographic extent of the packed tiles. The spec's header bounds are
    // always lon/lat degrees (e7) regardless of the tile type, and clients use
    // them for the initial view — leaving them zero makes a valid archive look
    // like it covers null island.
    let mut box4 = [180.0_f64, 90.0_f64, -180.0_f64, -90.0_f64];
    for ((z, x, y), _) in &sorted {
        let (minx, miny, maxx, maxy) = tile_lonlat(*z, *x, *y);
        box4[0] = box4[0].min(minx);
        box4[1] = box4[1].min(miny);
        box4[2] = box4[2].max(maxx);
        box4[3] = box4[3].max(maxy);
    }
    let e7 = |v: f64| (v * 1e7).round() as i32;

    let min_zoom = sorted.iter().map(|((z, _, _), _)| *z).min().unwrap_or(0);
    let max_zoom = sorted.iter().map(|((z, _, _), _)| *z).max().unwrap_or(0);

    let header = Header {
        root_dir_offset: root_off,
        root_dir_length: root_dir.len() as u64,
        metadata_offset: metadata_off,
        metadata_length: metadata_gz.len() as u64,
        // An empty leaf section still starts where the metadata ends: the
        // spec orders the sections header/root/metadata/leaf/data, and strict
        // third-party readers verify that adjacency.
        leaf_dirs_offset: tile_off,
        leaf_dirs_length: 0,
        tile_data_offset: tile_off,
        tile_data_length: tile_data.len() as u64,
        num_addressed_tiles: entries.iter().map(|e| e.run_length).sum::<u64>(),
        num_tile_entries: entries.len() as u64,
        // Distinct stored payloads, not entry count: deduplicated tiles share
        // one blob, and the spec uses this field to report that reuse.
        num_tile_contents: {
            let mut offs: Vec<u64> = entries.iter().map(|e| e.offset).collect();
            offs.sort_unstable();
            offs.dedup();
            offs.len() as u64
        },
        clustered: 1,
        internal_compression: super::compression::GZIP,
        tile_compression,
        tile_type,
        min_zoom: min_zoom as u8,
        max_zoom: max_zoom as u8,
        min_lon_e7: e7(box4[0]),
        min_lat_e7: e7(box4[1]),
        max_lon_e7: e7(box4[2]),
        max_lat_e7: e7(box4[3]),
        center_zoom: ((min_zoom + max_zoom) / 2) as u8,
        center_lon_e7: e7((box4[0] + box4[2]) / 2.0),
        center_lat_e7: e7((box4[1] + box4[3]) / 2.0),
    };

    let mut out = Vec::with_capacity(127 + root_dir.len() + metadata_gz.len() + tile_data.len());
    out.extend_from_slice(&write_header(&header));
    out.extend_from_slice(&root_dir);
    out.extend_from_slice(&metadata_gz);
    out.extend_from_slice(&tile_data);
    std::fs::write(path, &out).map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(())
}

/// `(z, x, y)` -> `(min_lon, min_lat, max_lon, max_lat)` in degrees.
///
/// PMTiles header bounds are always WGS84 degrees, and the TileID Hilbert
/// scheme it uses is the Web Mercator quad, so this is the inverse Mercator
/// tile formula rather than a CRS-parameterised transform.
fn tile_lonlat(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z as i32);
    let lon = |k: f64| k / n * 360.0 - 180.0;
    let lat = |k: f64| {
        (std::f64::consts::PI * (1.0 - 2.0 * k / n)).sinh().atan().to_degrees()
    };
    (
        lon(x as f64),
        lat(y as f64 + 1.0),
        lon(x as f64 + 1.0),
        lat(y as f64),
    )
}
