//! Simple vector rasterizer for WMS GetMap / tile serving of GeoJSON layers.
//!
//! Two entry points:
//!
//! * [`rasterize_geojson`] — legacy flat style: one [`VectorStyle`] applied
//!   uniformly to every feature (kept for the vector-style JSON paint path).
//! * [`rasterize_geojson_styled`] — SLD path: an [`SldDocument`] is evaluated
//!   **per feature** at the request scale denominator (all matching rules
//!   stack their symbolizers, `ElseFilter` chains), then
//!   all ops render in painter order (polygon fills → polygon outlines →
//!   lines → point marks → labels).
//!
//! Supported primitives: even-odd polygon fill, disc-stamped strokes with
//! `stroke-dasharray`, well-known point marks (circle / square / triangle /
//! star / cross / x / arrow / hexagon / pentagon, fill + stroke), and 5×7
//! bitmap text labels with halo, anchor point and displacement.
//!
//! GeoJSON coordinates are EPSG:4326 and are reprojected into the request CRS.

use serde_json::{Map, Value as Json};
use crate::wms_common::reproject::Reprojector;

use crate::renderer::blend::blend_px;
use crate::renderer::font;
use crate::renderer::sld::{MarkKind, PointSymbolizer, SldDocument, TextSymbolizer};

/// A simple flat vector style (legacy path: one style for all features).
#[derive(Clone, Debug)]
pub struct VectorStyle {
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub stroke_width: f64,
    pub point_color: [u8; 4],
    pub point_size: f64,
}

impl Default for VectorStyle {
    fn default() -> Self {
        Self {
            fill: [90, 170, 90, 160],
            stroke: [40, 80, 40, 255],
            stroke_width: 1.0,
            point_color: [200, 60, 60, 255],
            point_size: 3.0,
        }
    }
}

/// A rasterized geometry in canvas pixel space.
enum Shape {
    Polygon(Vec<Vec<(f64, f64)>>),
    Line(Vec<Vec<(f64, f64)>>),
    Point(Vec<(f64, f64)>),
}

/// Per-feature collected shapes + properties (SLD path).
struct FeatureShapes {
    props: Map<String, Json>,
    shapes: Vec<Shape>,
}

/// A render op in painter order.
enum Op {
    FillPoly {
        ring: Vec<(f64, f64)>,
        color: [u8; 4],
    },
    StrokePoly {
        ring: Vec<(f64, f64)>,
        color: [u8; 4],
        width: f64,
        dash: Vec<f64>,
    },
    StrokeLine {
        path: Vec<(f64, f64)>,
        color: [u8; 4],
        width: f64,
        dash: Vec<f64>,
    },
    Mark {
        x: f64,
        y: f64,
        sym: PointSymbolizer,
    },
    Text {
        x: f64,
        y: f64,
        sym: TextSymbolizer,
        label: String,
    },
}

// ---------------------------------------------------------------------------
// Legacy flat-style entry point
// ---------------------------------------------------------------------------

/// Rasterize a GeoJSON FeatureCollection with one flat style (legacy path).
///
/// `transformer` reprojects GeoJSON (EPSG:4326) coordinates into the request
/// CRS; it is injected by the caller (see `layer::render_vector`) so this
/// crate stays decoupled from the concrete `projection` backend.
pub fn rasterize_geojson(
    fc: &serde_json::Value,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    style: &VectorStyle,
    transformer: &dyn Reprojector,
) -> Vec<u8> {
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    let features = fc
        .get("features")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Collect shapes, applying the CRS transform + viewport mapping.
    let mut shapes: Vec<Shape> = Vec::new();
    for feat in &features {
        let geom = match feat.get("geometry") {
            Some(g) if !g.is_null() => g,
            _ => continue,
        };
        collect_shapes(geom, transformer, bbox, width, height, &mut shapes);
    }

    // Painter's algorithm: polygons, then lines, then points.
    for s in &shapes {
        if let Shape::Polygon(rings) = s {
            for ring in rings {
                fill_polygon(&mut canvas, ring, width as usize, height as usize, style.fill);
            }
        }
    }
    for s in &shapes {
        if let Shape::Polygon(rings) = s {
            for ring in rings {
                stroke_path(
                    &mut canvas,
                    ring,
                    width as usize,
                    height as usize,
                    style.stroke,
                    style.stroke_width,
                    &[],
                );
            }
        } else if let Shape::Line(paths) = s {
            for path in paths {
                stroke_path(
                    &mut canvas,
                    path,
                    width as usize,
                    height as usize,
                    style.stroke,
                    style.stroke_width,
                    &[],
                );
            }
        }
    }
    for s in &shapes {
        if let Shape::Point(pts) = s {
            for &(x, y) in pts {
                stamp_disc(
                    &mut canvas,
                    x,
                    y,
                    style.point_size,
                    width as usize,
                    height as usize,
                    style.point_color,
                );
            }
        }
    }
    canvas
}

// ---------------------------------------------------------------------------
// SLD styled entry point
// ---------------------------------------------------------------------------

/// Rasterize a GeoJSON FeatureCollection through an SLD document.
///
/// `scale` is the map scale denominator (1:X); rules with
/// Min/MaxScaleDenominator windows are filtered by it. Pass `f64::MAX` to
/// disable scale filtering (every scale window matches). `transformer`
/// reprojects GeoJSON (EPSG:4326) coordinates into the request CRS and is
/// injected by the caller (`layer::render_vector`).
pub fn rasterize_geojson_styled(
    fc: &serde_json::Value,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    doc: &SldDocument,
    scale: f64,
    transformer: &dyn Reprojector,
) -> Vec<u8> {
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    let features = match fc.get("features").and_then(|v| v.as_array()) {
        Some(f) => f,
        None => return canvas,
    };

    // 1) Per-feature geometry + properties.
    let mut collected: Vec<FeatureShapes> = Vec::new();
    for feat in features {
        let geom = match feat.get("geometry") {
            Some(g) if !g.is_null() => g,
            _ => continue,
        };
        let props = feat
            .get("properties")
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        let mut shapes: Vec<Shape> = Vec::new();
        collect_shapes(geom, transformer, bbox, width, height, &mut shapes);
        if !shapes.is_empty() {
            collected.push(FeatureShapes { props, shapes });
        }
    }

    // 2) Per-feature rule evaluation → render ops.
    let mut fills: Vec<Op> = Vec::new();
    let mut poly_strokes: Vec<Op> = Vec::new();
    let mut lines: Vec<Op> = Vec::new();
    let mut marks: Vec<Op> = Vec::new();
    let mut texts: Vec<Op> = Vec::new();
    for fs in &collected {
        let rules = doc.matching_rules(&fs.props, scale);
        for rule in rules {
            for sym in &rule.symbolizers {
                use crate::renderer::sld::Symbolizer;
                match sym {
                    Symbolizer::Polygon(p) => {
                        for s in &fs.shapes {
                            if let Shape::Polygon(rings) = s {
                                for ring in rings {
                                    if p.fill[3] > 0 {
                                        fills.push(Op::FillPoly {
                                            ring: ring.clone(),
                                            color: p.fill,
                                        });
                                    }
                                    if p.stroke[3] > 0 && p.stroke_width > 0.0 {
                                        poly_strokes.push(Op::StrokePoly {
                                            ring: ring.clone(),
                                            color: p.stroke,
                                            width: p.stroke_width,
                                            dash: p.dash.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Symbolizer::Line(l) => {
                        for s in &fs.shapes {
                            if let Shape::Line(paths) = s {
                                for path in paths {
                                    lines.push(Op::StrokeLine {
                                        path: path.clone(),
                                        color: l.stroke,
                                        width: l.stroke_width,
                                        dash: l.dash.clone(),
                                    });
                                }
                            }
                        }
                    }
                    Symbolizer::Point(p) => {
                        for s in &fs.shapes {
                            if let Shape::Point(pts) = s {
                                for &(x, y) in pts {
                                    marks.push(Op::Mark { x, y, sym: p.clone() });
                                }
                            }
                        }
                    }
                    Symbolizer::Text(t) => {
                        let label = t.label.resolve(&fs.props);
                        if label.is_empty() {
                            continue;
                        }
                        for s in &fs.shapes {
                            if let Some((x, y)) = text_anchor(s) {
                                texts.push(Op::Text {
                                    x,
                                    y,
                                    sym: t.clone(),
                                    label: label.clone(),
                                });
                            }
                        }
                    }
                    Symbolizer::Raster => {}
                }
            }
        }
    }

    // 3) Painter order: fills → polygon outlines → lines → marks → labels.
    for op in fills.iter().chain(poly_strokes.iter()) {
        match op {
            Op::FillPoly { ring, color } => {
                fill_polygon(&mut canvas, ring, width as usize, height as usize, *color)
            }
            Op::StrokePoly {
                ring,
                color,
                width: lw,
                dash,
            } => stroke_path(
                &mut canvas,
                ring,
                width as usize,
                height as usize,
                *color,
                *lw,
                dash,
            ),
            _ => {}
        }
    }
    for op in &lines {
        if let Op::StrokeLine {
            path,
            color,
            width: lw,
            dash,
        } = op
        {
            stroke_path(
                &mut canvas,
                path,
                width as usize,
                height as usize,
                *color,
                *lw,
                dash,
            );
        }
    }
    for op in &marks {
        if let Op::Mark { x, y, sym } = op {
            stamp_mark(&mut canvas, width as usize, height as usize, *x, *y, sym);
        }
    }
    for op in &texts {
        if let Op::Text { x, y, sym, label } = op {
            draw_label(
                &mut canvas,
                width as usize,
                height as usize,
                *x,
                *y,
                sym,
                label,
            );
        }
    }
    canvas
}

/// Scale denominator (1:X) of a map frame, from its bbox span in meters.
/// `width` is the pixel width; the standardized rendering pixel (0.28 mm)
/// converts meters-per-pixel to the denominator. `transformer` reprojects the
/// bbox span into EPSG:3857 (injected by the caller, `layer::render_vector`).
pub fn scale_denominator(
    bbox: [f64; 4],
    width: u32,
    transformer: &dyn Reprojector,
) -> f64 {
    if width == 0 {
        return f64::MAX;
    }
    let [minx, miny, maxx, maxy] = bbox;
    let mid_y = (miny + maxy) / 2.0;
    let span_m = match (transformer.to_source(minx, mid_y), transformer.to_source(maxx, mid_y)) {
        (Some((x0, _)), Some((x1, _))) if x0.is_finite() && x1.is_finite() => (x1 - x0).abs(),
        _ => (maxx - minx).abs() * 111_320.0,
    };
    let mpp = span_m / width as f64;
    (mpp / 0.00028).max(1.0)
}

// ---------------------------------------------------------------------------
// Geometry collection
// ---------------------------------------------------------------------------

/// Recursively convert a GeoJSON geometry value into viewport shapes.
fn collect_shapes(
    geom: &serde_json::Value,
    transformer: &dyn Reprojector,
    bbox: [f64; 4],
    width: u32,
    height: u32,
    out: &mut Vec<Shape>,
) {
    let gtype = geom.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let coords = geom.get("coordinates");
    let [minx, miny, maxx, maxy] = bbox;
    let dx = (maxx - minx) / width as f64;
    let dy = (maxy - miny) / height as f64;
    let map = |lon: f64, lat: f64| -> Option<(f64, f64)> {
        let (x, y) = transformer.to_source(lon, lat)?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some(((x - minx) / dx, (maxy - y) / dy))
    };
    let map_pos = |c: &serde_json::Value| -> Option<(f64, f64)> {
        let a = c.as_array()?;
        let lon = a.first()?.as_f64()?;
        let lat = a.get(1)?.as_f64()?;
        map(lon, lat)
    };
    let map_line = |c: &serde_json::Value| -> Vec<(f64, f64)> {
        c.as_array()
            .map(|arr| arr.iter().filter_map(map_pos).collect())
            .unwrap_or_default()
    };

    match gtype {
        "Point" => {
            if let Some(p) = coords.and_then(map_pos) {
                out.push(Shape::Point(vec![p]));
            }
        }
        "MultiPoint" => {
            let pts: Vec<(f64, f64)> = coords
                .and_then(|c| c.as_array())
                .map(|arr| arr.iter().filter_map(map_pos).collect())
                .unwrap_or_default();
            if !pts.is_empty() {
                out.push(Shape::Point(pts));
            }
        }
        "LineString" => {
            if let Some(c) = coords {
                let line = map_line(c);
                if !line.is_empty() {
                    out.push(Shape::Line(vec![line]));
                }
            }
        }
        "MultiLineString" => {
            let lines: Vec<Vec<(f64, f64)>> = coords
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| {
                            let l = map_line(l);
                            (!l.is_empty()).then_some(l)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !lines.is_empty() {
                out.push(Shape::Line(lines));
            }
        }
        "Polygon" => {
            if let Some(c) = coords {
                let rings: Vec<Vec<(f64, f64)>> = c
                    .as_array()
                    .map(|arr| arr.iter().map(map_line).filter(|r| r.len() >= 3).collect())
                    .unwrap_or_default();
                if !rings.is_empty() {
                    out.push(Shape::Polygon(rings));
                }
            }
        }
        "MultiPolygon" => {
            if let Some(c) = coords {
                for poly in c.as_array().into_iter().flatten() {
                    let rings: Vec<Vec<(f64, f64)>> = poly
                        .as_array()
                        .map(|arr| arr.iter().map(map_line).filter(|r| r.len() >= 3).collect())
                        .unwrap_or_default();
                    if !rings.is_empty() {
                        out.push(Shape::Polygon(rings));
                    }
                }
            }
        }
        "GeometryCollection" => {
            if let Some(gs) = geom.get("geometries").and_then(|g| g.as_array()) {
                for g in gs {
                    collect_shapes(g, transformer, bbox, width, height, out);
                }
            }
        }
        _ => {}
    }
}

/// Label anchor for a shape: the point itself, the line's middle vertex, or
/// the polygon's vertex-average centroid of its first ring.
fn text_anchor(shape: &Shape) -> Option<(f64, f64)> {
    match shape {
        Shape::Point(pts) => pts.first().copied(),
        Shape::Line(paths) => paths
            .first()
            .and_then(|p| (!p.is_empty()).then(|| p[p.len() / 2])),
        Shape::Polygon(rings) => rings.first().and_then(|ring| {
            (!ring.is_empty()).then(|| {
                let n = ring.len() as f64;
                let (sx, sy) = ring
                    .iter()
                    .fold((0.0, 0.0), |(ax, ay), &(x, y)| (ax + x, ay + y));
                (sx / n, sy / n)
            })
        }),
    }
}

// ---------------------------------------------------------------------------
// Polygon fill / strokes
// ---------------------------------------------------------------------------

/// Even-odd scanline polygon fill.
fn fill_polygon(
    canvas: &mut [u8],
    ring: &[(f64, f64)],
    width: usize,
    height: usize,
    color: [u8; 4],
) {
    if ring.len() < 3 {
        return;
    }
    let ymin = ring
        .iter()
        .map(|p| p.1.floor() as i64)
        .min()
        .unwrap_or(0)
        .max(0);
    let ymax = ring
        .iter()
        .map(|p| p.1.ceil() as i64)
        .max()
        .unwrap_or(0)
        .min(height as i64 - 1);
    for y in ymin..=ymax {
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
            let x0 = pair[0].ceil() as i64;
            let x1 = pair[1].floor() as i64;
            for x in x0..=x1 {
                blend_px(canvas, x, y, width, height, color);
            }
        }
    }
}

/// Stroke a polyline (or ring) with a thickness via disc stamping, honouring
/// an optional `stroke-dasharray` pattern (pixels, on/off alternating).
fn stroke_path(
    canvas: &mut [u8],
    path: &[(f64, f64)],
    width: usize,
    height: usize,
    color: [u8; 4],
    thickness: f64,
    dash: &[f64],
) {
    if path.len() < 2 || thickness <= 0.0 || color[3] == 0 {
        return;
    }
    if dash.len() >= 2 {
        stroke_dashed(canvas, path, width, height, color, thickness, dash);
        return;
    }
    let r = (thickness / 2.0).max(0.5);
    for seg in path.windows(2) {
        let (x0, y0) = seg[0];
        let (x1, y1) = seg[1];
        let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let steps = (dist / 0.5).ceil().max(1.0) as u32;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            stamp_disc(
                canvas,
                x0 + (x1 - x0) * t,
                y0 + (y1 - y0) * t,
                r,
                width,
                height,
                color,
            );
        }
    }
}

/// Dash-pattern stroke: walk the polyline with a pattern cursor, stamping
/// discs only on "on" runs. The phase persists across segments.
fn stroke_dashed(
    canvas: &mut [u8],
    path: &[(f64, f64)],
    width: usize,
    height: usize,
    color: [u8; 4],
    thickness: f64,
    dash: &[f64],
) {
    let total: f64 = dash.iter().sum();
    if total <= 0.0 {
        return;
    }
    let r = (thickness / 2.0).max(0.5);
    let mut phase = 0.0f64;
    for seg in path.windows(2) {
        let (x0, y0) = seg[0];
        let (x1, y1) = seg[1];
        let seg_len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        if seg_len < 1e-9 {
            continue;
        }
        let (ux, uy) = ((x1 - x0) / seg_len, (y1 - y0) / seg_len);
        let mut s = 0.0;
        while s < seg_len - 1e-9 {
            // Remaining length of the current pattern run (and whether it is
            // an "on" run).
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
            let step = run.min(seg_len - s);
            if on {
                let steps_n = (step / 0.5).ceil().max(1.0) as u32;
                for k in 0..=steps_n {
                    let t = s + step * (k as f64 / steps_n as f64);
                    stamp_disc(canvas, x0 + ux * t, y0 + uy * t, r, width, height, color);
                }
            }
            s += step;
            phase = (phase + step) % total;
        }
    }
}

/// Stamp a filled disc at (x, y).
fn stamp_disc(
    canvas: &mut [u8],
    x: f64,
    y: f64,
    r: f64,
    width: usize,
    height: usize,
    color: [u8; 4],
) {
    let r = r.max(0.5);
    let x0 = (x - r).floor() as i64;
    let x1 = (x + r).ceil() as i64;
    let y0 = (y - r).floor() as i64;
    let y1 = (y + r).ceil() as i64;
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            let dx = xx as f64 - x;
            let dy = yy as f64 - y;
            if dx * dx + dy * dy <= r * r {
                blend_px(canvas, xx, yy, width, height, color);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Well-known marks
// ---------------------------------------------------------------------------

/// A mark decomposed into fillable polygons and thick lines.
struct MarkGeom {
    polys: Vec<Vec<(f64, f64)>>,
    lines: Vec<((f64, f64), (f64, f64), f64)>, // p0, p1, thickness
}

/// Mark geometry for radius `r`, centered on the origin (caller translates).
fn mark_geometry(kind: MarkKind, r: f64) -> MarkGeom {
    let mut g = MarkGeom {
        polys: Vec::new(),
        lines: Vec::new(),
    };
    match kind {
        MarkKind::Circle => {
            // 24-gon approximation.
            let n = 24usize;
            let pts: Vec<(f64, f64)> = (0..n)
                .map(|i| {
                    let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                    (r * a.cos(), r * a.sin())
                })
                .collect();
            g.polys.push(pts);
        }
        MarkKind::Square => {
            g.polys.push(vec![
                (-r, -r),
                (r, -r),
                (r, r),
                (-r, r),
            ]);
        }
        MarkKind::Triangle => {
            // Equilateral-ish triangle pointing up.
            g.polys.push(vec![
                (0.0, -r),
                (r * 0.866, r * 0.5),
                (-r * 0.866, r * 0.5),
            ]);
        }
        MarkKind::Star => {
            // 5-point star (outer r, inner 0.42r), pointing up.
            let inner = r * 0.42;
            let mut pts = Vec::with_capacity(10);
            for i in 0..10 {
                let a = -std::f64::consts::PI / 2.0 + std::f64::consts::PI * i as f64 / 5.0;
                let rad = if i % 2 == 0 { r } else { inner };
                pts.push((rad * a.cos(), rad * a.sin()));
            }
            g.polys.push(pts);
        }
        MarkKind::Cross => {
            let t = r * 0.36;
            g.polys.push(vec![(-t, -r), (t, -r), (t, r), (-t, r)]);
            g.polys.push(vec![(-r, -t), (r, -t), (r, t), (-r, t)]);
        }
        MarkKind::X => {
            let t = (r * 0.36).max(1.0);
            g.lines.push(((-r, -r), (r, r), t));
            g.lines.push(((-r, r), (r, -r), t));
        }
        MarkKind::Arrow => {
            // Arrow pointing up: head + shaft as one polygon.
            let w = r * 0.72;
            let shaft = r * 0.30;
            g.polys.push(vec![
                (0.0, -r),
                (w, -r + w * 0.9),
                (shaft, -r + w * 0.9),
                (shaft, r),
                (-shaft, r),
                (-shaft, -r + w * 0.9),
                (-w, -r + w * 0.9),
            ]);
        }
        MarkKind::Hexagon | MarkKind::Pentagon => {
            let n = kind.sides() as usize;
            let pts: Vec<(f64, f64)> = (0..n)
                .map(|i| {
                    let a = -std::f64::consts::PI / 2.0 + 2.0 * std::f64::consts::PI * i as f64
                        / n as f64;
                    (r * a.cos(), r * a.sin())
                })
                .collect();
            g.polys.push(pts);
        }
    }
    g
}

/// Draw mark geometry translated to (x, y).
fn draw_mark_geom(
    canvas: &mut [u8],
    cw: usize,
    ch: usize,
    x: f64,
    y: f64,
    g: &MarkGeom,
    color: [u8; 4],
) {
    for poly in &g.polys {
        let ring: Vec<(f64, f64)> = poly.iter().map(|&(px, py)| (x + px, y + py)).collect();
        fill_polygon(canvas, &ring, cw, ch, color);
    }
    for &(p0, p1, t) in &g.lines {
        stroke_path(canvas, &[(x + p0.0, y + p0.1), (x + p1.0, y + p1.1)], cw, ch, color, t, &[]);
    }
}

/// Stamp a point symbolizer's mark at (x, y): a bigger stroke-coloured pass
/// followed by the fill pass (cheap outline that reads well at small sizes).
/// Public so the legend renderer can reuse the exact same mark geometry.
pub fn stamp_mark(canvas: &mut [u8], cw: usize, ch: usize, x: f64, y: f64, sym: &PointSymbolizer) {
    let r = (sym.size / 2.0).max(0.5);
    let sw = sym.stroke_width.max(0.0);
    if sym.stroke[3] > 0 && sw > 0.0 {
        let g = mark_geometry(sym.mark, r + sw / 2.0);
        draw_mark_geom(canvas, cw, ch, x, y, &g, sym.stroke);
    }
    let r_fill = (r - sw / 2.0).max(0.0);
    if r_fill > 0.0 && sym.fill[3] > 0 {
        let g = mark_geometry(sym.mark, r_fill);
        draw_mark_geom(canvas, cw, ch, x, y, &g, sym.fill);
    }
}

// ---------------------------------------------------------------------------
// Text labels
// ---------------------------------------------------------------------------

/// Draw a text symbolizer's label anchored at (x, y) (canvas px, y down).
/// SE displacement is y-up, so it is negated for canvas space.
fn draw_label(
    canvas: &mut [u8],
    cw: usize,
    ch: usize,
    x: f64,
    y: f64,
    sym: &TextSymbolizer,
    label: &str,
) {
    if label.is_empty() || sym.fill[3] == 0 {
        return;
    }
    let scale = ((sym.font_size / font::GLYPH_H as f64).round() as usize).max(1);
    let tw = font::text_width(label, scale) as f64;
    let th = font::text_height(scale) as f64;
    let ax = x + sym.displacement.0 - tw * sym.anchor.0;
    let ay = y - sym.displacement.1 - th * sym.anchor.1;
    if let Some(halo) = sym.halo_fill {
        if sym.halo_radius > 0.0 {
            let hr = sym.halo_radius.round().max(1.0) as i64;
            for dy in -hr..=hr {
                for dx in -hr..=hr {
                    if dx * dx + dy * dy > hr * hr {
                        continue; // round halo
                    }
                    font::draw_text(
                        canvas,
                        cw,
                        ch,
                        (ax + dx as f64).round() as i64,
                        (ay + dy as f64).round() as i64,
                        label,
                        halo,
                        scale,
                    );
                }
            }
        }
    }
    font::draw_text(
        canvas,
        cw,
        ch,
        ax.round() as i64,
        ay.round() as i64,
        label,
        sym.fill,
        scale,
    );
    // Faux-bold: a second pass shifted 1 px right (the 5x7 font has no bold
    // variant). Skipped for italic (documented boundary).
    if sym.bold {
        font::draw_text(
            canvas,
            cw,
            ch,
            ax.round() as i64 + 1,
            ay.round() as i64,
            label,
            sym.fill,
            scale,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::sld::{Comparison, Filter, LabelExpr, Operand, PolygonSymbolizer, Rule, Value};
    use crate::projection::Transformer;

    /// Identity transformer (GeoJSON 4326 -> request 4326): no reprojection.
    fn identity_tr() -> Transformer {
        Transformer::new_any("EPSG:4326", "EPSG:4326", None).unwrap()
    }
    /// GeoJSON 4326 -> 3857 (meters), matching the frame's scale computation.
    fn to_3857_tr() -> Transformer {
        Transformer::new_any("EPSG:4326", "EPSG:3857", None).unwrap()
    }
    /// Identity transformer in 3857 (request already in meters).
    fn identity_3857_tr() -> Transformer {
        Transformer::new_any("EPSG:3857", "EPSG:3857", None).unwrap()
    }

    fn sample_fc() -> serde_json::Value {
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": { "name": "Kunming", "pop": 8460088.0, "kind": "city" },
                    "geometry": { "type": "Point", "coordinates": [102.8329, 24.8801] }
                },
                {
                    "type": "Feature",
                    "properties": { "name": "Dali", "pop": 400000.0, "kind": "town" },
                    "geometry": { "type": "Point", "coordinates": [100.2345, 25.6065] }
                },
                {
                    "type": "Feature",
                    "properties": { "name": "Lake", "kind": "water" },
                    "geometry": { "type": "Polygon", "coordinates": [[[102.7, 24.5], [103.0, 24.5], [103.0, 24.8], [102.7, 24.8], [102.7, 24.5]]] }
                }
            ]
        })
    }

    fn doc_from_rules(rules: Vec<Rule>) -> SldDocument {
        SldDocument {
            feature_type_styles: vec![crate::renderer::sld::FeatureTypeStyle { rules }],
            ..Default::default()
        }
    }

    fn count_color(canvas: &[u8], target: [u8; 3], tol: u8) -> usize {
        canvas
            .chunks_exact(4)
            .filter(|c| {
                (c[0] as i32 - target[0] as i32).unsigned_abs() <= tol as u32
                    && (c[1] as i32 - target[1] as i32).unsigned_abs() <= tol as u32
                    && (c[2] as i32 - target[2] as i32).unsigned_abs() <= tol as u32
                    && c[3] > 0
            })
            .count()
    }

    #[test]
    fn styled_renders_rule_specific_marks() {
        // pop > 8M → red star; else → blue circle.
        let big = Rule {
            filter: Some(Filter::Comparison(
                Comparison::Greater,
                Operand::Property("pop".into()),
                Operand::Literal(Value::Num(8000000.0)),
            )),
            symbolizers: vec![crate::renderer::sld::Symbolizer::Point(PointSymbolizer {
                mark: MarkKind::Star,
                fill: [255, 0, 0, 255],
                stroke: [0, 0, 0, 255],
                size: 20.0,
                stroke_width: 1.0,
            })],
            ..Default::default()
        };
        let small = Rule {
            filter: Some(Filter::Comparison(
                Comparison::LessOrEq,
                Operand::Property("pop".into()),
                Operand::Literal(Value::Num(8000000.0)),
            )),
            symbolizers: vec![crate::renderer::sld::Symbolizer::Point(PointSymbolizer {
                mark: MarkKind::Circle,
                fill: [0, 0, 255, 255],
                stroke: [0, 0, 0, 255],
                size: 12.0,
                stroke_width: 1.0,
            })],
            ..Default::default()
        };
        let doc = doc_from_rules(vec![big, small]);
        let canvas = rasterize_geojson_styled(
            &sample_fc(),
            [99.5, 24.0, 104.0, 26.0],
            256,
            256,
            &doc,
            f64::MAX,
            &identity_tr(),
        );
        let red = count_color(&canvas, [255, 0, 0], 8);
        let blue = count_color(&canvas, [0, 0, 255], 8);
        assert!(red > 0, "big city star (red) not rendered");
        assert!(blue > 0, "small city circle (blue) not rendered");
        // Star at size 20 paints more pixels than the size-12 circle.
        assert!(red > blue, "star {red} should exceed circle {blue}");
    }

    #[test]
    fn styled_renders_polygon_fill_and_text_label() {
        let rule = Rule {
            symbolizers: vec![
                crate::renderer::sld::Symbolizer::Polygon(PolygonSymbolizer {
                    fill: [0, 128, 0, 200],
                    stroke: [0, 64, 0, 255],
                    stroke_width: 2.0,
                    dash: vec![],
                }),
                crate::renderer::sld::Symbolizer::Text(TextSymbolizer {
                    label: LabelExpr::Property("name".into()),
                    font_size: 12.0,
                    bold: false,
                    italic: false,
                    fill: [255, 255, 255, 255],
                    halo_fill: Some([0, 0, 0, 255]),
                    halo_radius: 1.0,
                    anchor: (0.5, 0.5),
                    displacement: (0.0, 0.0),
                }),
            ],
            ..Default::default()
        };
        let doc = doc_from_rules(vec![rule]);
        let canvas = rasterize_geojson_styled(
            &sample_fc(),
            [102.5, 24.3, 103.2, 25.0],
            256,
            256,
            &doc,
            f64::MAX,
            &identity_tr(),
        );
        // Green polygon fill present.
        assert!(count_color(&canvas, [0, 128, 0], 40) > 100, "polygon fill missing");
        // White label pixels present (text over dark halo).
        assert!(count_color(&canvas, [255, 255, 255], 8) > 10, "label text missing");
    }

    #[test]
    fn styled_dashed_line_renders() {
        let rule = Rule {
            symbolizers: vec![crate::renderer::sld::Symbolizer::Line(crate::renderer::sld::LineSymbolizer {
                stroke: [255, 0, 255, 255],
                stroke_width: 3.0,
                dash: vec![6.0, 4.0],
            })],
            ..Default::default()
        };
        let fc = serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {},
                "geometry": { "type": "LineString", "coordinates": [[102.6, 24.6], [103.0, 24.7]] }
            }]
        });
        let doc = doc_from_rules(vec![rule]);
        let canvas = rasterize_geojson_styled(
            &fc,
            [102.5, 24.5, 103.1, 24.9],
            256,
            128,
            &doc,
            f64::MAX,
            &identity_tr(),
        );
        assert!(count_color(&canvas, [255, 0, 255], 8) > 20, "dashed line missing");
    }

    #[test]
    fn scale_denominator_matches_bbox_and_width() {
        // 1 degree at the equator ≈ 111,319 m; a 256px frame → ~436,246:1.
        let sd = scale_denominator([0.0, 0.0, 1.0, 1.0], 256, &to_3857_tr());
        assert!(
            (sd - 111_320.0 / 256.0 / 0.00028).abs() / sd < 0.05,
            "unexpected scale denominator {sd}"
        );
        // 3857 input: the same 111,320 m span expressed in meters (1 degree
        // of longitude at the equator) must give the same denominator.
        let sd2 = scale_denominator([0.0, 0.0, 111_319.49, 0.0], 256, &identity_3857_tr());
        assert!((sd2 - 111_320.0 / 256.0 / 0.00028).abs() / sd2 < 0.05, "3857 sd {sd2}");
        // Degenerate width guards against div-by-zero.
        assert_eq!(scale_denominator([0.0, 0.0, 1.0, 1.0], 0, &to_3857_tr()), f64::MAX);
    }

    #[test]
    fn legacy_flat_style_still_renders() {
        let canvas = rasterize_geojson(
            &sample_fc(),
            [99.5, 24.0, 104.0, 26.0],
            128,
            128,
            &VectorStyle::default(),
            &identity_tr(),
        );
        let painted = canvas.chunks_exact(4).filter(|c| c[3] > 0).count();
        assert!(painted > 0, "legacy path rendered nothing");
    }

    #[test]
    fn empty_document_renders_blank() {
        let doc = SldDocument::default();
        let canvas = rasterize_geojson_styled(
            &sample_fc(),
            [99.5, 24.0, 104.0, 26.0],
            64,
            64,
            &doc,
            f64::MAX,
            &identity_tr(),
        );
        assert!(canvas.iter().all(|&b| b == 0));
    }
}
