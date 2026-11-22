//! OGC API - Features `/items` query parameters: `limit`, `offset`, `bbox`.

use crate::wms_common::Bbox;

use crate::ogcapi_features::error::OgcError;

/// Default page size when `limit` is absent (spec minimum guarantee).
pub const DEFAULT_LIMIT: usize = 10;
/// Server-side cap for `limit` (spec allows an implementation maximum).
pub const MAX_LIMIT: usize = 1000;

/// A parsed `/items` query.
#[derive(Clone, Debug)]
pub struct ItemsQuery {
    /// Page size (already clamped to [`MAX_LIMIT`]).
    pub limit: usize,
    /// Number of leading matches to skip.
    pub offset: usize,
    /// bbox filter in WGS84 lon/lat (CRS84 order, per OAF spec).
    pub bbox: Option<Bbox>,
}

impl Default for ItemsQuery {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            offset: 0,
            bbox: None,
        }
    }
}

/// Parse `limit`, `offset` and `bbox` from KVP pairs. Unknown parameters are
/// ignored (lenient).
///
/// `bbox` accepts 4 numbers (`minx,miny,maxx,maxy`) or 6 numbers
/// (`minx,miny,minz,maxx,maxy,maxz` —— 高程位不能占掉 max 的位）。
/// `bbox-crs`（OAF 1.1）/ `crs`（1.0 遗留名）决定 bbox 的轴序：缺省 CRS84
/// 经度在前；地理系按 EPSG 注册表纬度在前（与 WMS 1.3.0/WFS 同一规则）。
/// 早先版本把这个参数“收下并忽略”，而集合元数据又宣告支持 EPSG:4326，
/// 客户端照宣告写的纬度在前 bbox 会被当成经度在前→静默错筛。
/// 几何输出恒为 GeoJSON 的 CRS84，与本参数无关。投影 CRS 不受理（要素
/// 只按 WGS84 存储，米单位 bbox 无从筛选），给 400 而不是错结果。
pub fn parse_items_query(kv: &[(String, String)]) -> Result<ItemsQuery, OgcError> {
    let get = |k: &str| {
        kv.iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
    };
    let mut q = ItemsQuery::default();
    if let Some(l) = get("limit") {
        let n = l
            .parse::<usize>()
            .map_err(|_| OgcError::bad_request(format!("limit '{l}' is not an integer")))?;
        q.limit = n.clamp(1, MAX_LIMIT);
    }
    if let Some(o) = get("offset") {
        q.offset = o
            .parse::<usize>()
            .map_err(|_| OgcError::bad_request(format!("offset '{o}' is not an integer")))?;
    }
    if let Some(b) = get("bbox") {
        let nums: Result<Vec<f64>, _> = b.split(',').map(|s| s.trim().parse::<f64>()).collect();
        let nums = nums
            .map_err(|_| OgcError::bad_request(format!("bbox '{b}' is not a number list")))?;
        let idx = match nums.len() {
            4 => [0usize, 1, 2, 3],
            6 => [0, 1, 3, 4], // minx,miny,minz,maxx,maxy,maxz
            _ => {
                return Err(OgcError::bad_request(
                    "bbox needs 4 or 6 numbers (minx,miny[,minz],maxx,maxy[,maxz])",
                ));
            }
        };
        let crs_raw = get("bbox-crs").or_else(|| get("crs")).unwrap_or("CRS:84");
        let northing_first = bbox_northing_first(crs_raw)?;
        let (x0, y0, x1, y1) = (nums[idx[0]], nums[idx[1]], nums[idx[2]], nums[idx[3]]);
        q.bbox = Some(if northing_first {
            Bbox::new(y0, x0, y1, x1)
        } else {
            Bbox::new(x0, y0, x1, y1)
        });
    }
    Ok(q)
}

/// bbox 是否纬度在前（仅对地理系成立）；CRS84 与其各写法经度在前。
fn bbox_northing_first(crs_raw: &str) -> Result<bool, OgcError> {
    use crate::wms_common::crs;
    if crs::is_crs84(crs_raw) {
        return Ok(false);
    }
    let n = crs::normalize(crs_raw);
    if crs::is_geographic(&n) {
        return Ok(true);
    }
    Err(OgcError::bad_request(format!(
        "bbox-crs '{crs_raw}' is not supported; features are filtered in a geographic CRS (CRS:84 or EPSG:4326)"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn defaults() {
        let q = parse_items_query(&kv(&[])).unwrap();
        assert_eq!(q.limit, DEFAULT_LIMIT);
        assert_eq!(q.offset, 0);
        assert!(q.bbox.is_none());
    }

    #[test]
    fn parses_and_clamps() {
        let q = parse_items_query(&kv(&[("limit", "5000"), ("offset", "7")])).unwrap();
        assert_eq!(q.limit, MAX_LIMIT);
        assert_eq!(q.offset, 7);
        // limit 0 clamps up to 1 (an empty page is never useful).
        let q = parse_items_query(&kv(&[("limit", "0")])).unwrap();
        assert_eq!(q.limit, 1);
    }

    #[test]
    fn bbox_lonlat_and_6d() {
        let q = parse_items_query(&kv(&[("bbox", "100,30,101,31")])).unwrap();
        let b = q.bbox.unwrap();
        assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 101.0, 31.0]);
        // 6 元组 = minx,miny,minz,maxx,maxy,maxz：高程位不能把 max 顶掉
        let q = parse_items_query(&kv(&[("bbox", "100,30,0,101,31,0")])).unwrap();
        let b = q.bbox.unwrap();
        assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 101.0, 31.0]);
        // Unknown params ignored (lenient).
        let q = parse_items_query(&kv(&[("datetime", "2020-01-01T00:00:00Z")])).unwrap();
        assert!(q.bbox.is_none());
    }

    /// `bbox-crs`/`crs` 必须真的参与轴序判定（集合元数据宣告了 4326）。
    #[test]
    fn bbox_crs_drives_axis_order() {
        // 缺省 / CRS:84 各写法：经度在前
        for crs in [
            "CRS:84",
            "urn:ogc:def:crs:OGC:1.3:CRS84",
            "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
        ] {
            let q = parse_items_query(&kv(&[("bbox", "100,30,101,31"), ("bbox-crs", crs)])).unwrap();
            let b = q.bbox.unwrap();
            assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 101.0, 31.0], "{crs}");
        }
        // 地理系：纬度在前（与 WMS 1.3.0 同规则），各种写法都要认
        for crs in [
            "EPSG:4326",
            "urn:ogc:def:crs:EPSG::4326",
            "http://www.opengis.net/def/crs/EPSG/0/4326",
        ] {
            let q = parse_items_query(&kv(&[("bbox", "30,100,31,101"), ("bbox-crs", crs)])).unwrap();
            let b = q.bbox.unwrap();
            assert_eq!([b.minx, b.miny, b.maxx, b.maxy], [100.0, 30.0, 101.0, 31.0], "{crs}");
        }
        // 1.0 的遗留参数名 `crs` 同样生效
        let q = parse_items_query(&kv(&[("bbox", "30,100,31,101"), ("crs", "EPSG:4326")])).unwrap();
        assert_eq!(q.bbox.unwrap().minx, 100.0);
    }

    /// 投影 CRS 受理不了（要素只按 WGS84 存），必须 400 而不是静默错筛。
    #[test]
    fn bbox_crs_rejects_projected_and_unknown() {
        for crs in ["EPSG:3857", "urn:ogc:def:crs:EPSG::32648", "EPSG:9999", "not-a-crs"] {
            let e = parse_items_query(&kv(&[("bbox", "100,30,101,31"), ("bbox-crs", crs)]));
            assert!(e.is_err(), "bbox-crs={crs} 应被拒");
        }
    }

    #[test]
    fn bad_values_rejected() {
        assert!(parse_items_query(&kv(&[("limit", "abc")])).is_err());
        assert!(parse_items_query(&kv(&[("offset", "-1")])).is_err());
        assert!(parse_items_query(&kv(&[("bbox", "1,2,3")])).is_err());
    }
}
