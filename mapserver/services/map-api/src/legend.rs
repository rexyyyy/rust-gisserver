//! Legend rendering for WMS `GetLegendGraphic` (GeoServer-aligned subset).
//!
//! Draws a small PNG swatch per (rule, symbolizer): point marks, line
//! samples, polygon blocks, text labels — or a palette ramp for raster
//! layers. Supports `LAYER`, `STYLE`, `RULE`, `FORMAT=image/png`,
//! `WIDTH`, `HEIGHT` (GeoServer's common KVP subset; `SCALE`, `DPI`,
//! `TRANSPARENT`, legend *rules* JSON and font metrics are out of scope).

use renderer::sld::{MarkKind, SldDocument, Symbolizer};
use renderer::vector::VectorStyle;

use crate::layer::LayerRuntime;

/// Default swatch canvas.
pub const DEF_W: u32 = 20;
pub const DEF_H: u32 = 20;
const PAD: i64 = 2;

/// Default legend height when the request omits `HEIGHT`.
///
/// A swatch legend stacks one row per (rule, symbolizer); GeoServer's fixed
/// 20x20 default would squeeze a multi-rule style into 2px rows, so the
/// default grows with the style (still clamped to the canvas budget).
pub fn auto_height(layer: &LayerRuntime, rule: Option<&str>) -> u32 {
    let rows = if let Some(doc) = &layer.sld {
        sld_swatches(doc, rule).len()
    } else {
        1 // raster ramp or legacy single swatch
    };
    if rows == 0 {
        return DEF_H; // empty style: single grey placeholder block
    }
    (DEF_H as usize + 18 * rows).clamp(DEF_H as usize, 512) as u32
}

/// Render a legend PNG for a layer. `rule` filters rules by name when given.
/// Returns the PNG bytes, or an error message for unsupported formats.
pub fn legend_png(
    layer: &LayerRuntime,
    width: u32,
    height: u32,
    rule: Option<&str>,
) -> Result<Vec<u8>, String> {
    let w = width.clamp(8, 512) as usize;
    let h = height.clamp(8, 512) as usize;
    let mut canvas = vec![255u8, 255, 255, 255]; // white, opaque
    canvas.resize(w * h * 4, 255);
    for px in canvas.chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 255, 255, 255]);
    }

    // Raster layer: vertical palette ramp (value-domain style).
    if let Some(rs) = layer.style.as_ref().and_then(|s| s.raster.as_ref()) {
        draw_palette_ramp(&mut canvas, w, h, rs);
        return renderer::png::encode_rgba(&canvas, w as u32, h as u32);
    }

    if let Some(doc) = &layer.sld {
        draw_sld_legend(&mut canvas, w, h, doc, rule);
    } else {
        draw_legacy_legend(&mut canvas, w, h, layer);
    }
    renderer::png::encode_rgba(&canvas, w as u32, h as u32)
}

/// Collect the (rule-name, symbolizer) pairs to draw, honouring `rule`.
fn sld_swatches<'a>(
    doc: &'a SldDocument,
    rule: Option<&str>,
) -> Vec<(Option<&'a str>, &'a Symbolizer)> {
    let mut out = Vec::new();
    for fts in &doc.feature_type_styles {
        for r in &fts.rules {
            if let Some(want) = rule {
                if r.name.as_deref() != Some(want) {
                    continue;
                }
            }
            for sym in &r.symbolizers {
                out.push((r.name.as_deref(), sym));
            }
        }
    }
    out
}

fn draw_sld_legend(
    canvas: &mut [u8],
    w: usize,
    h: usize,
    doc: &SldDocument,
    rule: Option<&str>,
) {
    let swatches = sld_swatches(doc, rule);
    if swatches.is_empty() {
        // Nothing to draw: a neutral grey block signals "empty style".
        let c = [200, 200, 200, 255];
        for y in PAD..(h as i64 - PAD) {
            for x in PAD..(w as i64 - PAD) {
                renderer::blend::blend_px(canvas, x, y, w, h, c);
            }
        }
        return;
    }
    let row_h = (h as i64 - 2 * PAD) / swatches.len() as i64;
    for (i, (_rname, sym)) in swatches.iter().enumerate() {
        let top = PAD + i as i64 * row_h;
        draw_symbolizer(canvas, w, h, sym, PAD, top, w as i64 - PAD, top + row_h);
    }
}

/// One symbolizer swatch inside the (x0, y0)-(x1, y1) box.
fn draw_symbolizer(
    canvas: &mut [u8],
    w: usize,
    h: usize,
    sym: &Symbolizer,
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
) {
    let cx = (x0 + x1) / 2;
    let cy = (y0 + y1) / 2;
    let bw = (x1 - x0).max(4);
    let bh = (y1 - y0).max(4);
    match sym {
        Symbolizer::Point(p) => {
            // Cell may be as small as 4px (explicit tiny HEIGHT + many
            // rows): keep the upper bound >= 2.0 or f64::clamp panics.
            let max_r = ((bw.min(bh) as f64) / 2.0 - 1.0).max(2.0);
            let r = (p.size / 2.0).clamp(2.0, max_r);
            let mut sym2 = p.clone();
            sym2.size = r * 2.0;
            renderer::vector::stamp_mark(canvas, w, h, cx as f64, cy as f64, &sym2);
        }
        Symbolizer::Line(l) => {
            let path = vec![
                (x0 as f64 + 1.0, cy as f64),
                (x1 as f64 - 1.0, cy as f64),
            ];
            stroke_legend_path(canvas, w, h, &path, l.stroke, l.stroke_width, &l.dash);
        }
        Symbolizer::Polygon(p) => {
            let rect = vec![
                (x0 as f64 + 1.0, y0 as f64 + 1.0),
                (x1 as f64 - 1.0, y0 as f64 + 1.0),
                (x1 as f64 - 1.0, y1 as f64 - 1.0),
                (x0 as f64 + 1.0, y1 as f64 - 1.0),
            ];
            fill_legend_poly(canvas, w, h, &rect, p.fill);
            if p.stroke_width > 0.0 && p.stroke[3] > 0 {
                let mut closed = rect.clone();
                closed.push(rect[0]);
                stroke_legend_path(canvas, w, h, &closed, p.stroke, p.stroke_width, &p.dash);
            }
        }
        Symbolizer::Text(t) => {
            // Show the literal label, or the property name it references.
            let label = match &t.label {
                renderer::sld::LabelExpr::Literal(s) => s.clone(),
                renderer::sld::LabelExpr::Property(p) => p.clone(),
            };
            let scale = ((t.font_size / 7.0).round() as usize).clamp(1, 4);
            let tw = renderer::font::text_width(&label, scale) as i64;
            let th = renderer::font::text_height(scale) as i64;
            let x = cx - tw / 2;
            let y = cy - th / 2;
            if let Some(halo) = t.halo_fill {
                renderer::font::draw_text(canvas, w, h, x, y, &label, halo, scale);
            }
            renderer::font::draw_text(canvas, w, h, x, y, &label, t.fill, scale);
        }
        Symbolizer::Raster => {}
    }
}

fn draw_legacy_legend(canvas: &mut [u8], w: usize, h: usize, layer: &LayerRuntime) {
    let style: VectorStyle = layer.effective_vector_style();
    // Geometry kind from the first feature of the layer (if any).
    let kind = layer
        .vector_fc
        .as_ref()
        .and_then(|fc| fc.get("features")?.as_array()?.first())
        .and_then(|f| f.get("geometry"))
        .and_then(|g| g.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("Point");
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let span = (w.min(h) as f64 / 2.0 - 3.0).max(3.0);
    match kind {
        "LineString" | "MultiLineString" => {
            let path = vec![(cx - span, cy), (cx + span, cy)];
            stroke_legend_path(
                canvas,
                w,
                h,
                &path,
                style.stroke,
                style.stroke_width,
                &[],
            );
        }
        "Polygon" | "MultiPolygon" => {
            let rect = vec![
                (cx - span, cy - span),
                (cx + span, cy - span),
                (cx + span, cy + span),
                (cx - span, cy + span),
            ];
            fill_legend_poly(canvas, w, h, &rect, style.fill);
            let mut closed = rect.clone();
            closed.push(rect[0]);
            stroke_legend_path(
                canvas,
                w,
                h,
                &closed,
                style.stroke,
                style.stroke_width,
                &[],
            );
        }
        _ => {
            let sym = renderer::sld::PointSymbolizer {
                mark: MarkKind::Circle,
                fill: style.point_color,
                stroke: style.stroke,
                size: style.point_size * 2.0,
                stroke_width: style.stroke_width,
            };
            renderer::vector::stamp_mark(canvas, w, h, cx, cy, &sym);
        }
    }
}

fn draw_palette_ramp(canvas: &mut [u8], w: usize, h: usize, rs: &wms_common::style::RasterStyle) {
    let (min, max) = match (rs.min, rs.max) {
        (Some(a), Some(b)) if b > a => (a, b),
        _ => {
            // No value domain: draw the palette stops side by side.
            let n = rs.palette.len().max(1);
            for (i, c) in rs.palette.iter().enumerate() {
                let x0 = PAD + (w as i64 - 2 * PAD) * i as i64 / n as i64;
                let x1 = PAD + (w as i64 - 2 * PAD) * (i + 1) as i64 / n as i64;
                let col = [c[0], c[1], c[2], 255];
                for y in PAD..(h as i64 - PAD) {
                    for x in x0..x1 {
                        renderer::blend::blend_px(canvas, x, y, w, h, col);
                    }
                }
            }
            return;
        }
    };
    let pal = &rs.palette;
    if pal.is_empty() {
        return;
    }
    for y in PAD..(h as i64 - PAD) {
        let t = 1.0 - (y - PAD) as f64 / (h as i64 - 2 * PAD) as f64;
        let v = min + t * (max - min);
        let pos = ((v - min) / (max - min)).clamp(0.0, 1.0) * (pal.len() - 1) as f64;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(pal.len() - 1);
        let f = (pos - i0 as f64) as f32;
        let col = [
            (pal[i0][0] as f32 + (pal[i1][0] as f32 - pal[i0][0] as f32) * f).round() as u8,
            (pal[i0][1] as f32 + (pal[i1][1] as f32 - pal[i0][1] as f32) * f).round() as u8,
            (pal[i0][2] as f32 + (pal[i1][2] as f32 - pal[i0][2] as f32) * f).round() as u8,
            255,
        ];
        for x in PAD..(w as i64 - PAD) {
            renderer::blend::blend_px(canvas, x, y, w, h, col);
        }
    }
}

// --- small local raster primitives (legend canvases are tiny) ----------------

fn fill_legend_poly(canvas: &mut [u8], w: usize, h: usize, ring: &[(f64, f64)], color: [u8; 4]) {
    for y in 0..h as i64 {
        let mut xs: Vec<f64> = Vec::new();
        for i in 0..ring.len() {
            let (x0, y0) = ring[i];
            let (x1, y1) = ring[(i + 1) % ring.len()];
            if (y0 <= y as f64 && y1 > y as f64) || (y1 <= y as f64 && y0 > y as f64) {
                let t = (y as f64 - y0) / (y1 - y0);
                xs.push(x0 + t * (x1 - x0));
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            for x in pair[0].ceil() as i64..=pair[1].floor() as i64 {
                renderer::blend::blend_px(canvas, x, y, w, h, color);
            }
        }
    }
}

fn stroke_legend_path(
    canvas: &mut [u8],
    w: usize,
    h: usize,
    path: &[(f64, f64)],
    color: [u8; 4],
    thickness: f64,
    dash: &[f64],
) {
    let r = (thickness / 2.0).max(0.75);
    if dash.len() >= 2 {
        let total: f64 = dash.iter().sum();
        if total > 0.0 {
            let mut phase = 0.0;
            for seg in path.windows(2) {
                let (x0, y0) = seg[0];
                let (x1, y1) = seg[1];
                let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
                if len < 1e-9 {
                    continue;
                }
                let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
                let mut s = 0.0;
                while s < len - 1e-9 {
                    let mut run = total;
                    let mut on = false;
                    let mut acc = 0.0;
                    for (i, d) in dash.iter().enumerate() {
                        acc += *d;
                        if phase < acc {
                            run = acc - phase;
                            on = i % 2 == 0;
                            break;
                        }
                    }
                    let step = run.min(len - s);
                    if on {
                        let steps_n = (step / 0.5).ceil().max(1.0) as u32;
                        for k in 0..=steps_n {
                            let t = s + step * (k as f64 / steps_n as f64);
                            legend_disc(canvas, w, h, x0 + ux * t, y0 + uy * t, r, color);
                        }
                    }
                    s += step;
                    phase = (phase + step) % total;
                }
            }
            return;
        }
    }
    for seg in path.windows(2) {
        let (x0, y0) = seg[0];
        let (x1, y1) = seg[1];
        let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let steps = (dist / 0.5).ceil().max(1.0) as u32;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            legend_disc(canvas, w, h, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, r, color);
        }
    }
}

fn legend_disc(canvas: &mut [u8], w: usize, h: usize, x: f64, y: f64, r: f64, color: [u8; 4]) {
    let x0 = (x - r).floor() as i64;
    let x1 = (x + r).ceil() as i64;
    let y0 = (y - r).floor() as i64;
    let y1 = (y + r).ceil() as i64;
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            let dx = xx as f64 - x;
            let dy = yy as f64 - y;
            if dx * dx + dy * dy <= r * r {
                renderer::blend::blend_px(canvas, xx, yy, w, h, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point_layer() -> crate::layer::LayerRuntime {
        crate::layer::LayerRuntime {
            def: wms_common::layer::Layer {
                name: "places".into(),
                title: "Places".into(),
                source: wms_common::layer::SourceKind::Vector {
                    path: "unused".into(),
                },
                src_crs: "EPSG:4326".into(),
                style: None,
                sld: None,
                grids: vec![],
                tile_px: 256,
                extent: None,
            },
            style: None,
            sld: None,
            cog: None,
            source: None,
            source_uri: String::new(),
            vector_fc: Some(serde_json::json!({
                "type": "FeatureCollection",
                "features": [{
                    "type": "Feature",
                    "properties": {},
                    "geometry": { "type": "Point", "coordinates": [102.8, 24.9] }
                }]
            })),
            pmtiles: None,
            mbtiles: None,
            arcgis: None,
        }
    }

    #[test]
    fn legend_png_renders_point_symbol() {
        let sld = r#"<StyledLayerDescriptor><NamedLayer><UserStyle><FeatureTypeStyle>
          <Rule><Name>only</Name>
            <PointSymbolizer><Graphic><Mark><WellKnownName>star</WellKnownName>
              <Fill><SvgParameter name="fill">#ff0000</SvgParameter></Fill>
            </Mark><Size>14</Size></Graphic></PointSymbolizer>
          </Rule>
        </FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let mut layer = point_layer();
        layer.sld = Some(doc);
        let png = legend_png(&layer, 24, 24, None).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // Decode is out of scope; instead verify via the renderer that the
        // canvas actually carries red pixels (legend_png composites onto white).
        let mut canvas = vec![255u8; 24 * 24 * 4];
        draw_sld_legend(&mut canvas, 24, 24, layer.sld.as_ref().unwrap(), None);
        let red = canvas
            .chunks_exact(4)
            .filter(|c| c[0] > 200 && c[1] < 60 && c[2] < 60)
            .count();
        assert!(red > 5, "red star pixels missing from legend canvas");
    }

    #[test]
    fn legend_png_renders_polygon_and_line() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle>
          <Rule><PolygonSymbolizer>
            <Fill><SvgParameter name="fill">#00ff00</SvgParameter></Fill>
            <Stroke><SvgParameter name="stroke">#003300</SvgParameter>
              <SvgParameter name="stroke-width">2</SvgParameter></Stroke>
          </PolygonSymbolizer></Rule>
          <Rule><LineSymbolizer><Stroke>
            <SvgParameter name="stroke">#0000ff</SvgParameter>
            <SvgParameter name="stroke-width">3</SvgParameter>
            <SvgParameter name="stroke-dasharray">4 2</SvgParameter>
          </Stroke></LineSymbolizer></Rule>
        </FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let mut layer = point_layer();
        layer.sld = Some(doc);
        let mut canvas = vec![255u8; 24 * 48 * 4];
        draw_sld_legend(&mut canvas, 24, 48, layer.sld.as_ref().unwrap(), None);
        let green = canvas
            .chunks_exact(4)
            .filter(|c| c[1] > 200 && c[0] < 60)
            .count();
        let blue = canvas
            .chunks_exact(4)
            .filter(|c| c[2] > 200 && c[0] < 60)
            .count();
        assert!(green > 20, "polygon fill missing");
        assert!(blue > 5, "dashed line missing");
    }

    #[test]
    fn legend_png_empty_style_draws_placeholder() {
        let layer = point_layer();
        let png = legend_png(&layer, 20, 20, None).unwrap();
        assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn legend_tiny_canvas_does_not_panic() {
        // Regression: default 20x20 legend with several (rule, symbolizer)
        // rows squeezes each cell to ~2px; the point-radius clamp used to
        // panic ("min > max" from f64::clamp) inside spawn_blocking.
        let sld = r#"<StyledLayerDescriptor><NamedLayer><UserStyle><FeatureTypeStyle>
          <Rule><Name>big</Name>
            <PointSymbolizer><Graphic><Mark><WellKnownName>star</WellKnownName>
              <Fill><SvgParameter name="fill">#ffcc00</SvgParameter></Fill>
            </Mark><Size>14</Size></Graphic></PointSymbolizer>
            <TextSymbolizer><Label><PropertyName>name</PropertyName></Label>
              <Font><SvgParameter name="font-size">11</SvgParameter></Font>
            </TextSymbolizer>
          </Rule>
          <Rule><Name>small</Name>
            <PointSymbolizer><Graphic><Mark><WellKnownName>circle</WellKnownName>
              <Fill><SvgParameter name="fill">#3388cc</SvgParameter></Fill>
            </Mark><Size>10</Size></Graphic></PointSymbolizer>
          </Rule>
        </FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let mut layer = point_layer();
        layer.sld = Some(doc);
        let png = legend_png(&layer, 20, 20, None).unwrap();
        assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
        // Even a 8x8 request (cell height clamped to 4px) must not panic.
        let png2 = legend_png(&layer, 8, 8, None).unwrap();
        assert_eq!(&png2[..4], &[0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn auto_height_grows_with_style() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle>
          <Rule><PointSymbolizer><Graphic><Mark><WellKnownName>circle</WellKnownName></Mark></Graphic></PointSymbolizer></Rule>
          <Rule><LineSymbolizer><Stroke><SvgParameter name="stroke">#ff0000</SvgParameter></Stroke></LineSymbolizer></Rule>
          <Rule><TextSymbolizer><Label><PropertyName>name</PropertyName></Label></TextSymbolizer></Rule>
        </FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let mut layer = point_layer();
        layer.sld = Some(doc);
        assert_eq!(auto_height(&layer, None), DEF_H + 18 * 3);
        assert_eq!(auto_height(&layer, Some("nope")), DEF_H);
        // Legacy / raster legends draw a single swatch row.
        let plain = point_layer();
        assert_eq!(auto_height(&plain, None), DEF_H + 18);
    }

    #[test]
    fn sld_swatches_filters_by_rule() {
        let sld = r#"<StyledLayerDescriptor><NamedLayer><UserStyle><FeatureTypeStyle>
          <Rule><Name>big</Name>
            <PointSymbolizer><Graphic><Mark><WellKnownName>star</WellKnownName></Mark><Size>12</Size></Graphic></PointSymbolizer>
          </Rule>
          <Rule><Name>small</Name>
            <PointSymbolizer><Graphic><Mark><WellKnownName>circle</WellKnownName></Mark><Size>8</Size></Graphic></PointSymbolizer>
          </Rule>
        </FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        assert_eq!(sld_swatches(&doc, None).len(), 2);
        assert_eq!(sld_swatches(&doc, Some("big")).len(), 1);
        assert_eq!(sld_swatches(&doc, Some("nope")).len(), 0);
    }
}
