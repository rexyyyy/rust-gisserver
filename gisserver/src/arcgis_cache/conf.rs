//! 缓存目录定位与元数据装载：把「ArcGIS Server 缓存目录 / `.tpk` / `.tpkx`」
//! 三种形态归到**同一条定义记录**上。
//!
//! # 目录骨架
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
//! `.tpkx`（Pro Compact Tile Package）则是另一种骨架：
//!
//! ```text
//! <包根>/
//! ├─ root.json     瓦片方案（等价于 Conf.xml，字段名不同）
//! └─ tile/Lxx/R{rrrr}C{cccc}.bundle                   紧凑缓存（v2）
//! ```
//!
//! # 本模块只做两件事
//!
//! 1. **定位**：在缓存目录（或其一级子目录）里找到配置文件——`Conf.xml` 优先，
//!    否则 `root.json`——并确定瓦片根目录（`_alllayers` / `tile`）；
//! 2. **装载**：把配置文件解析成**一条定义记录**
//!    （[`crate::arcgis_cache::definition::CacheDefinition`]），并按内容指纹
//!    在登记表里**引用或新建**。
//!
//! 网格推导（[`crate::arcgis_cache::grid`]）与瓦片读取
//! （[`crate::arcgis_cache::bundle`]）只认定义记录，因此三种形态**行为一致**。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::arcgis_cache::definition::{
    self, CacheDefinition, DefinitionEntry, DefinitionKind,
};
use crate::arcgis_cache::xmlscan;

/// 兼容旧名：`CacheInfo` 即当前的定义记录（`Conf.xml` 形态）。
pub type CacheInfo = CacheDefinition;

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

/// 定位并读取一个缓存服务的元数据。
///
/// `dir` 允许是缓存服务根本身（含配置文件），也可以是它的上层目录（真实产出
/// 常是 `<服务名>/<图层名|Layers>/Conf.xml`）。文件名大小写按磁盘实际为准
/// （`Conf.xml` / `conf.xml` 都出现过）。
pub struct CacheDocs {
    /// 配置文件所在目录（瓦片根目录的父目录）。
    pub cache_dir: PathBuf,
    /// **定义记录**（网格方案）。内容一致的多个缓存共享同一条记录。
    pub def: Arc<CacheDefinition>,
    /// 数据范围（缓存 CRS 单位）；`conf.cdi` 缺失时为 `None`。
    pub cdi: Option<Cdi>,
    /// 登记表条目（定义记录 + 数据范围）。
    pub entry: DefinitionEntry,
    /// 本次装载是否**引用了已有的**定义记录（内容一致，未新建）。
    ///
    /// 管理面导入 tpk 时要把这个结论回给用户（「这份缓存的网格方案与已有缓存
    /// 完全一致，已直接引用，未新建记录」），因此不能只在登记表内部知道。
    pub reused: bool,
}

impl CacheDocs {
    /// 装载缓存元数据。
    ///
    /// 处理顺序：
    ///
    /// 1. `.tpk` / `.tpkx` 包先幂等解压为目录（见 [`crate::arcgis_cache::tpk`]）；
    /// 2. 定位配置文件（`Conf.xml` 优先，否则 `root.json`）；
    /// 3. 按内容指纹在登记表里**引用已有定义记录**，或新建一条；
    /// 4. 读数据范围（`conf.cdi` 或 `root.json` 的 `fullExtent`）。
    pub fn load(dir: &Path) -> Result<CacheDocs, String> {
        // `.tpk` / `.tpkx`（ZIP 封装的缓存包）先幂等解压为目录；普通目录原样返回。
        // 这样 ArcgisCache::open / 启动校验 / 网格注入 / 运行时出图都经由同一
        // 解析点，对包与目录表现一致。
        let dir = crate::arcgis_cache::tpk::ensure_dir(dir)?;
        let (config, kind) = definition::find_config(&dir)?;
        let cache_dir = config.parent().unwrap_or(&dir).to_path_buf();

        // 范围文件：`Conf.xml` 形态取 conf.cdi；`root.json` 形态范围在 root.json 内。
        let cdi_path = if kind == DefinitionKind::ConfXml {
            find_sibling_file(&cache_dir, "conf.cdi")
        } else {
            None
        };

        // 登记键：路径 + 配置文件与范围文件的大小/修改时间。数据变了要重新解析。
        let key = definition::source_key(&cache_dir, &config, cdi_path.as_deref());

        // 命中登记 → 直接复用（连配置文件都不必再读）。
        if let Some(entry) = definition::lookup(&key) {
            let cdi = entry.bounds.map(|b| Cdi { bounds: b, wkid: entry.def.wkid });
            return Ok(CacheDocs {
                cache_dir,
                def: Arc::clone(&entry.def),
                cdi,
                entry,
                reused: true,
            });
        }

        let text = read_to_string_lossy(&config)?;
        let def = match kind {
            DefinitionKind::ConfXml => CacheDefinition::from_xml(&text)?,
            DefinitionKind::RootJson => CacheDefinition::from_root_json(&text)?,
        };

        // 数据范围：`conf.cdi` 优先；`root.json` 形态从 root.json 内部取。
        let cdi = match kind {
            DefinitionKind::ConfXml => cdi_path
                .as_ref()
                .and_then(|p| read_to_string_lossy(p).ok())
                .and_then(|t| Cdi::from_xml(&t).ok()),
            DefinitionKind::RootJson => CacheDefinition::root_json_bounds(&text)
                .map(|b| Cdi { bounds: b, wkid: def.wkid }),
        };

        // 内容一致 → 引用已有定义记录，不新建。
        let (entry, reused) = definition::acquire(&key, def, cdi.map(|c| c.bounds));
        Ok(CacheDocs {
            cache_dir,
            def: Arc::clone(&entry.def),
            cdi,
            entry,
            reused,
        })
    }

    /// 定义记录（网格方案）。
    pub fn definition(&self) -> &CacheDefinition {
        &self.def
    }
}

/// 在 `dir`（含一级子目录）里找 Conf.xml。
///
/// 对外暴露给管理面体检复用（见 map-api `admin::check`）：真实产出常是
/// `<path>/<图层名|Layers>/Conf.xml`，只探 `<path>/Conf.xml` 会把正常出图的
/// 缓存误报成「不是 ArcGIS 预切片缓存」，故定位逻辑必须与此处保持一致。
pub fn find_conf(dir: &Path) -> Result<PathBuf, String> {
    if let Some(p) = find_sibling_file(dir, "conf.xml") {
        return Ok(p);
    }
    // 允许 <dir>/<图层目录>/Conf.xml 这一层（只下一层，避免深扫 Status.gdb）。
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = find_sibling_file(&p, "conf.xml") {
                    return Ok(f);
                }
            }
        }
    }
    Err(format!("{} 下未找到 Conf.xml（缓存根目录或其一级图层子目录均需包含）", dir.display()))
}

/// 在目录里按不区分大小写的名字找文件（返回磁盘实际大小写）。
pub fn find_sibling_file(dir: &Path, name: &str) -> Option<PathBuf> {
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
pub fn read_to_string_lossy(path: &Path) -> Result<String, String> {
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
    use crate::arcgis_cache::definition::ImageType;

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
        assert_eq!(docs.def.crs, "EPSG:4490");
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

    /// `root.json` 形态（`.tpkx`）必须被同一套装载路径认出，且瓦片根为 `tile`。
    #[test]
    fn loads_root_json_layout() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("root.json"),
            r#"{"tileBundlesPath":"./tile","storageInfo":{"packetSize":128},
                "tileInfo":{"spatialReference":{"wkid":4522},"origin":{"x":28876800,"y":10002100},
                "rows":256,"cols":256,"dpi":96,
                "lods":[{"level":0,"resolution":4.233341800016934,"scale":16000}]},
                "fullExtent":{"xmin":1,"ymin":2,"xmax":3,"ymax":4}}"#,
        )
        .unwrap();
        let docs = CacheDocs::load(root.path()).unwrap();
        assert_eq!(docs.def.kind, DefinitionKind::RootJson);
        assert_eq!(docs.def.crs, "EPSG:4522");
        assert_eq!(docs.def.tile_root, "tile");
        assert_eq!(docs.cdi.map(|c| c.bounds), Some([1.0, 2.0, 3.0, 4.0]));
    }
}
