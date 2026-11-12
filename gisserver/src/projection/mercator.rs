//! Web Mercator (EPSG:3857) <-> WGS84 lon/lat (EPSG:4326).

/// Half the Web Mercator world extent (meters).
pub const MERC_MAX: f64 = 20037508.3427892;

/// WGS84 latitude beyond which Mercator Y diverges.
const MAX_LAT: f64 = 85.0511287798066;

/// Lon/lat (degrees) -> Web Mercator meters. `None` for latitudes at/over the
/// Mercator asymptote.
pub fn lonlat_to_mercator(lon: f64, lat: f64) -> Option<(f64, f64)> {
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > MAX_LAT {
        return None;
    }
    let x = lon.clamp(-180.0, 180.0) * (MERC_MAX / 180.0);
    let lat_r = lat.to_radians();
    let y = (lat_r.tan() + 1.0 / lat_r.cos()).ln() * (MERC_MAX / std::f64::consts::PI);
    Some((x, y.clamp(-MERC_MAX, MERC_MAX)))
}

/// Web Mercator meters -> lon/lat degrees.
pub fn mercator_to_lonlat(x: f64, y: f64) -> (f64, f64) {
    let lon = x / (MERC_MAX / 180.0);
    let lat = (2.0 * (y * std::f64::consts::PI / MERC_MAX).exp().atan() - std::f64::consts::FRAC_PI_2)
        .to_degrees();
    (lon, lat)
}

/// Lon/lat -> Web Mercator, batch; invalid latitudes map to `(NaN, NaN)`.
pub fn lonlat_to_mercator_vec(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    pts.iter()
        .map(|&(lon, lat)| lonlat_to_mercator(lon, lat).unwrap_or((f64::NAN, f64::NAN)))
        .collect()
}

/// Web Mercator -> lon/lat, batch.
pub fn mercator_to_lonlat_vec(pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
    pts.iter().map(|&(x, y)| mercator_to_lonlat(x, y)).collect()
}

/// XYZ (z/x/y) -> Web Mercator tile bounds `[minx, miny, maxx, maxy]`.
pub fn xyz_tile_bounds_merc(z: u32, x: u32, y: u32) -> [f64; 4] {
    let n = (1u64 << z) as f64;
    let span = 2.0 * MERC_MAX / n;
    let minx = -MERC_MAX + x as f64 * span;
    let maxy = MERC_MAX - y as f64 * span;
    [minx, maxy - span, minx + span, maxy]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_round_trip() {
        for (lon, lat) in [(0.0, 0.0), (104.06, 30.67), (-73.98, 40.75), (179.9, -80.0)] {
            let (x, y) = lonlat_to_mercator(lon, lat).unwrap();
            let (lon2, lat2) = mercator_to_lonlat(x, y);
            assert!((lon - lon2).abs() < 1e-9, "lon {lon}");
            assert!((lat - lat2).abs() < 1e-9, "lat {lat}");
        }
    }

    #[test]
    fn mercator_corners() {
        let (x, y) = lonlat_to_mercator(180.0, 85.0511287798066).unwrap();
        assert!((x - MERC_MAX).abs() < 1.0);
        assert!((y - MERC_MAX).abs() < 1.0);
        assert!(lonlat_to_mercator(0.0, 90.0).is_none());
    }
}
