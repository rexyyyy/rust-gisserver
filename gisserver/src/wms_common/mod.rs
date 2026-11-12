//! Shared building blocks for the map server (structure follows JoeGCServices'
//! `crates/wms-common`): bounding boxes, CRS helpers, the tile-grid model, layer
//! and style types, and the crate-wide error.

pub mod bbox;
pub mod crs;
pub mod error;
pub mod layer;
pub mod pg_source_def;
pub mod reproject;
pub mod style;
pub mod tile;

pub use bbox::Bbox;
pub use pg_source_def::{PgSourceDef, SimplifyOpts};
pub use error::{MapServerError, Result};
pub use reproject::Reprojector;
pub use tile::{TileMatrixSet, TmLevel};
