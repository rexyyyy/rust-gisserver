//! Tile codecs (ported from TerraServe's `decode.rs`).
//!
//! DEFLATE (+ horizontal predictor) is the required path; LZW and ZSTD are
//! supported as bonus codecs. Output is always a 4-channel RGBA tile buffer
//! (alpha = the source alpha band, or 255 when the source has no alpha).

use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::io::Read;

use super::Level;

/// Decode one tile to a `tile_w x tile_h x 4` RGBA buffer.
///
/// Supported TIFF compressions:
/// * `1`     -- none (raw samples)
/// * `5`     -- LZW
/// * `8`/`32946` -- DEFLATE (zlib)
/// * `50000` -- ZSTD
///
/// JPEG (6/7) and WEBP (50001) tiles are not decoded by this port and yield a
/// zero-filled tile; see the README's "unsupported" list.
pub fn decode_tile_rgba(bytes: &[u8], level: &Level) -> Vec<u8> {
    let tw = level.tile_w as usize;
    let th = level.tile_h as usize;
    let spp = level.samples_per_pixel as usize;
    let one_bit = level.bits_per_sample == 1;
    let row_bytes = if one_bit {
        (tw * spp + 7) / 8
    } else {
        tw * spp * bytes_per_sample(level.bits_per_sample)
    };
    let expected = row_bytes * th;

    let raw: Vec<u8> = match level.compression {
        8 | 32946 => inflate(bytes, expected),
        5 => lzw_decode(bytes, expected),
        50000 => zstd_decompress(bytes, expected),
        1 => bytes.to_vec(),
        // JPEG / WEBP / unknown: zero-fill rather than misinterpret.
        _ => vec![0u8; expected],
    };

    let mut samples = raw;
    // DEFLATE / LZW / ZSTD / raw use the TIFF predictor.
    if matches!(level.compression, 8 | 32946 | 5 | 1 | 50000) {
        if level.predictor == 2 && !one_bit {
            match bytes_per_sample(level.bits_per_sample) {
                1 => unpredict_horizontal(&mut samples, tw, th, spp),
                2 => unpredict_horizontal_16(&mut samples, tw, th, spp, level.little_endian),
                _ => {}
            }
        } else if level.predictor == 3 {
            unpredict_float(
                &mut samples,
                tw,
                th,
                spp,
                bytes_per_sample(level.bits_per_sample),
                level.little_endian,
            );
        }
    }

    // Expand 1-bit bilevel samples to one byte per pixel (TIFF is MSB-first).
    if one_bit {
        let mut expanded = vec![0u8; tw * th * spp];
        for row in 0..th {
            for i in 0..tw * spp {
                let bit_index = row * row_bytes * 8 + i;
                let byte = samples.get(bit_index / 8).copied().unwrap_or(0);
                let set = (byte >> (7 - (i % 8))) & 1;
                expanded[row * tw * spp + i] = if set == 1 { 255 } else { 0 };
            }
        }
        samples = expanded;
    }

    // 16-bit integer and float samples are scaled to 8-bit for display.
    samples = scale_to_u8(samples, level, tw * th * spp);

    to_rgba(&samples, tw, th, spp)
}

/// Bytes per sample for `bits_per_sample` (1 for 8-bit, 2 for 16-bit).
pub fn bytes_per_sample(bits_per_sample: u16) -> usize {
    (bits_per_sample as usize + 7) / 8
}

/// Scale 16-bit / float samples to 8-bit. 8-bit passes through unchanged.
/// 16-bit unsigned: `>> 8`; 16-bit signed: remap to unsigned then `>> 8`;
/// float: clamp [0, 1] to [0, 255] (values outside clamp to the ends).
fn scale_to_u8(samples: Vec<u8>, level: &Level, n: usize) -> Vec<u8> {
    let bps = bytes_per_sample(level.bits_per_sample);
    if bps == 1 {
        return samples;
    }
    let little = level.little_endian;
    let signed = level.sample_format == 2;
    let float = level.sample_format == 3;
    let mut out = vec![0u8; n];

    // 16-bit unsigned: use the high byte by default, but if the data only
    // occupies the low byte ("fake 16-bit" rasters, e.g. values 0..255 with
    // a stray >=256 sample), `>> 8` would render pure black, so take the low
    // byte instead. Decision is per tile by the share of samples whose high
    // byte is non-zero; real 16-bit data (DEM etc.) has most samples >= 256,
    // while 0..255-stored data has at most a few percent of overflow samples.
    // (Two plain `for` loops: the previous `chunks().take().fold()` form
    // stalled for minutes in debug builds on large tiles.)
    let mut ge256 = 0u32;
    for i in 0..n {
        let b = &samples[i * bps..(i + 1) * bps];
        let v = if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        };
        if v >= 256 {
            ge256 += 1;
        }
    }
    let shift = if ge256 as f64 > n as f64 * 0.05 { 8 } else { 0 };

    for i in 0..n {
        let b = &samples[i * bps..(i + 1) * bps];
        let v: u8 = if float {
            let f = if little {
                f32::from_le_bytes([b[0], b[1], b[2], b[3]])
            } else {
                f32::from_be_bytes([b[0], b[1], b[2], b[3]])
            };
            (f.max(0.0).min(1.0) * 255.0) as u8
        } else if signed {
            let v = if little {
                i16::from_le_bytes([b[0], b[1]])
            } else {
                i16::from_be_bytes([b[0], b[1]])
            };
            ((v as f64 / i16::MAX as f64) * 127.5 + 127.5).clamp(0.0, 255.0) as u8
        } else {
            let v = if little {
                u16::from_le_bytes([b[0], b[1]])
            } else {
                u16::from_be_bytes([b[0], b[1]])
            };
            (v >> shift) as u8
        };
        out[i] = v;
    }
    out
}

fn inflate(bytes: &[u8], expected: usize) -> Vec<u8> {
    let mut dec = ZlibDecoder::new(bytes);
    let mut out = Vec::with_capacity(expected);
    if dec.read_to_end(&mut out).is_err() {
        // Fall back to raw DEFLATE (no zlib header) if the zlib wrapper is absent.
        let mut d2 = DeflateDecoder::new(bytes);
        out.clear();
        let _ = d2.read_to_end(&mut out);
    }
    if out.len() < expected {
        out.resize(expected, 0);
    }
    out
}

fn zstd_decompress(bytes: &[u8], expected: usize) -> Vec<u8> {
    let mut out = zstd::stream::decode_all(bytes).unwrap_or_default();
    if out.len() < expected {
        out.resize(expected, 0);
    }
    out
}

/// Decode TIFF LZW (compression 5): MSB-first with the early size switch.
fn lzw_decode(bytes: &[u8], expected: usize) -> Vec<u8> {
    let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
    match dec.decode(bytes) {
        Ok(mut out) => {
            if out.len() < expected {
                out.resize(expected, 0);
            }
            out
        }
        Err(_) => vec![0u8; expected],
    }
}

/// Reverse the TIFF horizontal differencing predictor (predictor = 2), 8-bit
/// samples, `spp` interleaved samples per pixel, per row.
fn unpredict_horizontal(buf: &mut [u8], width: usize, height: usize, spp: usize) {
    let row_len = width * spp;
    for row in 0..height {
        let base = row * row_len;
        for col in 1..width {
            for s in 0..spp {
                let idx = base + col * spp + s;
                let prev = buf[idx - spp];
                buf[idx] = buf[idx].wrapping_add(prev);
            }
        }
    }
}

/// Reverse the 16-bit horizontal predictor (predictor = 2, 16-bit samples).
fn unpredict_horizontal_16(buf: &mut [u8], width: usize, height: usize, spp: usize, little: bool) {
    let row_len = width * spp;
    for row in 0..height {
        let base = row * row_len;
        for col in 1..width {
            for s in 0..spp {
                let idx = (base + col * spp + s) * 2;
                let prev = idx - 2 * spp;
                if little {
                    let a = u16::from_le_bytes([buf[prev], buf[prev + 1]]);
                    let v = u16::from_le_bytes([buf[idx], buf[idx + 1]]);
                    buf[idx..idx + 2].copy_from_slice(&v.wrapping_add(a).to_le_bytes());
                } else {
                    let a = u16::from_be_bytes([buf[prev], buf[prev + 1]]);
                    let v = u16::from_be_bytes([buf[idx], buf[idx + 1]]);
                    buf[idx..idx + 2].copy_from_slice(&v.wrapping_add(a).to_be_bytes());
                }
            }
        }
    }
}

/// Reverse the TIFF floating-point predictor (predictor = 3): undo the
/// byte-wise horizontal differencing, then de-shuffle the (MSB-first) byte
/// planes back into native-endian samples.
fn unpredict_float(buf: &mut [u8], width: usize, height: usize, spp: usize, bps: usize, little: bool) {
    let n = width * spp;
    let row_bytes = n * bps;
    if row_bytes == 0 {
        return;
    }
    for r in 0..height {
        let base = r * row_bytes;
        if base + row_bytes > buf.len() {
            break;
        }
        let row = &mut buf[base..base + row_bytes];
        for i in 1..row_bytes {
            row[i] = row[i].wrapping_add(row[i - 1]);
        }
        // De-shuffle: plane b holds the b-th byte of every sample.
        let mut tmp = vec![0u8; row_bytes];
        for i in 0..n {
            for b in 0..bps {
                tmp[i * bps + if little { b } else { bps - 1 - b }] = row[b * n + i];
            }
        }
        row.copy_from_slice(&tmp);
    }
}

/// Expand interleaved samples to RGBA.
fn to_rgba(samples: &[u8], width: usize, height: usize, spp: usize) -> Vec<u8> {
    let n = width * height;
    let mut out = vec![0u8; n * 4];
    for p in 0..n {
        let si = p * spp;
        let di = p * 4;
        match spp {
            4 => out[di..di + 4].copy_from_slice(&samples[si..si + 4]),
            3 => {
                out[di..di + 3].copy_from_slice(&samples[si..si + 3]);
                out[di + 3] = 255;
            }
            1 => {
                out[di] = samples[si];
                out[di + 1] = samples[si];
                out[di + 2] = samples[si];
                out[di + 3] = 255;
            }
            _ => {
                for k in 0..3 {
                    out[di + k] = if k < spp { samples[si + k] } else { 0 };
                }
                out[di + 3] = 255;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cog::Level;

    fn level(compression: u16, predictor: u16) -> Level {
        Level {
            width: 2,
            height: 2,
            tile_w: 2,
            tile_h: 2,
            compression,
            predictor,
            photometric: 2,
            samples_per_pixel: 3,
            bits_per_sample: 8,
            sample_format: 1,
            extra_samples: 0,
            index: crate::cog::TileIndex {
                offsets: vec![],
                bytecounts: vec![],
            },
            source_id: 0,
            level_index: 0,
            little_endian: true,
            geo: crate::cog::GeoTransform {
                origin_x: 0.0,
                origin_y: 2.0,
                px: 1.0,
                py: 1.0,
            },
        }
    }

    #[test]
    fn raw_rgb_round_trip() {
        let lvl = level(1, 1);
        // 2x2 RGB: r,g,b per pixel.
        let raw: Vec<u8> = vec![
            255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0,
        ];
        let rgba = decode_tile_rgba(&raw, &lvl);
        assert_eq!(rgba.len(), 2 * 2 * 4);
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[12..16], &[255, 255, 0, 255]);
    }

    #[test]
    fn low_byte_16bit_uses_low_byte() {
        // "Fake 16-bit": real values 0..255 stored in the LOW byte of a
        // 16-bit sample. `>> 8` would render all-black; the adaptive scale
        // must take the low byte instead.
        let mut lvl = level(1, 1);
        lvl.bits_per_sample = 16;
        lvl.samples_per_pixel = 3;
        // 2x2 RGB, LE u16: (10, 20, 30), (40, 50, 60), (70, 80, 90), (0, 128, 255)
        let raw: Vec<u8> = vec![
            10, 0, 20, 0, 30, 0, 40, 0, 50, 0, 60, 0,
            70, 0, 80, 0, 90, 0, 0, 0, 128, 0, 255, 0,
        ];
        let rgba = decode_tile_rgba(&raw, &lvl);
        assert_eq!(&rgba[0..4], &[10, 20, 30, 255]);
        assert_eq!(&rgba[12..16], &[0, 128, 255, 255]);
    }

    #[test]
    fn high_byte_16bit_uses_high_byte() {
        // Real 16-bit DEM-like values (e.g. 2000..3000): keep `>> 8`.
        let mut lvl = level(1, 1);
        lvl.bits_per_sample = 16;
        lvl.samples_per_pixel = 1;
        let raw: Vec<u8> = vec![
            0xD0, 0x07, 0xD1, 0x07, 0xD2, 0x07, 0xD3, 0x07, // 2000..2003
        ];
        let rgba = decode_tile_rgba(&raw, &lvl);
        assert_eq!(&rgba[0..4], &[7, 7, 7, 255]); // 2000 >> 8 = 7
        assert_eq!(&rgba[4..8], &[7, 7, 7, 255]);
    }
}
