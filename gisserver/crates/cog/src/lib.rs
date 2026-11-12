//! Bespoke Cloud-Optimized GeoTIFF reader (ported from TerraServe's `cog.rs`).
//!
//! NO `gdal` / `tiff` / `geotiff` / COG-reader crates: we parse the TIFF and
//! BigTIFF container ourselves -- header, IFD chain, tile offsets/bytecounts,
//! the overview IFDs, and the tags that map pixel->geo (`ModelPixelScaleTag`,
//! `ModelTiepointTag`). Tiles are decoded with codec crates (DEFLATE required;
//! LZW and ZSTD also supported). All byte reads flow through the
//! [`storage::RangeSource`] seam, so a COG can live on disk or behind an
//! `http(s)://` range endpoint (including S3 presigned URLs).

pub mod decode;
pub mod write;

use rayon::prelude::*;
use storage::{RangeSource, TileCache, TileKey};
use wms_common::MapServerError;

/// TIFF tag numbers the reader understands.
pub const T_IMAGE_WIDTH: u16 = 256;
pub const T_IMAGE_LENGTH: u16 = 257;
pub const T_BITS_PER_SAMPLE: u16 = 258;
pub const T_COMPRESSION: u16 = 259;
pub const T_PHOTOMETRIC: u16 = 262;
pub const T_NEW_SUBFILE_TYPE: u16 = 254;
pub const T_PLANAR_CONFIG: u16 = 284;
pub const T_SAMPLES_PER_PIXEL: u16 = 277;
pub const T_PREDICTOR: u16 = 317;
pub const T_TILE_WIDTH: u16 = 322;
pub const T_TILE_LENGTH: u16 = 323;
pub const T_TILE_OFFSETS: u16 = 324;
pub const T_TILE_BYTECOUNTS: u16 = 325;
pub const T_EXTRA_SAMPLES: u16 = 338;
pub const T_SAMPLE_FORMAT: u16 = 339;
pub const T_MODEL_PIXEL_SCALE: u16 = 33550;
pub const T_MODEL_TIEPOINT: u16 = 33922;
/// GeoTIFF GeoKeyDirectoryTag: the SHORT array carrying the source CRS keys.
pub const T_GEO_KEY_DIRECTORY: u16 = 34735;
/// GeoTIFF GeoAsciiParamsTag: `|`-separated citation strings referenced by
/// keys stored out-of-line (e.g. `GeogCitationGeoKey`).
pub const T_GEO_ASCII_PARAMS: u16 = 34737;

/// GeoKeyDirectory key ids used to resolve the source CRS.
pub const GK_MODEL_TYPE: u16 = 1024;
pub const GK_GEOGRAPHIC_TYPE: u16 = 2048;
pub const GK_PROJECTED_CRS_TYPE: u16 = 3072;

/// Geotransform mapping source pixel (col,row) corners to geo (X,Y).
/// Pixel-is-area: the geo of the upper-left *corner* of pixel (0,0) is
/// (origin_x, origin_y); x increases with col, y decreases with row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoTransform {
    pub origin_x: f64,
    pub origin_y: f64,
    pub px: f64, // pixel size in x (> 0)
    pub py: f64, // pixel size in y (> 0, applied downward)
}

impl GeoTransform {
    /// Geo -> continuous pixel coordinate (u across cols, v down rows):
    /// `u = (X - origin_x)/px`; a value in `[c, c+1)` lies in column c.
    #[inline]
    pub fn geo_to_pix(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.origin_x) / self.px, (self.origin_y - y) / self.py)
    }

    /// Pixel corner (col,row) -> geo (X,Y).
    #[inline]
    pub fn pix_to_geo(&self, col: f64, row: f64) -> (f64, f64) {
        (
            self.origin_x + col * self.px,
            self.origin_y - row * self.py,
        )
    }
}

/// A level's tile-offset/bytecount index (resident in memory).
#[derive(Clone, Debug)]
pub struct TileIndex {
    pub offsets: Vec<u64>,
    pub bytecounts: Vec<u64>,
}

/// One image directory: the full-res image or an overview level.
#[derive(Clone, Debug)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub tile_w: u32,
    pub tile_h: u32,
    pub compression: u16,
    pub predictor: u16,
    pub photometric: u16,
    pub samples_per_pixel: u16,
    pub bits_per_sample: u16,
    pub sample_format: u16, // 1 = uint, 2 = int, 3 = float
    pub extra_samples: u16, // 0 = none, 2 = unassociated alpha
    pub index: TileIndex,
    /// Which COG this level belongs to (cache-key component).
    pub source_id: u64,
    /// Position of this level in the level list (cache-key component).
    pub level_index: u32,
    /// Container byte order (for 16/32-bit sample decode).
    pub little_endian: bool,
    pub geo: GeoTransform,
}

impl Level {
    #[inline]
    pub fn tiles_across(&self) -> u32 {
        (self.width + self.tile_w - 1) / self.tile_w
    }

    #[inline]
    pub fn tiles_down(&self) -> u32 {
        (self.height + self.tile_h - 1) / self.tile_h
    }

    #[inline]
    pub fn ntiles(&self) -> usize {
        (self.tiles_across() as usize) * (self.tiles_down() as usize)
    }

    /// Downsample factor relative to a reference (native) width.
    pub fn factor(&self, native_width: u32) -> f64 {
        native_width as f64 / self.width as f64
    }
}

/// Parsed COG: the full-res level plus overviews (index 0 = full res).
pub struct Cog {
    pub levels: Vec<Level>,
    /// Little-endian container byte order (needed for 16/32-bit sample decode).
    pub little_endian: bool,
    /// Source CRS as an EPSG code, read from the GeoKeyDirectory. `None` when
    /// the file carries no geo keys (or the code is user-defined).
    pub srid: Option<u32>,
}

impl Cog {
    pub fn parse(source: &dyn RangeSource) -> Result<Cog, MapServerError> {
        parse_cog(source, 0)
    }

    /// Parse with an explicit `source_id` used in cache keys (so two COGs in
    /// one process never collide in the shared decoded-tile cache).
    pub fn parse_with_id(source: &dyn RangeSource, source_id: u64) -> Result<Cog, MapServerError> {
        parse_cog(source, source_id)
    }

    #[inline]
    pub fn native_width(&self) -> u32 {
        self.levels[0].width
    }

    #[inline]
    pub fn native_height(&self) -> u32 {
        self.levels[0].height
    }
}

// ---------------------------------------------------------------------------
// TIFF / BigTIFF container parsing (bespoke).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Endian {
    Little,
    Big,
}

impl Endian {
    fn u16(self, b: &[u8]) -> u16 {
        if matches!(self, Endian::Little) {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    }

    fn u32(self, b: &[u8]) -> u32 {
        if matches!(self, Endian::Little) {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
    }

    fn u64(self, b: &[u8]) -> u64 {
        if matches!(self, Endian::Little) {
            u64::from_le_bytes(b[0..8].try_into().unwrap())
        } else {
            u64::from_be_bytes(b[0..8].try_into().unwrap())
        }
    }

    fn f64(self, b: &[u8]) -> f64 {
        if matches!(self, Endian::Little) {
            f64::from_le_bytes(b[0..8].try_into().unwrap())
        } else {
            f64::from_be_bytes(b[0..8].try_into().unwrap())
        }
    }
}

/// A raw IFD entry (tag, type, count, value/offset).
struct Entry {
    tag: u16,
    typ: u16,
    count: u64,
    /// The 4- or 8-byte value field, or the offset to out-of-line data.
    value_field: [u8; 8],
    big: bool,
}

impl Entry {
    /// Byte size of one value of `typ`.
    fn type_size(typ: u16) -> Option<u64> {
        Some(match typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 => 8,
            16 | 17 | 18 => 8,
            _ => return None,
        })
    }

    /// Total value bytes.
    fn value_bytes(&self) -> Option<u64> {
        Self::type_size(self.typ)?.checked_mul(self.count)
    }

    fn is_inline(&self) -> bool {
        let inline_cap = if self.big { 8 } else { 4 };
        self.value_bytes().unwrap_or(0) <= inline_cap
    }

    /// Where out-of-line value bytes live: `(offset, byte_len)`.
    /// The offset field follows the container byte order, so it must be decoded
    /// with `endian` (a big-endian TIFF/BigTIFF stores it big-endian).
    fn value_location(&self, endian: Endian) -> (u64, u64) {
        let n = self.value_bytes().unwrap_or(0);
        let off = if self.big {
            endian.u64(&self.value_field[0..8])
        } else {
            endian.u32(&self.value_field[0..4]) as u64
        };
        (off, n)
    }
}

/// Read integer values of an entry as `u64`s (works for BYTE/SHORT/LONG/LONG8
/// and their signed variants; rationals are read separately).
fn entry_uints(
    e: &Entry,
    src: &dyn RangeSource,
    endian: Endian,
) -> Result<Vec<u64>, MapServerError> {
    let n = e.count as usize;
    let cap = if e.big { 8 } else { 4 };
    if e.is_inline() {
        let bytes = &e.value_field[..cap.min(n * e_byte(e.typ).unwrap_or(1) as usize)];
        Ok(decode_uints(e.typ, bytes, endian))
    } else {
        let (off, len) = e.value_location(endian);
        let bytes = src.read_range(off, len as usize).map_err(MapServerError::Io)?;
        Ok(decode_uints(e.typ, &bytes, endian))
    }
}

fn e_byte(typ: u16) -> Option<usize> {
    Entry::type_size(typ).map(|v| v as usize)
}

/// Read an ASCII tag value (TIFF type ASCII = 2), stopping at the first NUL.
fn entry_ascii(
    e: &Entry,
    src: &dyn RangeSource,
    endian: Endian,
) -> Result<String, MapServerError> {
    let raw: Vec<u8> = if e.is_inline() {
        let cap = if e.big { 8 } else { 4 };
        let n = (e.count as usize).min(cap);
        e.value_field[..n].to_vec()
    } else {
        let (off, len) = e.value_location(endian);
        src.read_range(off, len as usize)
            .map_err(MapServerError::Io)?
    };
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).to_string())
}

fn decode_uints(typ: u16, bytes: &[u8], endian: Endian) -> Vec<u64> {
    let sz = e_byte(typ).unwrap_or(1);
    let n = bytes.len() / sz.max(1);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = &bytes[i * sz..(i + 1) * sz];
        let v = match sz {
            1 => b[0] as u64,
            2 => endian.u16(b) as u64,
            4 => endian.u32(b) as u64,
            8 => endian.u64(b),
            _ => 0,
        };
        out.push(v);
    }
    out
}

/// Read floating values (DOUBLE or RATIONAL) of an entry.
fn entry_floats(
    e: &Entry,
    src: &dyn RangeSource,
    endian: Endian,
) -> Result<Vec<f64>, MapServerError> {
    let sz = e_byte(e.typ).unwrap_or(8);
    let n = e.count as usize;
    let cap = if e.big { 8 } else { 4 };
    let raw: Vec<u8> = if e.is_inline() {
        e.value_field[..cap.min(n * sz)].to_vec()
    } else {
        let (off, len) = e.value_location(endian);
        src.read_range(off, len as usize).map_err(MapServerError::Io)?
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let b = &raw[i * sz..(i + 1) * sz];
        let v = match e.typ {
            12 => endian.f64(b),
            5 | 10 => {
                // RATIONAL: num / den (u32 pairs).
                let num = endian.u32(&b[..4]) as f64;
                let den = endian.u32(&b[4..8]) as f64;
                if den == 0.0 {
                    0.0
                } else {
                    num / den
                }
            }
            11 => f32::from_bits(endian.u32(b)) as f64,
            _ => 0.0,
        };
        out.push(v);
    }
    Ok(out)
}

/// The geotransform defaults: 1 unit per pixel, origin (0, 0) -- used when a
/// COG carries no ModelPixelScale/Tiepoint tags.
fn default_geo(_width: u32, height: u32) -> GeoTransform {
    GeoTransform {
        origin_x: 0.0,
        origin_y: height as f64,
        px: 1.0,
        py: 1.0,
    }
}

fn parse_cog(src: &dyn RangeSource, source_id: u64) -> Result<Cog, MapServerError> {
    // TIFF header: endianness + magic + first-IFD offset.
    let head = src.read_range(0, 16).map_err(MapServerError::Io)?;
    let endian = match &head[0..2] {
        b"II" => Endian::Little,
        b"MM" => Endian::Big,
        _ => return Err(MapServerError::Format("not a TIFF (bad byte order)".into())),
    };
    let magic = endian.u16(&head[2..4]);
    let big = match magic {
        42 => false,
        43 => true,
        _ => return Err(MapServerError::Format(format!("bad TIFF magic {magic}"))),
    };
    let first_ifd = if big {
        endian.u64(&head[8..16])
    } else {
        endian.u32(&head[4..8]) as u64
    };

    let mut levels: Vec<Level> = Vec::new();
    let mut srid: Option<u32> = None;
    let mut next = first_ifd;
    let mut visited = 0usize;
    while next != 0 {
        if visited > 64 {
            return Err(MapServerError::Format("IFD chain too deep".into()));
        }
        visited += 1;
        let (entries, next_ifd) = parse_ifd(src, endian, big, next)?;
        next = next_ifd;

        // Internal transparency-mask IFDs (NewSubfileType bit 2) are skipped.
        let subfile = entries
            .iter()
            .find(|e| e.tag == T_NEW_SUBFILE_TYPE)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(0);
        if subfile & 2 != 0 {
            continue;
        }

        let get = |tag: u16| -> Option<&Entry> { entries.iter().find(|e| e.tag == tag) };
        let width = get(T_IMAGE_WIDTH)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .ok_or_else(|| MapServerError::Format("missing ImageWidth".into()))? as u32;
        let height = get(T_IMAGE_LENGTH)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .ok_or_else(|| MapServerError::Format("missing ImageLength".into()))? as u32;
        let tile_w = get(T_TILE_WIDTH)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(width as u64) as u32;
        let tile_h = get(T_TILE_LENGTH)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(height as u64) as u32;
        let compression = get(T_COMPRESSION)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(1) as u16;
        let predictor = get(T_PREDICTOR)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(1) as u16;
        let photometric = get(T_PHOTOMETRIC)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(2) as u16;
        let spp = get(T_SAMPLES_PER_PIXEL)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(1) as u16;
        let bps = get(T_BITS_PER_SAMPLE)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(8) as u16;
        let sample_format = get(T_SAMPLE_FORMAT)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .and_then(|v| v.first().copied())
            .unwrap_or(1) as u16;
        let extra_samples = get(T_EXTRA_SAMPLES)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .map(|v| v.len() as u16)
            .unwrap_or(0);

        // Tile offsets / bytecounts.
        let offsets = get(T_TILE_OFFSETS)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .ok_or_else(|| MapServerError::Format("missing TileOffsets".into()))?;
        let bytecounts = get(T_TILE_BYTECOUNTS)
            .and_then(|e| entry_uints(e, src, endian).ok())
            .ok_or_else(|| MapServerError::Format("missing TileByteCounts".into()))?;
        if offsets.len() != bytecounts.len() {
            return Err(MapServerError::Format(
                "TileOffsets/TileByteCounts length mismatch".into(),
            ));
        }

        // Geotransform (overviews usually inherit the full-res one).
        let geo = if let (Some(ps), Some(tp)) = (get(T_MODEL_PIXEL_SCALE), get(T_MODEL_TIEPOINT)) {
            let scale = entry_floats(ps, src, endian).unwrap_or_default();
            let tie = entry_floats(tp, src, endian).unwrap_or_default();
            if scale.len() >= 2 && tie.len() >= 6 {
                GeoTransform {
                    origin_x: tie[3],
                    origin_y: tie[4],
                    px: scale[0],
                    py: scale[1].abs().max(f64::MIN_POSITIVE),
                }
            } else {
                default_geo(width, height)
            }
        } else {
            default_geo(width, height)
        };

        // Source CRS metadata: the first IFD carrying a GeoKeyDirectory wins.
        if srid.is_none() {
            if let Some(dir) = get(T_GEO_KEY_DIRECTORY).and_then(|e| entry_uints(e, src, endian).ok())
            {
                let citation =
                    get(T_GEO_ASCII_PARAMS).and_then(|e| entry_ascii(e, src, endian).ok());
                srid = geo_key_epsg_cited(&dir, citation.as_deref());
            }
        }

        levels.push(Level {
            width,
            height,
            tile_w,
            tile_h,
            compression,
            predictor,
            photometric,
            samples_per_pixel: spp,
            bits_per_sample: bps,
            sample_format,
            extra_samples,
            index: TileIndex {
                offsets,
                bytecounts,
            },
            source_id,
            level_index: levels.len() as u32,
            little_endian: matches!(endian, Endian::Little),
            geo,
        });
    }

    if levels.is_empty() {
        return Err(MapServerError::Format("no image IFDs found".into()));
    }
    // Propagate the full-res geotransform to levels lacking one.
    let geo0 = levels[0].geo;
    for lvl in levels.iter_mut().skip(1) {
        if lvl.geo.px == 1.0 && lvl.geo.py == 1.0 && lvl.geo.origin_x == 0.0 {
            lvl.geo = geo0;
        }
    }
    Ok(Cog {
        levels,
        little_endian: matches!(endian, Endian::Little),
        srid,
    })
}

/// Resolve an EPSG code from a GeoKeyDirectory SHORT array.
///
/// Layout: `[version, revision, minorRevision, numKeys, (keyId, location,
/// count, value) * n]`. `location == 0` means the value is inline in the fourth
/// slot (the case for every CRS key we read); anything else points into another
/// tag we don't consult here. Geographic data takes key 2048, projected data
/// key 3072; 0 and 32767 are the "undefined"/"user-defined" sentinels and are
/// reported as unknown.
pub fn geo_key_epsg(dir: &[u64]) -> Option<u32> {
    geo_key_epsg_cited(dir, None)
}

/// Same as [`geo_key_epsg`], but falls back to the `GeoAsciiParams` citation
/// when the file marks its geographic CRS as user-defined (32767).
///
/// Files written by GDAL commonly use 32767 plus `GCS Name = WGS 84|…` instead
/// of the numeric EPSG key; without this fallback a valid WGS 84 COG would be
/// reported as "no source CRS".
pub fn geo_key_epsg_cited(dir: &[u64], citation: Option<&str>) -> Option<u32> {
    if dir.len() < 4 {
        return None;
    }
    let num_keys = dir[3] as usize;
    let mut model_type: Option<u64> = None;
    let mut geographic: Option<u64> = None;
    let mut projected: Option<u64> = None;
    for i in 0..num_keys {
        let base = 4 + i * 4;
        if base + 3 >= dir.len() {
            break;
        }
        if dir[base + 1] != 0 {
            continue; // value stored in another tag
        }
        let val = dir[base + 3];
        match dir[base] as u16 {
            GK_MODEL_TYPE => model_type = Some(val),
            GK_GEOGRAPHIC_TYPE => geographic = Some(val),
            GK_PROJECTED_CRS_TYPE => projected = Some(val),
            _ => {}
        }
    }
    // 0 = undefined, 32767 = user-defined: neither is a usable EPSG code.
    let pick = |v: Option<u64>| -> Option<u32> {
        v.filter(|x| *x > 0 && *x != 32767 && *x <= u32::MAX as u64)
            .map(|x| x as u32)
    };
    // 1 = projected, 2 = geographic; without a model type fall back to
    // whichever CRS key actually carried a code.
    let resolved = match model_type {
        Some(1) => pick(projected).or_else(|| pick(geographic)),
        Some(2) => pick(geographic).or_else(|| pick(projected)),
        _ => pick(projected).or_else(|| pick(geographic)),
    };
    if resolved.is_some() {
        return resolved;
    }
    // User-defined geographic CRS with a WGS 84 citation → EPSG:4326.
    if model_type == Some(2) && geographic == Some(32767) {
        let low = citation.unwrap_or("").to_ascii_lowercase();
        if low.contains("wgs 84") || low.contains("wgs84") {
            return Some(4326);
        }
    }
    None
}

/// Parse one IFD at `off`; returns its entries and the next-IFD offset.
fn parse_ifd(
    src: &dyn RangeSource,
    endian: Endian,
    big: bool,
    off: u64,
) -> Result<(Vec<Entry>, u64), MapServerError> {
    let entry_bytes = if big { 20 } else { 12 };
    // IFD header = entry count only: 8 bytes for BigTIFF, 2 for classic TIFF.
    // (BigTIFF used to read 16 here — the *file* header size — which shifted the
    // entry body by 8 bytes and made every GDAL-written BigTIFF fail with
    // "missing ImageWidth".)
    let head_len = if big { 8 } else { 2 };
    let head = src.read_range(off, head_len as usize).map_err(MapServerError::Io)?;
    let n = if big {
        endian.u64(&head[0..8]) as usize
    } else {
        endian.u16(&head[0..2]) as usize
    };
    if n > 65536 {
        return Err(MapServerError::Format(format!("IFD with {n} entries looks corrupt")));
    }
    let body_len = n * entry_bytes;
    let body = src
        .read_range(off + head_len as u64, body_len)
        .map_err(MapServerError::Io)?;
    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let b = &body[i * entry_bytes..(i + 1) * entry_bytes];
        let tag = endian.u16(&b[0..2]);
        let typ = endian.u16(&b[2..4]);
        let (count, vf) = if big {
            let count = endian.u64(&b[4..12]);
            let mut vf = [0u8; 8];
            vf.copy_from_slice(&b[12..20]);
            (count, vf)
        } else {
            let count = endian.u32(&b[4..8]) as u64;
            let mut vf = [0u8; 8];
            vf[..4].copy_from_slice(&b[8..12]);
            (count, vf)
        };
        entries.push(Entry {
            tag,
            typ,
            count,
            value_field: vf,
            big,
        });
    }
    let next = if big {
        src.read_range(off + head_len as u64 + body_len as u64, 8)
            .map_err(MapServerError::Io)
            .map(|b| endian.u64(&b))?
    } else {
        src.read_range(off + head_len as u64 + body_len as u64, 4)
            .map_err(MapServerError::Io)
            .map(|b| endian.u32(&b) as u64)?
    };
    Ok((entries, next))
}

// ---------------------------------------------------------------------------
// Windowed tile reads.
// ---------------------------------------------------------------------------

/// Read the decoded RGBA tile at tile column `tx`, tile row `ty` of `level`,
/// through the decoded-tile cache when given.
pub fn read_tile_rgba(
    level: &Level,
    src: &dyn RangeSource,
    tx: u32,
    ty: u32,
    cache: Option<&TileCache>,
) -> Result<Vec<u8>, MapServerError> {
    let key = TileKey::new(level.source_id, level.level_index, tx, ty);
    if let Some(c) = cache {
        if let Some(t) = c.get(key) {
            return Ok((*t).clone());
        }
    }
    let idx = (ty as usize) * level.tiles_across() as usize + tx as usize;
    let Some(&off) = level.index.offsets.get(idx) else {
        return Ok(vec![0u8; tile_bytes(level)]);
    };
    let &len = level.index.bytecounts.get(idx).unwrap_or(&0);
    let bytes = src.read_range(off, len as usize).map_err(MapServerError::Io)?;
    let tile = decode::decode_tile_rgba(&bytes, level);
    if let Some(c) = cache {
        c.insert(key, std::sync::Arc::new(tile.clone()));
    }
    Ok(tile)
}

/// Allocate RGBA tile byte count for a level.
pub fn tile_bytes(level: &Level) -> usize {
    (level.tile_w as usize) * (level.tile_h as usize) * 4
}

/// Decode a pixel window `[c0, r0, c1, r1]` (inclusive, level pixels) into a
/// `(c1-c0+1) x (r1-r0+1)` RGBA buffer. Tiles are decoded in parallel.
pub fn read_window_rgba(
    level: &Level,
    src: &dyn RangeSource,
    c0: u32,
    r0: u32,
    c1: u32,
    r1: u32,
    cache: Option<&TileCache>,
) -> Result<Vec<u8>, MapServerError> {
    let (w, h) = (c1 - c0 + 1, r1 - r0 + 1);
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let ta = level.tiles_across();
    let tx0 = c0 / level.tile_w;
    let tx1 = (c1 / level.tile_w).min(ta.saturating_sub(1));
    let ty0 = r0 / level.tile_h;
    let ty1 = (r1 / level.tile_h).min(level.tiles_down().saturating_sub(1));

    // (tx, ty) -> decoded RGBA tile, parallel over tiles.
    let positions: Vec<(u32, u32)> = (ty0..=ty1)
        .flat_map(|ty| (tx0..=tx1).map(move |tx| (tx, ty)))
        .collect();
    let results: Vec<Result<(u32, u32, Vec<u8>), MapServerError>> = positions
        .par_iter()
        .map(|&(tx, ty)| {
            read_tile_rgba(level, src, tx, ty, cache).map(|rgba| (tx, ty, rgba))
        })
        .collect();
    for r in results {
        let (tx, ty, rgba) = r?;
        // Copy the tile's intersection with the window.
        let tile_c0 = tx * level.tile_w;
        let tile_r0 = ty * level.tile_h;
        let i0 = tile_c0.max(c0);
        let i1 = (tile_c0 + level.tile_w - 1).min(c1);
        let j0 = tile_r0.max(r0);
        let j1 = (tile_r0 + level.tile_h - 1).min(r1);
        let tw = level.tile_w as usize;
        for row in j0..=j1 {
            let src_row = (row - tile_r0) as usize;
            let dst_row = (row - r0) as usize;
            let cols = (i1 - i0 + 1) as usize;
            let src_off = (src_row * tw + (i0 - tile_c0) as usize) * 4;
            let dst_off = (dst_row * (w as usize) + (i0 - c0) as usize) * 4;
            out[dst_off..dst_off + cols * 4]
                .copy_from_slice(&rgba[src_off..src_off + cols * 4]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory `RangeSource` for container-parsing tests.
    struct MemSource(Vec<u8>);

    impl RangeSource for MemSource {
        fn read_range(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
            let start = offset as usize;
            let end = (start + len).min(self.0.len());
            if start > self.0.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "offset past end",
                ));
            }
            Ok(self.0[start..end].to_vec())
        }

        fn len(&self) -> Option<u64> {
            Some(self.0.len() as u64)
        }
    }

    /// Minimal single-IFD BigTIFF: 100x50, 16x16 tiles, 1 uncompressed tile.
    /// Regression for the BigTIFF IFD header length (must be 8 bytes, not 16)
    /// and for byte-order-aware out-of-line offsets.
    fn build_bigtiff(little: bool) -> Vec<u8> {
        fn u16b(v: u16, le: bool) -> [u8; 2] {
            if le { v.to_le_bytes() } else { v.to_be_bytes() }
        }
        fn u64b(v: u64, le: bool) -> [u8; 8] {
            if le { v.to_le_bytes() } else { v.to_be_bytes() }
        }
        let le = little;
        let mut out = Vec::new();
        out.extend_from_slice(if le { b"II" } else { b"MM" });
        out.extend_from_slice(&u16b(43, le)); // BigTIFF magic
        out.extend_from_slice(&u16b(8, le)); // offset size
        out.extend_from_slice(&u16b(0, le)); // reserved
        out.extend_from_slice(&u64b(16, le)); // first IFD right after header

        let entries: [(u16, u16, u64, u64); 7] = [
            (256, 3, 1, 100),  // ImageWidth (SHORT, inline)
            (257, 3, 1, 50),   // ImageLength (inline)
            (259, 3, 1, 1),    // Compression = none (inline)
            (322, 3, 1, 16),   // TileWidth (inline)
            (323, 3, 1, 16),   // TileLength (inline)
            (324, 16, 2, 400), // TileOffsets: 2 LONG8 values, out-of-line @400
            (325, 16, 2, 416), // TileByteCounts: 2 LONG8 values, out-of-line @416
        ];
        out.extend_from_slice(&u64b(entries.len() as u64, le));
        for (tag, typ, count, val) in entries {
            out.extend_from_slice(&u16b(tag, le));
            out.extend_from_slice(&u16b(typ, le));
            out.extend_from_slice(&u64b(count, le));
            // Inline value field is left-justified for the value's own width
            // (SHORT -> first 2 bytes, LONG8 -> all 8), then zero padded.
            let mut field = [0u8; 8];
            if typ == 3 {
                field[..2].copy_from_slice(&u16b(val as u16, le));
            } else {
                field.copy_from_slice(&u64b(val, le));
            }
            out.extend_from_slice(&field);
        }
        out.extend_from_slice(&u64b(0, le)); // next IFD

        // Out-of-line tile offset table: [500, 0] then bytecounts [256, 256].
        out.resize(400, 0);
        out.extend_from_slice(&u64b(500, le));
        out.extend_from_slice(&u64b(0, le));
        out.extend_from_slice(&u64b(256, le));
        out.extend_from_slice(&u64b(256, le));
        out.resize(756, 0); // tile bytes from offset 500
        out
    }

    #[test]
    fn parses_bigtiff_le() {
        let cog = Cog::parse(&MemSource(build_bigtiff(true))).expect("bigtiff LE parse");
        assert_eq!(cog.levels.len(), 1);
        assert_eq!(cog.levels[0].width, 100);
        assert_eq!(cog.levels[0].height, 50);
        assert_eq!(cog.levels[0].tile_w, 16);
        assert_eq!(cog.levels[0].index.offsets, vec![500, 0]);
        assert_eq!(cog.levels[0].index.bytecounts, vec![256, 256]);
    }

    #[test]
    fn parses_bigtiff_be_offsets() {
        // Big-endian container: out-of-line offsets and inline values must both
        // be decoded big-endian.
        let cog = Cog::parse(&MemSource(build_bigtiff(false))).expect("bigtiff BE parse");
        assert_eq!(cog.levels[0].width, 100);
        assert_eq!(cog.levels[0].index.offsets, vec![500, 0]);
    }

    #[test]
    fn geo_key_epsg_reads_geographic_and_projected() {
        // [version, revision, minorRevision, numKeys] then (key, location,
        // count, value) quadruples.
        let geographic = [1u64, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 4326];
        assert_eq!(geo_key_epsg(&geographic), Some(4326));
        let projected = [1u64, 1, 0, 2, 1024, 0, 1, 1, 3072, 0, 1, 4547];
        assert_eq!(geo_key_epsg(&projected), Some(4547));
        // Projected data must not pick up a stray geographic code when both
        // keys are present.
        let both = [
            1u64, 1, 0, 3, 1024, 0, 1, 1, 2048, 0, 1, 4326, 3072, 0, 1, 4547,
        ];
        assert_eq!(geo_key_epsg(&both), Some(4547));
    }

    #[test]
    fn geo_key_epsg_rejects_unknown() {
        // 0 and 32767 are the undefined / user-defined sentinels; a file with
        // no CRS keys at all is unknown.
        let user_defined = [1u64, 1, 0, 1, 3072, 0, 1, 0];
        assert_eq!(geo_key_epsg(&user_defined), None);
        let user_defined_32767 = [1u64, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 32767];
        assert_eq!(geo_key_epsg(&user_defined_32767), None);
        assert_eq!(geo_key_epsg(&[]), None);
        assert_eq!(geo_key_epsg(&[1, 1, 9]), None);
        // Value stored in another tag (location != 0) is not our inline case.
        let out_of_line = [1u64, 1, 0, 1, 2048, 34737, 1, 0];
        assert_eq!(geo_key_epsg(&out_of_line), None);
    }

    #[test]
    fn geo_key_epsg_infers_wgs84_from_citation() {
        // GDAL often writes 32767 + `GCS Name = WGS 84|…` for EPSG:4326.
        let dir = [1u64, 1, 0, 2, 1024, 0, 1, 2, 2048, 0, 1, 32767];
        assert_eq!(
            geo_key_epsg_cited(&dir, Some("GCS Name = WGS 84|Datum = WGS 1984|")),
            Some(4326)
        );
        // No citation / unrelated citation stays unknown.
        assert_eq!(geo_key_epsg_cited(&dir, None), None);
        assert_eq!(
            geo_key_epsg_cited(&dir, Some("GCS Name = NAD83|")),
            None
        );
    }
}
