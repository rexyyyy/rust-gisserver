//! Windowed raster rendering (ported from TerraServe's `render.rs`).
//!
//! Given a request bbox (in the request CRS), the renderer reprojects output
//! pixel centers into the COG's native pixel space, selects the coarsest
//! overview that still resolves the request, reads a tile-aligned window via
//! the COG's `RangeSource`, and resamples (nearest / bilinear) into the output
//! RGBA canvas.

pub mod blend;
pub mod font;
pub mod png;
pub mod sld;
pub mod vector;

use crate::cog::{read_window_rgba, Cog};
use crate::storage::{RangeSource, TileCache};
use crate::wms_common::reproject::Reprojector;
use crate::wms_common::style::RasterStyle;

/// Resampling mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resample {
    Nearest,
    Bilinear,
}

/// A render request (the WMS GetMap / tile-serving core).
#[derive(Clone, Debug)]
pub struct RenderRequest<'a> {
    pub bbox: [f64; 4],
    /// Request CRS (EPSG:4326 / EPSG:3857 / source CRS).
    pub crs: &'a str,
    pub src_crs: &'a str,
    pub width: u32,
    pub height: u32,
    pub resample: Resample,
    pub style: Option<&'a RasterStyle>,
}

/// Render error.
#[derive(Debug)]
pub enum RenderError {
    Projection(String),
    Cog(String),
    Format(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Projection(m) => write!(f, "projection: {m}"),
            RenderError::Cog(m) => write!(f, "cog: {m}"),
            RenderError::Format(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<crate::wms_common::MapServerError> for RenderError {
    fn from(e: crate::wms_common::MapServerError) -> Self {
        RenderError::Cog(e.to_string())
    }
}

/// Render a window of a COG to an RGBA canvas.
///
/// `transformer` reprojects output-pixel center coordinates from the request
/// CRS into the COG's source CRS. It is injected by the caller (which owns
/// the projection backend) so this crate stays decoupled from `projection`.
pub fn render_with_cog(
    req: &RenderRequest<'_>,
    cog: &Cog,
    src: &dyn RangeSource,
    cache: Option<&TileCache>,
    transformer: &dyn Reprojector,
) -> Result<Vec<u8>, RenderError> {
    let (w, h) = (req.width as usize, req.height as usize);
    if w == 0 || h == 0 {
        return Err(RenderError::Format("width/height must be > 0".into()));
    }
    let mut out = vec![0u8; w * h * 4];

    let [minx, miny, maxx, maxy] = req.bbox;
    let dx = (maxx - minx) / w as f64;
    let dy = (maxy - miny) / h as f64;
    let native = cog.levels[0].geo;

    // Pass 1: map each output pixel center to NATIVE source-pixel coordinates.
    //
    // Identity fast path: when the request CRS equals the source CRS the
    // transform is a passthrough and `geo_to_pix` is affine, so the whole
    // output grid maps affinely. Solving the map from the four output corners
    // (4 projections + 1 midpoint check) replaces `w*h` per-pixel projections
    // — 262,144 `to_source` calls for a 512x256 GetMap collapse to 5.
    let mut native_coords = vec![[f64::NAN; 2]; w * h];
    // The affine fast path only applies when the transform is the identity
    // (request CRS == source CRS) over an affine geotransform. Anything else
    // -- every real reprojection, e.g. ALL Web Mercator tiles on a 4326
    // source -- must take the exact per-pixel path below. Skipping it leaves
    // `native_coords` all-NaN and silently returns a blank canvas (that
    // regression shipped once: the fast-path gate accidentally swallowed the
    // non-identity case). Keep the structure flat: one `fast_ok` flag, one
    // fallback loop.
    let fast_ok = transformer.is_identity()
        && fill_affine_native_coords(req, &native, w, h, &mut native_coords);
    if !fast_ok {
        // Degenerate geometry (zero-width bbox, non-finite): the exact
        // per-pixel path yields all-NaN coords -> empty canvas.
        for j in 0..h {
            let oy = maxy - (j as f64 + 0.5) * dy;
            for i in 0..w {
                let ox = minx + (i as f64 + 0.5) * dx;
                if let Some((sx, sy)) = transformer.to_source(ox, oy) {
                    if sx.is_finite() && sy.is_finite() {
                        let (u, v) = native.geo_to_pix(sx, sy);
                        native_coords[j * w + i] = [u, v];
                    }
                }
            }
        }
    }

    // Desired downsample factor: SOURCE pixels spanned per output pixel.
    // Measured from Pass 1's source-pixel coordinates, never from the raw
    // request bbox: `req.bbox` is in the *request* CRS while the source
    // geotransform is in *degrees*, so estimating from the bbox mixed units --
    // every Web Mercator tile over a 4326/4490 source got a factor of ~1e5 and
    // the pyramid walk below always stopped at the coarsest overview (deep
    // zoom came back as a blurry upsample). Same window, same level whatever
    // the request CRS is.
    let desired = desired_from_native_coords(&native_coords, w, h);

    // Overview selection: coarsest level whose factor does not upsample.
    let native_w = cog.native_width();
    let mut level_idx = 0usize;
    for (idx, lvl) in cog.levels.iter().enumerate() {
        let f = lvl.factor(native_w);
        if f <= desired * (1.0 + 1e-3) {
            level_idx = idx;
        } else {
            break;
        }
    }
    let level = &cog.levels[level_idx];
    let lw = level.width as f64;
    let lh = level.height as f64;
    let sx_ratio = lw / native_w as f64;
    let sy_ratio = lh / cog.native_height() as f64;

    // Pass 2: native coords -> chosen-level pixel coords; track the window.
    let mut level_coords = vec![[f64::NAN; 2]; w * h];
    let (mut minc, mut maxc) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut minr, mut maxr) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut any = false;
    for p in 0..w * h {
        let [nu, nv] = native_coords[p];
        if !nu.is_finite() || !nv.is_finite() {
            continue;
        }
        let u = nu * sx_ratio;
        let v = nv * sy_ratio;
        if u >= 0.0 && u < lw && v >= 0.0 && v < lh {
            level_coords[p] = [u, v];
            any = true;
            minc = minc.min(u);
            maxc = maxc.max(u);
            minr = minr.min(v);
            maxr = maxr.max(v);
        }
    }
    if !any {
        return Ok(out);
    }

    // Source window: tile-aligned region covering the needed pixels (+1px margin).
    let margin = 1.0;
    let c0 = (minc - margin).floor().max(0.0) as u32;
    let c1 = (maxc + margin).ceil().min(lw - 1.0) as u32;
    let r0 = (minr - margin).floor().max(0.0) as u32;
    let r1 = (maxr + margin).ceil().min(lh - 1.0) as u32;

    let win = read_window_rgba(level, src, c0, r0, c1, r1, cache)?;
    let win_cols = (c1 - c0 + 1) as usize;
    let win_rows = (r1 - r0 + 1) as usize;

    // Resample into the output canvas.
    match req.resample {
        Resample::Nearest => {
            for p in 0..w * h {
                let [u, v] = level_coords[p];
                if !u.is_finite() {
                    continue;
                }
                let sx = (u - c0 as f64).floor() as usize;
                let sy = (v - r0 as f64).floor() as usize;
                if sx >= win_cols || sy >= win_rows {
                    continue;
                }
                let src_i = (sy * win_cols + sx) * 4;
                let dst_i = p * 4;
                out[dst_i..dst_i + 4].copy_from_slice(&win[src_i..src_i + 4]);
            }
        }
        Resample::Bilinear => {
            for p in 0..w * h {
                let [u, v] = level_coords[p];
                if !u.is_finite() {
                    continue;
                }
                let fx = u - c0 as f64;
                let fy = v - r0 as f64;
                let x0 = fx.floor() as usize;
                let y0 = fy.floor() as usize;
                let x1 = (x0 + 1).min(win_cols - 1);
                let y1 = (y0 + 1).min(win_rows - 1);
                let tx = (fx - x0 as f64) as f32;
                let ty = (fy - y0 as f64) as f32;
                let dst_i = p * 4;
                for ch in 0..4 {
                    let a = win[(y0 * win_cols + x0) * 4 + ch] as f32;
                    let b = win[(y0 * win_cols + x1) * 4 + ch] as f32;
                    let c = win[(y1 * win_cols + x0) * 4 + ch] as f32;
                    let d = win[(y1 * win_cols + x1) * 4 + ch] as f32;
                    let top = a + (b - a) * tx;
                    let bot = c + (d - c) * tx;
                    out[dst_i + ch] = (top + (bot - top) * ty).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    // Apply a value-domain palette style (grayscale value -> palette color).
    if let Some(style) = req.style {
        apply_palette(&mut out, w * h, style);
    }
    Ok(out)
}

/// Fill `out` (w*h) with native-pixel coordinates via the **affine fast path**.
///
/// Preconditions (checked by the caller): the transformer is the identity
/// (request CRS == source CRS) and the source geotransform is affine
/// `geo_to_pix`. The composite output-pixel -> native-pixel map is then
/// bilinear in the normalized pixel index, determined by the four output
/// corners. A midpoint projection guards the assumption: if the predicted
/// midpoint deviates from the actual projection by more than 0.05 px, the
/// function returns `false` and the caller falls back to per-pixel projection.
fn fill_affine_native_coords(
    req: &RenderRequest<'_>,
    native: &crate::cog::GeoTransform,
    w: usize,
    h: usize,
    out: &mut [[f64; 2]],
) -> bool {
    let [minx, miny, maxx, maxy] = req.bbox;
    let dx = (maxx - minx) / w as f64;
    let dy = (maxy - miny) / h as f64;
    let center = |i: usize, j: usize| -> (f64, f64) {
        (minx + (i as f64 + 0.5) * dx, maxy - (j as f64 + 0.5) * dy)
    };
    let project = |i: usize, j: usize| -> (f64, f64) {
        let (x, y) = center(i, j);
        // Identity transform: to_source is a passthrough; guard finiteness the
        // same way the exact path does.
        if !x.is_finite() || !y.is_finite() {
            return (f64::NAN, f64::NAN);
        }
        native.geo_to_pix(x, y)
    };
    let (u00, v00) = project(0, 0);
    let (u10, v10) = project(w - 1, 0);
    let (u01, v01) = project(0, h - 1);
    let (u11, v11) = project(w - 1, h - 1);
    if !u00.is_finite() || !v00.is_finite() {
        return false;
    }
    // Bilinear coefficients over the normalized pixel INDEX
    // fi = i / (w - 1)  (i = 0 -> 0, i = w-1 -> 1; the coefficients are the
    // values AT the corner pixel centers, so the fill loop must use the same
    // convention — and so must the midpoint guard below, which samples
    // fi = (w/2) / (w-1), not 0.5):
    //   u(fi,fj) = au + bu*fi + cu*fj + du*fi*fj
    let (au, bu, cu, du) = (
        u00,
        u10 - u00,
        u01 - u00,
        u00 - u10 - u01 + u11,
    );
    let (av, bv, cv, dv) = (
        v00,
        v10 - v00,
        v01 - v00,
        v00 - v10 - v01 + v11,
    );
    // Midpoint guard: the bilinear prediction at the canvas center must match
    // the actual projection. With an identity transform over an affine
    // geotransform this holds to float rounding; a surprise (e.g. a future
    // non-affine path) makes the caller fall back to per-pixel projection.
    let (mu, mv) = project(w / 2, h / 2);
    let fi_m = (w as f64 / 2.0) / (w.max(2) - 1) as f64;
    let fj_m = (h as f64 / 2.0) / (h.max(2) - 1) as f64;
    let pu = au + bu * fi_m + cu * fj_m + du * fi_m * fj_m;
    let pv = av + bv * fi_m + cv * fj_m + dv * fi_m * fj_m;
    if !mu.is_finite() || (mu - pu).abs() > 0.05 || (mv - pv).abs() > 0.05 {
        return false;
    }
    let fw = 1.0 / (w.max(2) - 1) as f64;
    let fh = 1.0 / (h.max(2) - 1) as f64;
    for j in 0..h {
        let fj = j as f64 * fh;
        let row = j * w;
        let (ru, rv) = (au + cu * fj, av + cv * fj);
        for i in 0..w {
            let fi = i as f64 * fw;
            let idx = row + i;
            out[idx] = [
                ru + bu * fi + du * fi * fj,
                rv + bv * fi + dv * fi * fj,
            ];
        }
    }
    true
}

/// Map a grayscale value (R channel) through a palette; nodata becomes transparent.
fn apply_palette(out: &mut [u8], n: usize, style: &RasterStyle) {
    let Some(min) = style.min else { return };
    let Some(max) = style.max else { return };
    if max <= min {
        return;
    }
    let pal = &style.palette;
    if pal.is_empty() {
        return;
    }
    let nodata = style.nodata;
    for p in 0..n {
        let v = out[p * 4] as f64;
        if let Some(nd) = nodata {
            if (v - nd).abs() < 0.5 {
                out[p * 4 + 3] = 0;
                continue;
            }
        }
        let t = ((v - min) / (max - min)).clamp(0.0, 1.0);
        let pos = t * (pal.len() - 1) as f64;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(pal.len() - 1);
        let f = (pos - i0 as f64) as f32;
        for ch in 0..3 {
            let a = pal[i0][ch] as f32;
            let b = pal[i1][ch] as f32;
            out[p * 4 + ch] = (a + (b - a) * f).round().clamp(0.0, 255.0) as u8;
        }
        out[p * 4 + 3] = 255;
    }
}

/// Estimate the downsample factor: source pixels spanned per output pixel,
/// measured in SOURCE-pixel space from Pass 1's mapped coordinates.
///
/// The per-pixel spacing along the middle row/column is the local scale, so a
/// reprojected request (Web Mercator tile over a geographic source) gets the
/// same factor as the equivalent identity request. Falls back to the source
/// pixel span when the middle row/column happens to be entirely outside the
/// data (e.g. a window clipping the source corner).
fn desired_from_native_coords(coords: &[[f64; 2]], w: usize, h: usize) -> f64 {
    let row = h / 2;
    let (mut du, mut nu) = (0.0f64, 0usize);
    for i in 1..w {
        let a = coords[row * w + i - 1];
        let b = coords[row * w + i];
        if a[0].is_finite() && b[0].is_finite() {
            du += (b[0] - a[0]).abs();
            nu += 1;
        }
    }
    let col = w / 2;
    let (mut dv, mut nv) = (0.0f64, 0usize);
    for j in 1..h {
        let a = coords[(j - 1) * w + col];
        let b = coords[j * w + col];
        if a[1].is_finite() && b[1].is_finite() {
            dv += (b[1] - a[1]).abs();
            nv += 1;
        }
    }
    let mut d = if nu > 0 { du / nu as f64 } else { 0.0 };
    d = d.max(if nv > 0 { dv / nv as f64 } else { 0.0 });
    if d <= 1e-9 {
        let (mut u0, mut u1) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut v0, mut v1) = (f64::INFINITY, f64::NEG_INFINITY);
        for c in coords {
            if c[0].is_finite() && c[1].is_finite() {
                u0 = u0.min(c[0]);
                u1 = u1.max(c[0]);
                v0 = v0.min(c[1]);
                v1 = v1.max(c[1]);
            }
        }
        if u1 > u0 && v1 > v0 {
            d = ((u1 - u0) / w.max(1) as f64).max((v1 - v0) / h.max(1) as f64);
        }
    }
    d.max(1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::Transformer;

    fn sample_req(bbox: [f64; 4], w: u32, h: u32) -> RenderRequest<'static> {
        RenderRequest {
            bbox,
            crs: "EPSG:4326",
            src_crs: "EPSG:4326",
            width: w,
            height: h,
            resample: Resample::Nearest,
            style: None,
        }
    }

    #[test]
    fn affine_fast_path_matches_per_pixel_projection() {
        // A regular lon/lat grid (px != py, asymmetric origin).
        let native = crate::cog::GeoTransform {
            origin_x: 100.0,
            origin_y: 40.0,
            px: 0.0015,
            py: 0.0025,
        };
        let req = sample_req([100.0, 36.0, 108.0, 40.0], 512, 256);
        let (w, h) = (512usize, 256usize);
        let mut fast = vec![[f64::NAN; 2]; w * h];
        assert!(fill_affine_native_coords(&req, &native, w, h, &mut fast));
        // Exact per-pixel reference (identity transform = passthrough).
        let [minx, miny, maxx, maxy] = req.bbox;
        let dx = (maxx - minx) / w as f64;
        let dy = (maxy - miny) / h as f64;
        for j in 0..h {
            for i in 0..w {
                let ox = minx + (i as f64 + 0.5) * dx;
                let oy = maxy - (j as f64 + 0.5) * dy;
                let (u, v) = native.geo_to_pix(ox, oy);
                let idx = j * w + i;
                assert!((fast[idx][0] - u).abs() < 1e-6, "u mismatch at ({i},{j})");
                assert!((fast[idx][1] - v).abs() < 1e-6, "v mismatch at ({i},{j})");
            }
        }
    }

    #[test]
    fn affine_fast_path_handles_single_pixel_canvas() {
        let native = crate::cog::GeoTransform {
            origin_x: 100.0,
            origin_y: 40.0,
            px: 0.001,
            py: 0.001,
        };
        let req = sample_req([100.0, 36.0, 108.0, 40.0], 1, 1);
        let mut out = vec![[f64::NAN; 2]; 1];
        assert!(fill_affine_native_coords(&req, &native, 1, 1, &mut out));
        let (u, v) = native.geo_to_pix(104.0, 38.0); // canvas center
        assert!((out[0][0] - u).abs() < 1e-6);
        assert!((out[0][1] - v).abs() < 1e-6);
    }

    #[test]
    fn affine_fast_path_rejects_non_finite_bbox() {
        let native = crate::cog::GeoTransform {
            origin_x: 0.0,
            origin_y: 0.0,
            px: 1.0,
            py: 1.0,
        };
        // A NaN bbox cannot be validated: the caller must fall back to the
        // exact per-pixel path (which yields an all-NaN, empty canvas).
        let req = sample_req([f64::NAN, 0.0, 1.0, 1.0], 4, 4);
        let mut out = vec![[f64::NAN; 2]; 16];
        assert!(!fill_affine_native_coords(&req, &native, 4, 4, &mut out));
    }

    /// Regression: a non-identity request CRS (every Web Mercator tile on a
    /// 4326 source, any 3857 GetMap) must render real content via the
    /// per-pixel projection path -- NOT silently return a blank canvas.
    /// The fast-path gate once swallowed the non-identity case.
    #[test]
    fn non_identity_crs_renders_content() {
        let dir = std::env::temp_dir().join("rms_renderer_nonidentity");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("grad.tif");
        // 100..104 E, 36..40 N gradient COG.
        let bytes = crate::cog::write::gradient_cog(256, 256, 128, 100.0, 40.0, 104.0, 36.0);
        std::fs::write(&path, &bytes).unwrap();
        let p = path.to_str().unwrap();
        let src = crate::storage::source::LocalFileRangeSource::open(p).unwrap();
        let cog = crate::cog::Cog::parse(&src).unwrap();

        let merc = |lon: f64, lat: f64| -> (f64, f64) {
            (
                lon * 20037508.3427892 / 180.0,
                (std::f64::consts::PI / 4.0 + lat.to_radians() / 2.0)
                    .tan()
                    .ln()
                    * 6378137.0,
            )
        };
        let (mx0, my0) = merc(100.0, 36.0);
        let (mx1, my1) = merc(104.0, 40.0);

        // 1) Web Mercator request over a 4326 source: must not be blank.
        let req3857 = RenderRequest {
            bbox: [mx0, my0, mx1, my1],
            crs: "EPSG:3857",
            src_crs: "EPSG:4326",
            width: 64,
            height: 64,
            resample: Resample::Bilinear,
            style: None,
        };
        let transformer = Transformer::new_any("EPSG:3857", "EPSG:4326", crate::projection::proj()).unwrap();
        let out = render_with_cog(&req3857, &cog, &src, None, &transformer).unwrap();
        let distinct: std::collections::HashSet<[u8; 4]> =
            out.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        assert!(
            distinct.len() > 4,
            "3857 render came back blank ({} distinct colors) -- the \
             non-identity per-pixel path is missing",
            distinct.len()
        );

        // 2) Same geographic window via the identity path: both renders must
        // agree per pixel (projection round-trip + identical resampling).
        let req4326 = RenderRequest {
            bbox: [100.0, 36.0, 104.0, 40.0],
            crs: "EPSG:4326",
            src_crs: "EPSG:4326",
            width: 64,
            height: 64,
            resample: Resample::Bilinear,
            style: None,
        };
        let transformer_ll = Transformer::new_any("EPSG:4326", "EPSG:4326", crate::projection::proj()).unwrap();
        let out_ll = render_with_cog(&req4326, &cog, &src, None, &transformer_ll).unwrap();
        let max_diff = out
            .iter()
            .zip(out_ll.iter())
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap();
        assert!(
            max_diff <= 2,
            "3857 vs 4326 render of the same window diverges (max channel diff {max_diff})"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: the pyramid factor must be measured in SOURCE pixels.
    /// A Web Mercator window reprojected onto a geographic COG spans the same
    /// ground as the equivalent 4326 window, so both must yield the same
    /// factor. Estimating it from the raw request bbox (meters) against the
    /// source geotransform (degrees) returned ~1e5 instead, which pinned every
    /// Mercator tile to the coarsest overview: deep zoom looked blurred out.
    #[test]
    fn desired_factor_is_measured_in_source_pixels_not_request_units() {
        let native = crate::cog::GeoTransform {
            origin_x: 100.0,
            origin_y: 40.0,
            px: 0.001,
            py: 0.001,
        };
        let (w, h) = (64usize, 64usize);
        // 4° x 4° window -> 0.0625 deg per output pixel = 62.5 source pixels.
        let (minx, miny, maxx, maxy) = (100.0, 36.0, 104.0, 40.0);
        let expect = (maxx - minx) / w as f64 / native.px;

        let mut geo_coords = vec![[f64::NAN; 2]; w * h];
        let dx = (maxx - minx) / w as f64;
        let dy = (maxy - miny) / h as f64;
        for j in 0..h {
            for i in 0..w {
                let (u, v) = native.geo_to_pix(minx + (i as f64 + 0.5) * dx, maxy - (j as f64 + 0.5) * dy);
                geo_coords[j * w + i] = [u, v];
            }
        }
        let from_geo = desired_from_native_coords(&geo_coords, w, h);
        assert!(
            (from_geo - expect).abs() / expect < 0.05,
            "identity path factor {from_geo} should be the source-pixel scale {expect}"
        );

        // Same ground via EPSG:3857: Pass 1 hands the helper source-pixel
        // coordinates already, so the estimate stays in the same range instead
        // of exploding with the request-CRS units.
        let merc = |lon: f64, lat: f64| -> (f64, f64) {
            (
                lon * 20037508.3427892 / 180.0,
                (std::f64::consts::PI / 4.0 + lat.to_radians() / 2.0)
                    .tan()
                    .ln()
                    * 6378137.0,
            )
        };
        let (mx0, my0) = merc(minx, miny);
        let (mx1, my1) = merc(maxx, maxy);
        let transformer =
            Transformer::new_any("EPSG:3857", "EPSG:4326", crate::projection::proj()).unwrap();
        let mut merc_coords = vec![[f64::NAN; 2]; w * h];
        let mdx = (mx1 - mx0) / w as f64;
        let mdy = (my1 - my0) / h as f64;
        for j in 0..h {
            for i in 0..w {
                if let Some((sx, sy)) =
                    transformer.to_source(mx0 + (i as f64 + 0.5) * mdx, my1 - (j as f64 + 0.5) * mdy)
                {
                    let (u, v) = native.geo_to_pix(sx, sy);
                    merc_coords[j * w + i] = [u, v];
                }
            }
        }
        let from_merc = desired_from_native_coords(&merc_coords, w, h);
        assert!(
            from_merc > expect / 3.0 && from_merc < expect * 3.0,
            "reprojected factor {from_merc} drifted from the source-pixel scale \
             {expect} (unit mixing: request CRS vs source geotransform)"
        );
    }

    /// CPU cost of Pass-1 projection, affine fast path vs per-pixel.
    /// Release-only (debug timings are meaningless); run with:
    /// `cargo test --release -p renderer -- --nocapture pass1`
    #[test]
    fn bench_pass1_affine_vs_per_pixel() {
        if cfg!(debug_assertions) {
            return;
        }
        let native = crate::cog::GeoTransform {
            origin_x: 100.0,
            origin_y: 40.0,
            px: 0.0015,
            py: 0.0025,
        };
        let req = sample_req([100.0, 36.0, 108.0, 40.0], 512, 256);
        let (w, h) = (512usize, 256usize);
        let iters = 100usize;

        let t0 = std::time::Instant::now();
        let mut acc = 0.0f64;
        for _ in 0..iters {
            let mut out = vec![[f64::NAN; 2]; w * h];
            assert!(fill_affine_native_coords(&req, &native, w, h, &mut out));
            acc += out[w * h / 2][0];
        }
        let fast_ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;

        let transformer =
            Transformer::new_any(req.crs, req.src_crs, crate::projection::proj()).unwrap();
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            let mut out = vec![[f64::NAN; 2]; w * h];
            let [minx, miny, maxx, maxy] = req.bbox;
            let dx = (maxx - minx) / w as f64;
            let dy = (maxy - miny) / h as f64;
            for j in 0..h {
                let oy = maxy - (j as f64 + 0.5) * dy;
                for i in 0..w {
                    let ox = minx + (i as f64 + 0.5) * dx;
                    if let Some((sx, sy)) = transformer.to_source(ox, oy) {
                        if sx.is_finite() && sy.is_finite() {
                            let (u, v) = native.geo_to_pix(sx, sy);
                            out[j * w + i] = [u, v];
                        }
                    }
                }
            }
            acc += out[w * h / 2][0];
        }
        let slow_ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;

        println!(
            "PASS-1 512x256: affine={fast_ms:.3}ms per-pixel={slow_ms:.3}ms speedup={:.1}x (acc={acc:.0})",
            slow_ms / fast_ms
        );
    }
}
