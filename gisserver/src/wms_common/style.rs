//! Layer style types.
//!
//! Raster layers carry an optional color style: either a value-domain palette
//! (min..max mapped onto a list of colors) or plain RGB passthrough. Vector
//! layers carry an optional style object that is served verbatim to MapLibre
//! clients; a minimal internal style drives WMS rasterization.

use serde::{Deserialize, Serialize};

/// Color style for a raster (COG) layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RasterStyle {
    /// Value range the palette maps onto. `None` = passthrough of RGB(A) bands.
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    /// Palette colors, each `[r, g, b]` in 0..=255.
    #[serde(default = "default_palette")]
    pub palette: Vec<[u8; 3]>,
    /// Source nodata value; pixels equal to it become transparent.
    #[serde(default)]
    pub nodata: Option<f64>,
}

impl Default for RasterStyle {
    fn default() -> Self {
        Self {
            min: None,
            max: None,
            palette: default_palette(),
            nodata: None,
        }
    }
}

fn default_palette() -> Vec<[u8; 3]> {
    vec![
        [0, 0, 0],
        [255, 255, 255],
    ]
}

/// The style file format (`config/styles/*.json`): `{"raster": {...}}` or
/// `{"vector": <object>}`. A plain object with a `"raster"` key is raster.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Style {
    #[serde(default)]
    pub raster: Option<RasterStyle>,
    /// For vector layers: the full style object served to clients and used by
    /// the built-in rasterizer (kept as raw JSON here).
    #[serde(default)]
    pub vector: Option<serde_json::Value>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            raster: Some(RasterStyle::default()),
            vector: None,
        }
    }
}
