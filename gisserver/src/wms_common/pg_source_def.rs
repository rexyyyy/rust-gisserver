//! PostGIS 图层源**配置定义**（`PgSourceDef` / `SimplifyOpts`）。
//!
//! 原属 pg-source crate；因公共基座 [`crate::wms_common::layer::LayerSource::Postgis`]
//! 需要引用该类型，而它只是纯 serde 配置结构体（无 tokio/连接池依赖），
//! 故下沉到本模块，保持「基础类型零重依赖、PostGIS 运行时可选」的依赖方向。
//! pg_source 模块经 `pub use` 再导出这些名字，旧调用点（`pg_source::PgSourceDef`）不变。

/// Default MVT tile extent (4096, the usual vector-tile coordinate space).
pub const DEFAULT_EXTENT: u32 = 4096;
/// Default tile buffer in tile units (64).
pub const DEFAULT_BUFFER: u32 = 64;
/// PostGIS version that introduced `ST_TileEnvelope` (required).
pub const MIN_POSTGIS_VERSION: &str = "3.0.0";

/// One PostGIS-backed layer source, as declared in the layer XML under
/// `source:` (with `type: postgis`).
///
/// One layer's connection, table, geometry and rendering knobs.
/// Everything has a default except `dsn`, `table` and `srid`; the field
/// set covers the PostGIS/MVT parameters this server needs.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PgSourceDef {
    /// Connection string, e.g.
    /// `postgres://postgres:123456@localhost:5434/gis`.
    pub dsn: String,
    /// Table schema (default `public`).
    #[serde(default = "default_public")]
    pub schema: String,
    /// Table (or view) name. Empty is allowed only for SQL/view layers
    /// (`from` set) — the FROM expression replaces it everywhere.
    #[serde(default)]
    pub table: String,
    /// Geometry column (default `geom`).
    #[serde(default = "default_geom")]
    pub geometry_column: String,
    /// Table SRID (e.g. 4326, 3857, 4543 for CGCS2000 GK CM 102E).
    pub srid: i32,
    /// MVT layer id inside the tile (defaults to the layer name at runtime).
    #[serde(default)]
    pub layer_id: Option<String>,
    /// Column used as the feature id (MVT feature id + WFS/OGC API id).
    #[serde(default)]
    pub id_column: Option<String>,
    /// Columns encoded as tile/feature properties. Empty = every non-geometry
    /// column discovered at startup.
    #[serde(default)]
    pub properties: Vec<String>,
    /// Minimum zoom served (advisory; also caps WMTS/TileJSON metadata).
    #[serde(default)]
    pub minzoom: Option<u8>,
    /// Maximum zoom served (advisory).
    #[serde(default)]
    pub maxzoom: Option<u8>,
    /// MVT extent in tile coordinate space (default 4096).
    #[serde(default)]
    pub extent: Option<u32>,
    /// MVT buffer in tile units (default 64).
    #[serde(default)]
    pub buffer: Option<u32>,
    /// Clip geometries to the tile envelope (default true).
    #[serde(default)]
    pub clip_geom: Option<bool>,
    /// Max features per tile (unbounded by default).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Data extent `[minx, miny, maxx, maxy]` in EPSG:4326. Computed from the
    /// table (`ST_Extent`) when absent.
    #[serde(default)]
    pub bounds: Option<[f64; 4]>,
    /// Feature count probed at startup (`count(*)`); `None` when not probed.
    /// Not part of the layer XML config — runtime-filled by the service layer.
    #[serde(skip)]
    pub feature_count: Option<u64>,
    /// SQL/view layer (P3): a FROM expression used verbatim instead of
    /// `"schema"."table"` — a view name or a parenthesized subquery **with an
    /// alias**, e.g. `(SELECT gid, height, geom FROM public.v_tall) AS v`.
    /// Admin-trusted SQL: every tile/feature/count/bounds template reuses it;
    /// `;` is rejected by [`PgSourceDef::validate`] at startup so a config
    /// typo can never smuggle a second statement.
    #[serde(default)]
    pub from: Option<String>,
    /// Zoom-based geometry simplification (P3): for `z < under_zoom` the tile
    /// SQL wraps geometry in `ST_SimplifyPreserveTopology` with a per-tile
    /// tolerance of `(envelope_width / extent) * tile_units` map units — the
    /// envelope width travels as a SQL expression, so the prepared statement
    /// stays shared across zoom levels.
    #[serde(default)]
    pub simplify: Option<SimplifyOpts>,
}

/// Zoom-based simplification knobs ([`PgSourceDef::simplify`]).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimplifyOpts {
    /// Apply simplification only for zoom levels strictly below this value.
    pub under_zoom: u8,
    /// Tolerance in tile coordinate units (extent space); `2.0` with the
    /// default extent 4096 means ~0.05% of the tile width.
    #[serde(default = "default_tile_units")]
    pub tile_units: f64,
}

pub fn default_tile_units() -> f64 {
    2.0
}

pub fn default_public() -> String {
    "public".to_string()
}

pub fn default_geom() -> String {
    "geom".to_string()
}

impl PgSourceDef {
    /// Startup sanity checks. An `Err` aborts service startup: a misconfigured
    /// `from` expression is a deploy-time bug, not a per-request one.
    pub fn validate(&self) -> Result<(), String> {
        if self.from.is_none() && self.table.trim().is_empty() {
            return Err("postgis source needs either `table` or `from`".into());
        }
        if let Some(f) = &self.from {
            if f.trim().is_empty() {
                return Err("postgis `from` is set but empty".into());
            }
            if f.contains(';') {
                return Err("postgis `from` must be a single expression (no ';')".into());
            }
        }
        if let Some(s) = &self.simplify {
            if !(s.tile_units.is_finite() && s.tile_units > 0.0) {
                return Err("postgis simplify.tile_units must be a finite value > 0".into());
            }
        }
        Ok(())
    }

    /// Layer id used inside MVT tiles and TileJSON.
    pub fn layer_id_or(&self, fallback: &str) -> String {
        self.layer_id.clone().unwrap_or_else(|| fallback.to_string())
    }

    pub fn extent(&self) -> u32 {
        self.extent.unwrap_or(DEFAULT_EXTENT).max(1)
    }

    pub fn buffer(&self) -> u32 {
        self.buffer.unwrap_or(DEFAULT_BUFFER)
    }

    pub fn clip_geom(&self) -> bool {
        self.clip_geom.unwrap_or(true)
    }

    /// Key identifying the connection this source needs, used by
    /// [`PgRegistry`] to share one pool across layers.
    pub fn pool_key(&self) -> String {
        // Same DSN => same pool. The key is the raw DSN; layers configured
        // with textually identical DSNs are expected to share credentials.
        self.dsn.clone()
    }
}
