//! map-api: HTTP map server for WMS / WMTS / TMS / MVT / PMTiles / COG.
//!
//! A thin `main.rs` builds the runtime and router, with handlers split into
//! `handlers/*` and shared state in `state.rs`. Rendering/encoding logic
//! lives in the `crates/*` library crates.

pub mod admin;
pub mod apikeys;
pub mod cache;
pub mod catalog_import;
pub mod config;
pub mod db_catalog;
pub mod handlers;
pub mod i18n;
pub mod legend;
pub mod layer;
pub mod manager;
pub mod mbtiles;
pub mod state;
pub mod terrain;
pub mod tilesets;
pub mod tiles;
pub mod xmlcfg;

pub use state::AppState;
