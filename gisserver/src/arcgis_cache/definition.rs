//! 缓存**定义记录**（CacheDefinition）与**登记表**（DefinitionRegistry）。
//!
//! # 为什么要有这一层
//!
//! 同一份 ArcGIS 切片缓存，配置可能来自三种文件，字段名各不相同：
//!
//! | 来源 | 配置文件 | 目录布局 | 瓦片根 |
//! |---|---|---|---|
//! | ArcGIS Server 缓存目录 | `Conf.xml`（+ `conf.cdi`） | `<svc>/Conf.xml`、`_alllayers/L{zz}/` | `_alllayers` |
//! | `.tpk` 包（ZIP） | `Conf.xml`（+ `conf.cdi`） | 解开后与上一行同形 | `_alllayers` |
//! | `.tpkx` 包（Pro Compact Tile Package） | `root.json` | `<pkg>/root.json`、`tile/L{zz}/` | `tile` |
//!
//! 但**网格、坐标、LOD 的语义三者完全一致**（同一套 ArcGIS 瓦片方案）。若各写一套
//! 解析器，网格推导（[`crate::arcgis_cache::grid`]）就得跟着分叉——而网格推导是
//! **最容易写错且错了不报错**的一环（取瓦片整体错位）。
//!
//! 因此这里把三者归一成**同一条定义记录** [`CacheDefinition`]：无论配置文件是
//! `Conf.xml` 还是 `root.json`，都产出同一形状的记录；后续（网格推导、瓦片寻址、
//! 管理面展示）只认这一个类型。
//!
//! # 定义记录 vs 数据
//!
//! 定义记录**只描述网格方案**，不含瓦片数据，也不含数据范围（范围是数据的属性：
//! `Conf.xml` 形态取自 `conf.cdi`，`root.json` 形态取自 `fullExtent`）。
//! 一个缓存目录/包（"数据"）对应一条定义记录；两者是**多对一**关系。
//!
//! # 去重与复用
//!
//! [`CacheDefinition::fingerprint`] 只覆盖**决定瓦片寻址与网格**的字段：
//! CRS、原点、瓦片尺寸、DPI、LOD 阶梯、PacketSize、瓦片根目录名。
//!
//! 刻意**不**纳入指纹的字段：
//!
//! * `kind`（`Conf.xml` / `root.json`）：只是来源形态，同一网格方案两种写法互不影响；
//! * `image_type`：读取时按瓦片魔数嗅探（PNG/JPEG/WebP），声明值仅兜底；
//! * `storage_format`：`PacketSize` 已经决定了 bundle 索引布局；
//! * 数据范围：同一套瓦片方案完全可以装不同范围的数据（详见 [`DefinitionEntry`]）。
//!
//! 于是：`.tpk` 解出来的缓存、它原本的目录形态、乃至另一份网格完全相同的缓存，
//! 都**引用同一条定义记录**，内存里只存一份 LOD 阶梯。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::arcgis_cache::xmlscan;

// ── 定义记录的字段 ───────────────────────────────────────────────────────

/// 瓦片影像编码（`TileImageInfo/CacheTileFormat`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
    /// 自定义扩展：WebP 瓦片（bundle 扩展名为 .webpdle）。
    Webp,
    /// 混合：同一缓存里既有 PNG 又有 JPEG（最常见），需按瓦片魔数嗅探。
    Mixed,
    /// 其他/未知（LERC 等），按二进制透传。
    Other,
}

impl ImageType {
    pub fn parse(s: &str) -> ImageType {
        match s.trim().to_ascii_uppercase().as_str() {
            // PNG24 是 ArcGIS 对 24 位真彩 PNG 的写法（实际 tpk 样例即为此值），
            // 与 PNG8 同属 PNG，声明值只作兜底、读取仍按魔数。
            "PNG" | "PNG8" | "PNG24" | "PNG32" => ImageType::Png,
            "JPEG" | "MIXEDJPEG" => ImageType::Jpeg,
            // WEBPDLE 是自定义写端在 CacheTileFormat 里打的定制标识；
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
    /// `LevelID`（缓存层级，等于瓦片目录的 `Lxx`）。
    pub level: u32,
    /// `Scale`：ArcGIS 口径的比例尺分母（按 `0.0254/DPI` 像素物理尺寸导出，
    /// 与 OGC 的 0.28 mm 口径不同，见 [`crate::arcgis_cache::grid`] 的说明）。
    pub scale: f64,
    /// `Resolution`：CRS 单位/像素。
    pub resolution: f64,
}

/// 定义记录的来源形态（仅作展示与排查用，不参与指纹）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefinitionKind {
    /// `Conf.xml`：ArcGIS Server 缓存目录与 `.tpk` 包。
    ConfXml,
    /// `root.json`：ArcGIS Pro Compact Tile Package（`.tpkx`）。
    RootJson,
}

impl DefinitionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DefinitionKind::ConfXml => "conf.xml",
            DefinitionKind::RootJson => "root.json",
        }
    }
}

/// 一条缓存定义记录：**网格方案**的全部参数。
///
/// 字段与 `Conf.xml` 的 `TileCacheInfo` 一一对应；`root.json` 走
/// [`CacheDefinition::from_root_json`] 映射到同一形状。
#[derive(Clone, Debug)]
pub struct CacheDefinition {
    /// 记录编号（由 [`DefinitionRegistry`] 分配；被复用时沿用已有编号）。
    pub id: u64,
    /// 来源形态。
    pub kind: DefinitionKind,
    /// 缓存 CRS 标识（`EPSG:wkid`）。
    pub crs: String,
    /// 采用的 WKID（`LatestWKID` 优先；0 = 未知）。
    pub wkid: u32,
    /// `TileOrigin`：**左上角** `[x, y]`（ArcGIS 缓存行列即从该点起算）。
    pub origin: [f64; 2],
    /// `TileCols` / `TileRows`（非正方形时以列宽为准并保留行高）。
    pub tile_cols: u32,
    pub tile_rows: u32,
    /// `DPI`。
    pub dpi: f64,
    /// 按 `LevelID` 升序的 LOD 阶梯。
    pub lods: Vec<LodInfo>,
    /// 声明的瓦片影像编码（读取时仍按魔数嗅探，此值仅兜底）。
    pub image_type: ImageType,
    /// `CacheStorageInfo/StorageFormat`。
    pub storage_format: String,
    /// `PacketSize`（紧凑缓存一个 bundle 的边长，通常 128）。
    pub packet_size: u32,
    /// 显式 `BundleVersion`（缺失时按文件形态判定）。
    pub bundle_version: Option<u32>,
    /// 瓦片根目录名（相对缓存目录）：`_alllayers` / `tile`。
    pub tile_root: String,
}

impl CacheDefinition {
    /// 正方形瓦片边长（行列不等时取列，行高另由 [`Self::tile_rows`] 表达）。
    pub fn tile_px(&self) -> u32 {
        self.tile_cols.max(1)
    }

    /// 最粗/最细层级（瓦片目录名以此为准）。
    pub fn min_zoom(&self) -> u32 {
        self.lods.first().map(|l| l.level).unwrap_or(0)
    }

    pub fn max_zoom(&self) -> u32 {
        self.lods.last().map(|l| l.level).unwrap_or(0)
    }

    /// 内容指纹：只覆盖**决定瓦片寻址与网格**的字段（见模块文档的取舍说明）。
    pub fn fingerprint(&self) -> String {
        let mut h = Fnv::new();
        h.write(self.crs.as_bytes());
        h.write(&self.origin[0].to_bits().to_le_bytes());
        h.write(&self.origin[1].to_bits().to_le_bytes());
        h.write(&self.tile_cols.to_le_bytes());
        h.write(&self.tile_rows.to_le_bytes());
        h.write(&self.dpi.to_bits().to_le_bytes());
        h.write(&self.packet_size.to_le_bytes());
        h.write(self.tile_root.as_bytes());
        for l in &self.lods {
            h.write(&l.level.to_le_bytes());
            h.write(&l.scale.to_bits().to_le_bytes());
            h.write(&l.resolution.to_bits().to_le_bytes());
        }
        format!("{:016x}", h.finish())
    }
}

/// 内容比较：忽略 `id`（编号只是登记产物），其余逐字段比。
impl PartialEq for CacheDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.crs == other.crs
            && self.wkid == other.wkid
            && self.origin == other.origin
            && self.tile_cols == other.tile_cols
            && self.tile_rows == other.tile_rows
            && self.dpi == other.dpi
            && self.lods == other.lods
            && self.image_type == other.image_type
            && self.storage_format == other.storage_format
            && self.packet_size == other.packet_size
            && self.bundle_version == other.bundle_version
            && self.tile_root == other.tile_root
    }
}

// ── 登记表 ───────────────────────────────────────────────────────────────

/// 一条**数据**（缓存目录/包）引用定义记录的登记条目。
///
/// 数据范围（`conf.cdi` / `root.json` 的 `fullExtent`）挂在条目上而不是定义记录上：
/// 同一套瓦片方案可以装不同范围的数据，混进指纹会让「同网格不同数据」无法复用。
#[derive(Clone, Debug)]
pub struct DefinitionEntry {
    /// 被引用的定义记录（多条数据可引用同一条）。
    pub def: Arc<CacheDefinition>,
    /// 数据范围（缓存 CRS 单位）。
    pub bounds: Option<[f64; 4]>,
    /// 本条目引用的定义记录编号。
    pub def_id: u64,
}

/// 定义记录登记表：内容一致则**引用已有的**，不新建。
#[derive(Default)]
pub struct DefinitionRegistry {
    /// 内容指纹 → 定义记录编号。
    by_fingerprint: HashMap<String, u64>,
    /// 数据源键 → 登记条目编号。
    by_source: HashMap<String, u64>,
    /// 定义记录编号 → 定义记录。
    defs: HashMap<u64, Arc<CacheDefinition>>,
    /// 登记条目编号 → 条目。
    entries: HashMap<u64, DefinitionEntry>,
    next_def_id: u64,
    next_entry_id: u64,
}

/// 全局登记表：定义记录在进程内唯一，跨线程共享。
static REGISTRY: OnceLock<Mutex<DefinitionRegistry>> = OnceLock::new();

/// 全局登记表。
pub fn registry() -> &'static Mutex<DefinitionRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(DefinitionRegistry::default()))
}

/// 数据源键：目录/文件路径 + 配置文件与范围文件的大小与修改时间。
///
/// 带 mtime/size 是为了「数据变了要重新解析」：缓存被重新切片后同名目录的内容
/// 已经不同，沿用旧记录会给出过期的 LOD 阶梯。配置文件与范围文件分别计入，
/// 避免「只改了 conf.cdi」时被误判成没变。
pub fn source_key(dir: &Path, config: &Path, bounds_file: Option<&Path>) -> String {
    let mut s = normalized_path(dir);
    s.push('|');
    s.push_str(&file_stamp(config));
    if let Some(b) = bounds_file {
        s.push('|');
        s.push_str(&file_stamp(b));
    }
    s
}

/// 规范化路径（Windows 下大小写不敏感，统一小写）。
fn normalized_path(p: &Path) -> String {
    let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let mut s = canon.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

/// 文件指纹片段：大小 + 修改时间（纳秒）。
fn file_stamp(p: &Path) -> String {
    match std::fs::metadata(p) {
        Ok(md) => {
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{}:{}", md.len(), mtime)
        }
        Err(_) => "-".to_string(),
    }
}

impl DefinitionRegistry {
    /// 取某个数据源已登记的条目（不新建）。
    pub fn lookup(&self, source_key: &str) -> Option<DefinitionEntry> {
        let id = *self.by_source.get(source_key)?;
        self.entries.get(&id).cloned()
    }

    /// 登记一个数据的定义记录：内容一致则**引用已有记录**，否则新建。
    ///
    /// 返回 `(条目, 是否复用了已有定义记录)`。
    pub fn acquire(
        &mut self,
        source_key: &str,
        def: CacheDefinition,
        bounds: Option<[f64; 4]>,
    ) -> (DefinitionEntry, bool) {
        // 规则 1：同一数据 → 已有条目。
        if let Some(existing) = self.lookup(source_key) {
            return (existing, true);
        }
        // 规则 2：内容完全一致 → 引用已有定义记录，不新建。
        let fp = def.fingerprint();
        let (def_id, def_arc, reused) = match self.by_fingerprint.get(&fp) {
            Some(id) => {
                let arc = self.defs.get(id).cloned().unwrap();
                (*id, arc, true)
            }
            None => {
                let id = self.next_def_id.max(1);
                self.next_def_id = id + 1;
                let mut d = def;
                d.id = id;
                let arc = Arc::new(d);
                self.defs.insert(id, Arc::clone(&arc));
                self.by_fingerprint.insert(fp, id);
                (id, arc, false)
            }
        };
        let entry_id = self.next_entry_id.max(1);
        self.next_entry_id = entry_id + 1;
        let entry = DefinitionEntry {
            def: def_arc,
            bounds,
            def_id,
        };
        self.by_source.insert(source_key.to_string(), entry_id);
        self.entries.insert(entry_id, entry.clone());
        (entry, reused)
    }

    /// 登记表的当前规模：(定义记录数, 登记的数据数)。
    pub fn stats(&self) -> (usize, usize) {
        (self.defs.len(), self.entries.len())
    }

    /// 全部定义记录（按编号升序），供管理面展示。
    pub fn all(&self) -> Vec<Arc<CacheDefinition>> {
        let mut v: Vec<Arc<CacheDefinition>> = self.defs.values().cloned().collect();
        v.sort_by_key(|d| d.id);
        v
    }

    /// 某条定义记录被多少个数据引用。
    pub fn ref_count(&self, id: u64) -> usize {
        self.entries.values().filter(|e| e.def_id == id).count()
    }
}

/// 便捷入口：登记一条定义记录。
pub fn acquire(
    source_key: &str,
    def: CacheDefinition,
    bounds: Option<[f64; 4]>,
) -> (DefinitionEntry, bool) {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.acquire(source_key, def, bounds)
}

/// 便捷入口：查某个数据源的定义记录条目。
pub fn lookup(source_key: &str) -> Option<DefinitionEntry> {
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.lookup(source_key)
}

// ── Conf.xml 解析（ArcGIS Server 缓存 / .tpk）────────────────────────────

impl CacheDefinition {
    /// 解析 `Conf.xml` 文本。
    pub fn from_xml(text: &str) -> Result<CacheDefinition, String> {
        let root = xmlscan::parse(text)?;
        let tci = root
            .child("TileCacheInfo")
            .ok_or("Conf.xml 缺少 TileCacheInfo")?;

        // 空间参考：TileCacheInfo 下常有内嵌 SpatialReference；WKID 有时只在
        // LatestWKID（高版本），因此两条都认。**LatestWKID 优先**：它是
        // 同一空间参考给出的现行官方码（如 Web Mercator 的 WKID=102100 已废弃、
        // LatestWKID=3857），旧产出里两值相同（本仓库样例 4490/4490），
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
        if wkid == 0 {
            return Err("Conf.xml 无法确定缓存 CRS（缺 WKID/LatestWKID/WKT AUTHORITY）".to_string());
        }
        let crs = format!("EPSG:{wkid}");

        let origin_el = tci
            .child("TileOrigin")
            .ok_or("Conf.xml 缺少 TileOrigin")?;
        let ox = origin_el
            .f64_of("X")
            .ok_or("Conf.xml TileOrigin 缺少 X")?;
        let oy = origin_el
            .f64_of("Y")
            .ok_or("Conf.xml TileOrigin 缺少 Y")?;

        let tile_cols = tci
            .u32_of("TileCols")
            .filter(|v| *v > 0)
            .ok_or("Conf.xml 缺少 TileCols")?;
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
            let resolution = el
                .f64_of("Resolution")
                .ok_or("LODInfo 缺少 Resolution")?;
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

        Ok(CacheDefinition {
            id: 0,
            kind: DefinitionKind::ConfXml,
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
            tile_root: "_alllayers".to_string(),
        })
    }

    /// 解析 `root.json`（`.tpkx`）文本，映射到与 `Conf.xml` 同一形状。
    ///
    /// 字段对应：
    ///
    /// * `tileInfo.spatialReference.wkid` → CRS；
    /// * `tileInfo.origin{x,y}` → `TileOrigin`（左上角）；
    /// * `tileInfo.cols` / `rows` → `TileCols` / `TileRows`；
    /// * `tileInfo.dpi` → DPI；
    /// * `tileInfo.lods[]{level,resolution,scale}` → LOD 阶梯；
    /// * `storageInfo.packetSize` → `PacketSize`；
    /// * `storageInfo.storageFormat` → `StorageFormat`；
    /// * `tileImageInfo.format` → 瓦片影像编码；
    /// * `tileBundlesPath`（`"./tile"`）→ 瓦片根目录名。
    pub fn from_root_json(text: &str) -> Result<CacheDefinition, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("root.json 解析失败：{e}"))?;

        let tile_info = v.get("tileInfo").ok_or("root.json 缺少 tileInfo")?;
        let sr = tile_info.get("spatialReference");
        let wkid = sr
            .and_then(|s| s.get("latestWkid").or_else(|| s.get("wkid")))
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as u32;
        if wkid == 0 {
            return Err("root.json 的 tileInfo.spatialReference 缺少 wkid".to_string());
        }
        let crs = format!("EPSG:{wkid}");

        let origin_el = tile_info.get("origin").ok_or("root.json 缺少 tileInfo.origin")?;
        let ox = origin_el
            .get("x")
            .and_then(|x| x.as_f64())
            .ok_or("root.json 的 tileInfo.origin 缺少 x")?;
        let oy = origin_el
            .get("y")
            .and_then(|x| x.as_f64())
            .ok_or("root.json 的 tileInfo.origin 缺少 y")?;

        let tile_cols = tile_info
            .get("cols")
            .and_then(|x| x.as_u64())
            .filter(|v| *v > 0)
            .ok_or("root.json 缺少 tileInfo.cols")? as u32;
        let tile_rows = tile_info
            .get("rows")
            .and_then(|x| x.as_u64())
            .filter(|v| *v > 0)
            .unwrap_or(tile_cols as u64) as u32;
        let dpi = tile_info
            .get("dpi")
            .and_then(|x| x.as_f64())
            .filter(|v| *v > 0.0)
            .unwrap_or(96.0);

        let lods_el = tile_info
            .get("lods")
            .and_then(|x| x.as_array())
            .ok_or("root.json 缺少 tileInfo.lods")?;
        let mut lods: Vec<LodInfo> = Vec::with_capacity(lods_el.len());
        for el in lods_el {
            let level = el
                .get("level")
                .and_then(|x| x.as_u64())
                .ok_or("root.json 的 lods 缺少 level")? as u32;
            let resolution = el
                .get("resolution")
                .and_then(|x| x.as_f64())
                .ok_or("root.json 的 lods 缺少 resolution")?;
            if resolution <= 0.0 {
                return Err(format!("root.json 的 lods level={level} 的 resolution 非正"));
            }
            let scale = el.get("scale").and_then(|x| x.as_f64()).unwrap_or(0.0);
            lods.push(LodInfo { level, scale, resolution });
        }
        if lods.is_empty() {
            return Err("root.json 的 tileInfo.lods 为空".to_string());
        }
        lods.sort_by(|a, b| a.level.cmp(&b.level));

        let storage = v.get("storageInfo");
        let packet_size = storage
            .and_then(|s| s.get("packetSize"))
            .and_then(|x| x.as_u64())
            .filter(|v| *v > 0)
            .unwrap_or(128) as u32;
        let storage_format = storage
            .and_then(|s| s.get("storageFormat"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let image_type = v
            .get("tileImageInfo")
            .and_then(|t| t.get("format"))
            .and_then(|x| x.as_str())
            .map(ImageType::parse)
            .unwrap_or(ImageType::Mixed);

        // 瓦片根目录：`tileBundlesPath`（"./tile"）→ 目录名 "tile"；只取末段，
        // 避免包里写了 ".." 之类的相对路径越出缓存目录。
        let tile_root = v
            .get("tileBundlesPath")
            .and_then(|x| x.as_str())
            .map(|s| {
                s.replace('\\', "/")
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("tile")
                    .to_string()
            })
            .filter(|s| !s.is_empty() && s != "." && s != "..")
            .unwrap_or_else(|| "tile".to_string());

        Ok(CacheDefinition {
            id: 0,
            kind: DefinitionKind::RootJson,
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
            bundle_version: None,
            tile_root,
        })
    }

    /// `root.json` 里的数据范围（`fullExtent` 优先，退回 `initialExtent`）。
    ///
    /// 与 `conf.cdi` 等价（同一 CRS 单位），但**不属于定义记录**：范围是数据的
    /// 属性，同一套瓦片方案可以装不同范围的数据。
    pub fn root_json_bounds(text: &str) -> Option<[f64; 4]> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        v.get("fullExtent")
            .or_else(|| v.get("initialExtent"))
            .and_then(|e| {
                let g = |k: &str| e.get(k).and_then(|x| x.as_f64());
                match (g("xmin"), g("ymin"), g("xmax"), g("ymax")) {
                    (Some(a), Some(b), Some(c), Some(d)) if c > a && d > b => {
                        Some([a, b, c, d])
                    }
                    _ => None,
                }
            })
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

/// 缓存目录里定位配置文件：优先 `conf.xml`，否则 `root.json`。
///
/// 返回 `(配置文件路径, 形态)`；找不到返回 `Err`。
pub fn find_config(dir: &Path) -> Result<(PathBuf, DefinitionKind), String> {
    if let Ok(p) = crate::arcgis_cache::conf::find_conf(dir) {
        return Ok((p, DefinitionKind::ConfXml));
    }
    if let Some(p) = crate::arcgis_cache::conf::find_sibling_file(dir, "root.json") {
        return Ok((p, DefinitionKind::RootJson));
    }
    Err(format!(
        "{} 下未找到 Conf.xml 或 root.json（缓存根目录或其一级图层子目录均需包含）",
        dir.display()
    ))
}

// ── FNV-1a（指纹用）─────────────────────────────────────────────────────

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_JSON: &str = r#"{
      "version": 1.0,
      "tileBundlesPath": "./tile",
      "storageInfo": {"packetSize": 128, "storageFormat": "esriMapCacheStorageModeCompactV2"},
      "tileImageInfo": {"format": "PNG"},
      "tileInfo": {
        "spatialReference": {"wkid": 4522, "latestWkid": 4522},
        "origin": {"x": 28876800.0, "y": 10002100.0},
        "rows": 256, "cols": 256, "dpi": 96,
        "lods": [
          {"level": 0, "resolution": 4.233341800016934, "scale": 16000},
          {"level": 1, "resolution": 2.116670900008467, "scale": 8000}
        ]
      },
      "fullExtent": {"xmin": 34585761.99, "ymin": 2750238.97, "xmax": 34587850.72, "ymax": 2751854.89}
    }"#;

    const CONF_XML: &str = r#"<CacheInfo><TileCacheInfo><SpatialReference><WKID>4490</WKID></SpatialReference>
      <TileOrigin><X>-180</X><Y>90</Y></TileOrigin><TileCols>256</TileCols><TileRows>256</TileRows><DPI>96</DPI>
      <LODInfos><LODInfo><LevelID>0</LevelID><Scale>295829355.454566</Scale><Resolution>0.70391441568404511</Resolution></LODInfo></LODInfos>
      </TileCacheInfo><TileImageInfo><CacheTileFormat>MIXED</CacheTileFormat></TileImageInfo>
      <CacheStorageInfo><StorageFormat>esriMapCacheStorageModeCompact</StorageFormat><PacketSize>128</PacketSize></CacheStorageInfo></CacheInfo>"#;

    #[test]
    fn root_json_maps_to_same_shape_as_conf_xml() {
        let d = CacheDefinition::from_root_json(ROOT_JSON).unwrap();
        assert_eq!(d.kind, DefinitionKind::RootJson);
        assert_eq!(d.crs, "EPSG:4522");
        assert_eq!(d.wkid, 4522);
        assert_eq!(d.origin, [28876800.0, 10002100.0]);
        assert_eq!((d.tile_cols, d.tile_rows), (256, 256));
        assert_eq!(d.dpi, 96.0);
        assert_eq!(d.packet_size, 128);
        assert_eq!(d.tile_root, "tile");
        assert_eq!(d.lods.len(), 2);
        assert_eq!(d.lods[0].level, 0);
        assert!((d.lods[0].resolution - 4.233341800016934).abs() < 1e-15);
        assert_eq!(d.image_type, ImageType::Png);
        assert_eq!(d.min_zoom(), 0);
        assert_eq!(d.max_zoom(), 1);
        // 范围与 conf.cdi 等价，但不属于定义记录
        let b = CacheDefinition::root_json_bounds(ROOT_JSON).unwrap();
        assert!((b[0] - 34585761.99).abs() < 1e-6);
    }

    #[test]
    fn conf_xml_definition_keeps_tile_root_alllayers() {
        let d = CacheDefinition::from_xml(CONF_XML).unwrap();
        assert_eq!(d.kind, DefinitionKind::ConfXml);
        assert_eq!(d.crs, "EPSG:4490");
        assert_eq!(d.tile_root, "_alllayers");
        assert_eq!(d.packet_size, 128);
        assert_eq!(d.lods.len(), 1);
    }

    /// PNG24 是 ArcGIS 的常见写法，必须识别为 PNG。
    #[test]
    fn png24_is_png() {
        assert_eq!(ImageType::parse("PNG24"), ImageType::Png);
        assert_eq!(ImageType::parse("png24"), ImageType::Png);
    }

    /// 指纹只覆盖网格方案：来源形态/声明影像格式/存储格式不同也不影响复用。
    #[test]
    fn fingerprint_ignores_source_shape_but_covers_grid() {
        let a = CacheDefinition::from_xml(CONF_XML).unwrap();
        let mut b = a.clone();
        b.kind = DefinitionKind::RootJson;
        b.image_type = ImageType::Png;
        b.storage_format = "esriMapCacheStorageModeCompactV2".to_string();
        assert_eq!(a.fingerprint(), b.fingerprint());

        let mut c = a.clone();
        c.origin[0] += 1.0;
        assert_ne!(a.fingerprint(), c.fingerprint());
        let mut d = a.clone();
        d.packet_size = 256;
        assert_ne!(a.fingerprint(), d.fingerprint());
    }

    /// 内容一致的第二个数据源应**引用**已有记录，不新建。
    #[test]
    fn registry_reuses_identical_definitions() {
        let mut reg = DefinitionRegistry::default();
        let base = CacheDefinition::from_xml(CONF_XML).unwrap();
        let (first, reused) = reg.acquire("src-A", base.clone(), Some([1.0, 2.0, 3.0, 4.0]));
        assert!(!reused);
        assert_eq!(first.def.id, 1);

        let (second, reused2) = reg.acquire("src-B", base.clone(), Some([5.0, 6.0, 7.0, 8.0]));
        assert!(reused2, "内容一致应复用");
        assert_eq!(second.def.id, first.def.id, "应引用同一条定义记录");
        assert_eq!(second.bounds, Some([5.0, 6.0, 7.0, 8.0]), "范围仍按数据各自保留");
        assert_eq!(reg.stats(), (1, 2), "1 条定义记录被 2 个数据引用");
        assert_eq!(reg.ref_count(first.def.id), 2);

        // 内容不同 → 新建
        let mut other = base.clone();
        other.origin[1] -= 7.0;
        let (third, reused3) = reg.acquire("src-C", other, None);
        assert!(!reused3);
        assert_ne!(third.def.id, first.def.id);
        assert_eq!(reg.stats(), (2, 3));

        // 同一数据源重复打开 → 直接拿回
        let (again, reused4) = reg.acquire("src-A", base, None);
        assert!(reused4);
        assert_eq!(again.def.id, first.def.id);
    }

    /// 定义记录的来源形态不影响登记：root.json 与 conf.xml 描述**同一套网格**
    /// 时 → 引用同一条记录。
    #[test]
    fn registry_shares_record_across_config_shapes() {
        let mut reg = DefinitionRegistry::default();
        let mut x = CacheDefinition::from_root_json(ROOT_JSON).unwrap();
        x.tile_root = "_alllayers".to_string();
        let (a, _) = reg.acquire("json-src", x, None);

        let mut y = CacheDefinition::from_xml(CONF_XML).unwrap();
        // 造一份与 x 网格完全一致、但形态是 conf.xml 的记录。
        y.crs = "EPSG:4522".to_string();
        y.wkid = 4522;
        y.origin = [28876800.0, 10002100.0];
        y.lods = vec![
            LodInfo { level: 0, scale: 16000.0, resolution: 4.233341800016934 },
            LodInfo { level: 1, scale: 8000.0, resolution: 2.116670900008467 },
        ];
        y.tile_root = "_alllayers".to_string();
        let (b, reused) = reg.acquire("xml-src", y, None);
        assert!(reused, "同一套网格应复用同一条定义记录");
        assert_eq!(a.def.id, b.def.id);
        assert_eq!(reg.stats(), (1, 2));
    }

    /// CRS 属于网格方案的一部分：坐标系不同的两份配置**不得**复用同一条记录。
    #[test]
    fn different_crs_never_shares_a_record() {
        let mut reg = DefinitionRegistry::default();
        let mut x = CacheDefinition::from_root_json(ROOT_JSON).unwrap();
        x.tile_root = "_alllayers".to_string();
        let (a, _) = reg.acquire("json-src", x, None);

        let mut y = CacheDefinition::from_xml(CONF_XML).unwrap();
        y.origin = [28876800.0, 10002100.0];
        y.lods = vec![
            LodInfo { level: 0, scale: 16000.0, resolution: 4.233341800016934 },
            LodInfo { level: 1, scale: 8000.0, resolution: 2.116670900008467 },
        ];
        y.tile_root = "_alllayers".to_string();
        let (b, reused) = reg.acquire("xml-src", y, None);
        assert!(!reused, "CRS 不同不得复用");
        assert_ne!(a.def.id, b.def.id);
    }
}
