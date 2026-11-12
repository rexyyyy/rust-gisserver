//! WMTS 1.0.0 protocol: capabilities XML and OWS exceptions.

use wms_common::crs::to_urn;
use wms_common::tile::{is_preset_id, strip_size_suffix, TileMatrixSet};
use wms_common::Bbox;

use crate::wms::xml_escape;

/// A WMTS-advertised layer.
#[derive(Clone, Debug)]
pub struct WmtsCapLayer {
    pub name: String,
    pub title: String,
    /// Grid ids this layer publishes on (as configured).
    pub grids: Vec<String>,
    /// Data bounds in the grid CRS, when known (drives TileMatrixSetLimits).
    pub bounds: Option<Bbox>,
}

/// A WMTS error (OWS exception).
#[derive(Clone, Debug)]
pub struct WmtsError {
    pub http: u16,
    pub code: String,
    pub text: String,
    pub locator: Option<String>,
}

impl WmtsError {
    pub fn invalid(text: impl Into<String>, locator: &str) -> Self {
        Self {
            http: 400,
            code: "InvalidParameterValue".into(),
            text: text.into(),
            locator: Some(locator.into()),
        }
    }

    pub fn missing(text: impl Into<String>, locator: &str) -> Self {
        Self {
            http: 400,
            code: "MissingParameterValue".into(),
            text: text.into(),
            locator: Some(locator.into()),
        }
    }

    /// The requested tile address exists in the grid but carries no data:
    /// outside the matrix limits, a zoom the layer does not publish, or simply
    /// absent from the archive. OGC keeps this distinct from
    /// `InvalidParameterValue` so a client stops retrying the address.
    pub fn out_of_range(text: impl Into<String>) -> Self {
        Self {
            http: 404,
            code: "TileOutOfRange".into(),
            text: text.into(),
            locator: Some("TILEMATRIX".into()),
        }
    }

    pub fn internal(text: impl Into<String>) -> Self {
        Self {
            http: 500,
            code: "NoApplicableCode".into(),
            text: text.into(),
            locator: None,
        }
    }
}

/// OWS ExceptionReport document.
pub fn exception_xml(e: &WmtsError) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ows:ExceptionReport xmlns:ows=\"http://www.opengis.net/ows/1.1\" version=\"1.0.0\">\n\
         <ows:Exception exceptionCode=\"{}\"{}><ows:ExceptionText>{}</ows:ExceptionText></ows:Exception>\n\
         </ows:ExceptionReport>",
        xml_escape(&e.code),
        e.locator
            .as_deref()
            .map(|l| format!(" locator=\"{}\"", xml_escape(l)))
            .unwrap_or_default(),
        xml_escape(&e.text)
    )
}

/// A full WMTS capabilities document.
///
/// * `layers` -- advertised layers (each references grids by id).
/// * `grids` -- every TileMatrixSet to publish.
/// * `rest_base` -- base URL for RESTful tile templates, e.g. `http://host/wmts/rest`.
pub fn capabilities_xml(layers: &[WmtsCapLayer], grids: &[TileMatrixSet], rest_base: &str) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<Capabilities xmlns=\"http://www.opengis.net/wmts/1.0\" xmlns:ows=\"http://www.opengis.net/ows/1.1\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"1.0.0\">\n");
    s.push_str("  <ows:ServiceIdentification>\n    <ows:Title>rust-gisserver</ows:Title>\n    <ows:ServiceType>OGC WMTS</ows:ServiceType>\n    <ows:ServiceTypeVersion>1.0.0</ows:ServiceTypeVersion>\n  </ows:ServiceIdentification>\n");
    s.push_str("  <ows:ServiceProvider><ows:ProviderName>rust-gisserver</ows:ProviderName></ows:ServiceProvider>\n");
    s.push_str("  <ows:OperationsMetadata>\n");
    s.push_str(&format!("    <ows:Operation name=\"GetCapabilities\"><ows:DCP><ows:HTTP><ows:Get xlink:href=\"{}/wmts\"><ows:Constraint name=\"GetEncoding\"><ows:AllowedValues><ows:Value>KVP</ows:Value></ows:AllowedValues></ows:Constraint></ows:Get></ows:HTTP></ows:DCP></ows:Operation>\n", xml_escape(rest_base)));
    s.push_str(&format!("    <ows:Operation name=\"GetTile\"><ows:DCP><ows:HTTP><ows:Get xlink:href=\"{}/wmts\"><ows:Constraint name=\"GetEncoding\"><ows:AllowedValues><ows:Value>KVP</ows:Value><ows:Value>RESTful</ows:Value></ows:AllowedValues></ows:Constraint></ows:Get></ows:HTTP></ows:DCP></ows:Operation>\n", xml_escape(rest_base)));
    s.push_str("  </ows:OperationsMetadata>\n");

    // Contents: layers.
    s.push_str("  <Contents>\n");
    for l in layers {
        s.push_str("    <Layer>\n");
        s.push_str(&format!("      <ows:Title>{}</ows:Title>\n", xml_escape(&l.title)));
        s.push_str(&format!("      <ows:Identifier>{}</ows:Identifier>\n", xml_escape(&l.name)));
        s.push_str("      <Style isDefault=\"true\"><ows:Title>Default</ows:Title><ows:Identifier>default</ows:Identifier></Style>\n");
        s.push_str("      <Format>image/png</Format>\n");
        s.push_str("      <Format>application/vnd.mapbox-vector-tile</Format>\n");
        for gid in &l.grids {
            s.push_str(&format!("      <TileMatrixSetLink><TileMatrixSet>{}</TileMatrixSet></TileMatrixSetLink>\n", xml_escape(&strip_size_suffix(gid))));
        }
        if let Some(b) = l.bounds {
            // Data bounds in EPSG:4326 for the WGS84BoundingBox.
            s.push_str(&format!("      <ows:WGS84BoundingBox><ows:LowerCorner>{} {}</ows:LowerCorner><ows:UpperCorner>{} {}</ows:UpperCorner></ows:WGS84BoundingBox>\n", b.minx, b.miny, b.maxx, b.maxy));
        }
        s.push_str(&format!("      <ResourceURL format=\"image/png\" resourceType=\"tile\" template=\"{}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png\"/>\n", xml_escape(&format!("{rest_base}/{}/default", l.name))));
        s.push_str(&format!("      <ResourceURL format=\"application/vnd.mapbox-vector-tile\" resourceType=\"tile\" template=\"{}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.pbf\"/>\n", xml_escape(&format!("{rest_base}/{}/default", l.name))));
        s.push_str("    </Layer>\n");
    }

    // Contents: TileMatrixSets.
    for g in grids {
        s.push_str(&tile_matrix_set_xml(g));
    }
    s.push_str("  </Contents>\n");
    s.push_str("</Capabilities>\n");
    s
}

/// One `<TileMatrixSet>` block (also served standalone at `/tileMatrixSets/:id`).
pub fn tile_matrix_set_xml(g: &TileMatrixSet) -> String {
    let id = strip_size_suffix(&g.id);
    let mut s = String::new();
    s.push_str("    <TileMatrixSet>\n");
    s.push_str(&format!("      <ows:Identifier>{}</ows:Identifier>\n", xml_escape(&id)));
    s.push_str(&format!("      <ows:SupportedCRS>{}</ows:SupportedCRS>\n", supported_crs(g)));
    // `<TileMatrixSetURI>` 只对 OGC 正式登记的网格有意义：配置里自定义的网格
    // （如 CGCS2000Geo）拼上 OGC 命名空间会指向一个 404 的 URI，属“说谎的
    // 元数据”。该元素在 schema 里可选，故非内置网格省略（定义仍可在
    // `/tileMatrixSets/{id}` 取到）。判定与 OGC API JSON 面共用 `is_preset_id`。
    if is_preset_id(&g.id) {
        s.push_str(&format!(
            "      <TileMatrixSetURI>http://www.opengis.net/def/tilematrixset/OGC/1.0/{id}</TileMatrixSetURI>\n"
        ));
    }
    let [_minx, _miny, _maxx, _maxy] = g.full_extent().unwrap_or([0.0, 0.0, 0.0, 0.0]);
    s.push_str(&format!("      <WellKnownScaleSet>{}</WellKnownScaleSet>\n", well_known_scale_set(g)));
    // TileMatrix limits (level-0 derived).
    let scale0 = g.scale_denominator(0);
    if scale0.is_finite() {
        s.push_str(&format!("      <TileMatrix>\n        <ows:Identifier>0</ows:Identifier>\n        <ScaleDenominator>{scale0}</ScaleDenominator>\n"));
        s.push_str(&format!("        <TopLeftCorner>{} {}</TopLeftCorner>\n", g.origin_x, g.origin_y));
        s.push_str(&format!("        <TileWidth>{}</TileWidth>\n        <TileHeight>{}</TileHeight>\n", g.tile_w, g.tile_h));
        if let Some(l0) = g.levels.first() {
            s.push_str(&format!("        <MatrixWidth>{}</MatrixWidth>\n        <MatrixHeight>{}</MatrixHeight>\n", l0.matrix_w, l0.matrix_h));
        }
        s.push_str("      </TileMatrix>\n");
    }
    // Full ladder.
    for lvl in &g.levels {
        if lvl.z == 0 {
            continue;
        }
        s.push_str(&format!("      <TileMatrix>\n        <ows:Identifier>{}</ows:Identifier>\n        <ScaleDenominator>{}</ScaleDenominator>\n", lvl.z, g.scale_denominator(lvl.z)));
        s.push_str(&format!("        <TopLeftCorner>{} {}</TopLeftCorner>\n", g.origin_x, g.origin_y));
        s.push_str(&format!("        <TileWidth>{}</TileWidth>\n        <TileHeight>{}</TileHeight>\n", g.tile_w, g.tile_h));
        s.push_str(&format!("        <MatrixWidth>{}</MatrixWidth>\n        <MatrixHeight>{}</MatrixHeight>\n", lvl.matrix_w, lvl.matrix_h));
        s.push_str("      </TileMatrix>\n");
    }
    s.push_str("    </TileMatrixSet>\n");
    s
}

/// `<ows:SupportedCRS>` 的取值必须与网格书写 `TopLeftCorner` 的坐标序一致。
///
/// `WorldCRS84Quad` 的原点是 `(-180, 90)`、列数 2×行数 1，即“经度在前”，WKSS 也是
/// `GlobalCRS84Scale`；若按 `urn:ogc:def:crs:EPSG::4326` 宣告，EPSG 注册表里 4326 的
/// 轴序是（纬度,经度），严格按轴序解析的客户端会把原点读成 `(90, -180)` → 整层
/// 镜像错位（OL 用内置 `EPSG:4326`='neu' 时正是这个现象）。OGC 对该网格的定义本就
/// 是 CRS84，故直接宣告 CRS84 的 URN。
fn supported_crs(g: &TileMatrixSet) -> String {
    if matches!(strip_size_suffix(&g.id).as_str(), "WorldCRS84Quad") {
        return "urn:ogc:def:crs:OGC:1.3:CRS84".into();
    }
    to_urn(&g.crs)
}

/// The well-known scale set for a grid, when it matches a preset.
fn well_known_scale_set(g: &TileMatrixSet) -> String {
    match strip_size_suffix(&g.id).as_str() {
        "WebMercatorQuad" => "http://www.opengis.net/def/wkss/OGC/1.0/GoogleMapsCompatible".into(),
        "WorldCRS84Quad" => "http://www.opengis.net/def/wkss/OGC/1.0/GlobalCRS84Scale".into(),
        _ => format!("urn:ogc:def:wkss:rust-gisserver:{}", strip_size_suffix(&g.id)),
    }
}

/// Parse a WMTS KVP GetTile request's tile address.
/// Returns `(layer, style, tms, z, row, col, format)`.
pub fn parse_get_tile(
    kv: &[(String, String)],
) -> Result<(String, String, String, u32, u32, u32, String), WmtsError> {
    let get = |k: &str| crate::kvp::lookup(kv, k);
    let layer = get("layer").ok_or_else(|| WmtsError::missing("missing LAYER", "LAYER"))?.to_string();
    let style = get("style").unwrap_or("default").to_string();
    let tms = get("tilematrixset")
        .ok_or_else(|| WmtsError::missing("missing TILEMATRIXSET", "TILEMATRIXSET"))?
        .to_string();
    let z = get("tilematrix")
        .ok_or_else(|| WmtsError::missing("missing TILEMATRIX", "TILEMATRIX"))?
        .parse::<u32>()
        .map_err(|_| WmtsError::invalid("invalid TILEMATRIX", "TILEMATRIX"))?;
    let row = get("tilerow")
        .ok_or_else(|| WmtsError::missing("missing TILEROW", "TILEROW"))?
        .parse::<u32>()
        .map_err(|_| WmtsError::invalid("invalid TILEROW", "TILEROW"))?;
    let col = get("tilecol")
        .ok_or_else(|| WmtsError::missing("missing TILECOL", "TILECOL"))?
        .parse::<u32>()
        .map_err(|_| WmtsError::invalid("invalid TILECOL", "TILECOL"))?;
    let format = get("format").unwrap_or("image/png").to_ascii_lowercase();
    Ok((layer, style, tms, z, row, col, format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wms_common::tile::TmLevel;

    fn grid(id: &str, crs: &str) -> TileMatrixSet {
        TileMatrixSet {
            id: id.into(),
            crs: crs.into(),
            origin_x: -180.0,
            origin_y: 90.0,
            tile_w: 256,
            tile_h: 256,
            levels: vec![TmLevel {
                z: 0,
                resolution: 0.703125,
                matrix_w: 2,
                matrix_h: 1,
                scale_denominator: None,
            }],
        }
    }

    // 自定义网格不得伪挂 OGC 登记 URI（那个地址不存在）；内置网格保留。
    #[test]
    fn tile_matrix_set_uri_only_for_registered_grids() {
        let custom = tile_matrix_set_xml(&grid("CGCS2000Geo", "EPSG:4490"));
        assert!(!custom.contains("<TileMatrixSetURI>"), "{custom}");
        assert!(custom.contains("urn:ogc:def:crs:EPSG::4490"));
        // 服务端自有命名空间的 WellKnownScaleSet 仍照旧给出
        assert!(custom.contains("urn:ogc:def:wkss:rust-gisserver:CGCS2000Geo"));
        let builtin = tile_matrix_set_xml(&TileMatrixSet::web_mercator_quad(256));
        assert!(builtin.contains(
            "<TileMatrixSetURI>http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad</TileMatrixSetURI>"
        ));
    }

    // WorldCRS84Quad 的坐标是经度在前，SupportedCRS 必须宣告 CRS84 而不是 EPSG:4326
    // （后者在 EPSG 注册表里是纬度在前，严格客户端会据此交换原点）。
    #[test]
    fn supported_crs_matches_grid_axis_order() {
        let wgs = tile_matrix_set_xml(&TileMatrixSet::world_crs84_quad(256));
        assert!(wgs.contains("<ows:SupportedCRS>urn:ogc:def:crs:OGC:1.3:CRS84</ows:SupportedCRS>"), "{wgs}");
        assert!(!wgs.contains("EPSG::4326"), "{wgs}");
        // 角点仍是 (经度, 纬度)
        assert!(wgs.contains("<TopLeftCorner>-180 90</TopLeftCorner>"), "{wgs}");
        // 投影网格照旧走 EPSG URN
        let merc = tile_matrix_set_xml(&TileMatrixSet::web_mercator_quad(256));
        assert!(merc.contains("<ows:SupportedCRS>urn:ogc:def:crs:EPSG::3857</ows:SupportedCRS>"), "{merc}");
    }
}
