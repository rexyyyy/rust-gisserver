//! `Conf.xml` / `conf.cdi` 解析：ArcGIS 切片缓存的自描述元数据。
//!
//! 预切片缓存目录的骨架：
//!
//! ```text
//! <缓存服务根>/
//! ├─ Conf.xml      瓦片方案：CRS、TileOrigin、TileCols/Rows、DPI、LODInfos、
//! │                CacheTileFormat(PNG|JPEG|MIXED|LERC)、StorageFormat、PacketSize
//! ├─ conf.cdi      数据范围（EnvelopeN，单位同缓存 CRS）
//! ├─ Layers.properties / Status.gdb（服务端内部管理，本模块不读）
//! └─ _alllayers/Lxx/R{rrrr}C{cccc}.bundle[.bundlx]   紧凑缓存（v1/v2）
//! ```
//!
//! 本模块只认 Conf.xml + conf.cdi：二者足够推导出 custom grid（见
//! [`crate::arcgis_cache::grid`]）与 bundle 寻址（见 [`crate::arcgis_cache::bundle`]）。`BundleVersion`
//! 元素在真实产出里经常缺失，版本判定因此以**文件形态**为准（是否存在
//! `.bundlx`），显式元素仅作为优先线索。

use std::path::{Path, PathBuf};

use crate::arcgis_cache::xmlscan;

/// 瓦片影像编码（`TileImageInfo/CacheTileFormat`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
    /// gis-wt 定制：WebP 瓦片（bundle 扩展名为 .webpdle）。
    Webp,
    /// 混合：同一缓存里既有 PNG 又有 JPEG（最常见），需按瓦片魔数嗅探。
    Mixed,
    /// 其他/未知（LERC 等），按二进制透传。
    Other,
}

impl ImageType {
    pub fn parse(s: &str) -> ImageType {
        match s.trim().to_ascii_uppercase().as_str() {
            "PNG" | "PNG8" => ImageType::Png,
            "JPEG" | "MIXEDJPEG" => ImageType::Jpeg,
            // WEBPDLE 是 gis-wt 写端在 CacheTileFormat 里打的定制标识；
            // 也兼容标准 "WEBP"，便于将来读标准 WebP 缓存。
            "WEBP" | "WEBPDLE" => ImageType::Webp,
            "MIXED" => ImageType::Mixed,
            _ => ImageType::Other,
        }
    }
}

/// 一级细节（Level of Detail）：分辨率（CRS 单位/像素）+ 地图比例尺分母。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodInfo {
    /// `LevelID`（缓存层级，等于 `_alllayers/Lxx` 的 xx）。
    pub level: u32,
    /// `Scale`：ArcGIS 口径的比例尺分母（按 `0.0254/DPI` 像素物理尺寸导出，
    /// 与 OGC 的 0.28 mm 口径不同，见 [`crate::arcgis_cache::grid`] 的说明）。
    pub scale: f64,
    /// `Resolution`：CRS 单位/像素。
    pub resolution: f64,
}

/// `Conf.xml` 的读取结果（本服务关心的子集）。
#[derive(Clone, Debug)]
pub struct CacheInfo {
    /// 缓存 CRS 标识（`EPSG:wkid`，取 LatestWKID → WKID → WKT `AUTHORITY`）。
    pub crs: String,
    /// 采用的 WKID（`LatestWKID` 优先；0 = 未知）。
    pub wkid: u32,
    /// `TileOrigin`：**左上角** `[x, y]`（ArcGIS 缓存行列即从该点起算）。
    pub origin: [f64; 2],
    /// `TileCols` / `TileRows`（非正方形时以列宽为准并保留行高，见 [`Self::tile_rows`]）。
    pub tile_cols: u32,
    pub tile_rows: u32,
    /// `DPI`（缺失回退 `PreciseDPI`，再缺失回退 96）。
    pub dpi: f64,
    /// 按 `LevelID` 升序的 LOD 阶梯。
    pub lods: Vec<LodInfo>,
    pub image_type: ImageType,
    /// `CacheStorageInfo/StorageFormat`（`esriMapCacheStorageModeCompact` 等）。
    pub storage_format: String,
    /// `PacketSize`（紧凑缓存一个 bundle 的边长，通常 128）。
    pub packet_size: u32,
    /// 显式 `BundleVersion`（1 = 10.0~10.2，2 = 10.3+）；真实数据常缺失。
    pub bundle_version: Option<u32>,
}

impl CacheInfo {
    /// 正方形瓦片边长（行列不等时取列，行高另由 [`Self::tile_rows`] 表达）。
    pub fn tile_px(&self) -> u32 {
        self.tile_cols.max(1)
    }

    /// 最粗/最细层级（`_alllayers` 目录名以此为准）。
    pub fn min_zoom(&self) -> u32 {
        self.lods.first().map(|l| l.level).unwrap_or(0)
    }

    pub fn max_zoom(&self) -> u32 {
        self.lods.last().map(|l| l.level).unwrap_or(0)
    }

    /// 解析 Conf.xml 文本。
    pub fn from_xml(text: &str) -> Result<CacheInfo, String> {
        let root = xmlscan::parse(text)?;
        let tci = root
            .child("TileCacheInfo")
            .ok_or("Conf.xml 缺少 TileCacheInfo")?;

        // 空间参考：TileCacheInfo 下常有内嵌 SpatialReference；WKID 有时只在
        // LatestWKID（高版本），因此两条都认。**LatestWKID 优先**：它是
        // 同一空间参考给出的现行官方码（如 Web Mercator 的 WKID=102100 已废弃、
        // LatestWKID=3857），旧产出里两值相同（本仓库 E 盘样例 4490/4490），
        // 优先它不改变既有行为。
        let sr = tci
            .child("SpatialReference")
            .or_else(|| root.find("SpatialReference"))
            .cloned()
            .unwrap_or_default();
        let wkid = sr
            .u32_of("LatestWKID")
            .or_else(|| sr.u32_of("WKID"))
            .or_else(|| wkt_authority(sr.text_of("WKT").unwrap_or("")))
            .unwrap_or(0);
        let crs = if wkid > 0 {
            format!("EPSG:{wkid}")
        } else {
            return Err("Conf.xml 无法确定缓存 CRS（缺 WKID/LatestWKID/WKT AUTHORITY）".to_string());
        };

        let origin_el = tci
            .child("TileOrigin")
            .ok_or("Conf.xml 缺少 TileOrigin")?;
        let ox = origin_el
            .f64_of("X")
            .ok_or("Conf.xml TileOrigin 缺少 X")?;
        let oy = origin_el
            .f64_of("Y")
            .ok_or("Conf.xml TileOrigin 缺少 Y")?;

        let tile_cols = tci.u32_of("TileCols").filter(|v| *v > 0).ok_or("Conf.xml 缺少 TileCols")?;
        let tile_rows = tci.u32_of("TileRows").filter(|v| *v > 0).unwrap_or(tile_cols);

        let dpi = tci
            .f64_of("DPI")
            .or_else(|| tci.f64_of("PreciseDPI"))
            .filter(|v| *v > 0.0)
            .unwrap_or(96.0);

        // LODInfos 里是重复的 LODInfo；LevelID 可能不连续（缓存只切了部分层级），
        // 因此按 LevelID 升序保留原值，不用数组下标当层级。
        let lod_infos = tci
            .child("LODInfos")
            .ok_or("Conf.xml 缺少 LODInfos")?;
        let mut lods: Vec<LodInfo> = Vec::new();
        for el in lod_infos.children_named("LODInfo") {
            let level = el.u32_of("LevelID").ok_or("LODInfo 缺少 LevelID")?;
            let resolution = el.f64_of("Resolution").ok_or("LODInfo 缺少 Resolution")?;
            let scale = el.f64_of("Scale").unwrap_or(0.0);
            if resolution <= 0.0 {
                return Err(format!("LODInfo LevelID={level} 的 Resolution 非正"));
            }
            lods.push(LodInfo { level, scale, resolution });
        }
        if lods.is_empty() {
            return Err("Conf.xml 的 LODInfos 为空".to_string());
        }
        lods.sort_by(|a, b| a.level.cmp(&b.level));

        let image_type = root
            .child("TileImageInfo")
            .and_then(|t| t.text_of("CacheTileFormat"))
            .map(ImageType::parse)
            .unwrap_or(ImageType::Mixed);

        let storage = root.child("CacheStorageInfo");
        let storage_format = storage
            .as_ref()
            .and_then(|s| s.text_of("StorageFormat"))
            .unwrap_or("")
            .to_string();
        let packet_size = storage
            .as_ref()
            .and_then(|s| s.u32_of("PacketSize"))
            .filter(|v| *v > 0)
            .unwrap_or(128);
        let bundle_version = root
            .u32_of("BundleVersion")
            .or_else(|| storage.as_ref().and_then(|s| s.u32_of("BundleVersion")));

        Ok(CacheInfo {
            crs,
            wkid,
            origin: [ox, oy],
            tile_cols,
            tile_rows,
            dpi,
            lods,
            image_type,
            storage_format,
            packet_size,
            bundle_version,
        })
    }
}

/// `conf.cdi`：数据范围（EnvelopeN，单位与缓存 CRS 一致）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cdi {
    /// `[minx, miny, maxx, maxy]`
    pub bounds: [f64; 4],
    pub wkid: u32,
}

impl Cdi {
    pub fn from_xml(text: &str) -> Result<Cdi, String> {
        let root = xmlscan::parse(text)?;
        // 有的产出把包络嵌在 <Envelope><XMin>… 里，先在本层找，再深搜一层。
        let get = |name: &str| -> Option<f64> {
            root.f64_of(name)
                .or_else(|| root.find(name).and_then(|e| e.text.trim().parse().ok()))
        };
        let (minx, miny, maxx, maxy) = match (get("XMin"), get("YMin"), get("XMax"), get("YMax")) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => return Err("conf.cdi 缺少 XMin/YMin/XMax/YMax".to_string()),
        };
        if !(maxx > minx && maxy > miny) {
            return Err(format!("conf.cdi 包络无效: [{minx},{miny},{maxx},{maxy}]"));
        }
        let wkid = root
            .u32_of("WKID")
            .or_else(|| root.find("SpatialReference").and_then(|s| s.u32_of("WKID")))
            .unwrap_or(0);
        Ok(Cdi { bounds: [minx, miny, maxx, maxy], wkid })
    }
}

/// 从 WKT 里取 EPSG 码（ArcGIS 常把 WKID 省略只留 WKT）。
///
/// 常见写法是 `AUTHORITY["EPSG",4490]`——**码不带引号**，少数工具会写成
/// `"4490"`，两种都认。取**最后**一个 AUTHORITY：外层对象（即 CRS 本身）
/// 的 AUTHORITY 写在括号末尾，内层的 SPHEROID/DATUM/UNIT 各有一个。
fn wkt_authority(wkt: &str) -> Option<u32> {
    let pos = wkt.rfind("AUTHORITY")?;
    let tail = &wkt[pos + "AUTHORITY".len()..];
    let l = tail.find('[')?;
    let inner = &tail[l + 1..];
    let r = inner.find(']')?;
    let mut it = inner[..r].split(',');
    let org = it.next()?.trim().trim_matches('"');
    let code = it.next()?.trim().trim_matches('"');
    if org.eq_ignore_ascii_case("EPSG") {
        code.parse().ok()
    } else {
        None
    }
}

/// 定位并读取一个缓存服务的元数据文件。
///
/// `dir` 允许是缓存服务根本身（含 `Conf.xml`），也可以是它的上层目录（真实产出
/// 常是 `<服务名>/<图层名|Layers>/Conf.xml`）。文件名大小写按磁盘实际为准
/// （`Conf.xml` / `conf.xml` 都出现过）。
pub struct CacheDocs {
    /// `Conf.xml` 所在目录（`_alllayers` 的父目录）。
    pub cache_dir: PathBuf,
    pub info: CacheInfo,
    /// `conf.cdi` 缺失时为 `None`（范围可由 bundle 探测兜底，见 `crate::arcgis_cache::grid`）。
    pub cdi: Option<Cdi>,
}

impl CacheDocs {
    pub fn load(dir: &Path) -> Result<CacheDocs, String> {
        // `.tpk`（ZIP 封装的缓存包）先幂等解压为目录；普通目录原样返回。
        // 这样 ArcgisCache::open / 启动校验 / 网格注入 / 运行时出图都经由同一
        // 解析点，对 `.tpk` 与目录表现一致（见 crate::arcgis_cache::tpk）。
        let dir = crate::arcgis_cache::tpk::ensure_dir(dir)?;
        let conf = find_conf(&dir)?;
        let cache_dir = conf.parent().unwrap_or(&dir).to_path_buf();
        let text = read_to_string_lossy(&conf)?;
        let info = CacheInfo::from_xml(&text)
            .map_err(|e| format!("{}: {e}", conf.display()))?;
        let cdi = find_sibling(&cache_dir, "conf.cdi")
            .and_then(|p| read_to_string_lossy(&p).ok())
            .and_then(|t| Cdi::from_xml(&t).ok());
        Ok(CacheDocs { cache_dir, info, cdi })
    }
}

/// 在 `dir`（含一级子目录）里找 Conf.xml。
///
/// 对外暴露给管理面体检复用（见 map-api `admin::check`）：真实产出常是
/// `<path>/<图层名|Layers>/Conf.xml`，只探 `<path>/Conf.xml` 会把正常出图的
/// 缓存误报成「不是 ArcGIS 预切片缓存」，故定位逻辑必须与此处保持一致。
pub fn find_conf(dir: &Path) -> Result<PathBuf, String> {
    if let Some(p) = find_sibling(dir, "conf.xml") {
        return Ok(p);
    }
    // 允许 <dir>/<图层目录>/Conf.xml 这一层（只下一层，避免深扫 Status.gdb）。
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = find_sibling(&p, "conf.xml") {
                    return Ok(f);
                }
            }
        }
    }
    Err(format!("{} 下未找到 Conf.xml（缓存根目录或其一级图层子目录均需包含）", dir.display()))
}

/// 在目录里按不区分大小写的名字找文件（返回磁盘实际大小写）。
fn find_sibling(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file()
            && p.file_name()
                .map(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
                .unwrap_or(false)
        {
            return Some(p);
        }
    }
    None
}

/// ArcGIS 产出里偶见 UTF-8 BOM 与 UTF-16 LE；两者都能被这段兜底处理。
fn read_to_string_lossy(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读 {}: {e}", path.display()))?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes[3..].to_vec()).map_err(|e| e.to_string());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let (le, body) = if bytes.starts_with(&[0xFF, 0xFE]) {
            (true, &bytes[2..])
        } else {
            (false, &bytes[2..])
        };
        let units: Vec<u16> = body
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return String::from_utf16(&units).map_err(|e| e.to_string());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = r#"<?xml version="1.0" encoding="utf-8" ?>
<CacheInfo xsi:type='typens:CacheInfo'>
  <TileCacheInfo xsi:type='typens:TileCacheInfo'>
    <SpatialReference xsi:type='typens:GeographicCoordinateSystem'>
      <WKT>GEOGCS[&quot;GCS_China_Geodetic_Coordinate_System_2000&quot;,DATUM[&quot;D_China_2000&quot;,SPHEROID[&quot;CGCS2000&quot;,6378137.0,298.257222101]],PRIMEM[&quot;Greenwich&quot;,0.0],UNIT[&quot;Degree&quot;,0.0174532925199433],AUTHORITY[&quot;EPSG&quot;,4490]]</WKT>
      <XOrigin>-400</XOrigin><YOrigin>-400</YOrigin><XYScale>11258999068426.24</XYScale>
      <HighPrecision>true</HighPrecision><LeftLongitude>-180</LeftLongitude>
      <WKID>4490</WKID><LatestWKID>4490</LatestWKID>
    </SpatialReference>
    <TileOrigin xsi:type='typens:PointN'><X>-180</X><Y>90</Y></TileOrigin>
    <TileCols>256</TileCols><TileRows>256</TileRows><DPI>96</DPI><PreciseDPI>96</PreciseDPI>
    <LODInfos xsi:type='typens:ArrayOfLODInfo'>
      <LODInfo xsi:type='typens:LODInfo'><LevelID>0</LevelID><Scale>295829355.454566</Scale><Resolution>0.70391441568404511</Resolution></LODInfo>
      <LODInfo xsi:type='typens:LODInfo'><LevelID>1</LevelID><Scale>147914677.727283</Scale><Resolution>0.35195720784202256</Resolution></LODInfo>
      <LODInfo xsi:type='typens:LODInfo'><LevelID>2</LevelID><Scale>73957338.863641396</Scale><Resolution>0.17597860392101103</Resolution></LODInfo>
    </LODInfos>
  </TileCacheInfo>
  <TileImageInfo xsi:type='typens:TileImageInfo'>
    <CacheTileFormat>MIXED</CacheTileFormat><CompressionQuality>75</CompressionQuality>
  </TileImageInfo>
  <CacheStorageInfo xsi:type='typens:CacheStorageInfo'>
    <StorageFormat>esriMapCacheStorageModeCompact</StorageFormat><PacketSize>128</PacketSize>
  </CacheStorageInfo>
</CacheInfo>"#;

    fn conf() -> CacheInfo {
        CacheInfo::from_xml(CONF).unwrap()
    }

    #[test]
    fn parses_crs_origin_lods() {
        let c = conf();
        assert_eq!(c.crs, "EPSG:4490");
        assert_eq!(c.wkid, 4490);
        assert_eq!(c.origin, [-180.0, 90.0]);
        assert_eq!((c.tile_cols, c.tile_rows, c.dpi), (256, 256, 96.0));
        assert_eq!(c.lods.len(), 3);
        assert_eq!(c.lods[2].level, 2);
        assert!((c.lods[0].resolution - 0.70391441568404511).abs() < 1e-15);
        assert!((c.lods[0].scale - 295829355.454566).abs() < 1e-3);
        assert_eq!(c.image_type, ImageType::Mixed);
        assert_eq!(c.storage_format, "esriMapCacheStorageModeCompact");
        assert_eq!(c.packet_size, 128);
        assert_eq!(c.bundle_version, None);
        assert_eq!((c.min_zoom(), c.max_zoom()), (0, 2));
    }

    /// WKID 缺失时，必须能从 WKT 的 AUTHORITY 兜底拿码，否则 CRS 无从判定。
    #[test]
    fn falls_back_to_wkt_authority() {
        let text = CONF.replace("<WKID>4490</WKID><LatestWKID>4490</LatestWKID>", "");
        let c = CacheInfo::from_xml(&text).unwrap();
        assert_eq!(c.crs, "EPSG:4490");
    }

    /// LatestWKID 是现行官方码：与废弃码并存时必须取前者
    /// （例：WKID=102100 + LatestWKID=3857）。
    #[test]
    fn latest_wkid_wins_over_legacy_wkid() {
        let text = CONF.replace("<WKID>4490</WKID><LatestWKID>4490</LatestWKID>", "<WKID>102100</WKID><LatestWKID>3857</LatestWKID>");
        let c = CacheInfo::from_xml(&text).unwrap();
        assert_eq!(c.crs, "EPSG:3857");
        assert_eq!(c.wkid, 3857);
    }

    /// 缺关键字段必须报错并指名缺什么，而不是给出一个静默错误的网格。
    #[test]
    fn reports_missing_pieces() {
        assert!(CacheInfo::from_xml(&CONF.replace("<TileOrigin xsi:type='typens:PointN'><X>-180</X><Y>90</Y></TileOrigin>", "")).unwrap_err().contains("TileOrigin"));
        let no_res = CONF.replace("<Resolution>0.17597860392101103</Resolution>", "");
        assert!(CacheInfo::from_xml(&no_res).unwrap_err().contains("Resolution"));
    }

    #[test]
    fn parses_cdi_envelope() {
        let cdi = Cdi::from_xml(
            r#"<?xml version="1.0" encoding="utf-8" ?><EnvelopeN xsi:type='typens:EnvelopeN'><XMin>102.23285257581954</XMin><YMin>24.438884054211364</YMin><XMax>102.83157693</XMax><YMax>24.757753353357</YMax><SpatialReference><WKID>4490</WKID></SpatialReference></EnvelopeN>"#,
        )
        .unwrap();
        assert_eq!(cdi.wkid, 4490);
        assert!((cdi.bounds[0] - 102.23285257581954).abs() < 1e-12);
        assert!((cdi.bounds[3] - 24.757753353357).abs() < 1e-12);
        // 反向包络（min>max）是产出错误，必须拒绝
        assert!(Cdi::from_xml("<EnvelopeN><XMin>10</XMin><YMin>10</YMin><XMax>1</XMax><YMax>1</YMax></EnvelopeN>").is_err());
    }

    #[test]
    fn wkt_authority_helper() {
        assert_eq!(wkt_authority(r#"GEOGCS[..,AUTHORITY["EPSG",4490]]"#), Some(4490));
        assert_eq!(wkt_authority(r#"GEOGCS[..,AUTHORITY["EPSG","4490"]]"#), Some(4490));
        assert_eq!(wkt_authority(r#"PROJCS[..,AUTHORITY["ESRI",102100]]"#), None);
        assert_eq!(wkt_authority("no authority"), None);
        // 内层也有 AUTHORITY，取最外层（最后一个）才是 CRS 码
        assert_eq!(
            wkt_authority(r#"GEOGCS[..,UNIT[..,AUTHORITY["EPSG",9102]],AUTHORITY["EPSG",4490]]"#),
            Some(4490)
        );
    }

    /// UTF-16LE 的 Conf.xml（ArcGIS 10.x 默认产出编码之一）必须能读。
    #[test]
    fn reads_utf16_documents() {
        let mut utf16: Vec<u8> = vec![0xFF, 0xFE];
        for u in CONF.encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Conf.xml"), &utf16).unwrap();
        let docs = CacheDocs::load(dir.path()).unwrap();
        assert_eq!(docs.info.crs, "EPSG:4490");
        assert_eq!(docs.cache_dir, dir.path());
    }

    /// `<服务根>/<图层目录>/Conf.xml` 这一层要能自动下一跳。
    #[test]
    fn loads_through_layer_subdir() {
        let root = tempfile::tempdir().unwrap();
        let layer = root.path().join("图层");
        std::fs::create_dir_all(&layer).unwrap();
        std::fs::write(layer.join("Conf.xml"), CONF).unwrap();
        let docs = CacheDocs::load(root.path()).unwrap();
        assert_eq!(docs.cache_dir, layer);
        assert!(docs.cdi.is_none());
    }
}
