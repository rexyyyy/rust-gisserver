//! Generic tile-grid model: `TileMatrixSet` + presets.
//!
//! One internal model serves TMS, WMTS, MVT and XYZ (ported from TerraServe's
//! `tms.rs`): WMTS/top-left convention, origin = top-left corner, row increases
//! south. A tile request is a GetMap with a computed bbox.

use serde::{Deserialize, Serialize};

use crate::crs::{is_geographic, meters_per_unit, normalize, EPSG_3857, EPSG_4326, EPSG_4490};

/// One zoom level of a grid. `resolution` = CRS units per pixel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TmLevel {
    pub z: u32,
    pub resolution: f64,
    pub matrix_w: u32,
    pub matrix_h: u32,
    /// Scale denominator as the **source** declares it, when the source ships
    /// one (an ArcGIS cache's `LODInfo/Scale`). Absent = derived from
    /// `resolution` with the OGC 0.28 mm/pixel rule. The two conventions differ
    /// by ~5.8 % at DPI 96 (`0.0254/DPI` vs `0.28 mm`), which is visible in
    /// WMTS `ScaleDenominator` and in clients that pick a zoom from a scale —
    /// so a pre-cut cache must be advertised with its own numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_denominator: Option<f64>,
}

/// A tile grid: CRS + top-left origin + tile size + per-zoom pyramid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileMatrixSet {
    pub id: String,
    pub crs: String,
    pub origin_x: f64,
    pub origin_y: f64,
    pub tile_w: u32,
    pub tile_h: u32,
    pub levels: Vec<TmLevel>,
}

impl TileMatrixSet {
    pub fn level(&self, z: u32) -> Option<&TmLevel> {
        self.levels.iter().find(|l| l.z == z)
    }

    /// Tile bbox `[minx, miny, maxx, maxy]` in CRS units; `row` is top-left.
    pub fn tile_bounds(&self, z: u32, col: u32, row: u32) -> Option<[f64; 4]> {
        let lvl = self.level(z)?;
        if col >= lvl.matrix_w || row >= lvl.matrix_h {
            return None;
        }
        let span_x = self.tile_w as f64 * lvl.resolution;
        let span_y = self.tile_h as f64 * lvl.resolution;
        let minx = self.origin_x + col as f64 * span_x;
        let maxy = self.origin_y - row as f64 * span_y;
        Some([minx, maxy - span_y, minx + span_x, maxy])
    }

    /// Inclusive tile row/col range covering `bounds` (grid CRS) at zoom z.
    pub fn tile_limits(&self, bounds: [f64; 4], z: u32) -> Option<(u32, u32, u32, u32)> {
        let lvl = self.level(z)?;
        let span_x = self.tile_w as f64 * lvl.resolution;
        let span_y = self.tile_h as f64 * lvl.resolution;
        let [minx, miny, maxx, maxy] = bounds;
        let col_lo = ((minx - self.origin_x) / span_x).floor();
        let col_hi = ((maxx - self.origin_x) / span_x).ceil() - 1.0;
        let row_lo = ((self.origin_y - maxy) / span_y).floor();
        let row_hi = ((self.origin_y - miny) / span_y).ceil() - 1.0;
        let (mw, mh) = (lvl.matrix_w as f64, lvl.matrix_h as f64);
        let mincol = col_lo.max(0.0);
        let maxcol = col_hi.min(mw - 1.0);
        let minrow = row_lo.max(0.0);
        let maxrow = row_hi.min(mh - 1.0);
        if maxcol < mincol || maxrow < minrow {
            return None;
        }
        Some((mincol as u32, maxcol as u32, minrow as u32, maxrow as u32))
    }

    /// Whole-grid extent `[minx, miny, maxx, maxy]` (level 0).
    pub fn full_extent(&self) -> Option<[f64; 4]> {
        let l = self.levels.first()?;
        let ex = l.matrix_w as f64 * self.tile_w as f64 * l.resolution;
        let ey = l.matrix_h as f64 * self.tile_h as f64 * l.resolution;
        Some([
            self.origin_x,
            self.origin_y - ey,
            self.origin_x + ex,
            self.origin_y,
        ])
    }

    /// Scale denominator at zoom z: the source's own value when the grid
    /// carries one, otherwise the OGC 0.28 mm/pixel rule.
    pub fn scale_denominator(&self, z: u32) -> f64 {
        match self.level(z) {
            Some(l) => l
                .scale_denominator
                .unwrap_or_else(|| l.resolution * meters_per_unit(&self.crs) / 0.00028),
            None => f64::NAN,
        }
    }

    /// Web Mercator quad grid (XYZ convention): origin top-left, dyadic.
    pub fn web_mercator_quad(tile_px: u32) -> TileMatrixSet {
        build_quad(
            "WebMercatorQuad",
            EPSG_3857,
            -20037508.3427892,
            20037508.3427892,
            20037508.3427892 * 2.0,
            tile_px,
            25,
        )
    }

    /// WGS84 quad grid (`WorldCRS84Quad`): 2^(z+1) cols x 2^z rows.
    pub fn world_crs84_quad(tile_px: u32) -> TileMatrixSet {
        build_quad(
            "WorldCRS84Quad",
            EPSG_4326,
            -180.0,
            90.0,
            360.0,
            tile_px,
            24,
        )
    }

    /// CGCS2000 quad grid: identical geometry to `WorldCRS84Quad` but declared
    /// in EPSG:4490, so a 4490 layer can be tiled natively without reprojection.
    pub fn cgcs2000_quad(tile_px: u32) -> TileMatrixSet {
        build_quad(
            "CGCS2000Quad",
            EPSG_4490,
            -180.0,
            90.0,
            360.0,
            tile_px,
            24,
        )
    }

    /// Resolve a grid id: a built-in preset, or `custom:...` ids from config.
    pub fn resolve(id: &str, tile_px: u32, custom: &std::collections::BTreeMap<String, TileMatrixSet>) -> Option<TileMatrixSet> {
        let base = strip_size_suffix(id);
        if let Some(c) = custom.get(id) {
            return Some(c.clone());
        }
        match base.as_str() {
            "WebMercatorQuad" => Some(TileMatrixSet::web_mercator_quad(tile_px)),
            "WorldCRS84Quad" => Some(TileMatrixSet::world_crs84_quad(tile_px)),
            "CGCS2000Quad" => Some(TileMatrixSet::cgcs2000_quad(tile_px)),
            _ => None,
        }
    }

    /// True when the grid's covered extent is identical at every level (the
    /// property that makes a single advertised TMS bottom-left `<Origin>` correct).
    pub fn is_level_invariant(&self) -> bool {
        let Some(l0) = self.levels.first() else {
            return true;
        };
        let ext_x0 = l0.matrix_w as f64 * self.tile_w as f64 * l0.resolution;
        let ext_y0 = l0.matrix_h as f64 * self.tile_h as f64 * l0.resolution;
        self.levels.iter().all(|l| {
            let ex = l.matrix_w as f64 * self.tile_w as f64 * l.resolution;
            let ey = l.matrix_h as f64 * self.tile_h as f64 * l.resolution;
            (ex - ext_x0).abs() <= ext_x0.abs() * 1e-6 + 1e-6
                && (ey - ext_y0).abs() <= ext_y0.abs() * 1e-6 + 1e-6
        })
    }
}

/// Strip a size suffix from a grid id: `WebMercatorQuad_512` /
/// `WebMercatorQuad@512` -> `WebMercatorQuad`. Both `_` and `@` separators are
/// accepted (README/config examples use `@`; tile size disambiguation in URLs
/// is friendlier with either). Ids without a numeric tail pass through.
pub fn strip_size_suffix(id: &str) -> String {
    for sep in ['_', '@'] {
        if let Some((base, n)) = id.rsplit_once(sep) {
            if n.parse::<u32>().is_ok() {
                return base.to_string();
            }
        }
    }
    id.to_string()
}

/// OGC 正式登记、由代码生成金字塔的**标准/内置**网格名（比较时先去尺寸尾缀）。
/// 其余网格一律视为**自定义**——来自配置的 `custom_grids:`（如 EPSG:4522 投影带），
/// 或启动时由 ArcGIS `Conf.xml` 的 LOD 派生（`arcgis_<图层>`，EPSG:4490）。
///
/// 这是「标准 vs 自定义」判定的**唯一真理源**：WMTS 能力文档（wms-protocol::wmts）
/// 与 OGC API - Tiles JSON（ogcapi-tiles）都据此决定是否下发官方 `TileMatrixSetURI`
/// / `uri`，避免给自定义网格挂一个指向 404 的 OGC 命名空间「说谎元数据」。
pub const PRESET_IDS: &[&str] = &["WebMercatorQuad", "WorldCRS84Quad"];

/// 网格 id 是否属于标准/内置预设（尺寸变体 `WebMercatorQuad_512` 也算）。
pub fn is_preset_id(id: &str) -> bool {
    let base = strip_size_suffix(id);
    PRESET_IDS.contains(&base.as_str())
}

/// `base(z) = (4^z - 1)/3` -- number of tiles in zooms `< z`.
pub fn zoom_base(z: u32) -> u64 {
    (0..z).fold(0u64, |a, tz| a + (1u64 << (2 * tz)))
}

fn build_quad(
    id: &str,
    crs: &str,
    origin_x: f64,
    origin_y: f64,
    extent_span: f64,
    tile_px: u32,
    max_zoom: u32,
) -> TileMatrixSet {
    let levels = (0..=max_zoom)
        .map(|z| {
            let n = (1u64 << z) as f64;
            let resolution = if is_geographic(crs) {
                // 2^(z+1) columns of 360 degrees.
                extent_span / (tile_px as f64 * 2.0 * n)
            } else {
                extent_span / (tile_px as f64 * n)
            };
            let (matrix_w, matrix_h) = if is_geographic(crs) {
                ((2u32 << z), (1u32 << z))
            } else {
                ((1u32 << z), (1u32 << z))
            };
            TmLevel {
                z,
                resolution,
                matrix_w,
                matrix_h,
                scale_denominator: None,
            }
        })
        .collect();
    TileMatrixSet {
        id: id.to_string(),
        crs: normalize(crs),
        origin_x,
        origin_y,
        tile_w: tile_px,
        tile_h: tile_px,
        levels,
    }
}

/// XY tile -> Web Mercator tile bounds (slice bounds helper, standard XYZ math).
pub fn xyz_tile_bounds(z: u32, x: u32, y: u32) -> [f64; 4] {
    let n = (1u64 << z) as f64;
    let span = 20037508.3427892 * 2.0 / n;
    let minx = -20037508.3427892 + x as f64 * span;
    let maxy = 20037508.3427892 - y as f64 * span;
    [minx, maxy - span, minx + span, maxy]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_size_suffix_both_separators() {
        assert_eq!(strip_size_suffix("WebMercatorQuad_512"), "WebMercatorQuad");
        assert_eq!(strip_size_suffix("WebMercatorQuad@512"), "WebMercatorQuad");
        assert_eq!(strip_size_suffix("WorldCRS84Quad@256"), "WorldCRS84Quad");
        // No numeric tail -> untouched (incl. custom grid ids).
        assert_eq!(strip_size_suffix("WebMercatorQuad"), "WebMercatorQuad");
        assert_eq!(strip_size_suffix("my_custom_grid"), "my_custom_grid");
        assert_eq!(strip_size_suffix("CGCS2000Geo"), "CGCS2000Geo");
    }

    /// 「标准 vs 自定义」判定的唯一真理源：内置（含尺寸变体）为 true，
    /// 配置自定义与 ArcGIS 派生网格为 false；以数字结尾的自定义 id 不能被
    /// 尺寸尾缀剥离误判成内置。
    #[test]
    fn is_preset_id_distinguishes_standard_from_custom() {
        assert!(is_preset_id("WebMercatorQuad"));
        assert!(is_preset_id("WorldCRS84Quad"));
        assert!(is_preset_id("WebMercatorQuad_512"));
        assert!(!is_preset_id("arcgis_km_td"));
        assert!(!is_preset_id("wh_custom_grid_4522"));
        assert!(!is_preset_id("CGCS2000Geo"));
    }

    #[test]
    fn resolve_preset_with_size_suffix() {
        let custom = std::collections::BTreeMap::new();
        for id in ["WebMercatorQuad@512", "WebMercatorQuad_512"] {
            let g = TileMatrixSet::resolve(id, 512, &custom)
                .unwrap_or_else(|| panic!("resolve({id})"));
            assert_eq!(g.id, "WebMercatorQuad");
            assert_eq!(g.tile_w, 512);
            assert_eq!(g.tile_h, 512);
        }
        // Suffix id resolves at the layer's own tile size too.
        let g = TileMatrixSet::resolve("WebMercatorQuad_512", 256, &custom).unwrap();
        assert_eq!(g.tile_w, 256, "tile size comes from the layer, not the id");
        // Unknown base stays unknown.
        assert!(TileMatrixSet::resolve("NopeQuad@512", 512, &custom).is_none());
    }
}
