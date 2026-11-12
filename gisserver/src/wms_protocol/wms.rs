//! WMS 1.1.1 / 1.3.0 protocol: capabilities XML, GetMap parameter parsing and
//! validation, and ServiceException / ExceptionReport documents.
//!
//! Protocol logic only -- rendering orchestration lives in the service.

use crate::wms_common::crs::{is_geographic, is_northing_first, normalize};
use crate::wms_common::Bbox;

/// A layer as advertised in capabilities (a projection of the runtime layer).
#[derive(Clone, Debug)]
pub struct CapLayer {
    pub name: String,
    pub title: String,
    /// Bounds in the layer's source CRS, when known.
    pub bounds: Option<Bbox>,
    /// The source CRS; capabilities advertise it plus EPSG:4326 / EPSG:3857.
    pub src_crs: String,
    /// CRS `bounds` are actually expressed in. Usually `src_crs`, but MBTiles
    /// `metadata.bounds` and GeoJSON footprints are always EPSG:4326 degrees
    /// even when the layer declares EPSG:3857 as its tile scheme CRS --
    /// stamping `src_crs` on those advertised "-180..180 in metres".
    /// Empty means "same as src_crs".
    pub bounds_crs: String,
    /// The same bounds in EPSG:4326 degrees, precomputed by the service layer
    /// (projected sources are transformed there where PROJ is available).
    /// Advertised as EX_GeographicBoundingBox / LatLonBoundingBox so generic
    /// clients can fit the map without knowing the source CRS.
    pub geo_bounds: Option<Bbox>,
    /// Whether the layer supports GetFeatureInfo (sampling).
    pub queryable: bool,
}

impl CapLayer {
    /// CRS to stamp on the advertised `<BoundingBox>`: the explicit
    /// `bounds_crs` when the caller knows the bounds are not in `src_crs`
    /// (MBTiles/GeoJSON), otherwise `src_crs`.
    pub fn bbox_crs(&self) -> &str {
        if self.bounds_crs.is_empty() {
            &self.src_crs
        } else {
            &self.bounds_crs
        }
    }
}

/// A parsed GetMap request (validated).
#[derive(Clone, Debug)]
pub struct GetMapParams {
    pub version: String,
    pub crs: String,
    pub bbox: Bbox,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<String>,
    pub styles: Vec<String>,
    pub transparent: bool,
    pub bgcolor: [u8; 3],
    pub format: String,
}

/// A WMS exception: the OGC `code` plus a human message.
#[derive(Clone, Debug)]
pub struct WmsError {
    pub code: Option<String>,
    pub message: String,
}

impl WmsError {
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    pub fn coded(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.to_string()),
            message: message.into(),
        }
    }

    /// An internal (server-side) failure -- no OGC code, generic message.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }
}

/// Parse and validate a GetMap request.
///
/// * `version` -- already negotiated ("1.1.1" / "1.3.0").
/// * `layers` -- configured layer names (to resolve LAYERS=).
/// * `src_crs` -- the selected layer's source CRS (for native-CRS acceptance).
pub fn parse_get_map(
    version: &str,
    kv: &[(String, String)],
    configured_layers: &[String],
    src_crs: &str,
) -> Result<GetMapParams, WmsError> {
    let get = |k: &str| crate::wms_protocol::kvp::lookup(kv, k);

    let layers: Vec<String> = get("layers")
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if layers.is_empty() {
        return Err(WmsError::plain("missing LAYERS parameter"));
    }
    for l in &layers {
        if !configured_layers.iter().any(|c| c == l) {
            return Err(WmsError::coded("LayerNotDefined", format!("layer '{l}' is not defined")));
        }
    }

    // STYLES: empty entries mean "default style", and layers expose only the
    // default style, so any non-empty entry is StyleNotDefined.
    let styles: Vec<String> = get("styles")
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(s) = styles.first() {
        return Err(WmsError::coded("StyleNotDefined", format!("style '{s}' is not defined")));
    }

    let format = get("format").unwrap_or("image/png").trim().to_ascii_lowercase();
    if !format.is_empty() && format != "image/png" {
        return Err(WmsError::coded("InvalidFormat", format!("unsupported FORMAT '{format}'")));
    }

    let crs_raw = get("crs").or_else(|| get("srs")).ok_or_else(|| WmsError::plain("missing CRS/SRS parameter"))?;
    let crs = normalize(crs_raw);
    let native_match = crs == normalize(src_crs);
    if !crate::wms_common::crs::is_supported(&crs) && !native_match {
        let code = if version.starts_with("1.3") { "InvalidCRS" } else { "InvalidSRS" };
        return Err(WmsError::coded(code, format!("unsupported CRS '{crs_raw}'")));
    }

    let bbox_raw = get("bbox").ok_or_else(|| WmsError::plain("missing BBOX parameter"))?;
    let vals: Vec<f64> = bbox_raw
        .split(',')
        .map(|s| s.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| WmsError::plain("invalid BBOX"))?;
    if vals.len() != 4 {
        return Err(WmsError::plain("BBOX must have 4 values"));
    }
    if vals.iter().any(|v| !v.is_finite()) {
        return Err(WmsError::plain("invalid BBOX: values must be finite"));
    }
    // WMS 1.3.0 axis order: EPSG:4326 (and other geographic EPSG codes) are
    // northing-first -> swap. CRS:84 is lon/lat BY DEFINITION and must NOT be
    // swapped -- decide on the raw identifier, since `normalize` folds it onto
    // EPSG:4326 (which would wrongly trigger the swap).
    let is_130 = version.starts_with("1.3");
    let bbox = if is_130 && !crate::wms_common::crs::is_crs84(crs_raw) && is_northing_first(&crs) {
        Bbox::new(vals[1], vals[0], vals[3], vals[2])
    } else {
        Bbox::new(vals[0], vals[1], vals[2], vals[3])
    };
    if !bbox.is_valid() {
        return Err(WmsError::plain("invalid BBOX: min must be less than max"));
    }

    let width: u32 = get("width").ok_or_else(|| WmsError::plain("missing WIDTH"))?.trim().parse().map_err(|_| WmsError::plain("invalid WIDTH"))?;
    let height: u32 = get("height").ok_or_else(|| WmsError::plain("missing HEIGHT"))?.trim().parse().map_err(|_| WmsError::plain("invalid HEIGHT"))?;
    const MAX_DIM: u32 = 8192;
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(WmsError::plain(format!("WIDTH/HEIGHT must be 1..={MAX_DIM}")));
    }

    let transparent = match get("transparent").unwrap_or("false").to_ascii_lowercase().as_str() {
        "true" | "1" => true,
        _ => false,
    };
    let bgcolor = parse_bgcolor(get("bgcolor"));

    Ok(GetMapParams {
        version: version.to_string(),
        crs,
        bbox,
        width,
        height,
        layers,
        styles,
        transparent,
        bgcolor,
        format,
    })
}

/// Parse `BGCOLOR=0xRRGGBB` (default white).
pub fn parse_bgcolor(raw: Option<&str>) -> [u8; 3] {
    let Some(raw) = raw else { return [255, 255, 255] };
    let s = raw.trim().trim_start_matches("0x").trim_start_matches('#');
    if s.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        ) {
            return [r, g, b];
        }
    }
    [255, 255, 255]
}

/// Negotiate the WMS version: 1.3.0 when asked (or unknown), else 1.1.1.
pub fn negotiate_version(requested: Option<&str>) -> String {
    let Some(r) = requested else { return "1.3.0".to_string() };
    let v: Vec<u32> = r.split('.').filter_map(|p| p.parse().ok()).collect();
    match v.as_slice() {
        [1, 1, _] => "1.1.1".to_string(),
        _ => "1.3.0".to_string(),
    }
}

/// Render the capabilities document.
pub fn capabilities_xml(layers: &[CapLayer], version: &str, base_url: &str) -> String {
    let is_130 = version.starts_with("1.3");
    let crs_tag = if is_130 { "CRS" } else { "SRS" };
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if is_130 {
        s.push_str("<WMS_Capabilities version=\"1.3.0\" xmlns=\"http://www.opengis.net/wms\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n");
    } else {
        s.push_str("<WMS_Capabilities version=\"1.1.1\" xmlns=\"http://www.opengis.net/wms\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n");
    }
    s.push_str("  <Service>\n    <Name>WMS</Name>\n    <Title>rust-gisserver</Title>\n");
    s.push_str(&format!("    <Abstract>Rust map server: WMS / WMTS / TMS / MVT / PMTiles / COG tile services</Abstract>\n"));
    s.push_str(&format!("    <OnlineResource xlink:href=\"{}\"/>\n", xml_escape(&format!("{base_url}/wms"))));
    s.push_str("    <Fees/>\n    <AccessConstraints/>\n    <MaxWidth>8192</MaxWidth>\n    <MaxHeight>8192</MaxHeight>\n  </Service>\n");
    s.push_str("  <Capability>\n    <Request>\n");
    s.push_str("      <GetCapabilities>\n        <Format>text/xml</Format>\n");
    s.push_str(&format!("        <DCPType><HTTP><Get><OnlineResource xlink:href=\"{}\"/></Get></HTTP></DCPType>\n", xml_escape(&format!("{base_url}/wms"))));
    s.push_str("      </GetCapabilities>\n");
    s.push_str("      <GetMap>\n        <Format>image/png</Format>\n");
    s.push_str(&format!("        <DCPType><HTTP><Get><OnlineResource xlink:href=\"{}\"/></Get></HTTP></DCPType>\n", xml_escape(&format!("{base_url}/wms"))));
    s.push_str("      </GetMap>\n");
    s.push_str("      <GetFeatureInfo>\n        <Format>text/plain</Format>\n        <Format>application/json</Format>\n");
    s.push_str(&format!("        <DCPType><HTTP><Get><OnlineResource xlink:href=\"{}\"/></Get></HTTP></DCPType>\n", xml_escape(&format!("{base_url}/wms"))));
    s.push_str("      </GetFeatureInfo>\n");
    s.push_str("      <GetLegendGraphic>\n        <Format>image/png</Format>\n");
    s.push_str(&format!("        <DCPType><HTTP><Get><OnlineResource xlink:href=\"{}\"/></Get></HTTP></DCPType>\n", xml_escape(&format!("{base_url}/wms"))));
    s.push_str("      </GetLegendGraphic>\n    </Request>\n");
    s.push_str("    <Exception><Format>XML</Format></Exception>\n");
    // GeoServer advertises SLD support here; we support UserStyle documents
    // (SLD parameter itself is a documented boundary).
    s.push_str("    <UserDefinedSymbolization SupportSLD=\"1\" UserLayer=\"0\" UserStyle=\"1\" RemoteWFS=\"0\"/>\n");

    // Root layer: common CRSs + bounding boxes.
    s.push_str("    <Layer>\n      <Title>rust-gisserver layers</Title>\n");
    for crs in crate::wms_common::crs::advertised() {
        s.push_str(&format!("      <{crs_tag}>{crs}</{crs_tag}>\n"));
    }
    let mut root_bbox: Option<Bbox> = None;
    for l in layers {
        // EX_GeographicBoundingBox is degrees-only: merge geographic bounds and
        // skip projected ones (a 3857 metre box merged in would publish nonsense
        // longitudes and break a client that fits the map to the root extent).
        if let Some(b) = l.bounds {
            if !is_geographic(&l.bbox_crs()) {
                continue;
            }
            root_bbox = Some(match root_bbox {
                Some(mut r) => {
                    r.merge(&b);
                    r
                }
                None => b,
            });
        }
    }
    if let Some(b) = root_bbox {
        if is_130 {
            s.push_str(&format!(
                "      <EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox>\n",
                b.minx, b.maxx, b.miny, b.maxy
            ));
        } else {
            s.push_str(&format!(
                "      <LatLonBoundingBox minx=\"{}\" miny=\"{}\" maxx=\"{}\" maxy=\"{}\"/>\n",
                b.minx, b.miny, b.maxx, b.maxy
            ));
        }
    }

    for l in layers {
        s.push_str("      <Layer queryable=\"1\">\n");
        s.push_str(&format!("        <Name>{}</Name>\n", xml_escape(&l.name)));
        s.push_str(&format!("        <Title>{}</Title>\n", xml_escape(&l.title)));
        let mut layer_crs: Vec<String> = vec![l.src_crs.clone()];
        for c in crate::wms_common::crs::advertised() {
            let c = c.to_string();
            if !layer_crs.contains(&c) {
                layer_crs.push(c);
            }
        }
        for crs in layer_crs {
            s.push_str(&format!("        <{crs_tag}>{crs}</{crs_tag}>\n"));
        }
        s.push_str("        <Style><Name>default</Name><Title>Default</Title></Style>\n");
        if let Some(g) = l.geo_bounds {
            if is_130 {
                s.push_str(&format!(
                    "        <EX_GeographicBoundingBox><westBoundLongitude>{}</westBoundLongitude><eastBoundLongitude>{}</eastBoundLongitude><southBoundLatitude>{}</southBoundLatitude><northBoundLatitude>{}</northBoundLatitude></EX_GeographicBoundingBox>\n",
                    g.minx, g.maxx, g.miny, g.maxy
                ));
            } else {
                s.push_str(&format!(
                    "        <LatLonBoundingBox minx=\"{}\" miny=\"{}\" maxx=\"{}\" maxy=\"{}\"/>\n",
                    g.minx, g.miny, g.maxx, g.maxy
                ));
            }
        }
        if let Some(b) = l.bounds {
            s.push_str(&format!(
                "        <BoundingBox CRS=\"{}\" minx=\"{}\" miny=\"{}\" maxx=\"{}\" maxy=\"{}\"/>\n",
                l.bbox_crs(), b.minx, b.miny, b.maxx, b.maxy
            ));
        }
        s.push_str("      </Layer>\n");
    }
    s.push_str("    </Layer>\n  </Capability>\n</WMS_Capabilities>\n");
    s
}

/// The content type for a WMS response (`application/vnd.ogc.se_xml` for 1.1.1,
/// `text/xml` for 1.3.0 GetCapabilities).
pub fn capabilities_content_type(version: &str) -> &'static str {
    if version.starts_with("1.3") {
        "text/xml"
    } else {
        "application/vnd.ogc.se_xml"
    }
}

/// A ServiceException (1.1.1) / ExceptionReport (1.3.0) document.
pub fn exception_xml(version: &str, code: Option<&str>, message: &str) -> (String, &'static str) {
    let msg = xml_escape(message);
    let (body, ctype) = if version.starts_with("1.3") {
        (
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ServiceExceptionReport version=\"1.3.0\" xmlns=\"http://www.opengis.net/ogc\"><ServiceException{}>{}</ServiceException></ServiceExceptionReport>",
                code.map(|c| format!(" code=\"{}\"", xml_escape(c))).unwrap_or_default(),
                msg
            ),
            "application/vnd.ogc.se_xml",
        )
    } else {
        (
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ServiceExceptionReport version=\"1.1.1\" xmlns=\"http://www.opengis.net/ogc\"><ServiceException{}>{}</ServiceException></ServiceExceptionReport>",
                code.map(|c| format!(" code=\"{}\"", xml_escape(c))).unwrap_or_default(),
                msg
            ),
            "application/vnd.ogc.se_xml",
        )
    };
    (body, ctype)
}

/// Escape text for XML text/attribute positions.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        // parse_kvp uppercases keys; mirror that when building pairs directly.
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_uppercase(), v.to_string()))
            .collect()
    }

    fn base_kv(crs: &str, bbox: &str) -> Vec<(String, String)> {
        kv(&[
            ("layers", "world"),
            ("format", "image/png"),
            ("crs", crs),
            ("bbox", bbox),
            ("width", "256"),
            ("height", "256"),
        ])
    }

    #[test]
    fn getmap_130_epsg4326_is_northing_first() {
        // WMS 1.3.0 + EPSG:4326: BBOX = miny,minx,maxy,maxx -> swapped.
        let p = parse_get_map("1.3.0", &base_kv("EPSG:4326", "36,100,40,108"), &["world".to_string()], "EPSG:4326").unwrap();
        assert_eq!(p.bbox.to_array(), [100.0, 36.0, 108.0, 40.0]);
    }

    #[test]
    fn getmap_130_crs84_is_lonlat_not_swapped() {
        // CRS:84 is lon/lat BY DEFINITION: the same numbers must NOT swap.
        // (Regression: normalize() folds CRS:84 onto EPSG:4326, which once
        // triggered the northing-first swap and silently rendered blank.)
        for crs in ["CRS:84", "OGC:CRS84", "urn:ogc:def:crs:OGC:1.3:CRS84"] {
            let p = parse_get_map("1.3.0", &base_kv(crs, "100,36,108,40"), &["world".to_string()], "EPSG:4326").unwrap();
            assert_eq!(p.bbox.to_array(), [100.0, 36.0, 108.0, 40.0], "crs {crs}");
        }
    }

    #[test]
    fn getmap_111_never_swaps() {
        let p = parse_get_map("1.1.1", &base_kv("EPSG:4326", "100,36,108,40"), &["world".to_string()], "EPSG:4326").unwrap();
        assert_eq!(p.bbox.to_array(), [100.0, 36.0, 108.0, 40.0]);
    }

    #[test]
    fn getmap_130_3857_not_swapped() {
        let p = parse_get_map("1.3.0", &base_kv("EPSG:3857", "-2e7,-2e7,2e7,2e7"), &["world".to_string()], "EPSG:4326").unwrap();
        assert_eq!(p.bbox.to_array(), [-2e7, -2e7, 2e7, 2e7]);
    }

    fn sample_layers() -> Vec<CapLayer> {
        vec![CapLayer {
            name: "places".into(),
            title: "Places".into(),
            bounds: Some(crate::wms_common::Bbox::new(100.0, 24.0, 104.0, 26.0)),
            src_crs: "EPSG:4326".into(),
            bounds_crs: String::new(),
            geo_bounds: Some(crate::wms_common::Bbox::new(100.0, 24.0, 104.0, 26.0)),
            queryable: true,
        }]
    }

    #[test]
    fn capabilities_advertise_legend_and_sld_support() {
        for version in ["1.3.0", "1.1.1"] {
            let xml = capabilities_xml(&sample_layers(), version, "http://localhost:8080");
            assert!(
                xml.contains("<GetLegendGraphic>"),
                "{version}: GetLegendGraphic request missing"
            );
            assert!(
                xml.contains("<Format>image/png</Format>"),
                "{version}: image/png format missing"
            );
            assert!(
                xml.contains("UserDefinedSymbolization"),
                "{version}: UserDefinedSymbolization missing"
            );
            assert!(
                xml.contains("SupportSLD=\"1\""),
                "{version}: SupportSLD flag missing"
            );
            // GetFeatureInfo now advertises json as well.
            assert!(
                xml.contains("<Format>application/json</Format>"),
                "{version}: GetFeatureInfo json format missing"
            );
        }
    }

    #[test]
    fn capabilities_bbox_uses_bounds_crs_not_src_crs() {
        // MBTiles 图层：src_crs=EPSG:3857（瓦片方案），但 bounds 是 EPSG:4326 度数。
        // 旧写法按 src_crs 标注，会把 "-180..180" 说成 Web Mercator 米值。
        let mut l = sample_layers().remove(0);
        l.src_crs = "EPSG:3857".into();
        l.bounds_crs = "EPSG:4326".into();
        let xml = capabilities_xml(&[l], "1.3.0", "http://localhost:8080");
        assert!(
            xml.contains("<BoundingBox CRS=\"EPSG:4326\" minx=\"100\""),
            "degrees bounds must be labelled EPSG:4326, not the 3857 tile CRS"
        );
        // 缺省（空 bounds_crs）回退 src_crs。
        let xml = capabilities_xml(&sample_layers(), "1.3.0", "http://localhost:8080");
        assert!(xml.contains("<BoundingBox CRS=\"EPSG:4326\""));
    }
}
