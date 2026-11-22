//! Runtime layer: a configured layer plus its opened sources.

use std::path::Path;
use std::sync::Arc;

use gisserver::cog::Cog;
use gisserver::pmtiles::PmtilesReader;
use gisserver::projection::Transformer;
use gisserver::renderer::sld::SldDocument;
use gisserver::renderer::vector::VectorStyle;
use gisserver::storage::RangeSource;
use gisserver::wms_common::crs::EPSG_3857;
use gisserver::wms_common::layer::{Layer, SourceKind};
use gisserver::wms_common::style::Style;

use crate::mbtiles::MbtilesMeta;

/// Startup pre-flight for layer source paths: local files must exist on disk
/// (`http(s)://` sources are checked lazily by the range reader). Reporting a
/// missing dataset here — instead of inside a format-specific reader — gives
/// operators one actionable error per layer.
fn check_source_path(kind: &SourceKind) -> anyhow::Result<()> {
    let path = match kind {
        SourceKind::Cog { path }
        | SourceKind::Vector { path }
        | SourceKind::Pmtiles { path }
        | SourceKind::Mbtiles { path }
        | SourceKind::Arcgis { path } => path,
        // PostGIS lives on a database server; reachability is probed when the
        // connection pool is built at startup (see state::build_pg_registry).
        SourceKind::Postgis(_) => return Ok(()),
    };
    if path.starts_with("http://")
        || path.starts_with("https://")
        || path.starts_with("s3://")
    {
        return Ok(());
    }
    if !Path::new(path).exists() {
        // ArcGIS 缓存的 path 是目录：说“数据文件”会把人引向找不存在的文件。
        if matches!(kind, SourceKind::Arcgis { .. }) {
            anyhow::bail!("arcgis cache directory not found: {path}");
        }
        anyhow::bail!("data file not found: {path}");
    }
    if matches!(kind, SourceKind::Arcgis { .. }) {
        // A cache directory without a `Conf.xml` is not a cache (an empty
        // folder, a file path, or the wrong level of the export): fail at
        // startup with the format-specific reason instead of a per-tile 500.
        gisserver::arcgis_cache::conf::CacheDocs::load(Path::new(path))
            .map_err(|e| anyhow::anyhow!("invalid arcgis cache {path}: {e}"))?;
    }
    Ok(())
}

/// A fully opened layer.
pub struct LayerRuntime {
    pub def: Layer,
    pub style: Option<Style>,
    /// SLD document parsed from a colocated SLD file (see [`Layer::sld`]);
    /// `None` when no SLD was found/parsed. When present, WMS GetMap and tile
    /// serving render through the rule-based SLD path (per-feature rule
    /// evaluation, marks, dashes, labels) instead of the flat legacy style.
    pub sld: Option<SldDocument>,
    /// COG reader (for `cog` layers).
    pub cog: Option<Arc<Cog>>,
    /// Byte-range source backing the COG (also serves `/cog/{layer}`).
    pub source: Option<Box<dyn RangeSource>>,
    /// The original source URI (for error messages).
    pub source_uri: String,
    /// Parsed GeoJSON FeatureCollection (for `vector` layers).
    pub vector_fc: Option<serde_json::Value>,
    /// Opened PMTiles archive (for `pmtiles` layers).
    pub pmtiles: Option<PmtilesReader>,
    /// Opened MBTiles archive (for `mbtiles` layers); serves pre-rendered
    /// tiles at `/mbtiles/{name}/{z}/{x}/{y}` and through the XYZ / TMS /
    /// WMTS / MVT front-ends.
    pub mbtiles: Option<Arc<MbtilesMeta>>,
    /// Opened ArcGIS tile cache (for `arcgis` layers): read-through of a
    /// pre-cut compact cache (v1/v2). Blocking file IO, so tile reads run
    /// inside `spawn_blocking` like every other raster source here.
    pub arcgis: Option<Arc<gisserver::arcgis_cache::ArcgisCache>>,
}

impl LayerRuntime {
    /// Open a layer. `s3` carries the global `server.s3` credentials —
    /// required only when a source URI is `s3://bucket/key`.
    pub fn open(
        def: Layer,
        style_dir: Option<&Path>,
        source_id: u64,
        s3: Option<&gisserver::storage::S3Config>,
    ) -> anyhow::Result<LayerRuntime> {
        // Startup pre-flight: every local source path must exist BEFORE any
        // reader tries to open it. Without this, a typo'd path surfaces as a
        // backend-specific error (SQLite "unable to open database file",
        // TIFF parse failure) or, worse, is half-swallowed; the check turns
        // all of them into one clear "data file not found" message.
        check_source_path(&def.source)?;

        let style = match &def.style {
            Some(s) => {
                let path = style_dir.map(|d| d.join(s)).unwrap_or_else(|| Path::new(s).to_path_buf());
                if path.exists() {
                    let text = std::fs::read_to_string(&path)?;
                    let value = crate::xmlcfg::parse_to_value(&text)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    Some(serde_json::from_value::<Style>(value)?)
                } else {
                    // An inline style path that isn't a file: treat as "no style"
                    // (only the raster default applies).
                    None
                }
            }
            None => None,
        };

        // Vector SLD: read + parse into a full document model. A missing or
        // non-SLD file is non-fatal (falls back to JSON paint / default
        // styling through the legacy flat path).
        let sld = def.sld.as_ref().and_then(|p| {
            let path = Path::new(p);
            let text = std::fs::read_to_string(path).ok()?;
            let doc = gisserver::renderer::sld::SldDocument::parse(&text)?;
            if doc.feature_type_styles.is_empty() {
                return None;
            }
            Some(doc)
        });

        let source_kind = def.source.clone();
        match source_kind {
            SourceKind::Cog { path } => {
                let source = gisserver::storage::source_from_uri_with_s3(&path, s3)?;
                let cog = Cog::parse_with_id(source.as_ref(), source_id)?;
                Ok(LayerRuntime {
                    def,
                    style,
                    sld,
                    cog: Some(Arc::new(cog)),
                    source: Some(source),
                    source_uri: path.clone(),
                    vector_fc: None,
                    pmtiles: None,
                    mbtiles: None,
                    arcgis: None,
                })
            }
            SourceKind::Vector { path } => {
                let text = std::fs::read_to_string(&path)?;
                let fc: serde_json::Value = serde_json::from_str(&text)?;
                Ok(LayerRuntime {
                    def,
                    style,
                    sld,
                    cog: None,
                    source: None,
                    source_uri: path.clone(),
                    vector_fc: Some(fc),
                    pmtiles: None,
                    mbtiles: None,
                    arcgis: None,
                })
            }
            SourceKind::Pmtiles { path } => {
                let reader = PmtilesReader::open(Path::new(&path))
                    .map_err(|e| anyhow::anyhow!("open pmtiles {path}: {e}"))?;
                Ok(LayerRuntime {
                    def,
                    style,
                    sld,
                    cog: None,
                    source: None,
                    source_uri: path.clone(),
                    vector_fc: None,
                    pmtiles: Some(reader),
                    mbtiles: None,
                    arcgis: None,
                })
            }
            SourceKind::Mbtiles { path } => {
                // Read-only open: verifies the `tiles` table exists and parses
                // the `metadata` map. Per-tile connections are opened lazily
                // inside `spawn_blocking` (see `MbtilesMeta::read_tile`), so
                // nothing here holds a SQLite handle across threads.
                let meta = crate::mbtiles::MbtilesMeta::open(&path)
                    .map_err(|e| anyhow::anyhow!("open mbtiles {path}: {e}"))?;
                Ok(LayerRuntime {
                    def,
                    style,
                    sld,
                    cog: None,
                    source: None,
                    source_uri: path.clone(),
                    vector_fc: None,
                    pmtiles: None,
                    mbtiles: Some(Arc::new(meta)),
                    arcgis: None,
                })
            }
            SourceKind::Arcgis { path } => {
                // Parses Conf.xml / conf.cdi once and probes `_alllayers`
                // (level list, bundle addressing), then serves tiles from the
                // compact cache read-through. Bundle index tables are cached
                // inside the reader, not here.
                let cache = gisserver::arcgis_cache::ArcgisCache::open(Path::new(&path))
                    .map_err(|e| anyhow::anyhow!("open arcgis cache {path}: {e}"))?;
                Ok(LayerRuntime {
                    def,
                    style,
                    sld,
                    cog: None,
                    source: None,
                    source_uri: path.clone(),
                    vector_fc: None,
                    pmtiles: None,
                    mbtiles: None,
                    arcgis: Some(Arc::new(cache)),
                })
            }
            SourceKind::Postgis(pg_def) => {
                // The connection pool is NOT opened here (open() is sync and
                // per-layer; pools are shared per-DSN and built once at
                // startup in `state::build_pg_registry`). Everything this
                // layer serves is computed from the pool on demand.
                let uri = match &pg_def.from {
                    Some(f) => format!("postgis-sql:{f}"),
                    None => format!("postgis://{}/{}", pg_def.schema, pg_def.table),
                };
                Ok(LayerRuntime {
                    def,
                    style,
                    sld,
                    cog: None,
                    source: None,
                    source_uri: uri,
                    vector_fc: None,
                    pmtiles: None,
                    mbtiles: None,
                    arcgis: None,
                })
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.def.name
    }

    pub fn is_cog(&self) -> bool {
        self.cog.is_some()
    }

    pub fn is_vector(&self) -> bool {
        self.vector_fc.is_some()
    }

    /// PostGIS-backed layer (feature queries + dynamic MVT tiles).
    pub fn is_postgis(&self) -> bool {
        matches!(self.def.source, SourceKind::Postgis(_))
    }

    /// Feature-queryable layer (WFS GetFeature / OGC API - Features): a file
    /// vector layer or a PostGIS layer. PostGIS features are queried live
    /// from the database instead of an in-memory FeatureCollection.
    pub fn is_queryable(&self) -> bool {
        self.vector_fc.is_some() || self.is_postgis()
    }

    /// Rasterize a vector FeatureCollection for WMS GetMap / tile serving.
    /// SLD layers take the rule-based styled path (scale denominator computed
    /// from the frame); everything else falls back to the legacy flat style.
    pub fn render_vector(
        &self,
        fc: &serde_json::Value,
        crs: &str,
        bbox: [f64; 4],
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        // GeoJSON features are EPSG:4326 and must be mapped into the request
        // CRS; the scale denominator is computed in Web Mercator (EPSG:3857).
        // The reprojectors are built here (composition root) and injected into
        // the renderer so `renderer` stays decoupled from the `projection`
        // backend. On a projection-backend failure we degrade to a transparent
        // frame instead of panicking.
        let tr_geo = match Transformer::new_any("EPSG:4326", crs, gisserver::projection::proj()) {
            Ok(t) => t,
            Err(_) => return vec![0u8; width as usize * height as usize * 4],
        };
        if let Some(doc) = &self.sld {
            let tr_scale = match Transformer::new_any(crs, EPSG_3857, gisserver::projection::proj()) {
                Ok(t) => t,
                Err(_) => return vec![0u8; width as usize * height as usize * 4],
            };
            let scale = gisserver::renderer::vector::scale_denominator(bbox, width, &tr_scale);
            return gisserver::renderer::vector::rasterize_geojson_styled(
                fc, bbox, width, height, doc, scale, &tr_geo,
            );
        }
        gisserver::renderer::vector::rasterize_geojson(
            fc,
            bbox,
            width,
            height,
            &self.effective_vector_style(),
            &tr_geo,
        )
    }

    /// Effective flat vector style (legacy path, used when no SLD is
    /// configured), in precedence:
    /// 1. legacy style JSON paint (`style.vector.paint.fill-color`);
    /// 2. [`VectorStyle::default`].
    pub fn effective_vector_style(&self) -> VectorStyle {
        if let Some(v) = self.style.as_ref().and_then(|s| s.vector.as_ref()) {
            if let Some(fill) = v.pointer("/paint/fill-color").and_then(|c| c.as_str()) {
                if let Some(c) = parse_hex3(fill) {
                    return VectorStyle {
                        fill: [c[0], c[1], c[2], 160],
                        ..Default::default()
                    };
                }
            }
        }
        VectorStyle::default()
    }

    /// Advertised bounds in the source CRS (config or computed).
    pub fn bounds(&self) -> Option<gisserver::wms_common::Bbox> {
        if let Some(b) = self.def.bounds() {
            return Some(b);
        }
        if let Some(fc) = &self.vector_fc {
            return vector_bounds(fc);
        }
        if let Some(c) = &self.cog {
            let g = c.levels[0].geo;
            let (w, h) = (c.native_width() as f64, c.native_height() as f64);
            let (x1, y0) = g.pix_to_geo(w, h);
            return Some(gisserver::wms_common::Bbox::new(g.origin_x, y0, x1, g.origin_y));
        }
        if let Some(mb) = &self.mbtiles {
            return mb.bounds();
        }
        None
    }

    /// CRS [`Self::bounds`] is expressed in -- capabilities must stamp the
    /// `<BoundingBox>`/root bbox with it, not blindly with `src_crs`.
    /// Config-declared bounds follow `src_crs`; a GeoJSON footprint and MBTiles
    /// `metadata.bounds` (west,south,east,north per spec) are EPSG:4326 degrees
    /// even when the layer declares EPSG:3857 as its tile scheme CRS.
    pub fn bounds_crs(&self) -> String {
        if self.def.bounds().is_some() {
            return self.def.src_crs.clone();
        }
        if self.vector_fc.is_some() || self.mbtiles.is_some() {
            return "EPSG:4326".to_string();
        }
        self.def.src_crs.clone()
    }
}

/// Parse `#rrggbb` / `#rrggbbaa` into `[r,g,b]` (alpha ignored). Used by
/// [`LayerRuntime::effective_vector_style`] to lift a legacy style JSON
/// `paint.fill-color` into a flat [`VectorStyle`].
fn parse_hex3(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    if hex.len() == 6 || hex.len() == 8 {
        return Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ]);
    }
    None
}

/// FeatureCollection bounds in EPSG:4326.
pub fn vector_bounds(fc: &serde_json::Value) -> Option<gisserver::wms_common::Bbox> {
    let mut bbox: Option<gisserver::wms_common::Bbox> = None;
    for feat in fc.get("features").and_then(|f| f.as_array()).into_iter().flatten() {
        if let Some(g) = feat.get("geometry") {
            collect_geom_bounds(g, &mut bbox);
        }
    }
    bbox
}

fn collect_geom_bounds(g: &serde_json::Value, bbox: &mut Option<gisserver::wms_common::Bbox>) {
    let gtype = g.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let coords = g.get("coordinates");
    let mut update = |lon: f64, lat: f64| {
        let b = gisserver::wms_common::Bbox::new(lon, lat, lon, lat);
        *bbox = Some(match *bbox {
            Some(mut cur) => {
                cur.merge(&b);
                cur
            }
            None => b,
        });
    };
    // Recursively walk nested coordinate arrays; an array of two numbers is a
    // position [lon, lat].
    fn walk(c: &serde_json::Value, update: &mut dyn FnMut(f64, f64)) {
        if let Some(arr) = c.as_array() {
            if arr.len() >= 2 && arr[0].is_number() && arr[1].is_number() {
                if let (Some(x), Some(y)) = (arr[0].as_f64(), arr[1].as_f64()) {
                    update(x, y);
                }
            } else {
                for item in arr {
                    walk(item, update);
                }
            }
        }
    }
    match gtype {
        "Point" | "MultiPoint" | "LineString" | "MultiLineString" | "Polygon" | "MultiPolygon" => {
            if let Some(c) = coords {
                walk(c, &mut update);
            }
        }
        "GeometryCollection" => {
            if let Some(gs) = g.get("geometries").and_then(|x| x.as_array()) {
                for sub in gs {
                    collect_geom_bounds(sub, bbox);
                }
            }
        }
        _ => {}
    }
}
