//! CRS helpers: classification and WMS axis-order handling.
//!
//! The projection *math* lives in the `projection` crate; this module only
//! classifies CRS identifiers for protocol behaviour (axis order, meters-per-unit).

/// WGS84 longitude/latitude.
pub const EPSG_4326: &str = "EPSG:4326";
/// OGC CRS:84 (lon/lat, easting-first by definition).
pub const CRS84: &str = "CRS:84";
/// Web Mercator.
pub const EPSG_3857: &str = "EPSG:3857";
/// CGCS2000 geographic (China Geodetic Coordinate System 2000).
pub const EPSG_4490: &str = "EPSG:4490";

/// Geographic (degree-unit) EPSG codes served through the optional PROJ
/// backend, in addition to WGS84: CGCS2000 (4490), Xi'an 80 (4214) and
/// WGS72 / other national geodetic CRSs used in China.
pub const GEOGRAPHIC_CRS: &[&str] = &["EPSG:4326", "EPSG:4490", "EPSG:4214", "EPSG:4258"];

/// Extract the numeric code from a normalized `EPSG:NNNN` identifier.
fn epsg_code(crs: &str) -> Option<i32> {
    crs.strip_prefix("EPSG:").and_then(|s| s.parse().ok())
}

/// True for projected EPSG families accepted when PROJ is loaded: CGCS2000 /
/// Beijing 54 / Xi'an 80 Gauss-Krüger zones (3- and 6-degree, EPSG:4491..=4554)
/// and WGS84 UTM north (legacy 4800..=4999 and modern 32648..=32660).
fn is_projected_epsg(code: i32) -> bool {
    (4491..=4554).contains(&code) || (4800..=4999).contains(&code) || (32648..=32660).contains(&code)
}

/// Normalise a CRS id: trim, uppercase, map `CRS84`/`OGC:CRS84` to `EPSG:4326`.
///
/// 同一个 CRS 有好几种合法写法，入口必须同等宽容，不然只认短名的服务会对
/// 合规客户端报 400：
/// * 短名 `EPSG:4326`；
/// * OGC URN `urn:ogc:def:crs:EPSG::4326`（WMS 1.3.0 §6.6、WMTS
///   `<SupportedCRS>`、WFS `<DefaultCRS>` 都用这个形式，QGIS/ArcGIS/owslib
///   常常发这种）；版本式 `...:EPSG:6.6:4326` 与 GML 3.1 旧式
///   `urn:x-ogc:def:crs:EPSG:4326` 同等接受；
/// * OGC def-URI `http://www.opengis.net/def/crs/EPSG/0/4326`（OGC API 用）。
///
/// **轴序判定不适用本折叠**：CRS:84 的各形式也被归到 `EPSG:4326`（datum 相同），
/// 而它按定义是经度在前，所以下游必须用 [`is_crs84`] 看原始标识。
pub fn normalize(crs: &str) -> String {
    let input = crs.trim().to_ascii_uppercase();
    // 先把 URN / def-URI 剥成短名，再走同一张别名表（否则
    // `urn:ogc:def:crs:EPSG::900913` 会绕过 900913→3857 的归一）。
    let c = match epsg_form_code(&input) {
        Some(code) => format!("EPSG:{code}"),
        None => input,
    };
    match c.as_str() {
        "CRS:84" | "OGC:CRS84" | "OGC:2:84" => EPSG_4326.to_string(),
        "URN:OGC:DEF:CRS:OGC:1.3:CRS84" | "URN:OGC:DEF:CRS:OGC::CRS84" => EPSG_4326.to_string(),
        "HTTP://WWW.OPENGIS.NET/DEF/CRS/OGC/1.3/CRS84"
        | "HTTPS://WWW.OPENGIS.NET/DEF/CRS/OGC/1.3/CRS84" => EPSG_4326.to_string(),
        "EPSG:900913" | "EPSG:3785" | "EPSG:3857" => EPSG_3857.to_string(),
        _ => c,
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// 从 URN / def-URI 写式里取出 EPSG 码（输入已大写）。
fn epsg_form_code(c: &str) -> Option<String> {
    if let Some(tail) = c.split_once(":CRS:EPSG:").map(|(_, rest)| rest) {
        if let Some(code) = tail.rsplit(':').next() {
            if all_digits(code) {
                return Some(code.to_string());
            }
        }
    }
    for p in [
        "HTTP://WWW.OPENGIS.NET/DEF/CRS/EPSG/",
        "HTTPS://WWW.OPENGIS.NET/DEF/CRS/EPSG/",
    ] {
        if let Some(tail) = c.strip_prefix(p) {
            if let Some(code) = tail.rsplit('/').next() {
                if all_digits(code) {
                    return Some(code.to_string());
                }
            }
        }
    }
    None
}

/// True for a geographic (lon/lat, degrees) CRS.
pub fn is_geographic(crs: &str) -> bool {
    let n = normalize(crs);
    GEOGRAPHIC_CRS.contains(&n.as_str())
}

/// True for Web Mercator.
pub fn is_web_mercator(crs: &str) -> bool {
    normalize(crs) == EPSG_3857
}

/// True when a CRS is one the server can reproject to: the built-in 4326 /
/// 3857 path, or (when the PROJ backend is loaded) a CGCS2000 geographic CRS
/// or a supported Gauss-Krüger / UTM projected zone.
pub fn is_supported(crs: &str) -> bool {
    let n = normalize(crs);
    n == EPSG_4326 || n == EPSG_3857 || is_geographic(&n)
        || epsg_code(&n).map(is_projected_epsg).unwrap_or(false)
}

/// WMS 1.3.0 orders a BBOX by the CRS's own declared axis order: geographic
/// CRSs (4326 / 4490 / ...) are northing-first, projected / CRS:84 are not.
pub fn is_northing_first(crs: &str) -> bool {
    is_geographic(crs)
}

/// True when a **raw request identifier** spells OGC CRS:84 (any of its
/// accepted forms). CRS:84 is longitude/latitude by definition, so a WMS
/// 1.3.0 BBOX for it must NOT be axis-swapped -- even though `normalize`
/// maps it onto EPSG:4326 (which IS northing-first). Axis-order decisions
/// must therefore consult the raw identifier, not the normalized one.
/// 注意：列全形式很关键 —— 少了 def-URI 那一条，`normalize` 会把
/// `.../crs/OGC/1.3/CRS84` 折叠到 EPSG:4326，从而错误地启动纬度在前交换。
pub fn is_crs84(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_uppercase().as_str(),
        "CRS:84"
            | "OGC:CRS84"
            | "OGC:2:84"
            | "URN:OGC:DEF:CRS:OGC:1.3:CRS84"
            | "URN:OGC:DEF:CRS:OGC::CRS84"
            | "URN:X-OGC:DEF:CRS:OGC:1.3:CRS84"
            | "HTTP://WWW.OPENGIS.NET/DEF/CRS/OGC/1.3/CRS84"
            | "HTTPS://WWW.OPENGIS.NET/DEF/CRS/OGC/1.3/CRS84"
    )
}

/// CRSs advertised in GetCapabilities (geographic set + Web Mercator).
pub fn advertised() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = GEOGRAPHIC_CRS.to_vec();
    v.push(EPSG_3857);
    v
}

/// Meters per CRS unit, for the 0.28 mm scale-denominator rule.
pub fn meters_per_unit(crs: &str) -> f64 {
    if is_geographic(crs) {
        111319.4907932736
    } else {
        1.0
    }
}

/// A CRS advertised as `urn:ogc:def:crs:EPSG::<code>` (WMTS conventions).
pub fn to_urn(crs: &str) -> String {
    let n = normalize(crs);
    let code = n.rsplit(':').next().unwrap_or("4326");
    format!("urn:ogc:def:crs:EPSG::{code}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一个 CRS 的所有合法写法必须折叠到同一个内部形式（入口宽容度）。
    #[test]
    fn folds_urn_and_def_uri_forms() {
        for c in [
            "EPSG:4490",
            "urn:ogc:def:crs:EPSG::4490",
            "URN:OGC:DEF:CRS:EPSG:6.6:4490",
            "urn:x-ogc:def:crs:EPSG:4490",
            "http://www.opengis.net/def/crs/EPSG/0/4490",
            "https://www.opengis.net/def/crs/EPSG/0/4490",
            "  EPSG:4490  ",
        ] {
            assert_eq!(normalize(c), "EPSG:4490", "{c}");
        }
        assert_eq!(normalize("urn:ogc:def:crs:EPSG::3857"), EPSG_3857);
        assert_eq!(normalize("http://www.opengis.net/def/crs/EPSG/0/3857"), EPSG_3857);
        assert_eq!(normalize("urn:ogc:def:crs:EPSG::900913"), EPSG_3857);
        // 不可解的写法照旧原样返回（由 is_supported 拒掉），不能当 4326 用
        assert_eq!(normalize("urn:ogc:def:crs:EPSG::"), "URN:OGC:DEF:CRS:EPSG::");
        assert!(!is_supported("urn:ogc:def:crs:EPSG::"));
    }

    /// CRS:84 的任何写法都不能触发纬度在前交换（normalize 会把它归到 4326）。
    #[test]
    fn crs84_forms_are_easting_first() {
        for c in [
            "CRS:84",
            "OGC:CRS84",
            "urn:ogc:def:crs:OGC:1.3:CRS84",
            "urn:ogc:def:crs:OGC::CRS84",
            "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
            "https://www.opengis.net/def/crs/OGC/1.3/CRS84",
        ] {
            assert!(is_crs84(c), "{c} 应判为 CRS:84");
            // 折叠到 4326 后看上去是 northing-first，但组合判定必须不交换
            assert!(is_northing_first(&normalize(c)), "{c}");
            assert!(!is_northing_first(c) || is_crs84(c), "{c}");
        }
        assert!(!is_crs84("urn:ogc:def:crs:EPSG::4326"));
        // EPSG 写的 4326 才是纬度在前
        assert!(is_northing_first("urn:ogc:def:crs:EPSG::4326"));
        assert!(!is_northing_first("urn:ogc:def:crs:EPSG::3857"));
    }
}
