//! Tile matrix for the "source projection" MVT interface
//! (`/mvt/{layer}/{srid}/{z}/{x}/{y}.pbf`).
//!
//! Tiles are laid out **in the source CRS's own units**, because the whole
//! point of this interface is that the query does no `ST_Transform`. Two unit
//! systems reach it, so both are defined here:
//!
//! * geographic sources (`EPSG:4326`/`4490`/`4214`/`4258`, degrees): world
//!   extent lon `-180..180`, lat `-90..90`; zoom `z` has `2^(z+1)` columns by
//!   `2^z` rows of square `360 / 2^(z+1)` degree tiles. That is exactly the
//!   `WorldCRS84Quad` / `CGCS2000Quad` addressing, so a source-SRID tile and
//!   the identically numbered published-grid tile cover the same ground;
//! * projected sources (Gauss-Krüger / UTM zones, metres): world extent
//!   `±20 037 508.342 789 244` on both axes — the same magnitude as the Web
//!   Mercator half-circumference, so ground resolution per zoom is comparable
//!   to `WebMercatorQuad` — with `2^z x 2^z` tiles.
//!
//! XYZ addressing in both cases: tile `(0, 0)` covers the north-west corner
//! and `y` grows southwards.
//!
//! A degrees-based matrix is required for geographic sources: reusing the
//! metre extent for `EPSG:4326` makes one tile ~39 000 "degrees" wide, so
//! every building collapses into a sub-pixel patch and `ST_AsMVTGeom` returns
//! NULL for all of them — the interface then 404s at every address.
use crate::wms_common::crs;

/// Half world extent in metres for projected sources (same magnitude as the
/// Web Mercator half-circumference 20037508.342789244).
pub const PROJECTED_WORLD_HALF: f64 = 20_037_508.342_789_244;

/// Half width of the geographic world extent, degrees.
pub const GEOGRAPHIC_LON_HALF: f64 = 180.0;

/// Half height of the geographic world extent, degrees.
pub const GEOGRAPHIC_LAT_HALF: f64 = 90.0;

/// True when a source SRID is measured in degrees (lon/lat) rather than metres.
pub fn is_geographic_srid(srid: i32) -> bool {
    crs::is_geographic(&format!("EPSG:{srid}"))
}

/// Tiles per axis `(columns, rows)` at zoom `z` (`z` clamped to 30).
pub fn matrix_shape(srid: i32, z: u32) -> (u64, u64) {
    let n = 1u64 << z.min(30);
    if is_geographic_srid(srid) {
        (n * 2, n)
    } else {
        (n, n)
    }
}

/// Tile span `(width, height)` in the source CRS's own units at zoom `z`.
pub fn tile_span(srid: i32, z: u32) -> (f64, f64) {
    let (cols, rows) = matrix_shape(srid, z);
    if is_geographic_srid(srid) {
        (
            2.0 * GEOGRAPHIC_LON_HALF / cols as f64,
            2.0 * GEOGRAPHIC_LAT_HALF / rows as f64,
        )
    } else {
        let span = 2.0 * PROJECTED_WORLD_HALF / cols as f64;
        (span, span)
    }
}

/// Full extent `[minx, miny, maxx, maxy]` of the matrix. Used as the
/// source-SRID TileJSON bounds when the database cannot answer.
pub fn world_extent(srid: i32) -> [f64; 4] {
    if is_geographic_srid(srid) {
        [
            -GEOGRAPHIC_LON_HALF,
            -GEOGRAPHIC_LAT_HALF,
            GEOGRAPHIC_LON_HALF,
            GEOGRAPHIC_LAT_HALF,
        ]
    } else {
        [
            -PROJECTED_WORLD_HALF,
            -PROJECTED_WORLD_HALF,
            PROJECTED_WORLD_HALF,
            PROJECTED_WORLD_HALF,
        ]
    }
}

/// Tile bounds in the source CRS: `[minx, miny, maxx, maxy]`.
///
/// Returns `None` when the address is outside the matrix at this zoom.
pub fn tile_bounds(srid: i32, z: u32, x: u32, y: u32) -> Option<[f64; 4]> {
    let (cols, rows) = matrix_shape(srid, z);
    if u64::from(x) >= cols || u64::from(y) >= rows {
        return None;
    }
    let (tw, th) = tile_span(srid, z);
    let world = world_extent(srid);
    let minx = world[0] + f64::from(x) * tw;
    let maxy = world[3] - f64::from(y) * th;
    Some([minx, maxy - th, minx + tw, maxy])
}

/// Map a point (source-CRS units) to its tile address at zoom `z`.
/// Inverse of [`tile_bounds`].
pub fn point_to_tile(srid: i32, z: u32, px: f64, py: f64) -> Option<(u32, u32)> {
    let (cols, rows) = matrix_shape(srid, z);
    let (tw, th) = tile_span(srid, z);
    let world = world_extent(srid);
    let tx = ((px - world[0]) / tw).floor();
    let ty = ((world[3] - py) / th).floor();
    if tx < 0.0 || ty < 0.0 || tx as u64 >= cols || ty as u64 >= rows {
        return None;
    }
    Some((tx as u32, ty as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wms_common::tile::TileMatrixSet;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn z0_covers_whole_world() {
        let b = tile_bounds(4543, 0, 0, 0).unwrap();
        assert!(close(b[0], -PROJECTED_WORLD_HALF));
        assert!(close(b[1], -PROJECTED_WORLD_HALF));
        assert!(close(b[2], PROJECTED_WORLD_HALF));
        assert!(close(b[3], PROJECTED_WORLD_HALF));
    }

    #[test]
    fn tile_span_halves_per_zoom() {
        assert!(close(tile_span(4543, 0).0, 2.0 * PROJECTED_WORLD_HALF));
        assert!(close(tile_span(4543, 1).0, PROJECTED_WORLD_HALF));
        assert!(close(tile_span(4543, 14).0, 2.0 * PROJECTED_WORLD_HALF / 16384.0));
        // Geographic: 360 degrees split over 2 columns at z0.
        assert!(close(tile_span(4326, 0).0, 180.0));
        assert!(close(tile_span(4326, 10).0, 360.0 / 2048.0));
    }

    #[test]
    fn z14_wuhua_bounds_are_plausible() {
        // A Wuhua (Kunming) building centroid in EPSG:4543 map units.
        let (px, py) = (562_011.49_f64, 2_773_995.81_f64);
        let (x, y) = point_to_tile(4543, 14, px, py).expect("in range");
        let b = tile_bounds(4543, 14, x, y).unwrap();
        assert!(px >= b[0] && px <= b[2], "x in bounds");
        assert!(py >= b[1] && py <= b[3], "y in bounds");
        // Tile spans ~2.4 km (WebMercatorQuad z14 is ~2.4 km/px-equivalent).
        assert!(close(b[2] - b[0], tile_span(4543, 14).0));
    }

    /// The geographic source-SRID matrix must be the `WorldCRS84Quad` matrix:
    /// same tile count, same envelope. A client that reads one convention can
    /// therefore address the other interface with identical numbers.
    #[test]
    fn geographic_source_srid_matches_world_crs84_quad() {
        let quad = TileMatrixSet::world_crs84_quad(256);
        for z in [0u32, 1, 5, 10, 17] {
            let (cols, rows) = matrix_shape(4326, z);
            let lvl = quad.level(z).unwrap();
            assert_eq!((cols, rows), (lvl.matrix_w as u64, lvl.matrix_h as u64));
            for (x, y) in [(0, 0), (cols / 2, rows / 2), (cols - 1, rows - 1)] {
                let mine = tile_bounds(4326, z, x as u32, y as u32).unwrap();
                let theirs = quad.tile_bounds(z, x as u32, y as u32).unwrap();
                for i in 0..4 {
                    assert!(
                        close(mine[i], theirs[i]),
                        "z{z} {x}/{y} axis {i}: {mine:?} vs {theirs:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn geographic_addresses_resolve_where_metre_math_was_degenerate() {
        // Kunming Wuhua at lon 102.66 / lat 25.20 — the OGC-style column/row.
        let (x, y) = point_to_tile(4326, 10, 102.66, 25.20).unwrap();
        assert_eq!((x, y), (1608, 368));
        let b = tile_bounds(4326, 10, x, y).unwrap();
        assert!(b[0] <= 102.66 && 102.66 <= b[2]);
        assert!(b[1] <= 25.20 && 25.20 <= b[3]);
        // Span must shrink with zoom, or tiles would never get finer.
        assert!(close(tile_span(4326, 10).0, tile_span(4326, 10).1));
        assert!(close(tile_span(4326, 11).0, tile_span(4326, 10).0 / 2.0));
        // CGCS2000 geographic behaves the same way (degrees, 2:1 matrix).
        assert_eq!(matrix_shape(4490, 10), (2048, 1024));
    }

    #[test]
    fn out_of_range_address_is_none() {
        assert!(tile_bounds(4543, 2, 4, 0).is_none());
        assert!(tile_bounds(4543, 2, 0, 4).is_none());
        assert!(tile_bounds(4543, 2, 3, 3).is_some());
        assert!(tile_bounds(4326, 10, 2048, 0).is_none());
        assert!(tile_bounds(4326, 10, 0, 1024).is_none());
        assert!(point_to_tile(4543, 2, -30_000_000.0, 0.0).is_none());
        assert!(point_to_tile(4543, 2, 0.0, 30_000_000.0).is_none());
        assert!(point_to_tile(4326, 10, 190.0, 0.0).is_none());
    }

    #[test]
    fn tiles_tile_the_plane_without_gaps() {
        // Adjacent tiles at z3 share edges exactly, in both unit systems.
        for srid in [4543, 4326] {
            let a = tile_bounds(srid, 3, 1, 1).unwrap();
            let b = tile_bounds(srid, 3, 2, 1).unwrap();
            assert!(close(a[2], b[0]), "srid {srid} east edge");
            let c = tile_bounds(srid, 3, 1, 2).unwrap();
            assert!(close(a[1], c[3]), "srid {srid} south edge");
        }
    }

    #[test]
    fn world_extent_follows_the_unit_system() {
        assert_eq!(world_extent(4326), [-180.0, -90.0, 180.0, 90.0]);
        assert_eq!(world_extent(4490), [-180.0, -90.0, 180.0, 90.0]);
        assert_eq!(world_extent(4543)[0], -PROJECTED_WORLD_HALF);
        assert!(is_geographic_srid(4214));
        assert!(!is_geographic_srid(3857));
        assert!(!is_geographic_srid(4522));
    }
}
