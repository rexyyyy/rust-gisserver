//! In-memory GeoJSON FeatureCollection querying.
//!
//! Shared by WFS (`GetFeature`), OGC API - Features (`/items`) and the
//! schema/attribute inference used by `DescribeFeatureType`. The engine works
//! on the `serde_json::Value` FeatureCollection every `vector` layer already
//! holds in memory (see `map-api`'s `LayerRuntime::vector_fc`), so there is no
//! IO here: features are filtered by bounding-box intersection, paginated and
//! encoded to GML 3.1/3.2, GeoJSON or CSV.
//!
//! Coordinate order is GeoJSON-native (lon/lat, EPSG:4326); callers that
//! accept a Web Mercator bbox convert before calling (see `mercator_to_4326`).

use serde_json::Value;

pub mod csv;
pub mod gml;

/// A feature query: bbox filter + offset pagination (no limit — the caller
/// applies its own default/maximum so protocols keep their own semantics:
/// WFS `count`, OGC API `limit`).
#[derive(Clone, Debug, Default)]
pub struct FeatureQuery {
    /// Inclusive bbox `[minx, miny, maxx, maxy]` in lon/lat; `None` = no filter.
    pub bbox: Option<[f64; 4]>,
    /// Number of leading matches to skip.
    pub offset: usize,
}

impl FeatureQuery {
    pub fn new(bbox: Option<[f64; 4]>, offset: usize) -> Self {
        Self { bbox, offset }
    }
}

/// Inclusive bbox intersection test (touching edges count as intersecting).
pub fn bbox_intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    !(a[2] < b[0] || a[0] > b[2] || a[3] < b[1] || a[1] > b[3])
}

/// A feature's geometry bbox `[minx, miny, maxx, maxy]`, or `None` when the
/// feature has no (walkable) geometry.
pub fn geom_bbox(feature: &Value) -> Option<[f64; 4]> {
    let g = feature.get("geometry")?;
    let mut bbox: Option<[f64; 4]> = None;
    let mut update = |lon: f64, lat: f64| {
        bbox = Some(match bbox {
            Some([x0, y0, x1, y1]) => {
                [x0.min(lon), y0.min(lat), x1.max(lon), y1.max(lat)]
            }
            None => [lon, lat, lon, lat],
        });
    };
    walk_coords(g, &mut update);
    bbox
}

/// The union bbox of a whole FeatureCollection (`None` when empty).
pub fn geom_bbox_fc(fc: &Value) -> Option<[f64; 4]> {
    features(fc)
        .into_iter()
        .filter_map(geom_bbox)
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
}

/// Walk a GeoJSON geometry's positions, calling `update(lon, lat)` per vertex.
/// Recurses into nested coordinate arrays; a pair of numbers is a position.
pub fn walk_coords(g: &Value, update: &mut dyn FnMut(f64, f64)) {
    let gtype = g.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if gtype == "GeometryCollection" {
        if let Some(gs) = g.get("geometries").and_then(|x| x.as_array()) {
            for sub in gs {
                walk_coords(sub, update);
            }
        }
        return;
    }
    let Some(coords) = g.get("coordinates") else { return };
    walk_array(coords, update);
}

fn walk_array(c: &Value, update: &mut dyn FnMut(f64, f64)) {
    if let Some(arr) = c.as_array() {
        if arr.len() >= 2 && arr[0].is_number() && arr[1].is_number() {
            if let (Some(x), Some(y)) = (arr[0].as_f64(), arr[1].as_f64()) {
                update(x, y);
            }
        } else {
            for item in arr {
                walk_array(item, update);
            }
        }
    }
}

/// The features of a FeatureCollection (borrowed).
pub fn features(fc: &Value) -> Vec<&Value> {
    fc.get("features")
        .and_then(|f| f.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Total number of features matching the query's bbox (ignores `offset`).
pub fn count_matches(fc: &Value, q: &FeatureQuery) -> usize {
    features(fc)
        .into_iter()
        .filter(|f| q.bbox.is_none_or(|b| geom_bbox(f).is_some_and(|gb| bbox_intersects(gb, b))))
        .count()
}

/// Features matching the query, after `offset`, capped at `limit` entries.
/// `limit == None` means "no cap" (WFS semantics: everything left).
pub fn select_features<'a>(
    fc: &'a Value,
    q: &FeatureQuery,
    limit: Option<usize>,
) -> Vec<&'a Value> {
    features(fc)
        .into_iter()
        .filter(|f| q.bbox.is_none_or(|b| geom_bbox(f).is_some_and(|gb| bbox_intersects(gb, b))))
        .skip(q.offset)
        .take(limit.unwrap_or(usize::MAX))
        .collect()
}

/// Feature ids of a query result, aligned with `select_features`.
pub fn select_ids(fc: &Value, q: &FeatureQuery, limit: Option<usize>) -> Vec<String> {
    select_features(fc, q, limit)
        .into_iter()
        .enumerate()
        .map(|(i, f)| feature_id(f, q.offset + i))
        .collect()
}

/// The stable id of a feature: the top-level GeoJSON `id` (RFC 7946), else the
/// first present id-ish property (`id`, `fid`, `FID`, `objectid`, `OBJECTID`),
/// else the 0-based positional fallback `f{index}`.
pub fn feature_id(feature: &Value, index: usize) -> String {
    if let Some(v) = feature.get("id") {
        match v {
            Value::String(s) => return s.clone(),
            Value::Number(n) => return n.to_string(),
            _ => {}
        }
    }
    if let Some(props) = feature.get("properties").and_then(|p| p.as_object()) {
        for key in ["id", "fid", "FID", "objectid", "OBJECTID"] {
            if let Some(v) = props.get(key) {
                return match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
            }
        }
    }
    format!("f{index}")
}

/// Look up one feature by id (same keys as [`feature_id`]).
pub fn find_by_id<'a>(fc: &'a Value, id: &str) -> Option<&'a Value> {
    features(fc)
        .into_iter()
        .enumerate()
        .find(|(i, f)| feature_id(f, *i) == id)
        .map(|(_, f)| f)
}

/// Feature with its `properties` projected onto `names` (GeoJSON order kept).
/// Unknown names are skipped; an empty `names` list returns the feature as-is.
pub fn project_properties(feature: &Value, names: &[String]) -> Value {
    if names.is_empty() {
        return feature.clone();
    }
    let mut out = feature.clone();
    let src = feature.get("properties").and_then(|p| p.as_object());
    let projected = names
        .iter()
        .filter_map(|n| src.and_then(|s| s.get(n)).map(|v| (n.clone(), v.clone())))
        .collect::<serde_json::Map<String, Value>>();
    out["properties"] = Value::Object(projected);
    out
}

/// XSD type of a GeoJSON property value (DescribeFeatureType / queryables).
pub fn xsd_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "xs:string",
        Value::Bool(_) => "xs:boolean",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "xs:integer"
            } else {
                "xs:double"
            }
        }
        Value::String(_) => "xs:string",
        Value::Array(_) | Value::Object(_) => "xs:string",
    }
}

/// Attribute names of a collection with their inferred XSD types, first
/// feature's key order leading (merged across features so sparse attributes
/// still get a column).
pub fn attribute_schema(fc: &Value) -> Vec<(String, &'static str)> {
    let mut order: Vec<String> = Vec::new();
    let mut types: std::collections::HashMap<String, &'static str> = std::collections::HashMap::new();
    for f in features(fc) {
        let Some(props) = f.get("properties").and_then(|p| p.as_object()) else {
            continue;
        };
        for (k, v) in props {
            let t = xsd_type(v);
            match types.get(k) {
                Some(existing) => {
                    // A later feature refines integer -> double; keep the
                    // widest type seen.
                    if *existing == "xs:integer" && t == "xs:double" {
                        types.insert(k.clone(), "xs:double");
                    }
                }
                None => {
                    order.push(k.clone());
                    types.insert(k.clone(), t);
                }
            }
        }
    }
    order
        .into_iter()
        .map(|k| {
            let t = types.get(&k).copied().unwrap_or("xs:string");
            (k, t)
        })
        .collect()
}

/// Geometry type of the collection's first geometry (for schema + tileset
/// `itemType`), or `None` for an empty collection.
pub fn primary_geometry_type(fc: &Value) -> Option<&'static str> {
    features(fc)
        .into_iter()
        .find_map(|f| f.get("geometry")?.get("type")?.as_str())
        .map(|t| match t {
            "Point" | "MultiPoint" => "point",
            "LineString" | "MultiLineString" => "line",
            "Polygon" | "MultiPolygon" => "polygon",
            _ => "geometry",
        })
}

/// Web Mercator (EPSG:3857) meters -> lon/lat degrees (pure math, no PROJ).
pub fn mercator_to_4326(x: f64, y: f64) -> (f64, f64) {
    let lon = x * 180.0 / 20037508.3427892;
    let lat = (y / 6378137.0).sinh().atan().to_degrees();
    (lon, lat)
}

/// lon/lat degrees -> Web Mercator meters (inverse of [`mercator_to_4326`]).
pub fn from_4326_to_mercator(lon: f64, lat: f64) -> (f64, f64) {
    let x = lon * 20037508.3427892 / 180.0;
    let y = ((lat.to_radians() / 2.0 + std::f64::consts::PI / 4.0).tan()).ln() * 6378137.0;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_fc() -> Value {
        json!({
            "type": "FeatureCollection",
            "features": [
                {"type":"Feature","id":"a","properties":{"name":"a","pop":10},"geometry":{"type":"Point","coordinates":[100.0,30.0]}},
                {"type":"Feature","id":"b","properties":{"name":"b","pop":20.5},"geometry":{"type":"LineString","coordinates":[[101.0,31.0],[102.0,32.0]]}},
                {"type":"Feature","id":"c","properties":{"name":"c"},"geometry":{"type":"Polygon","coordinates":[[[103.0,33.0],[104.0,33.0],[104.0,34.0],[103.0,33.0]]]}}
            ]
        })
    }

    #[test]
    fn bbox_filtering() {
        let fc = sample_fc();
        // Only feature a.
        let q = FeatureQuery::new(Some([99.0, 29.0, 100.5, 30.5]), 0);
        assert_eq!(count_matches(&fc, &q), 1);
        let sel = select_features(&fc, &q, None);
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0]["properties"]["name"], "a");
        // Everything (touching edges included).
        let q = FeatureQuery::new(Some([99.0, 29.0, 105.0, 35.0]), 0);
        assert_eq!(count_matches(&fc, &q), 3);
        // Nothing.
        let q = FeatureQuery::new(Some([200.0, 29.0, 201.0, 30.0]), 0);
        assert_eq!(count_matches(&fc, &q), 0);
    }

    #[test]
    fn pagination_offset_limit() {
        let fc = sample_fc();
        let q = FeatureQuery::default();
        let sel = select_features(&fc, &q, Some(2));
        assert_eq!(sel.len(), 2);
        assert_eq!(sel[0]["properties"]["name"], "a");
        let q = FeatureQuery::new(None, 2);
        let sel = select_features(&fc, &q, Some(5));
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0]["properties"]["name"], "c");
    }

    #[test]
    fn feature_ids_from_properties() {
        let fc = json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"FID":"7"},"geometry":{"type":"Point","coordinates":[0,0]}},
            {"type":"Feature","properties":{"OBJECTID":9},"geometry":{"type":"Point","coordinates":[0,0]}},
            {"type":"Feature","properties":{"name":"z"},"geometry":{"type":"Point","coordinates":[0,0]}}
        ]});
        assert_eq!(feature_id(&fc["features"][0], 0), "7");
        assert_eq!(feature_id(&fc["features"][1], 1), "9");
        assert_eq!(feature_id(&fc["features"][2], 2), "f2");
        assert_eq!(select_ids(&fc, &FeatureQuery::default(), None), vec!["7", "9", "f2"]);
    }

    #[test]
    fn find_by_id_works() {
        let fc = sample_fc();
        assert_eq!(find_by_id(&fc, "a").unwrap()["geometry"]["type"], "Point");
        assert!(find_by_id(&fc, "nope").is_none());
    }

    #[test]
    fn property_projection() {
        let fc = sample_fc();
        let f = &fc["features"][0];
        let p = project_properties(f, &["pop".to_string()]);
        assert_eq!(p["properties"]["pop"], 10);
        assert!(p["properties"].get("name").is_none());
        // Empty names -> identity.
        assert_eq!(project_properties(f, &[]), *f);
    }

    #[test]
    fn attribute_schema_merges_keys_and_widens_int_to_double() {
        let fc = sample_fc();
        let schema = attribute_schema(&fc);
        let map: std::collections::HashMap<_, _> = schema.into_iter().collect();
        assert_eq!(map.get("name").copied(), Some("xs:string"));
        assert_eq!(map.get("pop").copied(), Some("xs:double"));
    }

    #[test]
    fn mercator_roundtrip() {
        let (x, y) = from_4326_to_mercator(102.65, 25.22);
        let (lon, lat) = mercator_to_4326(x, y);
        assert!((lon - 102.65).abs() < 1e-9);
        assert!((lat - 25.22).abs() < 1e-9);
    }

    #[test]
    fn primary_geometry_type_inference() {
        let fc = sample_fc();
        assert_eq!(primary_geometry_type(&fc), Some("point"));
        let fc = json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[0,1],[0,0]]]}}
        ]});
        assert_eq!(primary_geometry_type(&fc), Some("polygon"));
    }}
