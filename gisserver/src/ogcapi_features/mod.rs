//! OGC API - Features - Part 1 (1.0.1): landing page, conformance classes,
//! collections and items. GeoJSON output plus a minimal HTML rendering of the
//! same resources (the `html` conformance class).
//!
//! Path layout follows GeoServer's OGC API modules: the landing page is the
//! mount point (`/ogc/features/v1`), collections live under `/collections`,
//! features under `/collections/{collectionId}/items[/{featureId}]`.

use serde_json::{Value, json};
use crate::wms_common::Bbox;
use crate::wms_common::crs::EPSG_4326;

use crate::ogcapi_features::error::OgcError;
use crate::ogcapi_features::query::{ItemsQuery, DEFAULT_LIMIT, MAX_LIMIT};

pub mod error;
pub mod html;
pub mod query;

/// Conformance classes this implementation claims (OGC API - Features 1.0.1).
pub const CONFORMANCE: &[&str] = &[
    "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/oas30",
    "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/html",
    "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/geojson",
];

/// A collection advertised by `/collections` (one `vector` layer).
#[derive(Clone, Debug)]
pub struct CollectionInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Spatial extent in WGS84 lon/lat.
    pub extent: Option<Bbox>,
    /// Rough geometry kind (point/line/polygon) for `itemType` hints.
    pub geometry_kind: Option<&'static str>,
    /// Number of features in the collection.
    pub count: usize,
}

impl CollectionInfo {
    /// Build from a layer name + FeatureCollection.
    pub fn from_fc(id: &str, title: &str, fc: &Value) -> Self {
        let extent = crate::vector_query::geom_bbox_fc(fc)
            .map(|[a, b, c, d]| Bbox::new(a, b, c, d));
        let geometry_kind = crate::vector_query::primary_geometry_type(fc);
        let count = crate::vector_query::features(fc).len();
        CollectionInfo {
            id: id.to_string(),
            title: if title.is_empty() { id.to_string() } else { title.to_string() },
            description: format!("Feature collection '{id}' (rust-gisserver vector layer)."),
            extent,
            geometry_kind,
            count,
        }
    }

    /// Build from parts for a data source without an in-memory
    /// FeatureCollection (PostGIS): extent in EPSG:4326 and the feature
    /// count probed at startup.
    pub fn from_parts(id: &str, title: &str, extent: Option<Bbox>, count: usize) -> Self {
        CollectionInfo {
            id: id.to_string(),
            title: if title.is_empty() { id.to_string() } else { title.to_string() },
            description: format!("Feature collection '{id}' (rust-gisserver PostGIS layer)."),
            extent,
            geometry_kind: None,
            count,
        }
    }
}

fn links(base: &str, extra: Vec<(&str, String)>) -> Vec<Value> {
    let mut v = vec![
        json!({"rel": "self", "href": base, "type": "application/json"}),
        json!({"rel": "alternate", "href": base, "type": "text/html"}),
    ];
    for (rel, href) in extra {
        v.push(json!({"rel": rel, "href": href, "type": "application/json"}));
    }
    v
}

/// `GET {base}` — the landing page.
pub fn landing(base: &str) -> String {
    let doc = json!({
        "title": "rust-gisserver OGC API - Features",
        "description": "Vector feature access over the rust-gisserver catalog (GeoJSON layers).",
        "links": links(base, vec![
            ("service-desc", format!("{base}/api")),
            ("conformance", format!("{base}/conformance")),
            ("data", format!("{base}/collections")),
        ]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/conformance` — the conformsTo declaration.
pub fn conformance() -> String {
    let doc = json!({ "conformsTo": CONFORMANCE });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/api` — a minimal OpenAPI 3.0 document describing the endpoints.
pub fn api_doc(base: &str) -> String {
    let paths = json!({
        "/": { "get": { "summary": "landing page", "responses": { "200": { "description": "landing page" } } } },
        "/conformance": { "get": { "summary": "conformance declaration", "responses": { "200": { "description": "conformance classes" } } } },
        "/collections": { "get": { "summary": "list collections", "responses": { "200": { "description": "collection list" } } } },
        "/collections/{collectionId}": { "get": { "summary": "collection metadata", "responses": { "200": { "description": "collection metadata" } } } },
        "/collections/{collectionId}/items": { "get": {
            "summary": "features (limit/offset/bbox)",
            "parameters": [
                { "name": "limit", "in": "query", "schema": { "type": "integer", "default": DEFAULT_LIMIT, "maximum": MAX_LIMIT } },
                { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } },
                { "name": "bbox", "in": "query", "schema": { "type": "string" } }
            ],
            "responses": { "200": { "description": "GeoJSON FeatureCollection" } }
        } },
        "/collections/{collectionId}/items/{featureId}": { "get": { "summary": "one feature", "responses": { "200": { "description": "GeoJSON Feature" } } } }
    });
    let doc = json!({
        "openapi": "3.0.3",
        "info": {
            "title": "rust-gisserver OGC API - Features",
            "version": "1.0.1",
            "description": "Feature access over the rust-gisserver catalog.",
        },
        "servers": [{ "url": base }],
        "paths": paths,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/collections` — the collection list.
pub fn collections(base: &str, cols: &[CollectionInfo]) -> String {
    let items: Vec<Value> = cols
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "title": c.title,
                "description": c.description,
                "extent": extent_json(&c.extent),
                "itemType": "feature",
                "links": links(
                    &format!("{base}/collections/{}", c.id),
                    vec![
                        ("collection", format!("{base}/collections")),
                        ("items", format!("{base}/collections/{}/items", c.id)),
                    ],
                ),
            })
        })
        .collect();
    let doc = json!({
        "collections": items,
        "links": links(base, vec![("data", format!("{base}/collections"))]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/collections/{collectionId}` — one collection's metadata.
pub fn collection(base: &str, c: &CollectionInfo) -> String {
    let self_url = format!("{base}/collections/{}", c.id);
    let doc = json!({
        "id": c.id,
        "title": c.title,
        "description": c.description,
        "extent": extent_json(&c.extent),
        "itemType": "feature",
        "crs": [
            format!("http://www.opengis.net/def/crs/EPSG/0/{}", EPSG_4326.trim_start_matches("EPSG:")),
            "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
        ],
        "links": links(&self_url, vec![
            ("collection", format!("{base}/collections")),
            ("items", format!("{self_url}/items")),
        ]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

fn extent_json(extent: &Option<Bbox>) -> Value {
    match extent {
        Some(b) => json!({
            "spatial": {
                "bbox": [[b.minx, b.miny, b.maxx, b.maxy]],
                "crs": "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
            }
        }),
        None => Value::Null,
    }
}

/// Best-effort display id for a feature in HTML listings.
pub fn vector_id(f: &Value) -> String {
    if let Some(v) = f.get("id").and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }) {
        return v;
    }
    if let Some(props) = f.get("properties").and_then(|p| p.as_object()) {
        for key in ["id", "fid", "FID", "objectid", "OBJECTID"] {
            if let Some(v) = props.get(key) {
                return match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
            }
        }
    }
    "?".to_string()
}

/// Resolved items page (counters + borrowed feature refs).
pub struct ItemsPage<'a> {
    pub matched: usize,
    pub returned: usize,
    pub features: Vec<&'a Value>,
    /// Offset for the next page, when one exists.
    pub next_offset: Option<usize>,
    pub query: ItemsQuery,
}

/// Apply an [`ItemsQuery`] to a collection.
pub fn page_features<'a>(fc: &'a Value, q: &ItemsQuery) -> ItemsPage<'a> {
    let query = crate::vector_query::FeatureQuery::new(
        q.bbox.map(|b| [b.minx, b.miny, b.maxx, b.maxy]),
        q.offset,
    );
    let matched = crate::vector_query::count_matches(fc, &query);
    let features = crate::vector_query::select_features(fc, &query, Some(q.limit));
    let returned = features.len();
    let next_offset = (q.offset + returned < matched).then_some(q.offset + returned);
    ItemsPage {
        matched,
        returned,
        features,
        next_offset,
        query: q.clone(),
    }
}

/// `GET {base}/collections/{collectionId}/items` — GeoJSON FeatureCollection.
pub fn items(base: &str, collection_id: &str, page: &ItemsPage<'_>) -> String {
    let self_url = format!("{base}/collections/{collection_id}/items");
    let mut extra: Vec<(&str, String)> = vec![
        ("collection", format!("{base}/collections/{collection_id}")),
    ];
    if let Some(next) = page.next_offset {
        extra.push((
            "next",
            format!("{self_url}?limit={}&offset={}", page.query.limit, next),
        ));
    }
    if page.query.offset > 0 {
        let prev = page.query.offset.saturating_sub(page.query.limit);
        extra.push(("prev", format!("{self_url}?limit={}&offset={}", page.query.limit, prev)));
    }
    let doc = json!({
        "type": "FeatureCollection",
        "numberMatched": page.matched,
        "numberReturned": page.returned,
        "features": page.features,
        "links": links(&self_url, extra),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// `GET {base}/collections/{collectionId}/items/{featureId}` — one feature.
pub fn item(base: &str, collection_id: &str, feature: &Value, feature_id: &str) -> String {
    let self_url = format!("{base}/collections/{collection_id}/items/{feature_id}");
    let doc = json!({
        "type": "Feature",
        "id": feature_id,
        "geometry": feature.get("geometry").cloned().unwrap_or(Value::Null),
        "properties": feature.get("properties").cloned().unwrap_or(json!({})),
        "links": links(&self_url, vec![
            ("collection", format!("{base}/collections/{collection_id}")),
            ("items", format!("{base}/collections/{collection_id}/items")),
        ]),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

/// OGC API error document (`application/json` problem style used by GeoServer).
pub fn error_json(err: &OgcError) -> String {
    let doc = json!({
        "code": err.code,
        "description": err.message,
    });
    serde_json::to_string(&doc).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fc() -> Value {
        json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"name":"a"},"geometry":{"type":"Point","coordinates":[100.0,30.0]}},
            {"type":"Feature","properties":{"name":"b"},"geometry":{"type":"Point","coordinates":[101.0,31.0]}},
            {"type":"Feature","properties":{"name":"c"},"geometry":{"type":"Point","coordinates":[102.0,32.0]}}
        ]})
    }

    #[test]
    fn landing_has_links() {
        let d: Value = serde_json::from_str(&landing("http://h/ogc/features/v1")).unwrap();
        let rels: Vec<&str> = d["links"].as_array().unwrap().iter().map(|l| l["rel"].as_str().unwrap()).collect();
        assert!(rels.contains(&"self"));
        assert!(rels.contains(&"conformance"));
        assert!(rels.contains(&"data"));
        assert!(rels.contains(&"service-desc"));
    }

    #[test]
    fn conformance_lists_core_classes() {
        let d: Value = serde_json::from_str(&conformance()).unwrap();
        let cs = d["conformsTo"].as_array().unwrap();
        assert!(cs.iter().any(|c| c.as_str().unwrap().ends_with("/conf/core")));
        assert!(cs.iter().any(|c| c.as_str().unwrap().ends_with("/conf/geojson")));
        assert!(cs.iter().any(|c| c.as_str().unwrap().ends_with("/conf/html")));
    }

    #[test]
    fn collection_info_from_fc() {
        let c = CollectionInfo::from_fc("places", "", &fc());
        assert_eq!(c.id, "places");
        assert_eq!(c.title, "places"); // empty title falls back to id
        assert_eq!(c.count, 3);
        assert_eq!(c.geometry_kind, Some("point"));
        let b = c.extent.unwrap();
        assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 102.0, 32.0]);
    }

    #[test]
    fn items_page_pagination_and_links() {
        let f = fc();
        let q = ItemsQuery { limit: 2, offset: 0, bbox: None };
        let page = page_features(&f, &q);
        assert_eq!(page.matched, 3);
        assert_eq!(page.returned, 2);
        assert_eq!(page.next_offset, Some(2));
        let d: Value = serde_json::from_str(&items("http://h", "places", &page)).unwrap();
        assert_eq!(d["numberMatched"], 3);
        assert_eq!(d["features"].as_array().unwrap().len(), 2);
        let rels: Vec<&str> = d["links"].as_array().unwrap().iter().map(|l| l["rel"].as_str().unwrap()).collect();
        assert!(rels.contains(&"next"), "{rels:?}");
        // second page
        let q = ItemsQuery { limit: 2, offset: 2, bbox: None };
        let page = page_features(&f, &q);
        assert_eq!(page.returned, 1);
        assert_eq!(page.next_offset, None);
    }

    #[test]
    fn items_bbox_filter() {
        let f = fc();
        let q = ItemsQuery { limit: 10, offset: 0, bbox: Some(Bbox::new(100.5, 30.5, 101.5, 31.5)) };
        let page = page_features(&f, &q);
        assert_eq!(page.matched, 1);
        assert_eq!(page.features[0]["properties"]["name"], "b");
    }

    #[test]
    fn item_document() {
        let f = fc();
        let feat = &f["features"][0];
        let d: Value = serde_json::from_str(&item("http://h", "places", feat, "a")).unwrap();
        assert_eq!(d["type"], "Feature");
        assert_eq!(d["id"], "a");
        assert_eq!(d["properties"]["name"], "a");
        assert_eq!(d["geometry"]["coordinates"][0], 100.0);
    }

    #[test]
    fn collections_document_shape() {
        let cols = vec![CollectionInfo::from_fc("places", "Places", &fc())];
        let d: Value = serde_json::from_str(&collections("http://h", &cols)).unwrap();
        assert_eq!(d["collections"][0]["id"], "places");
        assert_eq!(d["collections"][0]["itemType"], "feature");
        assert_eq!(d["collections"][0]["extent"]["spatial"]["bbox"][0][0], 100.0);
    }

    #[test]
    fn error_document() {
        let e = OgcError::not_found("collection 'nope'");
        let d: Value = serde_json::from_str(&error_json(&e)).unwrap();
        assert_eq!(d["code"], "NotFound");
        assert!(d["description"].as_str().unwrap().contains("nope"));
    }
}
