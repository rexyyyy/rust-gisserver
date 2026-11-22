//! OGC protocol layer (WMS + WMTS).

pub mod kvp;
pub mod wms;
pub mod wmts;

pub use kvp::parse_kvp;
