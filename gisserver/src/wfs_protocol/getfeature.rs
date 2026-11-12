//! WFS `GetFeature` / `GetFeatureWithLock` parameter parsing and response
//! assembly (GML 3.1/3.2, GeoJSON, CSV).

use crate::wms_common::Bbox;
use crate::wms_common::crs::{is_northing_first, normalize};

use crate::wfs_protocol::error::WfsError;
use crate::vector_query::gml::{self, GmlVersion};
use crate::vector_query::csv::csv as csv_enc;
use crate::vector_query::{FeatureQuery, find_by_id, project_properties};

/// Supported output formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    /// `application/gml+xml` — version follows the WFS version.
    Gml,
    /// `application/json` — GeoJSON FeatureCollection.
    Json,
    /// `text/csv`.
    Csv,
}

impl OutputFormat {
    pub fn content_type(&self) -> &'static str {
        match self {
            OutputFormat::Gml => "application/gml+xml; charset=utf-8",
            OutputFormat::Json => "application/json",
            OutputFormat::Csv => "text/csv; charset=utf-8",
        }
    }
}

/// `RESULTTYPE`: full features, or just the hit count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultType {
    Results,
    Hits,
}

/// A parsed + validated `GetFeature` request.
#[derive(Clone, Debug)]
pub struct GetFeatureParams {
    pub version: String,
    pub type_names: Vec<String>,
    pub feature_ids: Vec<String>,
    /// Query bbox in lon/lat (converted from the request CRS).
    pub bbox: Option<Bbox>,
    /// `COUNT` (2.0) / `MAXFEATURES` (1.x).
    pub count: Option<usize>,
    pub start_index: usize,
    /// 请求里的**原始** CRS 标识（不归一化）。轴序与输出 srsName 的判定必须看
    /// 原始写法，因为 `crs::normalize` 会把 CRS:84 折叠到 EPSG:4326（后者注册表
    /// 轴序是纬度在前），一旦归一化就分不清客户端到底要哪个序。
    pub srs_name: String,
    pub output_format: OutputFormat,
    pub property_names: Vec<String>,
    pub result_type: ResultType,
}

/// Parse the output format, defaulting to GML for the negotiated version.
pub fn parse_output_format(fmt: Option<&str>, version: &str) -> Result<OutputFormat, WfsError> {
    let f = fmt.unwrap_or("application/gml+xml").trim().to_ascii_lowercase();
    Ok(match f.as_str() {
        "" | "application/gml+xml" | "application/gml+xml; version=3.2" | "gml32" => OutputFormat::Gml,
        "gml2" | "application/xml; subtype=gml/3.1.1" | "text/xml; subtype=gml/3.1.1"
            if !version.starts_with("2.") => OutputFormat::Gml,
        "application/json" | "geojson" => OutputFormat::Json,
        "text/csv" => OutputFormat::Csv,
        other => return Err(WfsError::bad_format(other)),
    })
}

/// Parse the 4-number BBOX parameter with CRS-appropriate axis order
/// (EPSG:4326 is northing-first per the WFS 2.0 spec; CRS:84 is lon/lat).
///
/// `srs_raw` is the request's raw CRS identifier and `srs` its normalized
/// form: axis order must consult the raw one, because `normalize` folds
/// CRS:84 onto EPSG:4326 (which IS northing-first).
fn parse_bbox(raw: &str, srs_raw: &str, srs: &str) -> Result<Bbox, WfsError> {
    let nums: Result<Vec<f64>, _> = raw
        .trim()
        .split(',')
        .map(|s| s.trim().parse::<f64>())
        .collect();
    let nums = nums.map_err(|_| WfsError::invalid("bbox", "not four comma-separated numbers"))?;
    if nums.len() < 4 {
        return Err(WfsError::invalid("bbox", "expected four numbers"));
    }
    let northing_first = is_northing_first(srs) && !crate::wms_common::crs::is_crs84(srs_raw);
    let b = if northing_first {
        Bbox::new(nums[1], nums[0], nums[3], nums[2])
    } else {
        Bbox::new(nums[0], nums[1], nums[2], nums[3])
    };
    if b.minx > b.maxx || b.miny > b.maxy {
        return Err(WfsError::invalid("bbox", "bbox is inverted"));
    }
    Ok(b)
}

/// Parse and validate `GetFeature` KVP parameters.
///
/// `vector_layers` are the published feature-type names; only one typeName is
/// served per request (rust-gisserver layers are flat, no joins).
pub fn parse_get_feature(
    version: &str,
    kv: &[(String, String)],
    vector_layers: &[String],
) -> Result<GetFeatureParams, WfsError> {
    let get = |k: &str| crate::wfs_protocol::getfeature::kvp_lookup(kv, k);
    let is20 = version.starts_with("2.");

    // TYPENAMES is the WFS 2.0 spelling; TYPENAME the 1.x one. Accept both
    // (lenient, like GeoServer's KVP dispatcher). A namespace prefix
    // (`ws:places`) is stripped: rust-gisserver layers are flat.
    let type_names: Vec<String> = get("typeNames")
        .or_else(|| get("typeName"))
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().rsplit(':').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if type_names.is_empty() {
        return Err(WfsError::missing(if is20 { "typeNames" } else { "typeName" }));
    }
    for t in &type_names {
        if !vector_layers.iter().any(|l| l == t) {
            return Err(WfsError::unknown_type(t));
        }
    }

    let feature_ids: Vec<String> = get("featureId")
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // BBOX: WFS 2.0 pairs it with SRSNAME; the 1.x BBOX parameter has the
    // same axis-order rule.
    let srs_raw = get("srsName").unwrap_or("EPSG:4326");
    let srs = normalize(srs_raw);
    let bbox = match get("bbox") {
        Some(raw) if !raw.trim().is_empty() => Some(parse_bbox(raw, srs_raw, &srs)?),
        _ => None,
    };

    // COUNT (2.0) / MAXFEATURES (1.x).
    let count_raw = get("count").or_else(|| get("maxFeatures"));
    let count = match count_raw {
        Some(c) if !c.trim().is_empty() => Some(
            c.trim()
                .parse::<usize>()
                .map_err(|_| WfsError::invalid("count", "not a non-negative integer"))?,
        ),
        _ => None,
    };

    let start_index = match get("startIndex") {
        Some(s) if !s.trim().is_empty() => s
            .trim()
            .parse::<usize>()
            .map_err(|_| WfsError::invalid("startIndex", "not a non-negative integer"))?,
        _ => 0,
    };

    let output_format = parse_output_format(get("outputFormat"), version)?;

    let property_names: Vec<String> = get("propertyName")
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let result_type = match get("resultType") {
        Some("hits") => ResultType::Hits,
        Some("results") | None | Some("") => ResultType::Results,
        Some(other) => {
            return Err(WfsError::invalid(
                "resultType",
                format!("expected 'results' or 'hits', got '{other}'"),
            ))
        }
    };

    Ok(GetFeatureParams {
        version: version.to_string(),
        type_names,
        feature_ids,
        bbox,
        count,
        start_index,
        srs_name: srs_raw.to_string(),
        output_format,
        property_names,
        result_type,
    })
}

/// Resolve the selected features for a parsed request against one collection.
/// Returns `(matched_total, selected)`; `selected` is empty for `hits`.
pub fn resolve_features<'a>(
    fc: &'a serde_json::Value,
    p: &GetFeatureParams,
) -> (usize, Vec<&'a serde_json::Value>) {
    // featureId filter takes precedence over bbox.
    if !p.feature_ids.is_empty() {
        let matched = p.feature_ids.len();
        let selected = p
            .feature_ids
            .iter()
            .filter_map(|id| find_by_id(fc, id))
            .skip(p.start_index)
            .take(p.count.unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        return (matched, selected);
    }
    let q = FeatureQuery::new(
        p.bbox.map(|b| [b.minx, b.miny, b.maxx, b.maxy]),
        p.start_index,
    );
    let matched = crate::vector_query::count_matches(fc, &q);
    let selected = if p.result_type == ResultType::Hits {
        Vec::new()
    } else {
        let sel = crate::vector_query::select_features(fc, &q, p.count);
        if p.property_names.is_empty() {
            sel
        } else {
            // Projection changes the shape: own the projected values. To keep
            // the borrow, store projections in a side buffer and rebuild refs
            // via a leaked-free approach is impossible for &Value; so callers
            // that need projections use `resolve_features_owned` instead.
            // Here the un-projected refs are returned and the handler falls
            // back when propertyName is set.
            sel
        }
    };
    (matched, selected)
}

/// Owned variant used when `propertyName` projection is active (the projected
/// features cannot borrow from the source collection).
pub fn resolve_features_owned(
    fc: &serde_json::Value,
    p: &GetFeatureParams,
) -> (usize, Vec<serde_json::Value>) {
    let (matched, refs) = resolve_features(fc, p);
    if p.property_names.is_empty() {
        (matched, refs.into_iter().cloned().collect())
    } else {
        (
            matched,
            refs.into_iter()
                .map(|f| project_properties(f, &p.property_names))
                .collect(),
        )
    }
}

/// GML version implied by the WFS version.
pub fn gml_version_for(version: &str) -> GmlVersion {
    if version.starts_with("2.") {
        GmlVersion::V32
    } else {
        GmlVersion::V31
    }
}

/// Assemble the full `wfs:FeatureCollection` document.
///
/// `features` must already be resolved/filtered. `matched` is the total the
/// filters would return (numberMatched / numberOfFeatures). `offset` is the
/// request's startIndex, used for positional id fallbacks. `srs_requested` is
/// the request's raw CRS identifier: it decides the emitted `srsName` and the
/// coordinate order ([`crate::vector_query::gml::resolve_srs`] / [`crate::vector_query::gml::northing_first`]).
pub fn feature_collection_xml(
    version: &str,
    type_name: &str,
    features: &[&serde_json::Value],
    matched: usize,
    offset: usize,
    srs_requested: &str,
) -> String {
    let gml = gml_version_for(version);
    // WFS 2.x 才明确要求按 CRS 注册表轴序输出；1.0.0/1.1.0 的业界惯例是一律经度
    // 在前，那就直接宣告 CRS84，而不是“宣告 4326 却写经度在前”。
    let out_srs = gml::resolve_srs(srs_requested, gml, version.starts_with("2."));
    let (wfs_ns, gml_ns) = if version.starts_with("2.") {
        ("http://www.opengis.net/wfs/2.0", gml.ns())
    } else {
        ("http://www.opengis.net/wfs", gml.ns())
    };
    let count_attr = if version.starts_with("2.") {
        format!(
            "numberMatched=\"{matched}\" numberReturned=\"{n}\"",
            n = features.len()
        )
    } else {
        format!("numberOfFeatures=\"{}\"", features.len())
    };
    let mut x = String::with_capacity(4096);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str(&format!(
        "<wfs:FeatureCollection {count_attr} \
         xmlns:wfs=\"{wfs_ns}\" xmlns:gml=\"{gml_ns}\" \
         xmlns:ws=\"http://rust-gisserver.local\">\n"
    ));
    for (i, f) in features.iter().enumerate() {
        let id = crate::vector_query::feature_id(f, offset + i);
        x.push_str("<wfs:member>\n");
        x.push_str(&format!(
            "<ws:{t} gml:id=\"{t}.{id}\">\n",
            t = crate::wfs_protocol::dft::xml_name(type_name),
            id = gml::xml_escape(&id),
        ));
        if let Some(g) = f.get("geometry") {
            let body = gml::geometry_xml(g, gml, &out_srs);
            if !body.is_empty() {
                x.push_str(&format!("<ws:{p}>{body}</ws:{p}>\n", p = crate::wfs_protocol::dft::GEOM_PROP));
            }
        }
        if let Some(props) = f.get("properties").and_then(|p| p.as_object()) {
            for (k, v) in props {
                if k == crate::wfs_protocol::dft::GEOM_PROP {
                    continue;
                }
                x.push_str(&format!(
                    "<ws:{k}>{v}</ws:{k}>\n",
                    k = crate::wfs_protocol::dft::xml_name(k),
                    v = gml::xml_escape(&value_text(v)),
                ));
            }
        }
        x.push_str(&format!(
            "</ws:{t}>\n",
            t = crate::wfs_protocol::dft::xml_name(type_name)
        ));
        x.push_str("</wfs:member>\n");
    }
    x.push_str("</wfs:FeatureCollection>\n");
    x
}

/// GeoJSON output: the features plus WFS match counters at the top level.
pub fn feature_collection_json(
    features: &[&serde_json::Value],
    matched: usize,
) -> String {
    let fc = serde_json::json!({
        "type": "FeatureCollection",
        "numberMatched": matched,
        "numberReturned": features.len(),
        "features": features,
    });
    serde_json::to_string(&fc).unwrap_or_else(|_| "{}".to_string())
}

/// CSV output.
pub fn feature_collection_csv(
    features: &[&serde_json::Value],
    columns: &[String],
) -> String {
    csv_enc(features, columns)
}

/// JSON scalar text for a GML property element (numbers/bools bare, strings
/// escaped, arrays/objects as JSON text).
fn value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Local re-export so `parse_get_feature` can share the KVP lookup.
pub fn kvp_lookup<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    crate::wms_protocol::kvp::lookup(kv, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fc() -> serde_json::Value {
        json!({"type":"FeatureCollection","features":[
            {"type":"Feature","id":"1","properties":{"name":"a","pop":10},"geometry":{"type":"Point","coordinates":[100.0,30.0]}},
            {"type":"Feature","id":"2","properties":{"name":"b","pop":20},"geometry":{"type":"LineString","coordinates":[[101.0,31.0],[102.0,32.0]]}},
            {"type":"Feature","id":"3","properties":{"name":"c","pop":30},"geometry":{"type":"Point","coordinates":[103.0,33.0]}}
        ]})
    }

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_uppercase(), v.to_string()))
            .collect()
    }

    #[test]
    fn parse_defaults_and_bbox_axis_order() {
        let layers = ["places".to_string()];
        // WFS 2.0: EPSG:4326 is northing-first -> swap.
        let p = parse_get_feature(
            "2.0.0",
            &kv(&[("typeNames", "places"), ("bbox", "30,100,31,101")]),
            &layers,
        )
        .unwrap();
        let b = p.bbox.unwrap();
        assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 101.0, 31.0]);
        // CRS:84 is lon/lat -> no swap.
        let p = parse_get_feature(
            "2.0.0",
            &kv(&[("typeNames", "places"), ("bbox", "100,30,101,31"), ("srsName", "CRS:84")]),
            &layers,
        )
        .unwrap();
        let b = p.bbox.unwrap();
        assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 101.0, 31.0]);
        // srs_name 必须留原始写法（归一化会把它折叠成 EPSG:4326，输出宣告就分不清了）
        assert_eq!(p.srs_name, "CRS:84");
    }

    #[test]
    fn parse_count_startindex_and_format() {
        let layers = ["places".to_string()];
        let p = parse_get_feature(
            "2.0.0",
            &kv(&[
                ("typeNames", "places"),
                ("count", "2"),
                ("startIndex", "1"),
                ("outputFormat", "application/json"),
                ("resultType", "hits"),
            ]),
            &layers,
        )
        .unwrap();
        assert_eq!(p.count, Some(2));
        assert_eq!(p.start_index, 1);
        assert_eq!(p.output_format, OutputFormat::Json);
        assert_eq!(p.result_type, ResultType::Hits);
    }

    #[test]
    fn parse_unknown_type_and_bad_format() {
        let layers = ["places".to_string()];
        assert!(parse_get_feature("2.0.0", &kv(&[("typeNames", "nope")]), &layers).is_err());
        let e = parse_get_feature(
            "2.0.0",
            &kv(&[("typeNames", "places"), ("outputFormat", "image/png")]),
            &layers,
        )
        .unwrap_err();
        assert_eq!(e.code, "InvalidParameterValue");
    }

    #[test]
    fn feature_collection_xml_20_gml32() {
        let f = fc();
        let (matched, sel) = resolve_features(&f, &GetFeatureParams {
            version: "2.0.0".into(),
            type_names: vec!["places".into()],
            feature_ids: vec![],
            bbox: None,
            count: None,
            start_index: 0,
            srs_name: "EPSG:4326".into(),
            output_format: OutputFormat::Gml,
            property_names: vec![],
            result_type: ResultType::Results,
        });
        assert_eq!(matched, 3);
        let x = feature_collection_xml("2.0.0", "places", &sel, matched, 0, "EPSG:4326");
        assert!(x.contains("numberMatched=\"3\" numberReturned=\"3\""), "{x}");
        assert!(x.contains("<wfs:member>"), "{x}");
        assert!(x.contains("<ws:places gml:id=\"places.1\">"), "{x}");
        assert!(x.contains("<ws:geometry><gml:Point srsName=\"urn:ogc:def:crs:EPSG::4326\""), "{x}");
        // 2.0 按注册表轴序：宣告 4326 就得写纬度在前
        assert!(x.contains("<gml:pos>30 100</gml:pos>"), "{x}");
        assert!(x.contains("<ws:name>a</ws:name>"), "{x}");
        assert!(x.contains("<ws:pop>10</ws:pop>"), "{x}");
        assert!(x.contains("<gml:LineString"), "{x}");
    }

    #[test]
    fn feature_collection_xml_11_gml31() {
        let f = fc();
        let (_, sel) = resolve_features(&f, &GetFeatureParams {
            version: "1.1.0".into(),
            type_names: vec!["places".into()],
            feature_ids: vec![],
            bbox: None,
            count: None,
            start_index: 0,
            srs_name: "EPSG:4326".into(),
            output_format: OutputFormat::Gml,
            property_names: vec![],
            result_type: ResultType::Results,
        });
        let x = feature_collection_xml("1.1.0", "places", &sel, 3, 0, "EPSG:4326");
        assert!(x.contains("numberOfFeatures=\"3\""), "{x}");
        assert!(x.contains("<gml:coordinates>100,30</gml:coordinates>"), "{x}");
        // 1.x 不按注册表轴序，所以不再“宣告 4326 却写经度在前”，改宣告 CRS84
        assert!(x.contains("srsName=\"urn:ogc:def:crs:OGC:1.3:CRS84\""), "{x}");
    }

    /// 请求的 CRS 决定输出宣告与书写序（两者永远同序）。
    #[test]
    fn requested_srs_drives_declared_axis_order() {
        let f = fc();
        let one: Vec<&serde_json::Value> = vec![&f["features"][0]];
        // 要 CRS:84 → 原样尊重，经度在前
        let x = feature_collection_xml("2.0.0", "places", &one, 1, 0, "CRS:84");
        assert!(x.contains("srsName=\"urn:ogc:def:crs:OGC:1.3:CRS84\""), "{x}");
        assert!(x.contains("<gml:pos>100 30</gml:pos>"), "{x}");
        // 要 4490（另一个地理 datum）而不做重投影 → 不谎报，落回 4326 纬度在前
        let x = feature_collection_xml("2.0.0", "places", &one, 1, 0, "urn:ogc:def:crs:EPSG::4490");
        assert!(x.contains("srsName=\"urn:ogc:def:crs:EPSG::4326\""), "{x}");
        assert!(x.contains("<gml:pos>30 100</gml:pos>"), "{x}");
        // 要投影系 → 米制标签不能贴在不重投影的度数上，同样落回 4326
        let x = feature_collection_xml("2.0.0", "places", &one, 1, 0, "EPSG:3857");
        assert!(x.contains("srsName=\"urn:ogc:def:crs:EPSG::4326\""), "{x}");
        assert!(x.contains("<gml:pos>30 100</gml:pos>"), "{x}");
        // 1.0.0 → 经度在前 + 宣告 CRS84
        let x = feature_collection_xml("1.0.0", "places", &one, 1, 0, "EPSG:4326");
        assert!(x.contains("srsName=\"urn:ogc:def:crs:OGC:1.3:CRS84\""), "{x}");
        assert!(x.contains("<gml:coordinates>100,30</gml:coordinates>"), "{x}");
    }

    #[test]
    fn featureid_filter_and_hits() {
        let f = fc();
        let p = GetFeatureParams {
            version: "2.0.0".into(),
            type_names: vec!["places".into()],
            feature_ids: vec!["2".into(), "3".into()],
            bbox: None,
            count: None,
            start_index: 0,
            srs_name: "EPSG:4326".into(),
            output_format: OutputFormat::Gml,
            property_names: vec![],
            result_type: ResultType::Results,
        };
        let (matched, sel) = resolve_features(&f, &p);
        assert_eq!(matched, 2);
        assert_eq!(sel.len(), 2);
        assert_eq!(crate::vector_query::feature_id(sel[0], 0), "2");
    }

    #[test]
    fn json_and_csv_output() {
        let f = fc();
        let (_, sel) = resolve_features(&f, &GetFeatureParams {
            version: "2.0.0".into(),
            type_names: vec!["places".into()],
            feature_ids: vec![],
            bbox: None,
            count: Some(2),
            start_index: 0,
            srs_name: "EPSG:4326".into(),
            output_format: OutputFormat::Json,
            property_names: vec![],
            result_type: ResultType::Results,
        });
        let j = feature_collection_json(&sel, 3);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["type"], "FeatureCollection");
        assert_eq!(v["numberMatched"], 3);
        assert_eq!(v["numberReturned"], 2);
        let c = feature_collection_csv(&sel, &["name".into(), "pop".into()]);
        assert_eq!(c, "name,pop\na,10\nb,20\n");
    }

    #[test]
    fn owned_projection_drops_unselected_properties() {
        let f = fc();
        let p = GetFeatureParams {
            version: "2.0.0".into(),
            type_names: vec!["places".into()],
            feature_ids: vec![],
            bbox: None,
            count: None,
            start_index: 0,
            srs_name: "EPSG:4326".into(),
            output_format: OutputFormat::Json,
            property_names: vec!["pop".into()],
            result_type: ResultType::Results,
        };
        let (_, owned) = resolve_features_owned(&f, &p);
        assert_eq!(owned.len(), 3);
        assert_eq!(owned[0]["properties"]["pop"], 10);
        assert!(owned[0]["properties"].get("name").is_none());
    }
}
