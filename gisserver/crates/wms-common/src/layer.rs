//! Published-layer configuration: one XML file per layer under the catalog,
//! or a legacy inline `layers:` entry in `mapserver.xml` (both go through the
//! same serde structs via the `xmlcfg` XML -> JSON bridge).

use serde::{Deserialize, Serialize};

use crate::Bbox;

/// What data backs a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceKind {
    /// Cloud-Optimized GeoTIFF: local path or `http(s)://` URL (range reads).
    Cog { path: String },
    /// Vector source: GeoJSON (`FeatureCollection`), EPSG:4326.
    Vector { path: String },
    /// Read-through PMTiles archive (MVT or PNG tiles).
    Pmtiles { path: String },
    /// MBTiles SQLite archive (PNG/JPEG raster or MVT tiles, TMS rows).
    Mbtiles { path: String },
    /// ArcGIS server tile cache (compact cache v1/v2) read-through. `path` is
    /// the cache service directory holding `Conf.xml` (or its parent, whose one
    /// level of layer sub-directories is probed). The tile addressing and the
    /// zoom ladder come from `Conf.xml` itself, so such a layer must publish on
    /// a grid derived from it (see the `arcgis-cache` crate), not a preset.
    Arcgis { path: String },
    /// PostGIS table: dynamic vector tiles + feature services. All fields
    /// live in [`pg_source::PgSourceDef`] (dsn/table/srid/properties/...).
    /// The connection pool is built **only** when at least one layer uses
    /// this type; with no postgis layer nothing PG-related is created.
    Postgis(pg_source::PgSourceDef),
}

/// One published layer. Mirrors JoeGCServices' `LayerConfig` shape: a named
/// layer, its data source, source CRS, style and the grids it publishes on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    /// WMS layer name (the `LAYERS=` / `LAYER=` value).
    pub name: String,
    /// Human-readable title for capabilities documents.
    #[serde(default)]
    pub title: String,
    /// One of: cog / vector / pmtiles / mbtiles / arcgis / postgis.
    pub source: SourceKind,
    /// Source CRS of the raster/vector data.
    #[serde(default = "default_src_crs")]
    pub src_crs: String,
    /// Style id or inline style path (raster palette / vector style JSON).
    #[serde(default)]
    pub style: Option<String>,
    /// SLD style file for **vector** layers (WMS GetMap rasterization).
    /// Auto-discovered as a `<layer-name>.sld` sibling of the layer config file
    /// when not set explicitly; relative paths resolve against the layer file's
    /// directory. Ignored by raster layers.
    #[serde(default)]
    pub sld: Option<String>,
    /// Tile grids this layer publishes on (WMTS/TMS/MVT). Presets:
    /// `WebMercatorQuad` / `WorldCRS84Quad`, or a custom id from the top-level `grids:` map.
    #[serde(default = "default_grids")]
    pub grids: Vec<String>,
    /// Tile pixel size for preset grids (128/256/512). Default 256.
    #[serde(default = "default_tile_px")]
    pub tile_px: u32,
    /// Advertised bounds in the source CRS `[minx, miny, maxx, maxy]`.
    /// Computed from the data when absent.
    #[serde(default)]
    pub extent: Option<[f64; 4]>,
}

impl Layer {
    pub fn bounds(&self) -> Option<Bbox> {
        self.extent.map(Bbox::from_array)
    }
}

pub fn default_src_crs() -> String {
    crate::crs::EPSG_4326.to_string()
}

pub fn default_grids() -> Vec<String> {
    vec!["WebMercatorQuad".to_string()]
}

pub fn default_tile_px() -> u32 {
    256
}
