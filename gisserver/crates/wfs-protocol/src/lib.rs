//! WFS 1.0.0 / 1.1.0 / 2.0.0 protocol: capabilities, DescribeFeatureType and
//! GetFeature (GML / GeoJSON / CSV) over in-memory GeoJSON feature
//! collections.
//!
//! Protocol logic only — HTTP wiring lives in the map-api service. GET KVP is
//! the only binding offered (same as the WMS/WMTS handlers); POST XML and
//! OGC Filter/FES are intentionally out of scope for this implementation.

pub mod caps;
pub mod dft;
pub mod error;
pub mod getfeature;

pub use caps::{CapFeatureType, DEFAULT_VERSION, SUPPORTED_VERSIONS, capabilities_xml, negotiate_version};
pub use dft::{GEOM_PROP, describe_feature_type_xml, xml_name};
pub use error::{WfsError, exception_report_xml, service_exception_xml};
pub use getfeature::{
    GetFeatureParams, OutputFormat, ResultType, feature_collection_csv, feature_collection_json,
    feature_collection_xml, gml_version_for, parse_get_feature, parse_output_format,
    resolve_features, resolve_features_owned,
};
