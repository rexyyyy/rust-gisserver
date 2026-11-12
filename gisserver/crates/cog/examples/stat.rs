//! Diagnostic: dump raw 16-bit sample stats for a COG before/after the
//! horizontal predictor, so real-world decode issues are distinguishable from
//! genuinely low-valued data. Run:
//!   cargo run -p cog --example stat -- <path.tif> [tx] [ty]

use cog::Cog;
use flate2::read::ZlibDecoder;
use std::io::Read;

fn inflate(bytes: &[u8]) -> Vec<u8> {
    let mut dec = ZlibDecoder::new(bytes);
    let mut out = Vec::new();
    if dec.read_to_end(&mut out).is_err() {
        let mut d2 = flate2::read::DeflateDecoder::new(bytes);
        out.clear();
        let _ = d2.read_to_end(&mut out);
    }
    out
}

fn unpredict_u16(buf: &mut [u8], width: usize, height: usize, spp: usize) {
    let row_len = width * spp;
    for row in 0..height {
        let base = row * row_len;
        for col in 1..width {
            for s in 0..spp {
                let idx = (base + col * spp + s) * 2;
                let prev = idx - 2 * spp;
                let a = u16::from_le_bytes([buf[prev], buf[prev + 1]]);
                let v = u16::from_le_bytes([buf[idx], buf[idx + 1]]);
                buf[idx..idx + 2].copy_from_slice(&v.wrapping_add(a).to_le_bytes());
            }
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: stat <path> [tx] [ty]");
    let tx: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let ty: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let source = storage::source_from_uri(&path).expect("open source");
    let cog = Cog::parse(source.as_ref()).expect("parse cog");
    println!(
        "cog: {}x{} levels={} tile={}x{} compression={} predictor={} spp={} bps={} photometric={}",
        cog.native_width(),
        cog.native_height(),
        cog.levels.len(),
        cog.levels[0].tile_w,
        cog.levels[0].tile_h,
        cog.levels[0].compression,
        cog.levels[0].predictor,
        cog.levels[0].samples_per_pixel,
        cog.levels[0].bits_per_sample,
        cog.levels[0].photometric,
    );
    println!("geotransform: origin=({}, {}) px={} py={}", cog.levels[0].geo.origin_x, cog.levels[0].geo.origin_y, cog.levels[0].geo.px, cog.levels[0].geo.py);

    for (li, lvl) in cog.levels.iter().enumerate() {
        if li >= 6 { break; } // keep runtime small; tile-level checks below
        let ta = lvl.tiles_across();
        let td = lvl.tiles_down();
        let (x, y) = if tx < ta && ty < td { (tx, ty) } else { (ta / 2, td / 2) };
        let idx = (y * ta + x) as usize;
        let off = lvl.index.offsets[idx] as u64;
        let len = lvl.index.bytecounts[idx] as usize;
        println!(
            "level {li}: {}x{} tiles_across={} tiles_down={}; tile({x},{y}) offset={off} len={len}",
            lvl.width, lvl.height, ta, td
        );
        let raw = source.read_range(off, len).expect("read tile bytes");
        let decoded = inflate(&raw);
        let expect_bytes = (lvl.tile_w as usize) * (lvl.tile_h as usize) * (lvl.samples_per_pixel as usize) * 2;
        println!("  inflated {} bytes (expect {expect_bytes})", decoded.len());
        // Stats on diff-coded data (before predictor inverse).
        let mut min_d = u16::MAX;
        let mut max_d = 0u16;
        let mut sum_d = 0u64;
        let n = decoded.len() / 2;
        let mut vals = vec![0u16; n];
        for i in 0..n {
            let v = u16::from_le_bytes([decoded[i * 2], decoded[i * 2 + 1]]);
            vals[i] = v;
            min_d = min_d.min(v);
            max_d = max_d.max(v);
            sum_d += v as u64;
        }
        println!("  pre-predictor : min={min_d} max={max_d} mean={}", sum_d / n.max(1) as u64);
        // Inverse predictor -> real sample values.
        let mut buf = decoded.clone();
        unpredict_u16(&mut buf, lvl.tile_w as usize, lvl.tile_h as usize, lvl.samples_per_pixel as usize);
        let mut min = u16::MAX;
        let mut max = 0u16;
        let mut sum = 0u64;
        for i in 0..n {
            let v = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
            min = min.min(v);
            max = max.max(v);
            sum += v as u64;
        }
        println!(
            "  post-predictor: min={min} max={max} mean={}  (8bit band0 first sample: {})",
            sum / n.max(1) as u64,
            (u16::from_le_bytes([buf[0], buf[1]]) >> 8)
        );
    }

    // Timing probes for the server-relevant paths.
    eprintln!("[probe] read_tile_rgba(level0, 9, 6) start");
    let lvl0 = &cog.levels[0];
    let lvl5 = &cog.levels[5];
    let t = std::time::Instant::now();
    let rgba = cog::read_tile_rgba(lvl0, source.as_ref(), 9, 6, None).expect("level0 tile");
    eprintln!("[probe] read_tile_rgba(level0, 9, 6) -> {} bytes in {:?}", rgba.len(), t.elapsed());
    eprintln!("[probe] read_window_rgba(level5 full) start");
    let t = std::time::Instant::now();
    let w = cog::read_window_rgba(
        lvl5,
        source.as_ref(),
        0,
        0,
        lvl5.width - 1,
        lvl5.height - 1,
        None,
    )
    .expect("level5 window");
    eprintln!("[probe] read_window_rgba(level5 full {}x{}) -> {} bytes in {:?}", lvl5.width, lvl5.height, w.len(), t.elapsed());
    for (x, y) in [(0u32, 0u32), (1, 0)] {
        let t = std::time::Instant::now();
        let r = cog::read_tile_rgba(lvl5, source.as_ref(), x, y, None).expect("tile");
        let mut nz = 0usize;
        let (mut mn, mut mx) = (255u8, 0u8);
        for px in 0..(lvl5.tile_w as usize * lvl5.tile_h as usize) {
            let v = r[px * 4];
            if v != 0 { nz += 1; }
            if v < mn { mn = v; }
            if v > mx { mx = v; }
        }
        eprintln!("[probe] level5 tile({x},{y}): nonzero_r={nz} r_min={mn} r_max={mx} in {:?}", t.elapsed());
    }
    eprintln!("[probe] read_window_rgba(level0 full) start");
    let t = std::time::Instant::now();
    let w0 = cog::read_window_rgba(
        lvl0,
        source.as_ref(),
        0,
        0,
        lvl0.width - 1,
        lvl0.height - 1,
        None,
    )
    .expect("level0 window");
    eprintln!("[probe] read_window_rgba(level0 full) -> {} bytes in {:?}", w0.len(), t.elapsed());
}
