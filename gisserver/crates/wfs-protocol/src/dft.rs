//! WFS `DescribeFeatureType`: an XSD schema for one feature type, inferred
//! from the collection's GeoJSON attributes (mirrors GeoServer's
//! `DescribeFeatureType` response for GeoJSON-backed stores).

use vector_query::{attribute_schema, primary_geometry_type};

/// The GML property name every GeoJSON geometry is published under
/// (GeoJSON has no attribute name for geometry; this is the convention).
pub const GEOM_PROP: &str = "geometry";

/// XML-safe name check: NCName-ish (letters/digits/underscore, no leading
/// digit). Attribute names that fail become `attr_<n>`.
pub fn xml_name(name: &str) -> String {
    let first_ok = name
        .chars()
        .next()
        .map(|c| c.is_alphabetic() || c == '_')
        .unwrap_or(false);
    if first_ok && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
        name.to_string()
    } else {
        format!("attr_{}", name.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
    }
}

/// Build the DescribeFeatureType response: one `xsd:schema` describing
/// `type_name` (attributes inferred from `fc`, geometry property included).
pub fn describe_feature_type_xml(
    _base_url: &str,
    type_name: &str,
    fc: &serde_json::Value,
) -> String {
    let attrs = attribute_schema(fc);
    let geom_kind = primary_geometry_type(fc);
    let ns = "http://r-mapserver.local";
    let mut x = String::with_capacity(2048);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str(&format!(
        "<xsd:schema xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:gml=\"http://www.opengis.net/gml/3.2\" \
         xmlns:ws=\"{ns}\" targetNamespace=\"{ns}\" \
         elementFormDefault=\"qualified\">\n"
    ));
    x.push_str(&format!(
        "<xsd:element name=\"{t}\" type=\"ws:{t}Type\" \
         substitutionGroup=\"gml:AbstractFeature\"/>\n",
        t = super::caps::xml_escape(&xml_name(type_name)),
    ));
    x.push_str(&format!(
        "<xsd:complexType name=\"{t}Type\">\n\
         <xsd:complexContent>\n\
         <xsd:extension base=\"gml:AbstractFeatureType\">\n\
         <xsd:sequence>\n",
        t = super::caps::xml_escape(&xml_name(type_name)),
    ));
    // Geometry property first (GML convention), typed by the observed kind.
    if let Some(kind) = geom_kind {
        let gml_ty = match kind {
            "point" => "gml:PointPropertyType",
            "line" => "gml:CurvePropertyType",
            "polygon" => "gml:SurfacePropertyType",
            _ => "gml:GeometryPropertyType",
        };
        x.push_str(&format!(
            "<xsd:element name=\"{g}\" type=\"{gml_ty}\" minOccurs=\"0\" maxOccurs=\"1\"/>\n",
            g = GEOM_PROP,
        ));
    }
    for (name, ty) in &attrs {
        x.push_str(&format!(
            "<xsd:element name=\"{n}\" type=\"{ty}\" minOccurs=\"0\" maxOccurs=\"1\"/>\n",
            n = super::caps::xml_escape(&xml_name(name)),
        ));
    }
    x.push_str("</xsd:sequence>\n");
    x.push_str("</xsd:extension>\n");
    x.push_str("</xsd:complexContent>\n");
    x.push_str("</xsd:complexType>\n");
    x.push_str("</xsd:schema>\n");
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_has_geometry_and_attributes() {
        let fc = json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"name":"a","pop":10,"ok":true},"geometry":{"type":"Point","coordinates":[0,0]}}
        ]});
        let x = describe_feature_type_xml("http://x", "places", &fc);
        assert!(x.contains("targetNamespace=\"http://r-mapserver.local\""), "{x}");
        assert!(x.contains("<xsd:element name=\"places\" type=\"ws:placesType\""), "{x}");
        assert!(x.contains("name=\"geometry\" type=\"gml:PointPropertyType\""), "{x}");
        assert!(x.contains("name=\"name\" type=\"xs:string\""), "{x}");
        assert!(x.contains("name=\"pop\" type=\"xs:integer\""), "{x}");
        assert!(x.contains("name=\"ok\" type=\"xs:boolean\""), "{x}");
        assert!(x.contains("substitutionGroup=\"gml:AbstractFeature\""), "{x}");
    }

    #[test]
    fn polygon_geometry_typed_as_surface() {
        let fc = json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[0,1],[0,0]]]}}
        ]});
        let x = describe_feature_type_xml("http://x", "ws", &fc);
        assert!(x.contains("type=\"gml:SurfacePropertyType\""), "{x}");
    }

    #[test]
    fn xml_name_sanitizes_bad_names() {
        assert_eq!(xml_name("good_name"), "good_name");
        assert_eq!(xml_name("9bad"), "attr_9bad");
        assert_eq!(xml_name("with space"), "attr_withspace");
    }
}
