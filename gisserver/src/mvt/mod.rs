//! MVT (spec 2.1) encoder.
//!
//! Two tile schemes are supported:
//! * [`encode`] — input GeoJSON is EPSG:4326, output tiles on the Web Mercator
//!   grid (the classic XYZ interface).
//! * [`encode_projected`] — input GeoJSON is already in a projected/planar CRS
//!   and tiles stay in that **source projection** (linear map into the tile's
//!   local space, no reprojection), so CGCS2000 / Gauss-Kruger vector layers
//!   emit real features instead
//!   of empty tiles.

mod proto;

use proto::{bytes_field, fixed32_field, packed_varints, varint_field, zigzag};
use serde_json::Value;

/// Default tile extent in local coordinates.
pub const EXTENT: u32 = 4096;

/// Geometry type codes (MVT enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeomType {
    Unknown = 0,
    Point = 1,
    LineString = 2,
    Polygon = 3,
}

/// A tile-layer payload: name + features (each with geometry + tags).
pub struct TileLayer {
    pub name: String,
    pub features: Vec<LayerFeature>,
}

pub struct LayerFeature {
    pub geom_type: GeomType,
    /// Command integers already in MVT wire form (with zigzag deltas).
    pub geometry: Vec<u32>,
    /// Property pairs (key index, value index).
    pub tags: Vec<u32>,
    pub id: Option<u64>,
}

/// Encode a GeoJSON FeatureCollection for tile `(z, x, y)` on the Web Mercator
/// grid, assuming the input is EPSG:4326 lon/lat. `layer_name` becomes the MVT
/// layer name.
///
/// Features are pre-filtered by tile intersection: a feature whose lon/lat
/// bbox does not intersect the tile's bbox is skipped before projection and
/// encoding (the old path cloned the whole feature array and encoded every
/// geometry, including ones landing entirely outside the tile).
pub fn encode(fc: &Value, z: u32, x: u32, y: u32, layer_name: &str) -> Vec<u8> {
    encode_fc(
        fc,
        layer_name,
        &webmercator_projector(z, x, y),
        &|b| bbox_hits_tile(b, z, x, y),
    )
}

/// Encode a GeoJSON FeatureCollection whose coordinates are **already in the
/// tile grid's CRS** (e.g. an EPSG:4522 CGCS2000 layer), clipping into
/// `tile_bounds = [minx, miny, maxx, maxy]` (source map units) and mapping
/// linearly into the 0..EXTENT tile-local space. No PROJ reprojection happens —
/// tiles stay in the source projection, mirroring the PostGIS `Iface::Projected`
/// path. A degenerate (zero width/height) bound is clamped to avoid a divide by
/// zero.
pub fn encode_projected(fc: &Value, layer_name: &str, tile_bounds: [f64; 4]) -> Vec<u8> {
    encode_fc(
        fc,
        layer_name,
        &projected_projector(tile_bounds),
        &bounds_hit_checker(tile_bounds),
    )
}

/// Web Mercator projector: lon/lat -> tile-local integer coords at `(z, x, y)`.
fn webmercator_projector(z: u32, x: u32, y: u32) -> impl Fn(f64, f64) -> (i64, i64) {
    let n = (1u64 << z) as f64;
    let xf = x as f64;
    let yf = y as f64;
    move |lon: f64, lat: f64| -> (i64, i64) {
        let xt = (lon + 180.0) / 360.0 * n;
        let lat_r = lat.to_radians();
        let yt = (1.0 - (lat_r.tan() + 1.0 / lat_r.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
        (
            ((xt - xf) * EXTENT as f64).round() as i64,
            ((yt - yf) * EXTENT as f64).round() as i64,
        )
    }
}

/// Projected projector: source map units -> tile-local integer coords via a
/// linear scale over `tile_bounds` (y axis is top-down, so `maxy` maps to 0).
fn projected_projector(tile_bounds: [f64; 4]) -> impl Fn(f64, f64) -> (i64, i64) {
    let [minx, miny, maxx, maxy] = tile_bounds;
    let w = (maxx - minx).max(1e-9);
    let h = (maxy - miny).max(1e-9);
    move |px: f64, py: f64| -> (i64, i64) {
        (
            ((px - minx) / w * EXTENT as f64).round() as i64,
            ((maxy - py) / h * EXTENT as f64).round() as i64,
        )
    }
}

/// Conservative bbox/tile intersection test against an explicit bound box
/// (touching counts as a hit, mirroring [`bbox_hits_tile`]).
fn bounds_hit_checker(tile_bounds: [f64; 4]) -> impl Fn(&[f64; 4]) -> bool {
    let [minx, miny, maxx, maxy] = tile_bounds;
    move |b: &[f64; 4]| !(b[0] > maxx || b[2] < minx || b[1] > maxy || b[3] < miny)
}

/// Shared encoding core: iterate features, clip with `hits`, project geometry
/// with `project`, build the key/value tables, and emit one MVT layer.
fn encode_fc(
    fc: &Value,
    layer_name: &str,
    project: &dyn Fn(f64, f64) -> (i64, i64),
    hits: &dyn Fn(&[f64; 4]) -> bool,
) -> Vec<u8> {
    let Some(features) = fc.get("features").and_then(|v| v.as_array()) else {
        return encode_tile(
            &TileLayer {
                name: layer_name.to_string(),
                features: Vec::new(),
            },
            &[],
            &[],
        );
    };

    let mut keys: Vec<String> = Vec::new();
    let mut key_idx: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut values: Vec<Value> = Vec::new();
    let mut val_idx: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    let mut layer_features: Vec<LayerFeature> = Vec::new();
    for feat in features {
        let geom = match feat.get("geometry") {
            Some(g) if !g.is_null() => g,
            _ => continue,
        };
        // Conservative tile-clip pre-filter: skip only when the feature bbox
        // provably misses the tile (a feature touching the tile is kept).
        if let Some(gb) = geom_bbox(geom) {
            if !hits(&gb) {
                continue;
            }
        }
        let (gtype, geometry) = match encode_geometry(geom, project) {
            Some(v) => v,
            None => continue,
        };
        if geometry.is_empty() {
            continue;
        }
        let mut tags: Vec<u32> = Vec::new();
        if let Some(props) = feat.get("properties").and_then(|p| p.as_object()) {
            for (k, v) in props {
                let ki = *key_idx.entry(k.clone()).or_insert_with(|| {
                    keys.push(k.clone());
                    (keys.len() - 1) as u32
                });
                let vkey = value_key(v);
                let vi = *val_idx.entry(vkey.clone()).or_insert_with(|| {
                    values.push(v.clone());
                    (values.len() - 1) as u32
                });
                tags.push(ki);
                tags.push(vi);
            }
        }
        let id = feat
            .get("id")
            .and_then(|i| i.as_u64())
            .or_else(|| feat.get("id").and_then(|i| i.as_i64()).map(|i| i as u64));
        layer_features.push(LayerFeature {
            geom_type: gtype,
            geometry,
            tags,
            id,
        });
    }

    encode_tile(&TileLayer {
        name: layer_name.to_string(),
        features: layer_features,
    }, &keys, &values)
}

/// Serialize one layer (plus its key/value tables) as a full Tile message.
fn encode_tile(layer: &TileLayer, keys: &[String], values: &[Value]) -> Vec<u8> {
    let mut layer_buf = Vec::new();
    varint_field(&mut layer_buf, 15, 2); // required string version = "2"
    bytes_field(&mut layer_buf, 1, layer.name.as_bytes());
    for k in keys {
        bytes_field(&mut layer_buf, 3, k.as_bytes());
    }
    for v in values {
        bytes_field(&mut layer_buf, 4, &encode_value(v));
    }
    for f in &layer.features {
        bytes_field(&mut layer_buf, 2, &encode_feature(f));
    }
    varint_field(&mut layer_buf, 5, EXTENT as u64);

    let mut tile = Vec::new();
    bytes_field(&mut tile, 3, &layer_buf);
    tile
}

fn encode_value(v: &Value) -> Vec<u8> {
    let mut b = Vec::new();
    match v {
        Value::String(s) => bytes_field(&mut b, 1, s.as_bytes()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    varint_field(&mut b, 5, i as u64); // uint_value
                } else {
                    varint_field(&mut b, 4, i as u64); // int_value
                }
            } else if let Some(f) = n.as_f64() {
                fixed32_field(&mut b, 2, (f as f32).to_bits()); // float_value: wiretype 5 fixed32
            }
        }
        Value::Bool(t) => varint_field(&mut b, 7, *t as u64),
        _ => {}
    }
    b
}

fn value_key(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn encode_feature(f: &LayerFeature) -> Vec<u8> {
    let mut b = Vec::new();
    if let Some(id) = f.id {
        varint_field(&mut b, 1, id);
    }
    if !f.tags.is_empty() {
        // tags is repeated uint32; each pair is (key_idx, value_idx).
        packed_varints(&mut b, 2, &f.tags.iter().map(|&t| t as u64).collect::<Vec<_>>());
    }
    varint_field(&mut b, 3, f.geom_type as u64);
    packed_varints(&mut b, 4, &f.geometry.iter().map(|&g| g as u64).collect::<Vec<_>>());
    b
}

// ---------------------------------------------------------------------------
// Geometry encoding.
// ---------------------------------------------------------------------------

/// The tile's geographic bbox `(west, south, east, north)` on the Web Mercator
/// grid (inverse of the per-point projection below).
fn tile_bbox(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = (1u64 << z) as f64;
    let west = x as f64 / n * 360.0 - 180.0;
    let east = (x as f64 + 1.0) / n * 360.0 - 180.0;
    let merc_lat = |row: u32| -> f64 {
        let t = std::f64::consts::PI * (1.0 - 2.0 * row as f64 / n);
        t.sinh().atan().to_degrees()
    };
    (west, merc_lat(y + 1), east, merc_lat(y))
}

/// Whether a lon/lat bbox `[west, south, east, north]` intersects the tile
/// (touching counts as a hit — the filter is deliberately conservative).
fn bbox_hits_tile(b: &[f64; 4], z: u32, x: u32, y: u32) -> bool {
    let (west, south, east, north) = tile_bbox(z, x, y);
    !(b[0] > east || b[2] < west || b[1] > north || b[3] < south)
}

/// A GeoJSON geometry's lon/lat bbox `[west, south, east, north]`, or `None`
/// when it contains no positions. Recursive over nested coordinate arrays.
fn geom_bbox(geom: &Value) -> Option<[f64; 4]> {
    let coords = geom.get("coordinates")?;
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut found = false;
    fn walk(c: &Value, b: &mut [f64; 4], found: &mut bool) {
        let Some(arr) = c.as_array() else { return };
        if arr.len() >= 2 {
            if let (Some(x), Some(y)) = (arr[0].as_f64(), arr[1].as_f64()) {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
                *found = true;
                return;
            }
        }
        for item in arr {
            walk(item, b, found);
        }
    }
    walk(coords, &mut b, &mut found);
    found.then_some(b)
}

/// Result of [`encode_geometry`]: `(type, command integers)`.
fn encode_geometry(geom: &Value, project: &dyn Fn(f64, f64) -> (i64, i64)) -> Option<(GeomType, Vec<u32>)> {
    let gtype = geom.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let coords = geom.get("coordinates")?;
    let mut out = Vec::new();
    let mut cur: (i64, i64) = (0, 0);
    match gtype {
        "Point" => {
            let mut c = Vec::new();
            project_point_with(coords, project, &mut c)?;
            if c.len() == 2 {
                out.push(cmd(1, 1)); // MoveTo 1
                push_deltas(&mut out, &[c[0]], &[c[1]], &mut cur);
            }
            Some((GeomType::Point, out))
        }
        "MultiPoint" => {
            let pts = points_of(coords, project);
            if pts.is_empty() {
                return None;
            }
            out.push(cmd(1, pts.len() as u32)); // MoveTo n
            push_deltas(&mut out, &xs(&pts), &ys(&pts), &mut cur);
            Some((GeomType::Point, out))
        }
        "LineString" => {
            let pts = points_of(coords, project);
            if pts.len() < 2 {
                return None;
            }
            encode_line(&pts, &mut out, &mut cur);
            Some((GeomType::LineString, out))
        }
        "MultiLineString" => {
            let lines = coords.as_array()?;
            for line in lines {
                let pts = points_of(line, project);
                if pts.len() >= 2 {
                    encode_line(&pts, &mut out, &mut cur);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some((GeomType::LineString, out))
            }
        }
        "Polygon" => {
            let rings = coords.as_array()?;
            for ring in rings {
                let pts = points_of(ring, project);
                if pts.len() >= 4 {
                    encode_ring(&pts, &mut out, &mut cur);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some((GeomType::Polygon, out))
            }
        }
        "MultiPolygon" => {
            let polys = coords.as_array()?;
            for poly in polys {
                let rings = poly.as_array()?;
                for ring in rings {
                    let pts = points_of(ring, project);
                    if pts.len() >= 4 {
                        encode_ring(&pts, &mut out, &mut cur);
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some((GeomType::Polygon, out))
            }
        }
        _ => None,
    }
}

/// A command integer: `(id & 7) | (count << 3)`.
fn cmd(id: u32, count: u32) -> u32 {
    (id & 0x7) | (count << 3)
}

/// Project one position (already in the scheme's input units) into tile-local
/// integer coords using the supplied `project` mapping.
fn project_point_with(
    c: &Value,
    project: &dyn Fn(f64, f64) -> (i64, i64),
    out: &mut Vec<i64>,
) -> Option<()> {
    let arr = c.as_array()?;
    let a = arr.first()?.as_f64()?;
    let b = arr.get(1)?.as_f64()?;
    let (px, py) = project(a, b);
    out.push(px);
    out.push(py);
    Some(())
}

fn points_of(c: &Value, project: &dyn Fn(f64, f64) -> (i64, i64)) -> Vec<(i64, i64)> {
    let mut flat = Vec::new();
    let mut tmp = Vec::new();
    if let Some(arr) = c.as_array() {
        for p in arr {
            tmp.clear();
            if project_point_with(p, project, &mut tmp).is_some() {
                flat.push((tmp[0], tmp[1]));
            }
        }
    }
    flat
}

/// Thin Web Mercator wrapper kept for the projection-grid unit test: projects a
/// single lon/lat position for tile `(z, x, y)` exactly as [`encode`] does.
#[cfg(test)]
fn project_point(c: &Value, z: u32, x: u32, y: u32, out: &mut Vec<i64>) -> Option<()> {
    project_point_with(c, &webmercator_projector(z, x, y), out)
}

fn xs(pts: &[(i64, i64)]) -> Vec<i64> {
    pts.iter().map(|p| p.0).collect()
}

fn ys(pts: &[(i64, i64)]) -> Vec<i64> {
    pts.iter().map(|p| p.1).collect()
}

fn encode_line(pts: &[(i64, i64)], out: &mut Vec<u32>, cursor: &mut (i64, i64)) {
    // Dedupe consecutive identical points.
    let mut dedup: Vec<(i64, i64)> = Vec::with_capacity(pts.len());
    for &p in pts {
        if dedup.last() != Some(&p) {
            dedup.push(p);
        }
    }
    if dedup.len() < 2 {
        return;
    }
    out.push(cmd(1, 1));
    push_deltas(out, &[dedup[0].0], &[dedup[0].1], cursor);
    out.push(cmd(2, (dedup.len() - 1) as u32));
    push_deltas(out, &xs(&dedup[1..]), &ys(&dedup[1..]), cursor);
}

fn encode_ring(pts: &[(i64, i64)], out: &mut Vec<u32>, cursor: &mut (i64, i64)) {
    // cursor is passed in: it persists across rings of one feature.
    let mut dedup: Vec<(i64, i64)> = Vec::with_capacity(pts.len());
    for &p in pts {
        if dedup.last() != Some(&p) {
            dedup.push(p);
        }
    }
    if dedup.len() < 3 {
        return;
    }
    // Close the ring: remove a trailing duplicate of the first point.
    if dedup.first() == dedup.last() {
        dedup.pop();
    }
    if dedup.len() < 3 {
        return;
    }
    out.push(cmd(1, 1));
    push_deltas(out, &[dedup[0].0], &[dedup[0].1], cursor);
    out.push(cmd(2, (dedup.len() - 1) as u32));
    push_deltas(out, &xs(&dedup[1..]), &ys(&dedup[1..]), cursor);
    out.push(cmd(7, 1));
    // ClosePath carries no parameters (spec 2.1).
}

fn push_deltas(out: &mut Vec<u32>, xs: &[i64], ys: &[i64], cursor: &mut (i64, i64)) {
    for i in 0..xs.len() {
        let dx = xs[i] - cursor.0;
        let dy = ys[i] - cursor.1;
        cursor.0 = xs[i];
        cursor.1 = ys[i];
        out.push(zigzag(dx) as u32);
        out.push(zigzag(dy) as u32);
    }
}

// ---------------------------------------------------------------------------
// TileJSON.
// ---------------------------------------------------------------------------

/// Build a minimal TileJSON document for a vector layer.
pub fn tilejson(
    tile_url_template: &str,
    name: &str,
    minzoom: u8,
    maxzoom: u8,
    bounds: [f64; 4],
) -> serde_json::Value {
    serde_json::json!({
        "tilejson": "3.0.0",
        "name": name,
        "scheme": "xyz",
        "tiles": [tile_url_template],
        "minzoom": minzoom,
        "maxzoom": maxzoom,
        "bounds": bounds,
        "vector_layers": [{"id": name, "fields": {}}],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tile_bbox_matches_the_projection_grid() {
        // z1 tile (0,0): lon [-180, 0], lat [0, 85.051...].
        let (west, south, east, north) = tile_bbox(1, 0, 0);
        assert!((west + 180.0).abs() < 1e-9);
        assert!(south.abs() < 1e-9);
        assert!(east.abs() < 1e-9);
        assert!((north - 85.051128).abs() < 1e-4);
        // A point inside the tile projects into [0, EXTENT).
        let mut c = Vec::new();
        project_point(&json!([-90.0, 45.0]), 1, 0, 0, &mut c).unwrap();
        assert_eq!(c.len(), 2);
        assert!((0..EXTENT as i64).contains(&c[0]));
        assert!((0..EXTENT as i64).contains(&c[1]));
    }

    #[test]
    fn geom_bbox_covers_nested_coordinates() {
        let geom = json!({
            "type": "Polygon",
            "coordinates": [[[10.0, 20.0], [12.0, 25.0], [8.0, 30.0], [10.0, 20.0]]],
        });
        assert_eq!(geom_bbox(&geom).unwrap(), [8.0, 20.0, 12.0, 30.0]);
        assert!(geom_bbox(&json!({"type": "Point", "coordinates": null})).is_none());
    }

    #[test]
    fn bbox_hit_test_is_conservative() {
        // z1/0/0 covers lon [-180,0] / lat [0,85.05].
        let inside = geom_bbox(&json!({"type": "Point", "coordinates": [-90.0, 45.0]})).unwrap();
        let outside = geom_bbox(&json!({"type": "Point", "coordinates": [90.0, 45.0]})).unwrap();
        assert!(bbox_hits_tile(&inside, 1, 0, 0));
        assert!(!bbox_hits_tile(&outside, 1, 0, 0));
        // Touching the tile edge still counts as a hit (must not be dropped).
        let edge = geom_bbox(&json!({"type": "Point", "coordinates": [-180.0, 0.0]})).unwrap();
        assert!(bbox_hits_tile(&edge, 1, 0, 0));
    }

    #[test]
    fn projected_features_inside_bounds_are_kept_and_outside_clipped() {
        // A ~2 km EPSG:4522 tile (map units, metres).
        let bounds = [562_000.0, 2_773_000.0, 564_000.0, 2_775_000.0];
        let inside = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {"h": 10}, "geometry": {"type": "Polygon",
                "coordinates": [[[562_100.0, 2_773_100.0], [562_200.0, 2_773_100.0],
                                  [562_200.0, 2_773_200.0], [562_100.0, 2_773_100.0]]]}},
        ]});
        let outside = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {}, "geometry": {"type": "Polygon",
                "coordinates": [[[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 10.0]]]}},
        ]});
        let empty = json!({"type": "FeatureCollection", "features": []});
        let b_in = encode_projected(&inside, "b", bounds);
        let b_out = encode_projected(&outside, "b", bounds);
        let b_empty = encode_projected(&empty, "b", bounds);
        // Inside -> carries a real feature: strictly more bytes than an empty layer
        // (this is exactly what the Web-Mercator-only path failed to do and returned
        // a 15-byte empty tile for projected sources).
        assert!(b_in.len() > b_empty.len(), "inside feature must survive");
        // Outside -> clipped away, byte-identical to the empty layer.
        assert_eq!(b_out, b_empty, "out-of-bounds feature must be clipped away");
    }

    #[test]
    fn projected_degenerate_bounds_does_not_panic() {
        let fc = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {}, "geometry": {"type": "Point", "coordinates": [1.0, 1.0]}},
        ]});
        // Zero-width/height bounds must not divide by zero.
        let bytes = encode_projected(&fc, "p", [5.0, 5.0, 5.0, 5.0]);
        assert!(!bytes.is_empty());
    }
}
