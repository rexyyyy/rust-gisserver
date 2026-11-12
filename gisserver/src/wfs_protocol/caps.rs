//! WFS capabilities document (`GetCapabilities`) — WFS 1.0.0 / 1.1.0 / 2.0.0.
//!
//! Mirrors GeoServer's `CapabilitiesTransformer`: ServiceIdentification /
//! ServiceProvider / OperationsMetadata / FeatureTypeList, built from the
//! server's vector layers. Version differences live in element names and
//! namespaces (WFS 2.0 moved metadata under `ows:`).

use crate::wms_common::Bbox;

pub use crate::vector_query::gml::xml_escape;

/// The default WFS version advertised / served.
pub const DEFAULT_VERSION: &str = "2.0.0";
/// Every version this implementation can serve (highest first).
pub const SUPPORTED_VERSIONS: &[&str] = &["2.0.0", "1.1.0", "1.0.0"];

/// One published feature type (a `vector` layer).
#[derive(Clone, Debug)]
pub struct CapFeatureType {
    /// Layer name — the `TYPENAME=` value.
    pub name: String,
    pub title: String,
    /// Bounds in WGS84 lon/lat, when known.
    pub bounds_4326: Option<Bbox>,
    /// Source CRS as an EPSG URN (e.g. `urn:ogc:def:crs:EPSG::4326`).
    pub default_crs_urn: String,
}

/// Pick the best version from `AcceptVersions` (comma-separated), honoring the
/// list order; falls back to [`DEFAULT_VERSION`] when absent.
pub fn negotiate_version(accept_versions: Option<&str>) -> String {
    if let Some(list) = accept_versions {
        for v in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if SUPPORTED_VERSIONS.contains(&v) {
                return v.to_string();
            }
        }
    }
    DEFAULT_VERSION.to_string()
}

/// Build the full capabilities document.
pub fn capabilities_xml(base_url: &str, version: &str, feature_types: &[CapFeatureType]) -> String {
    let (wfs_ns, gml_ns, ows_ns, use_ows) = match version {
        "1.0.0" => (
            "http://www.opengis.net/wfs",
            "http://www.opengis.net/gml",
            "http://www.opengis.net/ows",
            false,
        ),
        "1.1.0" => (
            "http://www.opengis.net/wfs",
            "http://www.opengis.net/gml",
            "http://www.opengis.net/ows",
            true,
        ),
        _ => (
            "http://www.opengis.net/wfs/2.0",
            "http://www.opengis.net/gml/3.2",
            "http://www.opengis.net/ows/1.1",
            true,
        ),
    };
    let get_url = format!("{base_url}/wfs");
    let mut x = String::with_capacity(4096);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str(&format!(
        "<wfs:WFS_Capabilities version=\"{version}\" \
         xmlns:wfs=\"{wfs_ns}\" xmlns:ows=\"{ows_ns}\" \
         xmlns:gml=\"{gml_ns}\" xmlns:ogc=\"http://www.opengis.net/ogc\" \
         xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n"
    ));
    // ServiceIdentification
    if use_ows {
        x.push_str("<ows:ServiceIdentification>\n");
        x.push_str("<ows:Title>rust-gisserver WFS</ows:Title>\n");
        x.push_str("<ows:ServiceType>WFS</ows:ServiceType>\n");
        x.push_str("<ows:ServiceTypeVersion>2.0.0</ows:ServiceTypeVersion>\n");
        x.push_str("<ows:ServiceTypeVersion>1.1.0</ows:ServiceTypeVersion>\n");
        x.push_str("<ows:ServiceTypeVersion>1.0.0</ows:ServiceTypeVersion>\n");
        x.push_str("</ows:ServiceIdentification>\n");
    } else {
        x.push_str("<Service>\n");
        x.push_str("<Name>WFS</Name>\n");
        x.push_str("<Title>rust-gisserver WFS</Title>\n");
        x.push_str("</Service>\n");
    }
    // ServiceProvider
    x.push_str(&format!(
        "<{p}ServiceProvider>\n\
         <{p}ProviderName>rust-gisserver</{p}ProviderName>\n\
         <{p}ServiceContact>\n\
         <{p}IndividualName>rust-gisserver</{p}IndividualName>\n\
         </{p}ServiceContact>\n\
         </{p}ServiceProvider>\n",
        p = if use_ows { "ows:" } else { "" }
    ));
    // OperationsMetadata
    x.push_str(&format!(
        "<{p}OperationsMetadata>\n",
        p = if use_ows { "ows:" } else { "" }
    ));
    for (op, params) in [
        ("GetCapabilities", None),
        (
            "DescribeFeatureType",
            Some(vec![("TypeName", "xs:string")]),
        ),
        (
            "GetFeature",
            Some(vec![
                ("TypeName", "xs:string"),
                ("FeatureID", "xs:string"),
                ("OutputFormat", "xs:string"),
                ("ResultType", "xs:string"),
                ("Count", "xs:nonNegativeInteger"),
            ]),
        ),
    ] {
        x.push_str(&format!(
            "<{p}Operation name=\"{op}\">\n\
             <{p}DCP>\n\
             <{p}HTTP>\n\
             <{p}Get xlink:href=\"{url}\"/>\n\
             </{p}HTTP>\n\
             </{p}DCP>\n",
            p = if use_ows { "ows:" } else { "" },
            op = op,
            url = xml_escape(&get_url),
        ));
        if let Some(ps) = params {
            for (name, ty) in ps {
                x.push_str(&format!(
                    "<{p}Parameter name=\"{name}\">\n\
                     <{p}AllowedValues>\n\
                     <{p}Value>{ty}</{p}Value>\n\
                     </{p}AllowedValues>\n\
                     </{p}Parameter>\n",
                    p = if use_ows { "ows:" } else { "" },
                ));
            }
        }
        x.push_str(&format!(
            "</{p}Operation>\n",
            p = if use_ows { "ows:" } else { "" }
        ));
    }
    // Constraint: only GET KVP is offered.
    x.push_str(&format!(
        "<{p}Constraint name=\"GetEncoding\">\n\
         <{p}AllowedValues>\n\
         <{p}Value>KVP</{p}Value>\n\
         </{p}AllowedValues>\n\
         </{p}Constraint>\n\
         </{p}OperationsMetadata>\n",
        p = if use_ows { "ows:" } else { "" }
    ));
    // FeatureTypeList
    x.push_str("<wfs:FeatureTypeList>\n");
    for ft in feature_types {
        x.push_str("<wfs:FeatureType>\n");
        x.push_str(&format!(
            "<wfs:Name>{}</wfs:Name>\n",
            xml_escape(&ft.name)
        ));
        x.push_str(&format!(
            "<wfs:Title>{}</wfs:Title>\n",
            xml_escape(&ft.title)
        ));
        if !version.starts_with("2.") {
            x.push_str(&format!(
                "<wfs:SRS>{}</wfs:SRS>\n",
                xml_escape(&ft.default_crs_urn)
            ));
        }
        x.push_str(&format!(
            "<wfs:DefaultCRS>{}</wfs:DefaultCRS>\n",
            xml_escape(&ft.default_crs_urn)
        ));
        // CRS84 也要列出来：1.x 的 GetFeature 输出直接宣告它（见
        // crate::vector_query::gml::resolve_srs），能力文档漏宣就等于输出里出现了
        // 没宣告过的 CRS；2.x 客户端显式请求 srsName=CRS:84 同样需要声明做依据。
        let crs_el = if version.starts_with("2.") { "OtherCRS" } else { "SRS" };
        x.push_str(&format!(
            "<wfs:{el}>{crs}</wfs:{el}>\n",
            el = crs_el,
            crs = xml_escape(crate::vector_query::gml::CRS84_URN)
        ));
        if let Some(b) = &ft.bounds_4326 {
            x.push_str(&format!(
                "<ows:WGS84BoundingBox>\n\
                 <ows:LowerCorner>{:.6} {:.6}</ows:LowerCorner>\n\
                 <ows:UpperCorner>{:.6} {:.6}</ows:UpperCorner>\n\
                 </ows:WGS84BoundingBox>\n",
                b.minx, b.miny, b.maxx, b.maxy
            ));
        }
        x.push_str("</wfs:FeatureType>\n");
    }
    x.push_str("</wfs:FeatureTypeList>\n");
    if !version.starts_with("2.") {
        x.push_str("<wfs:Filter_Capabilities>\n");
        x.push_str("<ogc:Spatial_Capabilities>\n");
        x.push_str("<ogc:GeometryOperands>\n");
        x.push_str("<ogc:GeometryOperand>gml:Envelope</ogc:GeometryOperand>\n");
        x.push_str("</ogc:GeometryOperands>\n");
        x.push_str("<ogc:SpatialOperators>\n");
        x.push_str("<ogc:SpatialOperator name=\"BBOX\"/>\n");
        x.push_str("</ogc:SpatialOperators>\n");
        x.push_str("</ogc:Spatial_Capabilities>\n");
        x.push_str("</wfs:Filter_Capabilities>\n");
    }
    x.push_str("</wfs:WFS_Capabilities>\n");
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(version: &str) -> String {
        capabilities_xml(
            "http://localhost:8080",
            version,
            &[CapFeatureType {
                name: "places".into(),
                title: "Places".into(),
                bounds_4326: Some(Bbox::new(100.0, 30.0, 101.0, 31.0)),
                default_crs_urn: "urn:ogc:def:crs:EPSG::4326".into(),
            }],
        )
    }

    #[test]
    fn negotiate_prefers_accept_versions_order() {
        assert_eq!(negotiate_version(None), "2.0.0");
        assert_eq!(negotiate_version(Some("1.1.0,1.0.0")), "1.1.0");
        assert_eq!(negotiate_version(Some("0.9.0,2.0.0")), "2.0.0");
        // Unsupported-only list -> default.
        assert_eq!(negotiate_version(Some("3.0.0")), "2.0.0");
    }

    #[test]
    fn caps_20_lists_feature_type_and_bounds() {
        let x = caps("2.0.0");
        assert!(x.starts_with("<?xml"), "{x}");
        assert!(x.contains("version=\"2.0.0\""), "{x}");
        assert!(x.contains("<wfs:Name>places</wfs:Name>"), "{x}");
        assert!(x.contains("<wfs:DefaultCRS>urn:ogc:def:crs:EPSG::4326</wfs:DefaultCRS>"), "{x}");
        assert!(
            x.contains("<wfs:OtherCRS>urn:ogc:def:crs:OGC:1.3:CRS84</wfs:OtherCRS>"),
            "{x}"
        );
        assert!(x.contains("<ows:LowerCorner>100.000000 30.000000</ows:LowerCorner>"), "{x}");
        assert!(x.contains("xlink:href=\"http://localhost:8080/wfs\""), "{x}");
        assert!(x.contains("<ows:Operation name=\"GetFeature\">"), "{x}");
    }

    #[test]
    fn caps_11_uses_srs_and_filter_capabilities() {
        let x = caps("1.1.0");
        assert!(x.contains("<wfs:SRS>urn:ogc:def:crs:EPSG::4326</wfs:SRS>"), "{x}");
        assert!(
            x.contains("<wfs:SRS>urn:ogc:def:crs:OGC:1.3:CRS84</wfs:SRS>"),
            "{x}"
        );
        assert!(x.contains("<wfs:Filter_Capabilities>"), "{x}");
    }

    #[test]
    fn caps_10_legacy_service_element() {
        let x = caps("1.0.0");
        // WFS 1.0.0 has no ows: metadata: Service/Name/Title are bare elements.
        assert!(x.contains("<Service>"), "{x}");
        assert!(x.contains("<Name>WFS</Name>"), "{x}");
        assert!(x.contains("<wfs:WFS_Capabilities version=\"1.0.0\""), "{x}");
    }
}
