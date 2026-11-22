//! Reprojection abstraction shared by the renderer and the projection backend.
//!
//! The render pipeline (COG rasterization in `renderer`, vector rasterization
//! in `crate::renderer::vector`) needs to map a request CRS into a source CRS. It does
//! **not** need to know *how* that mapping is performed — a built-in
//! EPSG:4326↔EPSG:3857 pair, or an arbitrary CRS pair via the optional PROJ
//! backend. [`Reprojector`] captures exactly the surface the renderer uses so
//! `renderer` depends on this trait (in `wms-common`) and not on the concrete
//! `projection` crate, keeping the projection backend an injectable dependency
//! rather than a hidden global the renderer reaches for.

/// A coordinate transform from a request CRS into a source CRS.
///
/// Implemented by `crate::projection::Transformer` (built-in 4326/3857 plus the
/// optional PROJ backend). Injected into `renderer` so the render path stays
/// decoupled from any specific projection implementation.
pub trait Reprojector {
    /// Transform one (x, y) point from the request CRS into the source CRS.
    /// Returns `None` for a point the transform cannot represent (e.g. a
    /// latitude outside Mercator's valid range).
    fn to_source(&self, x: f64, y: f64) -> Option<(f64, f64)>;

    /// Whether the transform is the identity (same CRS): the render pipeline's
    /// affine fast path only applies when this is true.
    fn is_identity(&self) -> bool;
}
