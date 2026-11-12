//! Tile factory: turns a (grid, z, col, row) address into tile bytes,
//! shared by the TMS / WMTS / MVT / XYZ front-ends.

use std::io::{Read, Write};

use pmtiles::tile_type as pt;
use projection::Transformer;
use renderer::{RenderRequest, Resample};
use wms_common::style::RasterStyle;
use wms_common::tile::TileMatrixSet;

use crate::layer::LayerRuntime;
use crate::state::AppState;

/// A produced tile with its response headers.
///
/// Owned strings + serde: the foyer output cache stores tiles on disk, so the
/// payload round-trips through serialization (see `map_api::cache`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TileOut {
    pub bytes: Vec<u8>,
    pub content_type: String,
    /// Set when the payload is gzip-compressed (PMTiles MVT read-through).
    pub content_encoding: Option<String>,
}

impl TileOut {
    pub fn png(bytes: Vec<u8>) -> TileOut {
        TileOut {
            bytes,
            content_type: "image/png".into(),
            content_encoding: None,
        }
    }

    pub fn pbf(bytes: Vec<u8>) -> TileOut {
        TileOut {
            bytes,
            content_type: "application/x-protobuf".into(),
            content_encoding: None,
        }
    }
}

/// Compress a freshly generated pbf **once**, on the cold path, and cache the
/// compressed bytes as the tile payload.
///
/// Why not let the HTTP compression layer gzip every response: a hot MVT tile
/// can be served thousands of times per second, and re-compressing a ~1 MB pbf
/// per request would turn a 0.8 ms cache hit into a CPU-bound handler. Storing
/// the gzip payload in the foyer output cache instead makes hot hits pure byte
/// copies and also shrinks the L1/L2 budget by ~50 %. Clients that do not send
/// `Accept-Encoding: gzip` get the payload decompressed on the fly (see
/// `handlers::pbf_response`).
pub fn gzip_pbf(bytes: Vec<u8>) -> TileOut {
    let mut enc = flate2::write::GzEncoder::new(
        Vec::with_capacity(bytes.len() / 2),
        flate2::Compression::default(),
    );
    match enc.write_all(&bytes).and_then(|_| enc.finish()) {
        // Incompressible tiles (already encoded / tiny) stay raw; the tower-http
        // compression layer still negotiates gzip for those.
        Ok(gz) if !gz.is_empty() && gz.len() < bytes.len() => TileOut {
            bytes: gz,
            content_type: "application/x-protobuf".into(),
            content_encoding: Some("gzip".into()),
        },
        _ => TileOut::pbf(bytes),
    }
}

/// Inverse of [`gzip_pbf`] for clients without gzip support.
pub fn gunzip_pbf(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len() * 3);
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// Render a raster tile at a top-left grid address `(z, col, row)`.
///
/// * MBTiles archive -> read-through (PNG/JPEG/WebP payload per `metadata`).
/// * ArcGIS tile cache -> read-through (compact cache v1/v2 bundle slot).
/// * COG layer -> windowed render + PNG encode.
/// * vector layer -> vector rasterization + PNG encode.
/// * PMTiles PNG archive -> read-through.
///
/// `row` is the **XYZ** (top-left) row; MBTiles stores TMS rows and flips
/// internally. An ArcGIS cache counts rows from its `TileOrigin` southward,
/// i.e. already top-left, so no flip is needed — but the address is only
/// meaningful on the grid derived from that cache's `Conf.xml` (see
/// `config::ServerConfig::inject_arcgis_grids`). Blocking SQLite IO is safe
/// here: every caller already runs this function inside `spawn_blocking`.
pub fn raster_tile(
    state: &AppState,
    layer: &LayerRuntime,
    grid: &TileMatrixSet,
    z: u32,
    col: u32,
    row: u32,
) -> Result<TileOut, String> {
    if let Some(mb) = &layer.mbtiles {
        // 矢量归档（format=pbf）只由 `/mvt` 服务。栅格端点原样透传会把 pbf 当
        // 图片返回（实测 200 + application/x-protobuf），前端解不出图，而能力
        // 探活按 200 判定“该层可发布 WMTS/XYZ”就会给出错误的勾选。与下方
        // PMTiles 分支的 `tile_type` 校验、以及 `mvt_tile` 的反向校验对称。
        if matches!(mb.format().as_str(), "pbf" | "protobuf" | "mvt") {
            return Err("layer's archive is a vector archive (use /mvt)".into());
        }
        let bytes = mb
            .read_tile(z, col, row)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("tile {z}/{col}/{row} not in archive"))?;
        return Ok(crate::mbtiles::tile_out(mb, bytes));
    }
    if let Some(ac) = &layer.arcgis {
        // 寻址直接沿用 (z, col, row)：ArcGIS 缓存的 row 自 TileOrigin（左上）
        // 向南递增，与本函数的 XYZ/左上约定一致。空槽/未切层级/缓存区外都是
        // Ok(None) —— 给 404，不是 500（客户端总会沿数据矩形外多取一圈）。
        let tile = ac
            .read_tile(z, col, row)
            .map_err(|e| format!("read arcgis cache {z}/{col}/{row}: {e}"))?
            .ok_or_else(|| format!("tile {z}/{col}/{row} not in cache"))?;
        return Ok(TileOut {
            bytes: tile.bytes,
            content_type: tile.content_type.to_string(),
            content_encoding: None,
        });
    }
    if let Some(pm) = &layer.pmtiles {
        if pm.tile_type() == pt::PNG {
            let blob = pm.get(z, col, row).map_err(|e| e.to_string())?;
            return match blob {
                Some(b) => Ok(TileOut::png(b)),
                None => Err(format!("tile {z}/{col}/{row} not in archive")),
            };
        }
        return Err("layer's archive is not a PNG archive".into());
    }
    let bounds = grid
        .tile_bounds(z, col, row)
        .ok_or_else(|| format!("tile {z}/{row}/{col} out of range"))?;

    if let Some(fc) = &layer.vector_fc {
        let rgba =
            layer.render_vector(fc, &grid.crs, bounds, grid.tile_w, grid.tile_h);
        let png = renderer::png::encode_rgba(&rgba, grid.tile_w, grid.tile_h)?;
        return Ok(TileOut::png(png));
    }

    let (cog, src) = match (&layer.cog, &layer.source) {
        (Some(c), Some(s)) => (c, s.as_ref()),
        _ => return Err("layer has no raster source".into()),
    };
    let style = layer.style.as_ref().and_then(|s| s.raster.as_ref());
    let req = RenderRequest {
        bbox: bounds,
        crs: &grid.crs,
        src_crs: &layer.def.src_crs,
        width: grid.tile_w,
        height: grid.tile_h,
        resample: Resample::Bilinear,
        style,
    };
    let transformer =
        Transformer::new_any(req.crs, req.src_crs, projection::proj()).map_err(|e| e.to_string())?;
    let rgba = renderer::render_with_cog(&req, cog, src, Some(&state.tile_cache), &transformer)
        .map_err(|e| e.to_string())?;
    let png = renderer::png::encode_rgba(&rgba, grid.tile_w, grid.tile_h)?;
    Ok(TileOut::png(png))
}

/// Produce an MVT tile at a grid address `(z, x, y)`.
///
/// * MBTiles vector archive (format `pbf`) -> read-through (gzip payloads
///   passed through with `Content-Encoding: gzip`).
/// * vector layer -> GeoJSON -> MVT encode, **grid-aware**:
///   - when the GeoJSON is stored in the requested grid's own CRS (a projected
///     source, e.g. EPSG:4522), tiles stay in that source projection via
///     [`mvt::encode_projected`] (linear map, no reprojection) — otherwise the
///     Web-Mercator encoder would clip every metre-unit feature away and emit a
///     15-byte empty tile;
///   - geographic sources (EPSG:4326) keep the classic [`mvt::encode`] path.
/// * PMTiles MVT archive -> read-through (gzip payloads passed through with
///   `Content-Encoding: gzip`).
pub fn mvt_tile(
    layer: &LayerRuntime,
    grid: &TileMatrixSet,
    z: u32,
    x: u32,
    y: u32,
) -> Result<TileOut, String> {
    if layer.arcgis.is_some() {
        // ArcGIS 预切片缓存里只有影像（PNG/JPEG/MIXED）：矢量端点必须明确拒绝，
        // 不能落到底下的 “not a vector layer” 之前被误当成可发布 MVT 的层。
        return Err("arcgis tile cache holds raster tiles only (use /wmts, /tms or /xyz)".into());
    }
    if let Some(mb) = &layer.mbtiles {
        let fmt = mb.format();
        if !matches!(fmt.as_str(), "pbf" | "protobuf" | "mvt") {
            return Err("layer's archive is not a vector archive".into());
        }
        let bytes = mb
            .read_tile(z, x, y)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("tile {z}/{x}/{y} not in archive"))?;
        return Ok(crate::mbtiles::tile_out(mb, bytes));
    }
    if let Some(pm) = &layer.pmtiles {
        if pm.tile_type() != pt::MVT {
            return Err("layer's archive is not an MVT archive".into());
        }
        let bytes = pm.get(z, x, y).map_err(|e| e.to_string())?;
        let bytes = bytes.ok_or_else(|| format!("tile {z}/{x}/{y} not in archive"))?;
        let encoding = match pm.tile_compression() {
            pmtiles::compression::GZIP => Some("gzip"),
            _ => None,
        };
        return Ok(TileOut {
            bytes,
            content_type: "application/x-protobuf".into(),
            content_encoding: encoding.map(|e| e.to_string()),
        });
    }
    let fc = layer.vector_fc.as_ref().ok_or("layer is not a vector layer")?;
    // Grid-aware dispatch (see the doc above). Only a **projected** grid whose
    // CRS equals the layer's source CRS takes the source-projection path; a
    // geographic source (or a Web Mercator request) keeps `mvt::encode`, so the
    // long-standing EPSG:4326 -> Web Mercator behaviour is byte-for-byte intact.
    let src = wms_common::crs::normalize(&layer.def.src_crs);
    let gcrs = wms_common::crs::normalize(&grid.crs);
    let bytes = if src == gcrs && !wms_common::crs::is_geographic(&gcrs) {
        let Some(bounds) = grid.tile_bounds(z, x, y) else {
            return Err(format!("tile {z}/{x}/{y} out of grid range"));
        };
        mvt::encode_projected(fc, layer.name(), bounds)
    } else {
        mvt::encode(fc, z, x, y, layer.name())
    };
    Ok(TileOut::pbf(bytes))
}

/// Raster style helper used by the tile factory (kept public for the WMS path).
pub fn raster_style(layer: &LayerRuntime) -> Option<&RasterStyle> {
    layer.style.as_ref().and_then(|s| s.raster.as_ref())
}
