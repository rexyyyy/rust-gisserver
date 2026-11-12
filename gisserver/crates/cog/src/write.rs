//! Minimal COG writer (tiled, DEFLATE, predictor=2, RGB) used by tests and by
//! the `map-api cog-create` CLI to produce fixture data. This is a *writer* for
//! fixtures only -- the server itself only ever *reads* COGs.

use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::Compression;

/// Layout of the COG to generate.
#[derive(Clone, Debug)]
pub struct CogWriteOptions {
    pub width: u32,
    pub height: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    /// Geo corner of the upper-left pixel.
    pub origin_x: f64,
    pub origin_y: f64,
    pub px: f64,
    pub py: f64,
}

/// Fill function: `(col, row) -> RGB`.
pub type PixelFn = dyn Fn(u32, u32) -> [u8; 3] + Sync;

const T_IMAGE_WIDTH: u16 = 256;
const T_IMAGE_LENGTH: u16 = 257;
const T_BITS_PER_SAMPLE: u16 = 258;
const T_COMPRESSION: u16 = 259;
const T_PHOTOMETRIC: u16 = 262;
const T_PLANAR_CONFIG: u16 = 284;
const T_SAMPLES_PER_PIXEL: u16 = 277;
const T_PREDICTOR: u16 = 317;
const T_TILE_WIDTH: u16 = 322;
const T_TILE_LENGTH: u16 = 323;
const T_TILE_OFFSETS: u16 = 324;
const T_TILE_BYTECOUNTS: u16 = 325;
const T_MODEL_PIXEL_SCALE: u16 = 33550;
const T_MODEL_TIEPOINT: u16 = 33922;

const TYP_SHORT: u16 = 3;
const TYP_LONG: u16 = 4;
const TYP_DOUBLE: u16 = 12;

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_f64(b: &mut Vec<u8>, v: f64) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Apply the TIFF horizontal predictor (predictor=2) to one RGB tile row.
fn predict_row(row: &mut [u8]) {
    for col in (1..row.len() / 3).rev() {
        for s in 0..3 {
            let idx = col * 3 + s;
            row[idx] = row[idx].wrapping_sub(row[idx - 3]);
        }
    }
}

/// Build a tiled RGB DEFLATE COG in memory.
pub fn write_cog(opts: &CogWriteOptions, pixel: &PixelFn) -> Vec<u8> {
    let w = opts.width;
    let h = opts.height;
    let tw = opts.tile_w;
    let th = opts.tile_h;
    let tac = (w + tw - 1) / tw;
    let tdc = (h + th - 1) / th;
    let ntiles = (tac * tdc) as usize;

    // --- Tile payloads (predictor applied, deflated) ----------------------
    let mut tile_blobs: Vec<Vec<u8>> = Vec::with_capacity(ntiles);
    for ty in 0..tdc {
        for tx in 0..tac {
            let mut raw = Vec::with_capacity((tw * th * 3) as usize);
            for row in 0..th {
                let gy = ty * th + row;
                if gy >= h {
                    raw.extend(std::iter::repeat(0u8).take((tw * 3) as usize));
                    continue;
                }
                let mut line = Vec::with_capacity((tw * 3) as usize);
                for col in 0..tw {
                    let gx = tx * tw + col;
                    let rgb = if gx < w {
                        pixel(gx, gy)
                    } else {
                        [0, 0, 0]
                    };
                    line.extend_from_slice(&rgb);
                }
                predict_row(&mut line);
                raw.extend_from_slice(&line);
            }
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&raw).unwrap();
            tile_blobs.push(enc.finish().unwrap());
        }
    }

    // --- Layout -----------------------------------------------------------
    let n_entries: u16 = 14;
    let ifd_size = 2 + n_entries as u64 * 12 + 4;
    let data_start = 8 + ifd_size;
    let mut off = data_start;
    let mut tile_offsets = Vec::with_capacity(ntiles);
    let mut tile_bytecounts = Vec::with_capacity(ntiles);
    for b in &tile_blobs {
        tile_offsets.push(off);
        tile_bytecounts.push(b.len() as u64);
        off += b.len() as u64;
    }
    let offsets_off = off;
    off += (ntiles as u64) * 4;
    let bytecounts_off = off;
    off += (ntiles as u64) * 4;
    let bps_off = off;
    off += 6;
    let scale_off = off;
    off += 24;
    let tiepoint_off = off;
    off += 48;
    let total = off;

    // --- Assemble ---------------------------------------------------------
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"II");
    put_u16(&mut out, 42);
    put_u32(&mut out, 8); // first IFD

    put_u16(&mut out, n_entries);

    // (tag, type, count, value-or-offset)
    let entry = |b: &mut Vec<u8>, tag: u16, typ: u16, count: u32, val: u32| {
        put_u16(b, tag);
        put_u16(b, typ);
        put_u32(b, count);
        put_u32(b, val);
    };

    entry(&mut out, T_IMAGE_WIDTH, TYP_LONG, 1, w);
    entry(&mut out, T_IMAGE_LENGTH, TYP_LONG, 1, h);
    entry(&mut out, T_BITS_PER_SAMPLE, TYP_SHORT, 3, bps_off as u32);
    entry(&mut out, T_COMPRESSION, TYP_SHORT, 1, 8); // DEFLATE
    entry(&mut out, T_PHOTOMETRIC, TYP_SHORT, 1, 2); // RGB
    entry(&mut out, T_PLANAR_CONFIG, TYP_SHORT, 1, 1); // chunky
    entry(&mut out, T_SAMPLES_PER_PIXEL, TYP_SHORT, 1, 3);
    entry(&mut out, T_PREDICTOR, TYP_SHORT, 1, 2); // horizontal
    entry(&mut out, T_TILE_WIDTH, TYP_LONG, 1, tw);
    entry(&mut out, T_TILE_LENGTH, TYP_LONG, 1, th);
    entry(&mut out, T_TILE_OFFSETS, TYP_LONG, ntiles as u32, offsets_off as u32);
    entry(&mut out, T_TILE_BYTECOUNTS, TYP_LONG, ntiles as u32, bytecounts_off as u32);
    entry(&mut out, T_MODEL_PIXEL_SCALE, TYP_DOUBLE, 3, scale_off as u32);
    entry(&mut out, T_MODEL_TIEPOINT, TYP_DOUBLE, 6, tiepoint_off as u32);
    put_u32(&mut out, 0); // next IFD

    for b in &tile_blobs {
        out.extend_from_slice(b);
    }
    for &o in &tile_offsets {
        put_u32(&mut out, o as u32);
    }
    for &c in &tile_bytecounts {
        put_u32(&mut out, c as u32);
    }
    put_u16(&mut out, 8);
    put_u16(&mut out, 8);
    put_u16(&mut out, 8);
    put_f64(&mut out, opts.px);
    put_f64(&mut out, opts.py);
    put_f64(&mut out, 0.0);
    // ModelTiepoint: (I, J, K, X, Y, Z) -- raster origin (0,0) maps to the
    // upper-left geo corner.
    put_f64(&mut out, 0.0);
    put_f64(&mut out, 0.0);
    put_f64(&mut out, 0.0);
    put_f64(&mut out, opts.origin_x);
    put_f64(&mut out, opts.origin_y);
    put_f64(&mut out, 0.0);

    debug_assert_eq!(out.len() as u64, total);
    out
}

/// A smooth RGB gradient COG over `[minx, maxy]` .. `[maxx, miny]`, useful for
/// smoke tests (each channel varies across the image, so warping/overview
/// errors are visible).
pub fn gradient_cog(
    width: u32,
    height: u32,
    tile: u32,
    minx: f64,
    maxy: f64,
    maxx: f64,
    miny: f64,
) -> Vec<u8> {
    let opts = CogWriteOptions {
        width,
        height,
        tile_w: tile,
        tile_h: tile,
        origin_x: minx,
        origin_y: maxy,
        px: (maxx - minx) / width as f64,
        py: (maxy - miny) / height as f64,
    };
    write_cog(&opts, &move |col, row| {
        let r = ((col as f64 / width as f64) * 255.0) as u8;
        let g = ((row as f64 / height as f64) * 255.0) as u8;
        let b = (((col + row) as f64 / (width + height) as f64) * 255.0) as u8;
        [r, g, b]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cog;
    use storage::source::LocalFileRangeSource;

    #[test]
    fn writer_produces_readable_cog() {
        let bytes = gradient_cog(64, 48, 16, 100.0, 50.0, 104.0, 46.0);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("g.tif");
        std::fs::write(&p, &bytes).unwrap();
        let src = LocalFileRangeSource::open(p.to_str().unwrap()).unwrap();
        let cog = Cog::parse(&src).unwrap();
        assert_eq!(cog.native_width(), 64);
        assert_eq!(cog.native_height(), 48);
        let lvl = &cog.levels[0];
        assert_eq!(lvl.tile_w, 16);
        assert_eq!(lvl.tiles_across(), 4);
        assert_eq!(lvl.tiles_down(), 3);
        assert!((lvl.geo.origin_x - 100.0).abs() < 1e-9);
        assert!((lvl.geo.origin_y - 50.0).abs() < 1e-9);
        assert!((lvl.geo.px - 4.0 / 64.0).abs() < 1e-12);

        // Decode the (0,0) tile and spot-check a pixel.
        let tile = crate::read_tile_rgba(lvl, &src, 0, 0, None).unwrap();
        assert_eq!(tile.len(), 16 * 16 * 4);
        // Pixel (0,0): r=0, g=0, b=0.
        assert_eq!(&tile[0..4], &[0, 0, 0, 255]);
        // Pixel (15,15): r=~59, g=~79, b=~63 (approx; uses integer division by
        // total width+height).
        assert_eq!(tile[(15 * 16 + 15) * 4], ((15u32 * 255 / 64) as u8));
    }
}
