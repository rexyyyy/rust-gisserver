//! Coordinate transforms.
//!
//! The render pipeline reprojects a request bbox (in the request CRS) into the
//! source CRS. EPSG:4326 (lon/lat) and EPSG:3857 (Web Mercator) use the
//! built-in, dependency-free implementations; any other CRS (CGCS2000
//! Gauss-Kruger zones, UTM, ...) is delegated to the optional PROJ backend
//! (see [`proj`]) when `proj_9.dll` was loaded at startup.

pub mod mercator;
pub mod proj;

use std::sync::Arc;

use wms_common::crs::{normalize, EPSG_3857, EPSG_4326};

pub use mercator::{lonlat_to_mercator, mercator_to_lonlat, lonlat_to_mercator_vec, mercator_to_lonlat_vec};
pub use proj::{crs_proj4, init_proj, proj, Proj, PjTransform};

/// Error returned by [`Transformer`].
#[derive(Debug)]
pub enum ProjectionError {
    UnsupportedCrs(String),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::UnsupportedCrs(c) => write!(
                f,
                "unsupported CRS '{c}': cross-CRS work outside EPSG:4326/EPSG:3857 needs the PROJ backend (PROJ_DLL / PROJ_DATA)"
            ),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// Transforms points between a request CRS and a source CRS.
pub struct Transformer {
    from: String,
    to: String,
    proj: Option<PjTransform>,
}

impl Transformer {
    pub fn new(from: &str, to: &str) -> Result<Transformer, ProjectionError> {
        Self::new_inner(from, to, None)
    }

    /// Like [`Transformer::new`], but allows arbitrary CRS pairs via the PROJ
    /// backend. Same-CRS pairs are always served as the identity; when a pair
    /// crosses CRSs and is outside the built-in 4326/3857 set, an
    /// `UnsupportedCrs` error is returned unless a PROJ backend is loaded.
    pub fn new_any(from: &str, to: &str, backend: Option<&Arc<Proj>>) -> Result<Transformer, ProjectionError> {
        Self::new_inner(from, to, backend)
    }

    fn new_inner(from: &str, to: &str, backend: Option<&Arc<Proj>>) -> Result<Transformer, ProjectionError> {
        let from = normalize(from);
        let to = normalize(to);
        // Same CRS on both sides: the mapping is the identity, which needs no
        // projection maths at all. Serving e.g. an EPSG:4490 COG to an
        // EPSG:4490 request therefore works on a bare install without PROJ;
        // only genuinely *cross*-CRS requests below depend on the backend.
        if from == to {
            return Ok(Transformer {
                from,
                to,
                proj: None,
            });
        }
        let builtin = |c: &str| c == EPSG_4326 || c == EPSG_3857;
        if builtin(&from) && builtin(&to) {
            return Ok(Transformer {
                from,
                to,
                proj: None,
            });
        }
        if let Some(p) = backend {
            if let Some(tr) = Proj::crs_to_crs(p, &from, &to) {
                return Ok(Transformer {
                    from,
                    to,
                    proj: Some(tr),
                });
            }
        }
        Err(ProjectionError::UnsupportedCrs(format!("{from} -> {to}")))
    }

    /// Transform one (x, y) point from the request CRS into the source CRS.
    /// Returns `None` for a point the transform cannot represent (e.g. a
    /// latitude outside Mercator's valid range).
    pub fn to_source(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if let Some(pj) = &self.proj {
            let (sx, sy) = pj.transform(x, y);
            return if sx.is_finite() && sy.is_finite() {
                Some((sx, sy))
            } else {
                None
            };
        }
        match (self.from.as_str(), self.to.as_str()) {
            (a, b) if a == b => Some((x, y)),
            (EPSG_4326, EPSG_3857) => lonlat_to_mercator(x, y),
            (EPSG_3857, EPSG_4326) => Some(mercator_to_lonlat(x, y)),
            _ => unreachable!("CRS set validated in new()"),
        }
    }

    /// Batch transform; out-of-range points become `(NaN, NaN)`.
    pub fn to_source_batch(&self, pts: &[(f64, f64)]) -> Vec<(f64, f64)> {
        pts.iter()
            .map(|&(x, y)| self.to_source(x, y).unwrap_or((f64::NAN, f64::NAN)))
            .collect()
    }

    /// Whether the transform is the identity (same CRS).
    pub fn is_identity(&self) -> bool {
        self.from == self.to
    }

    pub fn from(&self) -> &str {
        &self.from
    }

    pub fn to(&self) -> &str {
        &self.to
    }
}

impl wms_common::reproject::Reprojector for Transformer {
    fn to_source(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        Transformer::to_source(self, x, y)
    }

    fn is_identity(&self) -> bool {
        Transformer::is_identity(self)
    }
}

/// True when `crs` is one the transformer can serve: the built-in 4326 / 3857
/// pair always, plus (once the PROJ backend is loaded) the CGCS2000 geographic
/// CRSs and the Gauss-Krüger / UTM projected zones. The accepted identifier set
/// is a protocol-level decision and lives in [`wms_common::crs::is_supported`],
/// so capabilities advertising and request validation cannot drift apart from
/// what [`Transformer::new_any`] actually builds. Whether a non-built-in CRS
/// really transforms at runtime still depends on [`proj`] being loaded.
pub fn is_supported_crs(crs: &str) -> bool {
    wms_common::crs::is_supported(crs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内置 4326 ↔ 3857 不依赖 PROJ：未加载后端的裸机环境也必须可用。
    #[test]
    fn builtin_pairs_work_without_backend() {
        let tr = Transformer::new_any(EPSG_3857, EPSG_4326, proj()).unwrap();
        let (lon, lat) = tr.to_source(0.0, 0.0).unwrap();
        assert!(lon.abs() < 1e-9 && lat.abs() < 1e-9);
        assert_ne!(tr.from(), tr.to());
        assert!(!tr.is_identity());
        let id = Transformer::new_any(EPSG_4326, EPSG_4326, proj()).unwrap();
        assert!(id.is_identity());
        assert_eq!(id.to_source(116.4, 39.9), Some((116.4, 39.9)));
    }

    /// CGCS2000 地理系（4490）、高斯-克吕格分带（45xx）、WGS84 UTM 带
    /// （32648…）以及 URN 写法在协议层一律接受；能否真正变换由运行时的
    /// PROJ 后端决定，与能力文档宣告的 CRS 列表共用同一判定。
    #[test]
    fn cgcs2000_and_utm_zones_are_accepted() {
        for c in [
            "EPSG:4326",
            "EPSG:3857",
            "EPSG:4490",
            "EPSG:4523",
            "EPSG:4543",
            "EPSG:32648",
            "CRS:84",
            "urn:ogc:def:crs:EPSG::4490",
            "http://www.opengis.net/def/crs/EPSG/0/4543",
        ] {
            assert!(is_supported_crs(c), "{c} 应被判为支持");
        }
        for c in ["EPSG:1234", "EPSG:32647", "CRS:99", "urn:ogc:def:crs:EPSG::"] {
            assert!(!is_supported_crs(c), "{c} 应被拒");
        }
    }

    /// 同名 CRS（源 CRS == 请求 CRS）是恒等变换，不需要任何投影数学：
    /// 即使未加载 PROJ，CGCS2000 / 高斯-克吕格带也能直出原生产物。
    #[test]
    fn same_crs_is_identity_without_backend() {
        for c in ["EPSG:4490", "EPSG:4543", "EPSG:32649", "EPSG:4326", "EPSG:3857"] {
            let tr = Transformer::new(c, c).unwrap();
            assert!(tr.is_identity(), "{c} 应为恒等");
            assert_eq!(tr.to_source(102.65, 25.05), Some((102.65, 25.05)));
            // URN / def-URI 写法经 normalize 后同样判为恒等
            let tr_urn = Transformer::new(&wms_common::crs::to_urn(c), c).unwrap();
            assert!(tr_urn.is_identity(), "{c} URN 写法应为恒等");
        }
    }

    /// 跨 CRS（4326 ↔ 3857）在无后端时仍可用；非内置 CRS 的跨 CRS 互转在没有
    /// PROJ 的机器上必须被明确拒绝（而不是默默出错）。仅在确实无后端的进程
    /// 里断言后半句，避免与已加载 PROJ 的环境相互干扰。
    #[test]
    fn cross_crs_requires_backend_only_outside_builtin_pair() {
        assert!(Transformer::new(EPSG_3857, EPSG_4326).is_ok());
        assert!(Transformer::new(EPSG_4326, EPSG_3857).is_ok());
        if proj().is_none() {
            assert!(Transformer::new("EPSG:4326", "EPSG:4490").is_err());
            assert!(Transformer::new_any("EPSG:4326", "EPSG:4543", proj()).is_err());
        }
    }
}
