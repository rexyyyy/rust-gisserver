//! GML 3.1.1 / 3.2 geometry + feature encoding (WFS `GetFeature` output).
//!
//! GML 3.2 uses `<gml:pos>` position lists; GML 3.1.1 uses the older
//! `<gml:coordinates>` decimal tuples. Both are emitted from the same walker so
//! WFS 1.x and 2.0 clients each get the encoding their version mandates.
//!
//! 轴序：`srsName` 宣告什么序，坐标就必须写什么序。EPSG 注册表里 4326 的轴序是
//! （纬度, 经度），而内部几何是 GeoJSON 的（经度, 纬度）——早先版本无论
//! `srsName` 怎么写都按经度在前输出，声明与内容相互矛盾，严格按声明解析的
//! 客户端（QGIS/GeoTools）会把整个图层读成转置。现在由 [`northing_first`] 根据
//! 实际宣告的 CRS 决定书写序，见 [`resolve_srs`]。

use serde_json::Value;
use wms_common::crs;

/// Which GML version to emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GmlVersion {
    /// GML 3.1.1 (`http://www.opengis.net/gml`, WFS 1.0/1.1).
    V31,
    /// GML 3.2 (`http://www.opengis.net/gml/3.2`, WFS 2.0).
    V32,
}

impl GmlVersion {
    /// Namespace URI for this version.
    pub fn ns(&self) -> &'static str {
        match self {
            GmlVersion::V31 => "http://www.opengis.net/gml",
            GmlVersion::V32 => "http://www.opengis.net/gml/3.2",
        }
    }

    /// `srsName` URN for the feature geometry (GeoJSON is always lon/lat).
    pub fn srs_name(&self) -> &'static str {
        match self {
            GmlVersion::V31 => "urn:x-ogc:def:crs:EPSG:4326",
            GmlVersion::V32 => "urn:ogc:def:crs:EPSG::4326",
        }
    }
}

/// CRS84 的规范 URN（经度在前，与内部几何同序）。
pub const CRS84_URN: &str = "urn:ogc:def:crs:OGC:1.3:CRS84";

/// 选定实际宣告的输出 CRS，并与内部几何的真实坐标保持同一 datum。
///
/// 要素只按 WGS84 经纬度存储，服务端不对几何做投影变换，所以能诚实宣告的只有
/// 两个 CRS：宣告 CRS84（经度在前）或宣告 EPSG:4326（注册表轴序纬度在前）。
/// 客户端请求投影 CRS（如 3857）或另一个地理 datum（如 4490）时，不回显那个
/// 标识而落到 EPSG:4326——给米制坐标贴在不重投影的度数上、或把 WGS84 数值
/// 宣告成 CGCS2000，都是比轴序错序更严重的元数据说谎。能力文档也只宣告
/// `DefaultCRS=EPSG::4326`（无 OtherCRS），与本函数一致。
///
/// `honor_axis` 由调用方给：WFS 1.0.0 时代未规定要按 CRS 注册表轴序输出（业界
/// 习惯一律经度在前），此时宣告 CRS84 而不是“宣告 4326 却写经度在前”——
/// 宣告与内容同序是底线。WFS 1.1.0+/2.0 传 true。
pub fn resolve_srs(requested: &str, gml: GmlVersion, honor_axis: bool) -> String {
    if crs::is_crs84(requested) || !honor_axis {
        return CRS84_URN.to_string();
    }
    // 空/未知/投影系一律落回版本默认的 4326 URN。
    gml.srs_name().to_string()
}

/// 该宣告 CRS 下 GML 坐标是否应纬度在前（仅对非 CRS84 的地理系成立）。
/// 投影系（米制，easting-northing）与 CRS84 都是经度在前。
pub fn northing_first(srs: &str) -> bool {
    !crs::is_crs84(srs) && crs::is_geographic(&crs::normalize(srs))
}

/// Escape XML text nodes / attribute values.
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

/// Encode a GeoJSON geometry as a GML element (without the wrapping property
/// element). `srs` is the srsName attribute value; the coordinate order follows
/// that CRS' registry axis order (see [`northing_first`]). Unsupported
/// geometries (null) yield an empty string.
pub fn geometry_xml(g: &Value, gml: GmlVersion, srs: &str) -> String {
    let Some(gtype) = g.get("type").and_then(|t| t.as_str()) else {
        return String::new();
    };
    let nf = northing_first(srs);
    let srs_attr = format!(" srsName=\"{}\" srsDimension=\"2\"", xml_escape(srs));
    match gtype {
        "Point" => {
            let Some(c) = g.get("coordinates").and_then(|c| c.as_array()) else {
                return String::new();
            };
            format!(
                "<gml:Point{srs_attr}>{}</gml:Point>",
                position(c, gml, nf)
            )
        }
        "LineString" => {
            let Some(c) = g.get("coordinates").and_then(|c| c.as_array()) else {
                return String::new();
            };
            format!(
                "<gml:LineString{srs_attr}>{}</gml:LineString>",
                pos_list(c, gml, nf)
            )
        }
        "Polygon" => {
            let Some(c) = g.get("coordinates").and_then(|c| c.as_array()) else {
                return String::new();
            };
            let mut out = format!("<gml:Polygon{srs_attr}><gml:exterior>");
            if let Some(ring) = c.first().and_then(|r| r.as_array()) {
                out.push_str("<gml:LinearRing>");
                out.push_str(&pos_list(ring, gml, nf));
                out.push_str("</gml:LinearRing>");
            }
            out.push_str("</gml:exterior>");
            for ring in c.iter().skip(1).filter_map(|r| r.as_array()) {
                out.push_str("<gml:interior><gml:LinearRing>");
                out.push_str(&pos_list(ring, gml, nf));
                out.push_str("</gml:LinearRing></gml:interior>");
            }
            out.push_str("</gml:Polygon>");
            out
        }
        "MultiPoint" => member_list(g, gml, &srs_attr, "MultiPoint", "pointMember", "Point", nf),
        "MultiLineString" => member_list(
            g,
            gml,
            &srs_attr,
            "MultiLineString",
            "lineStringMember",
            "LineString",
            nf,
        ),
        "MultiPolygon" => {
            member_list(g, gml, &srs_attr, "MultiPolygon", "polygonMember", "Polygon", nf)
        }
        _ => String::new(),
    }
}

/// Multi-geometries: each member is encoded as its own GML element.
fn member_list(
    g: &Value,
    gml: GmlVersion,
    srs_attr: &str,
    multi: &str,
    member: &str,
    single: &str,
    nf: bool,
) -> String {
    let Some(coords) = g.get("coordinates").and_then(|c| c.as_array()) else {
        return String::new();
    };
    let mut out = format!("<gml:{multi}{srs_attr}>");
    for c in coords {
        out.push_str(&format!("<gml:{member}><gml:{single}{srs_attr}>"));
        match single {
            "Point" => {
                if let Some(arr) = c.as_array() {
                    out.push_str(&position(arr, gml, nf));
                }
            }
            "LineString" => {
                if let Some(arr) = c.as_array() {
                    out.push_str(&pos_list(arr, gml, nf));
                }
            }
            "Polygon" => {
                if let Some(ring) = c.get(0).and_then(|r| r.as_array()) {
                    out.push_str("<gml:exterior><gml:LinearRing>");
                    out.push_str(&pos_list(ring, gml, nf));
                    out.push_str("</gml:LinearRing></gml:exterior>");
                }
            }
            _ => {}
        }
        out.push_str(&format!("</gml:{single}></gml:{member}>"));
    }
    out.push_str(&format!("</gml:{multi}>"));
    out
}

/// `<gml:pos>x y</gml:pos>` (3.2) or `<gml:coordinates>x,y</gml:coordinates>` (3.1.1).
/// `nf`（纬度在前）时按声明的轴序交换为 `y x`。
fn position(c: &[Value], gml: GmlVersion, nf: bool) -> String {
    let (Some(x), Some(y)) = (c.first().and_then(|v| v.as_f64()), c.get(1).and_then(|v| v.as_f64()))
    else {
        return String::new();
    };
    let (a, b) = if nf { (y, x) } else { (x, y) };
    match gml {
        GmlVersion::V32 => format!("<gml:pos>{a} {b}</gml:pos>"),
        GmlVersion::V31 => format!("<gml:coordinates>{a},{b}</gml:coordinates>"),
    }
}

/// `<gml:posList>x y x y ...</gml:posList>` (3.2) or
/// `<gml:coordinates>x,y x,y ...</gml:coordinates>` (3.1.1). `nf` 时逐点交换。
fn pos_list(c: &[Value], gml: GmlVersion, nf: bool) -> String {
    let pts: Vec<String> = c
        .iter()
        .filter_map(|p| {
            let x = p.get(0).and_then(|v| v.as_f64())?;
            let y = p.get(1).and_then(|v| v.as_f64())?;
            let (a, b) = if nf { (y, x) } else { (x, y) };
            Some(match gml {
                GmlVersion::V32 => format!("{a} {b}"),
                GmlVersion::V31 => format!("{a},{b}"),
            })
        })
        .collect();
    match gml {
        GmlVersion::V32 => format!("<gml:posList>{}</gml:posList>", pts.join(" ")),
        GmlVersion::V31 => format!("<gml:coordinates>{}</gml:coordinates>", pts.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn point_gml32_uses_pos() {
        let g = json!({"type":"Point","coordinates":[100.0,30.0]});
        let s = geometry_xml(&g, GmlVersion::V32, GmlVersion::V32.srs_name());
        // 宣告的是 EPSG:4326 URN，注册表轴序为（纬度,经度）→ 内容必须跟着交换。
        assert!(s.contains("<gml:pos>30 100</gml:pos>"), "{s}");
        assert!(s.contains("srsName=\"urn:ogc:def:crs:EPSG::4326\""), "{s}");
    }

    #[test]
    fn point_gml31_uses_coordinates() {
        let g = json!({"type":"Point","coordinates":[100.0,30.0]});
        let s = geometry_xml(&g, GmlVersion::V31, GmlVersion::V31.srs_name());
        assert!(s.contains("<gml:coordinates>30,100</gml:coordinates>"), "{s}");
        assert!(s.contains("srsName=\"urn:x-ogc:def:crs:EPSG:4326\""), "{s}");
    }

    /// 声明与书写序必须同序：这是本轮修复的唯一底线。
    #[test]
    fn axis_order_follows_the_declared_crs() {
        let g = json!({"type":"Point","coordinates":[100.0,30.0]});
        // 经度在前的宣告（CRS84 各写法 / 投影系）→ 不交换
        for srs in [
            CRS84_URN,
            "urn:ogc:def:crs:OGC::CRS84",
            "CRS:84",
            "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
            "urn:ogc:def:crs:EPSG::3857",
            "EPSG:4543",
            "",
        ] {
            let s = geometry_xml(&g, GmlVersion::V32, srs);
            assert!(s.contains("<gml:pos>100 30</gml:pos>"), "{srs} -> {s}");
        }
        // 纬度在前的地理系（含 GML 3.1 旧式 URN 与带版本段写法）→ 交换
        for srs in [
            "urn:ogc:def:crs:EPSG::4326",
            "urn:x-ogc:def:crs:EPSG:4326",
            "urn:ogc:def:crs:EPSG:6.6:4326",
            "EPSG:4326",
            "urn:ogc:def:crs:EPSG::4490",
            "EPSG:4490",
        ] {
            let s = geometry_xml(&g, GmlVersion::V32, srs);
            assert!(s.contains("<gml:pos>30 100</gml:pos>"), "{srs} -> {s}");
        }
    }

    /// 线/面/多重几何逐点同序，不能只改单点。
    #[test]
    fn axis_order_applies_to_lists_and_members() {
        let line = json!({"type":"LineString","coordinates":[[100.0,30.0],[101.0,31.0]]});
        let s = geometry_xml(&line, GmlVersion::V32, "urn:ogc:def:crs:EPSG::4326");
        assert!(s.contains("<gml:posList>30 100 31 101</gml:posList>"), "{s}");
        let mp = json!({"type":"MultiPoint","coordinates":[[100.0,30.0],[101.0,31.0]]});
        let s = geometry_xml(&mp, GmlVersion::V31, CRS84_URN);
        assert!(s.contains("<gml:coordinates>100,30</gml:coordinates>"), "{s}");
        assert!(s.contains("<gml:coordinates>101,31</gml:coordinates>"), "{s}");
    }

    /// 服务端不重投影，所以能诚实宣告的只有 CRS84 与 4326：客户端要投影系或
    /// 另一个地理 datum 时落到 4326 URN，而不是把度数/米制坐标贴错标签。
    #[test]
    fn resolve_srs_never_lies_about_the_datum() {
        let v32 = GmlVersion::V32;
        // CRS84 的各写法一律原样尊重
        for r in [
            "CRS:84",
            "OGC:CRS84",
            CRS84_URN,
            "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
        ] {
            assert_eq!(resolve_srs(r, v32, true), CRS84_URN, "{r}");
        }
        // 2.x 按注册表轴序：地理系宣告 4326 URN（纬度在前）
        for r in ["EPSG:4326", "urn:ogc:def:crs:EPSG::4326", "EPSG:4490", "EPSG:3857", "", "EPSG:9999"] {
            assert_eq!(resolve_srs(r, v32, true), v32.srs_name(), "{r}");
        }
        // 1.x 不按注册表轴序：宣告 CRS84 而非“宣告 4326 却写经度在前”
        for r in ["EPSG:4326", "EPSG:4490", "EPSG:3857", ""] {
            assert_eq!(resolve_srs(r, v32, false), CRS84_URN, "{r}");
        }
        assert_eq!(resolve_srs("EPSG:4326", GmlVersion::V31, true), GmlVersion::V31.srs_name());
    }

    #[test]
    fn northing_first_classification() {
        assert!(northing_first("urn:ogc:def:crs:EPSG::4326"));
        assert!(northing_first("urn:ogc:def:crs:EPSG:6.6:4490"));
        assert!(!northing_first(CRS84_URN));
        assert!(!northing_first("CRS:84"));
        assert!(!northing_first("http://www.opengis.net/def/crs/OGC/1.3/CRS84"));
        assert!(!northing_first("urn:ogc:def:crs:EPSG::3857"));
        assert!(!northing_first("EPSG:4543"));
        assert!(!northing_first(""));
    }

    #[test]
    fn polygon_exterior_and_interior() {
        let g = json!({"type":"Polygon","coordinates":[
            [[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]],
            [[1.0,1.0],[2.0,1.0],[2.0,2.0],[1.0,2.0],[1.0,1.0]]
        ]});
        let s = geometry_xml(&g, GmlVersion::V32, "urn:ogc:def:crs:EPSG::4326");
        assert!(s.contains("<gml:exterior><gml:LinearRing><gml:posList>"), "{s}");
        assert!(s.contains("<gml:interior>"), "{s}");
    }

    #[test]
    fn multipoint_members() {
        let g = json!({"type":"MultiPoint","coordinates":[[0.0,0.0],[1.0,1.0]]});
        let s = geometry_xml(&g, GmlVersion::V32, "urn:ogc:def:crs:EPSG::4326");
        assert_eq!(s.matches("<gml:pointMember>").count(), 2, "{s}");
    }

    #[test]
    fn xml_escape_specials() {
        assert_eq!(xml_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }

    #[test]
    fn null_geometry_is_empty() {
        let g = json!({"type":"Point","coordinates":null});
        assert_eq!(geometry_xml(&g, GmlVersion::V32, "s"), "");
    }
}
