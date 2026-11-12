//! Tile matrix for projected-CRS dynamic vector tiles (the "original
//! projection" interface).
//!
//! WebMercator tiles (the standard XYZ interface) always use
//! `ST_TileEnvelope(z, x, y)` on EPSG:3857. Projected data (e.g. CGCS2000
//! Gauss-Kruger CM 102E, EPSG:4543) needs its own tile matrix so tiles stay
//! in the **source projection** — no `ST_Transform` in the query, which is
//! both faster and avoids reprojection error for downstream GIS clients.
//!
//! Grid definition (WebMercatorQuad structure, so zoom levels have
//! comparable ground resolution):
//!
//! * world extent: `±20037508.342789244` metres on **both** axes (same
//!   magnitude as the Web Mercator half-circumference);
//! * zoom `z` has `2^z x 2^z` tiles, tile size = `2*WORLD / 2^z` metres;
//! * XYZ addressing, top-left origin (`y` grows southwards).
//!
//! For EPSG:4543 (false easting 500 000 m, northing ~2.7-3.1 M m in Yunnan)
//! every coordinate falls comfortably inside the extent, so tile `x/y`
//! indices stay positive.

/// Half world extent in metres for the projected grid (same magnitude as the
/// Web Mercator half-circumference 20037508.342789244).
pub const PROJECTED_WORLD_HALF: f64 = 20_037_508.342_789_244;

/// Number of tiles per axis at zoom `z` (`2^z`, clamped to `2^30`).
pub fn matrix_size(z: u32) -> u64 {
    1u64 << z.min(30)
}

/// Tile size in map units (metres) at zoom `z`.
pub fn tile_size(z: u32) -> f64 {
    2.0 * PROJECTED_WORLD_HALF / matrix_size(z) as f64
}

/// Tile bounds in the projected CRS: `[minx, miny, maxx, maxy]`.
///
/// XYZ addressing: tile `(0, 0)` covers the top-left (north-west) corner.
/// Returns `None` when the address is outside the matrix at this zoom.
pub fn tile_bounds(z: u32, x: u32, y: u32) -> Option<[f64; 4]> {
    let n = matrix_size(z);
    if u64::from(x) >= n || u64::from(y) >= n {
        return None;
    }
    let ts = tile_size(z);
    let minx = -PROJECTED_WORLD_HALF + f64::from(x) * ts;
    let maxy = PROJECTED_WORLD_HALF - f64::from(y) * ts;
    Some([minx, maxy - ts, minx + ts, maxy])
}

/// Map a point (projected map units) to its tile address at zoom `z`.
/// Inverse of [`tile_bounds`] — used by tests and future grid calculators.
pub fn point_to_tile(z: u32, px: f64, py: f64) -> Option<(u32, u32)> {
    let ts = tile_size(z);
    let n = matrix_size(z);
    let tx = ((px + PROJECTED_WORLD_HALF) / ts).floor();
    let ty = ((PROJECTED_WORLD_HALF - py) / ts).floor();
    if tx < 0.0 || ty < 0.0 || tx as u64 >= n || ty as u64 >= n {
        return None;
    }
    Some((tx as u32, ty as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn z0_covers_whole_world() {
        let b = tile_bounds(0, 0, 0).unwrap();
        assert!(close(b[0], -PROJECTED_WORLD_HALF));
        assert!(close(b[1], -PROJECTED_WORLD_HALF));
        assert!(close(b[2], PROJECTED_WORLD_HALF));
        assert!(close(b[3], PROJECTED_WORLD_HALF));
    }

    #[test]
    fn tile_size_halves_per_zoom() {
        assert!(close(tile_size(0), 2.0 * PROJECTED_WORLD_HALF));
        assert!(close(tile_size(1), PROJECTED_WORLD_HALF));
        assert!(close(tile_size(14), 2.0 * PROJECTED_WORLD_HALF / 16384.0));
    }

    #[test]
    fn z14_wuhua_bounds_are_plausible() {
        // A Wuhua (Kunming) building centroid in EPSG:4543 map units.
        let (px, py) = (562_011.49_f64, 2_773_995.81_f64);
        let (x, y) = point_to_tile(14, px, py).expect("in range");
        let b = tile_bounds(14, x, y).unwrap();
        assert!(px >= b[0] && px <= b[2], "x in bounds");
        assert!(py >= b[1] && py <= b[3], "y in bounds");
        // Tile spans ~2.4 km (WebMercatorQuad z14 is ~2.4 km/px-equivalent).
        assert!(close(b[2] - b[0], tile_size(14)));
    }

    #[test]
    fn out_of_range_address_is_none() {
        assert!(tile_bounds(2, 4, 0).is_none());
        assert!(tile_bounds(2, 0, 4).is_none());
        assert!(tile_bounds(2, 3, 3).is_some());
        assert!(point_to_tile(2, -30_000_000.0, 0.0).is_none());
        assert!(point_to_tile(2, 0.0, 30_000_000.0).is_none());
    }

    #[test]
    fn tiles_tile_the_plane_without_gaps() {
        // Adjacent tiles at z3 share edges exactly.
        let a = tile_bounds(3, 1, 1).unwrap();
        let b = tile_bounds(3, 2, 1).unwrap();
        assert!(close(a[2], b[0]));
        let c = tile_bounds(3, 1, 2).unwrap();
        assert!(close(a[1], c[3]));
    }
}
