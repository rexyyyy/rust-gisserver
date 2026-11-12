//! PMTiles v3 directory + header codec: columnar ULEB128 varints with the
//! offset sentinel, gzip via flate2, and the fixed 127-byte header.

use super::{Entry, Header, PmResult};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read, Write};

pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(bytes).expect("gzip write");
    enc.finish().expect("gzip finish")
}

/// A ~1 KB crafted gzip blob can decompress to many GB -- a "gzip bomb".
/// 256 MiB is far above any legitimate directory or tile.
const MAX_GUNZIP_OUT: u64 = 256 * 1024 * 1024;

pub(crate) fn gunzip(bytes: &[u8]) -> PmResult<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .take(MAX_GUNZIP_OUT + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("gunzip: {e}"))?;
    if out.len() as u64 > MAX_GUNZIP_OUT {
        return Err(format!("gunzip: decompressed size exceeds cap {MAX_GUNZIP_OUT}"));
    }
    Ok(out)
}

pub(crate) fn write_uvarint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

pub(crate) fn read_uvarint(bytes: &[u8], pos: &mut usize) -> PmResult<u64> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let b = *bytes.get(*pos).ok_or("varint: unexpected end")?;
        *pos += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("varint: overflow".into());
        }
    }
}

/// Serialize a directory: sort by tile_id, write 5 columns of varints
/// (tile_id delta-encoded, offset sentinel-encoded), then gzip.
pub fn serialize_directory(entries: &[Entry]) -> Vec<u8> {
    let mut es = entries.to_vec();
    es.sort_by_key(|e| e.tile_id);
    let mut raw = Vec::new();
    write_uvarint(&mut raw, es.len() as u64);
    let mut last_id = 0u64;
    for e in &es {
        write_uvarint(&mut raw, e.tile_id - last_id);
        last_id = e.tile_id;
    }
    for e in &es {
        write_uvarint(&mut raw, e.run_length);
    }
    for e in &es {
        write_uvarint(&mut raw, e.length);
    }
    for (i, e) in es.iter().enumerate() {
        if i > 0 && e.offset == es[i - 1].offset + es[i - 1].length {
            write_uvarint(&mut raw, 0);
        } else {
            write_uvarint(&mut raw, e.offset + 1);
        }
    }
    gzip(&raw)
}

/// Inverse of [`serialize_directory`] (input is gzip'd).
pub fn deserialize_directory(bytes: &[u8]) -> PmResult<Vec<Entry>> {
    let raw = gunzip(bytes)?;
    let mut pos = 0usize;
    let n = read_uvarint(&raw, &mut pos)? as usize;
    if n.saturating_mul(4) > raw.len().saturating_sub(pos) {
        return Err(format!("directory: entry count {n} exceeds available bytes"));
    }
    let mut es = vec![
        Entry {
            tile_id: 0,
            offset: 0,
            length: 0,
            run_length: 0,
        };
        n
    ];
    let mut last = 0u64;
    for e in es.iter_mut() {
        last += read_uvarint(&raw, &mut pos)?;
        e.tile_id = last;
    }
    for e in es.iter_mut() {
        e.run_length = read_uvarint(&raw, &mut pos)?;
    }
    for e in es.iter_mut() {
        e.length = read_uvarint(&raw, &mut pos)?;
    }
    for i in 0..n {
        let v = read_uvarint(&raw, &mut pos)?;
        es[i].offset = if v == 0 {
            if i == 0 {
                return Err("directory: offset sentinel 0 at first entry".into());
            }
            es[i - 1].offset + es[i - 1].length
        } else {
            v - 1
        };
    }
    Ok(es)
}

/// The fixed 127-byte header, little-endian.
pub fn write_header(h: &Header) -> [u8; 127] {
    let mut b = [0u8; 127];
    b[0..7].copy_from_slice(b"PMTiles");
    b[7] = 3;
    let mut u = |off: usize, v: u64| b[off..off + 8].copy_from_slice(&v.to_le_bytes());
    u(8, h.root_dir_offset);
    u(16, h.root_dir_length);
    u(24, h.metadata_offset);
    u(32, h.metadata_length);
    u(40, h.leaf_dirs_offset);
    u(48, h.leaf_dirs_length);
    u(56, h.tile_data_offset);
    u(64, h.tile_data_length);
    u(72, h.num_addressed_tiles);
    u(80, h.num_tile_entries);
    u(88, h.num_tile_contents);
    b[96] = h.clustered;
    b[97] = h.internal_compression;
    b[98] = h.tile_compression;
    b[99] = h.tile_type;
    b[100] = h.min_zoom;
    b[101] = h.max_zoom;
    let mut i = |off: usize, v: i32| b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    i(102, h.min_lon_e7);
    i(106, h.min_lat_e7);
    i(110, h.max_lon_e7);
    i(114, h.max_lat_e7);
    i(119, h.center_lon_e7);
    i(123, h.center_lat_e7);
    b[118] = h.center_zoom;
    b
}

/// Decode a 127-byte header. Errors on bad magic/version/length.
pub fn read_header(bytes: &[u8]) -> PmResult<Header> {
    if bytes.len() < 127 {
        return Err("header: too short".into());
    }
    if &bytes[0..7] != b"PMTiles" {
        return Err("header: bad magic".into());
    }
    if bytes[7] != 3 {
        return Err(format!("header: unsupported version {}", bytes[7]));
    }
    let u = |off: usize| u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
    let i = |off: usize| i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    Ok(Header {
        root_dir_offset: u(8),
        root_dir_length: u(16),
        metadata_offset: u(24),
        metadata_length: u(32),
        leaf_dirs_offset: u(40),
        leaf_dirs_length: u(48),
        tile_data_offset: u(56),
        tile_data_length: u(64),
        num_addressed_tiles: u(72),
        num_tile_entries: u(80),
        num_tile_contents: u(88),
        clustered: bytes[96],
        internal_compression: bytes[97],
        tile_compression: bytes[98],
        tile_type: bytes[99],
        min_zoom: bytes[100],
        max_zoom: bytes[101],
        min_lon_e7: i(102),
        min_lat_e7: i(106),
        max_lon_e7: i(110),
        max_lat_e7: i(114),
        center_zoom: bytes[118],
        center_lon_e7: i(119),
        center_lat_e7: i(123),
    })
}
