//! 紧凑缓存（compact cache）v1/v2 的读-through 解析。
//!
//! # 目录与寻址
//!
//! ```text
//! _alllayers/L{zz}/R{rrrr}C{cccc}.bundle      数据 +（v2）内嵌索引
//! _alllayers/L{zz}/R{rrrr}C{cccc}.bundlx      （v1）独立索引
//! ```
//!
//! `zz` 是两位十进制层级，`rrrr`/`cccc` 是**该 bundle 基址的十六进制**（至少 4 位），
//! 基址 = `floor(行列 / PacketSize) * PacketSize`（PacketSize 通常 128）。瓦片在一个
//! bundle 内的矩阵槽位号取决于版本：
//!
//! * **v1（ArcGIS 10.0–10.2）**：`.bundlx` = 16 字节头 + 128×128 个 **5 字节偏移** +
//!   16 字节尾；槽位号 `idx = PacketSize * (col % PacketSize) + (row % PacketSize)`
//!   （**列优先**）。偏移指向 `.bundle` 内的 4 字节 size 字，影像数据在 `偏移 + 4`；
//!   size = 0 即该瓦片不存在。
//! * **v2（ArcGIS 10.3+）**：无 `.bundlx`，索引内嵌在 `.bundle` 的 64 字节头之后，
//!   每槽 **8 字节**（前 5 字节偏移 + 后 3 字节大小），槽位号 `idx =
//!   PacketSize * (row % PacketSize) + (col % PacketSize)`（**行优先**，与 v1 相反）。
//! * **WTC v1（gis-wt 自有，2026-11）**：`magic = "GISWTC01"` 开头，64B 自描述头 +
//!   `tile_count × 16B` 稀疏索引（只存有效瓦片，按 `(row_in,col_in)` 升序，二分查找），
//!   瓦片 `[4B size][WebP]`。不再是 ArcGIS 格式变体，详见 gis-wt `docs/12-WebP缓存格式规范.md`。
//!   WTC 的 `offset`/`size` 语义与 v2 一致（offset 指向图像数据首字节），故读瓦片时共用
//!   v2 分支；区别只在索引如何解析。扩展名沿用 `.webpdle`，靠 magic 分派。
//!
//! v1 的列优先 / v2 的行优先是这类读取最容易写错、且错了**不报错**（只会整体镜像
//! 错位）的一处，因此 [`tests`] 用两种顺序分别解出矩阵，再与 conf.cdi 的真实
//! 范围比对判定。
//!
//! # 行列朝向
//!
//! 缓存 row 自 `TileOrigin`（左上角）向南递增，与本仓库
//! [`crate::wms_common::tile::TileMatrixSet`] 的 WMTS/左上约定一致，无需像
//! MBTiles（TMS 行）那样翻转。

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::arcgis_cache::conf::{CacheDocs, ImageType};
use crate::arcgis_cache::grid::{self, GridSpec};

/// bundle 头长度（v1 的 `.bundlx`、v2 的 `.bundle` 各取自身格式的头）。
const HDR_V1_BUNDLX: u64 = 16;
const HDR_V2_BUNDLE: u64 = 64;
/// WTC v1 自描述头长度与 magic。
const HDR_WTC: u64 = 64;
const WTC_MAGIC: &[u8; 8] = b"GISWTC01";
/// 一个 bundle 最多容纳的瓦片数上限（PacketSize² 的实际取值就是 128²=16384）。
const MAX_PACKET: u32 = 256;
/// 索引表缓存的 bundle 个数上限：一张表 16384×(8B) ≈ 128 KB，64 张 ≈ 8 MB，
/// 覆盖一次平移/缩放扫过的 bundle 邻域，避免每瓦片都随机读索引。
const TABLE_CACHE_CAP: usize = 64;

/// 缓存格式版本。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheVersion {
    /// ArcGIS 10.0–10.2：`.bundlx` + `.bundle`。
    V1,
    /// ArcGIS 10.3+：索引内嵌 `.bundle`。
    V2,
    /// gis-wt WTC v1：magic `GISWTC01`，自描述头 + 16B 稀疏索引（扩展名沿用 `.webpdle`）。
    Wtc,
}

/// 一片瓦片的读取结果。
pub struct TileData {
    pub bytes: Vec<u8>,
    /// 响应 Content-Type（按影像魔数嗅探，MIXED 缓存里 PNG/JPEG 共存）。
    pub content_type: &'static str,
}

/// `_alllayers` 目录形态探测结果。
#[derive(Clone, Debug, Default)]
pub struct CacheProbe {
    /// 实际存在的层级（`Lxx` 目录名解析而来，可能少于 Conf.xml 的 LOD 数）。
    pub levels: Vec<u32>,
    /// 最细层级上观测到的最大列/行（全局索引，非 bundle 基址）。
    pub max_col: u32,
    pub max_row: u32,
    /// 观测到的层级跨度（= 可服务的 min/max zoom）。
    pub min_zoom: u32,
    pub max_zoom: u32,
}

/// 一个可读取的 ArcGIS 切片缓存。
pub struct ArcgisCache {
    docs: CacheDocs,
    allayers: PathBuf,
    /// 打开时传入的源路径（缓存目录或 `.tpk`）。`.tpk` 的解压目录在运行期被
    /// 删掉时，[ArcgisCache::relocate] 会按它重新解压——解压目录名由源指纹派生，
    /// 重解出来的路径与 `allayers` 完全一致，因此无需换指针。
    src: PathBuf,
    version: CacheVersion,
    probe: CacheProbe,
    /// bundle 索引表缓存（键：`(z, 基址行, 基址列)`）。
    tables: Mutex<TableCache>,
    /// 重解串行锁：并发瓦片请求里只有一个真去解压，其余等它做完直接复用。
    rescue: Mutex<()>,
    /// 上次**失败**的重解毫秒时间戳（源包已不在时别每请求重试，见 [`relocate`]）。
    rescue_at: AtomicU64,
}

/// 一个 bundle 的槽位表：`idx -> (偏移, 大小)`；v1 的大小在 `.bundle` 里，故为 0
/// 占位，读取时再取。
struct Table {
    entries: Vec<(u64, u32)>,
    bundle: Arc<PathBuf>,
}

#[derive(Default)]
struct TableCache {
    map: HashMap<(u32, u32, u32), Arc<Table>>,
    order: Vec<(u32, u32, u32)>,
}

impl TableCache {
    fn get(&self, k: &(u32, u32, u32)) -> Option<Arc<Table>> {
        self.map.get(k).map(Arc::clone)
    }

    fn put(&mut self, k: (u32, u32, u32), t: Arc<Table>) {
        if self.map.len() >= TABLE_CACHE_CAP {
            // FIFO：ArcGIS 缓存的访问局部性是空间邻域，插入序足够。
            if let Some(old) = self.order.first().cloned() {
                self.order.remove(0);
                self.map.remove(&old);
            }
        }
        if !self.map.contains_key(&k) {
            self.order.push(k);
        }
        self.map.insert(k, t);
    }
}

impl ArcgisCache {
    /// 打开缓存：`dir` 为缓存服务目录（或其上层，含一级图层子目录）。
    pub fn open(dir: &Path) -> Result<ArcgisCache, String> {
        let docs = CacheDocs::load(dir)?;
        check_packet(docs.info.packet_size)?;
        let allayers = find_alllayers(&docs.cache_dir)?;
        let version = detect_version(&docs, &allayers)?;
        let probe = probe_alllayers(&allayers);
        Ok(ArcgisCache {
            src: dir.to_path_buf(),
            docs,
            allayers,
            version,
            probe,
            tables: Mutex::new(TableCache::default()),
            rescue: Mutex::new(()),
            rescue_at: AtomicU64::new(0),
        })
    }

    /// 缓存目录整体消失时的自愈：`.tpk` 源重新解压（幂等，落点与原来同名）。
    ///
    /// 只在「本次请求的 bundle 不存在」且「`_alllayers` 目录本身也没了」时调用。
    /// 并发下靠 `rescue` 串行：一个线程解压（约 50–70 ms / 60 MB），其余等它完成后
    /// 发现目录已恢复就直接复用，不会有一批请求因为节流而 404——浏览器收到 404 的
    /// 图片不会自动重试，宁可让这几路请求多等几十毫秒。返回 true = 缓存已在位，
    /// 调用方可重试一次。
    fn relocate(&self) -> bool {
        /// 重解失败后的重试间隔：源 `.tpk` 本身被删/不可读时，不必每个瓦片请求都再试。
        const GAP_MS: u64 = 1_000;
        if !crate::arcgis_cache::tpk::is_tpk(&self.src) {
            return false;
        }
        let Ok(_guard) = self.rescue.lock() else {
            return false;
        };
        if self.allayers.is_dir() {
            // 已有别的线程解完（或本就只是这个 bundle 缺失）：无需重解。
            return true;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if now.saturating_sub(self.rescue_at.load(Ordering::Relaxed)) < GAP_MS {
            return false;
        }
        match crate::arcgis_cache::tpk::ensure_dir(&self.src) {
            Ok(_) => true,
            Err(_) => {
                self.rescue_at.store(now, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn info(&self) -> &crate::arcgis_cache::conf::CacheInfo {
        &self.docs.info
    }

    pub fn version(&self) -> CacheVersion {
        self.version
    }

    pub fn probe(&self) -> &CacheProbe {
        &self.probe
    }

    /// 实际取瓦片的 `_alllayers` 目录。`.tpk` 源时它位于解压目录内部，
    /// 供调用方（体检、自愈测试）判断缓存是否还在。
    pub fn allayers_dir(&self) -> &Path {
        &self.allayers
    }

    /// 数据范围（缓存 CRS 单位）：`conf.cdi` 优先，缺失时按 bundle 探测回推。
    pub fn bounds(&self) -> Option<[f64; 4]> {
        if let Some(cdi) = &self.docs.cdi {
            return Some(cdi.bounds);
        }
        let info = self.info();
        let z = self.probe.max_zoom;
        let i = info.lods.iter().position(|l| l.level == z)?;
        let span = info.lods[i].resolution * info.tile_px() as f64;
        Some([
            info.origin[0],
            info.origin[1] - (self.probe.max_row + 1) as f64 * span,
            info.origin[0] + (self.probe.max_col + 1) as f64 * span,
            info.origin[1],
        ])
    }

    /// Conf.xml 的 LOD 阶梯 → custom grid（层级比例尺一并携带）。
    pub fn grid_spec(&self, id: &str) -> Result<GridSpec, String> {
        let info = self.info();
        match self.bounds() {
            Some(b) => grid::derive(info, id, Some(b)),
            None => grid::derive_with_probe(
                info,
                id,
                None,
                Some((self.probe.max_zoom, self.probe.max_col, self.probe.max_row)),
            ),
        }
    }

    /// 一个 bundle 在磁盘上的基名（不含扩展名）。
    fn bundle_base(&self, z: u32, col: u32, row: u32) -> PathBuf {
        let packet = self.info().packet_size;
        let base_col = (col / packet) * packet;
        let base_row = (row / packet) * packet;
        self.allayers
            .join(format!("L{z:02}"))
            .join(format!("R{base_row:04x}C{base_col:04x}"))
    }

    /// 读取 `(z, col, row)` 处的瓦片；`col/row` 自左上角起算。
    ///
    /// `Ok(None)` = 该层级未切、bundle 不存在或槽位为空（缓存里的空白区），
    /// 由调用方决定 404 还是空白图兜底。
    pub fn read_tile(&self, z: u32, col: u32, row: u32) -> Result<Option<TileData>, String> {
        let info = self.info();
        if !info.lods.iter().any(|l| l.level == z) {
            return Ok(None);
        }
        if !self.probe.levels.contains(&z) {
            return Ok(None);
        }
        let packet = info.packet_size;
        let base = self.bundle_base(z, col, row);
        let mut absent = sidecar(&base, "bundle").is_err();
        // 缓存区外的 (col,row) 连 bundle 都不存在：这类请求占比不低
        //（浏览器/客户端会沿数据矩形外多取一圈），必须返回空瓦片而不是
        // 把请求打成 500。
        if absent && !self.allayers.is_dir() && self.relocate() {
            // 整个缓存目录都没了（如 `.tpk` 解压目录被删）：重解一次再试，
            // 否则这个图层会永久 404，只有重启进程才能恢复。
            absent = sidecar(&base, "bundle").is_err();
        }
        if absent {
            return Ok(None);
        }
        let key = (
            z,
            (row / packet) * packet,
            (col / packet) * packet,
        );
        let table = self.table(key, &base)?;
        let idx = slot(packet, col % packet, row % packet, self.version);
        let (off, size_v2) = table
            .entries
            .get(idx as usize)
            .copied()
            .ok_or_else(|| format!("bundle 索引槽 {idx} 越界（PacketSize={packet}）"))?;
        if off == 0 {
            return Ok(None);
        }
        let (payload_off, size) = match self.version {
            // v1：偏移指向 4 字节 size 字，影像紧随其后。
            CacheVersion::V1 => {
                let size = read_u32_le(&table.bundle, off)?;
                if size == 0 {
                    return Ok(None);
                }
                (off + 4, size)
            }
            // v2 / WTC v1：大小就在索引槽里，0 = 空槽；offset 指向图像数据首字节。
            // WTC 的稀疏索引在 table() 里已展开成同样的 (off, size) 密集槽，共用此分支。
            CacheVersion::V2 | CacheVersion::Wtc if size_v2 == 0 => return Ok(None),
            CacheVersion::V2 | CacheVersion::Wtc => (off, size_v2),
        };
        if size > 64 * 1024 * 1024 {
            return Err(format!(
                "{}: 槽位声明的瓦片大小 {size} 超过 64 MiB，索引疑似损坏（层级 {z} 列 {col} 行 {row}）",
                table.bundle.display()
            ));
        }
        let bytes = read_exact_at(&table.bundle, payload_off, size as usize)?;
        Ok(Some(TileData {
            content_type: sniff(&bytes, info.image_type),
            bytes,
        }))
    }

    /// 取（或建）一个 bundle 的索引表。
    fn table(&self, key: (u32, u32, u32), base: &Path) -> Result<Arc<Table>, String> {
        if let Some(t) = self.tables.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Ok(t);
        }
        let packet = self.info().packet_size as usize;
        let n = packet * packet;
        // 两种版本的差异只体现在「索引从哪读、每槽几字节」；数据文件都是 .bundle，
        // 故统一在这里得到 (entries, bundle)，再一起放进缓存，避免某一版绕过缓存。
        let (entries, bundle) = match self.version {
            CacheVersion::V1 => {
                let bx = sidecar(base, "bundlx")?;
                let bytes = std::fs::read(&bx).map_err(|e| format!("读 {}: {e}", bx.display()))?;
                let need = (HDR_V1_BUNDLX as usize) + n * 5;
                if bytes.len() < need {
                    return Err(format!(
                        "{}: 长度 {} 小于 v1 索引所需 {need}（PacketSize={packet}）",
                        bx.display(),
                        bytes.len()
                    ));
                }
                // 5 字节小端偏移；v1 的大小在 .bundle 里，占位 0。
                let entries = (0..n)
                    .map(|i| {
                        let p = HDR_V1_BUNDLX as usize + i * 5;
                        (le_u64(&bytes[p..p + 5]), 0)
                    })
                    .collect();
                (entries, Arc::new(sidecar(base, "bundle")?))
            }
            CacheVersion::V2 => {
                let bundle = sidecar(base, "bundle")?;
                let mut f =
                    std::fs::File::open(&bundle).map_err(|e| format!("开 {bundle:?}: {e}"))?;
                f.seek(SeekFrom::Start(HDR_V2_BUNDLE))
                    .map_err(|e| format!("seek {}: {e}", bundle.display()))?;
                // 尾部可能不足一整张表（缓存未切满），读到多少算多少，缺槽视为空。
                let mut buf = vec![0u8; n * 8];
                let got = read_fill(&mut f, &mut buf)?;
                let entries = buf[..got]
                    .chunks_exact(8)
                    .map(|c| (le_u64(&c[..5]), le_u64(&c[5..8]) as u32))
                    .collect();
                (entries, Arc::new(bundle))
            }
            CacheVersion::Wtc => {
                // WTC v1：magic + 64B 自描述头 + tile_count×16B 稀疏索引。
                // 把稀疏索引展开成 packet² 密集槽（与 V2 同构），缺槽 (0,0)。
                let bundle = sidecar(base, "bundle")?;
                let bytes = std::fs::read(&bundle)
                    .map_err(|e| format!("读 {}: {e}", bundle.display()))?;
                if bytes.len() < HDR_WTC as usize {
                    return Err(format!("{} 长度 {} < WTC 头 64B", bundle.display(), bytes.len()));
                }
                if &bytes[..8] != WTC_MAGIC {
                    return Err(format!("{} 非 WTC 格式（magic={:?}）", bundle.display(), &bytes[..8]));
                }
                let wpacket = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
                let tile_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
                let entry_size = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
                let index_off = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
                if wpacket == 0 || wpacket > MAX_PACKET as usize {
                    return Err(format!("{} WTC packet={wpacket} 越界", bundle.display()));
                }
                if entry_size != 16 {
                    return Err(format!("{} WTC 索引条目大小 {entry_size} != 16", bundle.display()));
                }
                let need = index_off.saturating_add(tile_count * 16);
                if bytes.len() < need {
                    return Err(format!("{} WTC 索引截断：需 {need}B，实际 {}B", bundle.display(), bytes.len()));
                }
                let wn = wpacket * wpacket;
                let mut dense = vec![(0u64, 0u32); wn];
                for i in 0..tile_count {
                    let p = index_off + i * 16;
                    let col_in = u16::from_le_bytes(bytes[p..p + 2].try_into().unwrap()) as usize;
                    let row_in = u16::from_le_bytes(bytes[p + 2..p + 4].try_into().unwrap()) as usize;
                    let size = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap());
                    let offset = u64::from_le_bytes(bytes[p + 8..p + 16].try_into().unwrap());
                    if col_in >= wpacket || row_in >= wpacket {
                        return Err(format!("{} WTC 索引 {i} 的 (col,row)=({col_in},{row_in}) 越界", bundle.display()));
                    }
                    // 行优先，与 slot(V2) 一致；wpacket 应与 Conf.xml packet 相同。
                    let idx = wpacket * row_in + col_in;
                    if idx < dense.len() {
                        dense[idx] = (offset, size);
                    }
                }
                (dense, Arc::new(bundle))
            }
        };
        let t = Arc::new(Table { entries, bundle });
        self.tables
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, Arc::clone(&t));
        Ok(t)
    }
}

/// 槽位号：v1 列优先、v2 / WTC 行优先（见模块文档）。
fn slot(packet: u32, col_in: u32, row_in: u32, version: CacheVersion) -> usize {
    match version {
        CacheVersion::V1 => (packet * col_in + row_in) as usize,
        CacheVersion::V2 | CacheVersion::Wtc => (packet * row_in + col_in) as usize,
    }
}

/// 把 `<base>` 换成同目录的 `<stem>.<ext>`，并按磁盘实际大小写解析。
///
/// gis-wt 定制：当 `ext == "bundle"` 但标准 `.bundle` 不存在时，回退查找
/// `.webpdle`（Compact Cache V2 结构 + WebP 瓦片，扩展名定制）。这样一份
/// 缓存里两种扩展名可共存，读端对老格式行为完全不变。
fn sidecar(base: &Path, ext: &str) -> Result<PathBuf, String> {
    // 依次尝试的扩展名：标准扩展优先，webpdle 仅作为 bundle 的回退。
    let exts: &[&str] = if ext == "bundle" {
        &["bundle", "webpdle"]
    } else {
        &[ext]
    };
    for &e in exts {
        let candidate = format!("{}.{}", base.display(), e);
        let p = Path::new(&candidate);
        if p.exists() {
            return Ok(actual_case(p));
        }
        // bundle 基名里的十六进制大小写可能不一致。
        if let Some(dir) = base.parent() {
            if let Some(stem) = base.file_name().and_then(|s| s.to_str()) {
                let want = format!("{stem}.{e}").to_ascii_lowercase();
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let np = entry.path();
                        if np.is_file()
                            && np.file_name()
                                .map(|n| n.to_string_lossy().to_ascii_lowercase() == want)
                                .unwrap_or(false)
                        {
                            return Ok(np);
                        }
                    }
                }
            }
        }
    }
    Err(format!(
        "{} 不存在（已尝试 {}）",
        base.display(),
        exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(" / ")
    ))
}

/// Windows/macOS 大小写不敏感文件系统上，把路径换成磁盘实际拼写（错误信息里
/// 给运维看的应当是真实名字）。
fn actual_case(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// 在缓存根目录（`Conf.xml` 所在目录，即 [`crate::arcgis_cache::conf::find_conf` 的父目录）
/// 里按不区分大小写的名字找 `_alllayers`；管理面体检（map-api `admin::check`）
/// 与打开路径共用同一套定位规则，避免两侧判定不一致。
pub fn find_alllayers(cache_dir: &Path) -> Result<PathBuf, String> {
    let want = "_alllayers";
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir()
                && p.file_name()
                    .map(|n| n.to_string_lossy().eq_ignore_ascii_case(want))
                    .unwrap_or(false)
            {
                return Ok(p);
            }
        }
    }
    Err(format!("{} 下未找到 _alllayers 目录", cache_dir.display()))
}

/// 版本判定：优先 Conf.xml 的显式 `BundleVersion`，否则看 `.bundlx` 是否存在，
/// 再否则看首个 `.bundle` 的头（v2 把 16384×8B 索引嵌在 64B 头之后）。
fn detect_version(docs: &CacheDocs, allayers: &Path) -> Result<CacheVersion, String> {
    // WTC v1 优先：magic 是格式自描述的权威判据，必须先看文件头，
    // 否则 Conf.xml 里若带 BundleVersion=2 会把 WTC 误判成 V2。
    for z in docs.info.min_zoom()..=docs.info.max_zoom() {
        let dir = allayers.join(format!("L{z:02}"));
        if !dir.is_dir() { continue; }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if name.ends_with(".webpdle") || name.ends_with(".bundle") {
                let mut head = [0u8; 8];
                if let Ok(mut f) = std::fs::File::open(&p) {
                    use std::io::Read;
                    if f.read_exact(&mut head).is_ok() && &head == WTC_MAGIC {
                        return Ok(CacheVersion::Wtc);
                    }
                }
            }
        }
    }
    if let Some(v) = docs.info.bundle_version {
        return match v {
            1 | 0 => Ok(CacheVersion::V1),
            _ => Ok(CacheVersion::V2),
        };
    }
    let packet = docs.info.packet_size;
    let expect_v1 = (HDR_V1_BUNDLX as usize) + (packet * packet * 5) as usize + 16;
    let mut saw_bundle = false;
    for z in docs.info.min_zoom()..=docs.info.max_zoom() {
        let dir = allayers.join(format!("L{z:02}"));
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if name.ends_with(".bundlx") {
                let len = p.metadata().map(|m| m.len() as usize).unwrap_or(0);
                if len == expect_v1 {
                    return Ok(CacheVersion::V1);
                }
            }
            if name.ends_with(".bundle") || name.ends_with(".webpdle") {
                saw_bundle = true;
            }
        }
    }
    if saw_bundle {
        Ok(CacheVersion::V2)
    } else {
        Err(format!("{} 里既无 .bundle/.webpdle 也无 .bundlx，不是紧凑缓存", allayers.display()))
    }
}

/// 扫 `_alllayers`：层级清单 + 最细层级上的最大列/行（用于范围回推与探活）。
fn probe_alllayers(allayers: &Path) -> CacheProbe {
    let mut probe = CacheProbe::default();
    let Ok(entries) = std::fs::read_dir(allayers) else {
        return probe;
    };
    let mut levels: Vec<u32> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Some(rest) = name.strip_prefix('L').or_else(|| name.strip_prefix('l')) {
            if let Ok(z) = rest.parse::<u32>() {
                levels.push(z);
            }
        }
    }
    levels.sort_unstable();
    levels.dedup();
    probe.levels = levels.clone();
    probe.min_zoom = levels.first().copied().unwrap_or(0);
    probe.max_zoom = levels.last().copied().unwrap_or(0);
    // 最细层级上扫 bundle 名，取最大基址 + PacketSize 作为观测上界。
    let dir = allayers.join(format!("L{:02}", probe.max_zoom));
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if let Some((r, c)) = parse_bundle_name(&name) {
                probe.max_col = probe.max_col.max(c);
                probe.max_row = probe.max_row.max(r);
            }
        }
    }
    probe
}

/// `r0100c0600.bundle` / `R0100C0600.bundlx` → `(基址列, 基址行)`（十进制）。
fn parse_bundle_name(lower: &str) -> Option<(u32, u32)> {
    let stem = lower
        .strip_suffix(".bundle")
        .or_else(|| lower.strip_suffix(".webpdle"))
        .or_else(|| lower.strip_suffix(".bundlx"))?;
    let pos = stem.find('c')?;
    let (rs, cs) = (&stem[1..pos], &stem[pos + 1..]);
    let row = u32::from_str_radix(rs, 16).ok()?;
    let col = u32::from_str_radix(cs, 16).ok()?;
    Some((col, row))
}

fn le_u64(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, &byte) in b.iter().enumerate().take(8) {
        v |= (byte as u64) << (8 * i);
    }
    v
}

fn read_u32_le(path: &Path, at: u64) -> Result<u32, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("开 {}: {e}", path.display()))?;
    f.seek(SeekFrom::Start(at)).map_err(|e| format!("seek {}: {e}", path.display()))?;
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).map_err(|e| format!("读 size {}: {e}", path.display()))?;
    Ok(u32::from_le_bytes(buf))
}

fn read_exact_at(path: &Path, at: u64, len: usize) -> Result<Vec<u8>, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("开 {}: {e}", path.display()))?;
    f.seek(SeekFrom::Start(at)).map_err(|e| format!("seek {}: {e}", path.display()))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|e| format!("读瓦片 {}@{at}: {e}", path.display()))?;
    Ok(buf)
}

/// 尽量填满 `buf`，返回实际读到的字节数（尾部不完整时不视为错误）。
fn read_fill<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize, String> {
    let mut got = 0usize;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(got)
}

/// 影像 Content-Type：MIXED 缓存里 PNG/JPEG 共存，只能按魔数嗅探。
fn sniff(bytes: &[u8], declared: ImageType) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return "image/png";
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return "image/jpeg";
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return "image/webp";
    }
    match declared {
        ImageType::Png => "image/png",
        ImageType::Jpeg => "image/jpeg",
        ImageType::Webp => "image/webp",
        // MIXED / Other 且魔数不认：按二进制透传，交由浏览器处理。
        _ => "application/octet-stream",
    }
}

/// PacketSize 合理性上限（防止 Conf.xml 异常值把索引表撑爆）。
pub fn check_packet(packet: u32) -> Result<(), String> {
    if packet == 0 || packet > MAX_PACKET {
        return Err(format!("PacketSize={packet} 不在 1..={MAX_PACKET} 范围内"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小 v1 索引：只在给定 (col_in,row_in) 槽位写偏移，其余全 0。
    fn v1_bundlx(packet: u32, slots: &[(u32, u32, u64)]) -> Vec<u8> {
        let n = (packet * packet) as usize;
        let mut x = vec![0u8; 16 + n * 5 + 16];
        for (col_in, row_in, off) in slots {
            let idx = (packet * col_in + row_in) as usize;
            let p = 16 + idx * 5;
            x[p..p + 5].copy_from_slice(&off.to_le_bytes()[..5]);
        }
        x
    }

    /// 造 v1 bundle：16B 头 + 每个槽位处 size + 数据；`sizes` 给出各偏移处的 size。
    fn v1_bundle(tiles: &[(u64, &[u8])]) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        for (off, payload) in tiles {
            let off = *off as usize;
            if b.len() < off + 4 {
                b.resize(off + 4, 0);
            }
            b[off..off + 4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            if b.len() < off + 4 + payload.len() {
                b.resize(off + 4 + payload.len(), 0);
            }
            b[off + 4..off + 4 + payload.len()].copy_from_slice(payload);
        }
        b
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];
    const JPG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 9, 9];

    #[test]
    fn slot_order_differs_between_versions() {
        // v1 列优先 / v2 行优先：同一个 (col_in,row_in) 必须落在不同槽位上，
        // 这是两种格式最容易被写反的地方。
        assert_eq!(slot(128, 1, 2, CacheVersion::V1), 128 * 1 + 2);
        assert_eq!(slot(128, 1, 2, CacheVersion::V2), 128 * 2 + 1);
    }

    #[test]
    fn parses_bundle_name_hex_bases() {
        assert_eq!(parse_bundle_name("r0100c0600.bundle"), Some((0x0600, 0x0100)));
        assert_eq!(parse_bundle_name("R0000C0000.BUNDLX".to_ascii_lowercase().as_str()), Some((0, 0)));
        assert_eq!(parse_bundle_name("r00ffc0004.bundle"), Some((4, 0x00ff)));
        assert_eq!(parse_bundle_name("rxyzc0000.bundle"), None);
        assert_eq!(parse_bundle_name("conf.xml"), None);
    }

    #[test]
    fn sniffs_mixed_payloads() {
        assert_eq!(sniff(PNG, ImageType::Mixed), "image/png");
        assert_eq!(sniff(JPG, ImageType::Mixed), "image/jpeg");
        assert_eq!(sniff(&[], ImageType::Png), "image/png");
        assert_eq!(sniff(&[1, 2, 3], ImageType::Other), "application/octet-stream");
        // gis-wt 定制：WebP 魔数优先于声明类型，WEBPDLE 声明也能识别。
        let webp = b"RIFF\x1a\x00\x00\x00WEBPVP8 ";
        assert_eq!(sniff(webp, ImageType::Webp), "image/webp");
        assert_eq!(sniff(webp, ImageType::Other), "image/webp");
    }

    #[test]
    fn v1_read_through_finds_slotted_tiles() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("svc");
        let l0 = cache.join("_alllayers").join("L00");
        std::fs::create_dir_all(&l0).unwrap();
        std::fs::write(
            cache.join("Conf.xml"),
            tiny_conf(128, "MIXED", &[0.703125, 0.3515625]),
        )
        .unwrap();
        std::fs::write(cache.join("conf.cdi"), "<EnvelopeN><XMin>100.0</XMin><YMin>20.0</YMin><XMax>101.0</XMax><YMax>21.0</YMax></EnvelopeN>")
            .unwrap();
        // 槽位：col_in=3,row_in=5 -> off 60；col_in=3,row_in=6 -> off 60+4+len(PNG)
        let off_a = 64u64;
        let off_b = off_a + 4 + PNG.len() as u64;
        std::fs::write(
            l0.join("R0000C0000.bundlx"),
            v1_bundlx(128, &[(3, 5, off_a), (3, 6, off_b)]),
        )
        .unwrap();
        std::fs::write(l0.join("R0000C0000.bundle"), v1_bundle(&[(off_a, JPG), (off_b, PNG)]))
            .unwrap();

        let ac = ArcgisCache::open(&cache).unwrap();
        assert_eq!(ac.version(), CacheVersion::V1);
        assert_eq!(ac.probe().levels, vec![0]);
        // 命中：全局 (col=3,row=5)
        let t = ac.read_tile(0, 3, 5).unwrap().expect("槽位应有瓦片");
        assert_eq!(t.content_type, "image/jpeg");
        assert_eq!(&t.bytes[..3], &[0xff, 0xd8, 0xff]);
        // 空槽：Conf 声明 2 级但只切了 L00
        assert!(ac.read_tile(1, 0, 0).unwrap().is_none());
        assert!(ac.read_tile(0, 4, 5).unwrap().is_none());
        // 缓存区外：bundle 文件根本不存在，也不得当成错误向上抛
        assert!(ac.read_tile(0, 999, 999).unwrap().is_none());
        // 索引表被缓存复用（第二次不再读 .bundlx）
        assert!(ac.read_tile(0, 3, 6).unwrap().is_some());
        let cached = ac.tables.lock().unwrap().map.len();
        assert_eq!(cached, 1, "同 bundle 的多次取瓦只建一张索引表");
        // 范围来自 cdi，grid 由 LOD 推导
        assert_eq!(ac.bounds(), Some([100.0, 20.0, 101.0, 21.0]));
        let spec = ac.grid_spec("g").unwrap();
        assert_eq!(spec.resolutions, vec![0.703125, 0.3515625]);
    }

    #[test]
    fn v2_embedded_index_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("svc2");
        let l0 = cache.join("_alllayers").join("L00");
        std::fs::create_dir_all(&l0).unwrap();
        std::fs::write(cache.join("Conf.xml"), tiny_conf(128, "PNG", &[0.703125])).unwrap();
        let packet = 128u32;
        let n = (packet * packet) as usize;
        let mut b = vec![0u8; 64 + n * 8];
        // 行优先槽位：col_in=7,row_in=9 -> idx = 128*9+7
        let idx = (packet * 9 + 7) as usize;
        let payload = 64 + n * 8; // 数据紧跟索引表之后
        b[64 + idx * 8..64 + idx * 8 + 5].copy_from_slice(&(payload as u64).to_le_bytes()[..5]);
        b[64 + idx * 8 + 5..64 + idx * 8 + 8].copy_from_slice(&(PNG.len() as u32).to_le_bytes()[..3]);
        let mut b = b;
        b.extend_from_slice(PNG);
        std::fs::write(l0.join("R0000C0000.bundle"), b).unwrap();

        let ac = ArcgisCache::open(&cache).unwrap();
        assert_eq!(ac.version(), CacheVersion::V2);
        let t = ac.read_tile(0, 7, 9).unwrap().expect("v2 行优先槽位");
        assert_eq!(t.content_type, "image/png");
        // 用 v1 的列优先顺序去取同一瓦片必须拿不到（证明两种顺序确实分叉）
        assert!(ac.read_tile(0, 9, 7).unwrap().is_none());
    }

    /// gis-wt writer 产物读回：定制扩展名 .webpdle + V2 索引 offset 指向 WebP 数据首字节
    /// （4B size 前缀在 offset-4），行优先槽位。覆盖老杨"格式必须完全保证"的读回闭环。
    #[test]
    fn gislab_webpdle_v2_roundtrip() {
        // 一个最小的合法 lossless WebP（1x1，RIFF....WEBPVP8L）
        let webp: &[u8] = b"RIFF\x1a\x00\x00\x00WEBPVP8L\x12\x00\x00\x00\x2f\x00\x00\x00\x10\x07\x10\x11\x11\x88\x88\xfe\x07\x00";
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("svc_webpdle");
        let l0 = cache.join("_alllayers").join("L00");
        std::fs::create_dir_all(&l0).unwrap();
        // CacheTileFormat=WEBPDLE：reader 需识别为 Webp；StorageFormat 声明 V2
        std::fs::write(
            cache.join("Conf.xml"),
            tiny_conf(128, "WEBPDLE", &[0.703125]),
        )
        .unwrap();
        let packet = 128u32;
        let n = (packet * packet) as usize;
        let header_idx = 64 + n * 8;
        let mut b = vec![0u8; header_idx];
        // 模拟 writer：col_in=3,row_in=5 -> idx=128*5+3；先写 4B size 前缀，再写 WebP；
        // 索引 offset 指向 WebP 首字节（= payload+4），size 用真实 WebP 长度。
        let col_in = 3u32;
        let row_in = 5u32;
        let idx = (packet * row_in + col_in) as usize;
        let size_prefix_pos = header_idx as u64;
        let data_off = size_prefix_pos + 4;
        b[64 + idx * 8..64 + idx * 8 + 5]
            .copy_from_slice(&data_off.to_le_bytes()[..5]);
        b[64 + idx * 8 + 5..64 + idx * 8 + 8]
            .copy_from_slice(&(webp.len() as u32).to_le_bytes()[..3]);
        b.extend_from_slice(&(webp.len() as u32).to_le_bytes()); // 4B size 前缀
        b.extend_from_slice(webp);
        assert_eq!(b.len() as u64, data_off + webp.len() as u64);
        // 关键：写 .webpdle 而非 .bundle
        std::fs::write(l0.join("R0000C0000.webpdle"), b).unwrap();

        let ac = ArcgisCache::open(&cache).unwrap();
        assert_eq!(ac.version(), CacheVersion::V2, ".webpdle 应识别为 V2 bundle");
        let t = ac.read_tile(0, col_in, row_in).unwrap().expect("应读回 WebP 瓦片");
        assert_eq!(t.content_type, "image/webp");
        assert!(t.bytes.starts_with(b"RIFF") && t.bytes.get(8..12) == Some(b"WEBP"));
        assert_eq!(t.bytes.len(), webp.len(), "读回长度应精确等于 WebP 长度（不多读 4B 前缀、不少读尾字节）");
        // 行列方向：V2 行优先，交换 col/row 不应命中
        assert!(ac.read_tile(0, row_in, col_in).unwrap().is_none());
    }

    /// WTC v1 格式读回：magic + 64B 自描述头 + 16B 稀疏索引（非 V2 固定槽）。
    /// 覆盖独立格式的核心：自描述头解析、稀疏索引展开、offset 指向 WebP 首字节。
    #[test]
    fn gislab_wtc_v1_roundtrip() {
        let webp: &[u8] = b"RIFF\x1a\x00\x00\x00WEBPVP8L\x12\x00\x00\x00\x2f\x00\x00\x00\x10\x07\x10\x11\x11\x88\x88\xfe\x07\x00";
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("svc_wtc");
        let l0 = cache.join("_alllayers").join("L00");
        std::fs::create_dir_all(&l0).unwrap();
        std::fs::write(cache.join("Conf.xml"), tiny_conf(128, "WEBPDLE", &[0.703125])).unwrap();

        let packet = 128u32;
        let tile_px = 256u32;
        // 放两张瓦片，验证稀疏索引（不写满 128×128）与排序/查找。
        let tiles = [(3u16, 5u16, webp), (7u16, 9u16, webp)];
        let tile_count = tiles.len() as u32;
        let index_off = 64u64;
        let data_off = index_off + tile_count as u64 * 16;
        let mut b = vec![0u8; data_off as usize];
        b[..8].copy_from_slice(WTC_MAGIC);
        b[8..12].copy_from_slice(&1u32.to_le_bytes());          // version
        b[12..16].copy_from_slice(&64u32.to_le_bytes());        // header_size
        b[16..20].copy_from_slice(&tile_px.to_le_bytes());      // tile_size_px
        b[20..24].copy_from_slice(&packet.to_le_bytes());       // packet
        b[24..28].copy_from_slice(&tile_count.to_le_bytes());   // tile_count
        b[28..32].copy_from_slice(&16u32.to_le_bytes());        // index_entry_size
        b[32..40].copy_from_slice(&index_off.to_le_bytes());
        b[40..48].copy_from_slice(&data_off.to_le_bytes());
        // 索引按 (row_in,col_in) 升序写：(3,5) 再 (7,9)
        let mut cursor = data_off as usize;
        for (i, (col_in, row_in, payload)) in tiles.iter().enumerate() {
            let p = index_off as usize + i * 16;
            b[p..p + 2].copy_from_slice(&col_in.to_le_bytes());
            b[p + 2..p + 4].copy_from_slice(&row_in.to_le_bytes());
            b[p + 4..p + 8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            b[p + 8..p + 16].copy_from_slice(&((cursor + 4) as u64).to_le_bytes()); // offset=WebP 首字节
            b.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // 4B size 前缀
            b.extend_from_slice(payload);
            cursor += 4 + payload.len();
        }
        let file_size = b.len() as u64;
        b[48..56].copy_from_slice(&file_size.to_le_bytes()); // file_size（回填）
        std::fs::write(l0.join("R0000C0000.webpdle"), b).unwrap();

        let ac = ArcgisCache::open(&cache).unwrap();
        assert_eq!(ac.version(), CacheVersion::Wtc, "magic GISWTC01 应判为 WTC");
        for (col_in, row_in, payload) in tiles {
            let t = ac.read_tile(0, col_in as u32, row_in as u32)
                .unwrap().expect("应读回 WTC 瓦片");
            assert_eq!(t.content_type, "image/webp");
            assert!(t.bytes.starts_with(b"RIFF") && t.bytes.get(8..12) == Some(b"WEBP"));
            assert_eq!(t.bytes.len(), payload.len());
        }
        // 不存在的槽位应返回 None（稀疏索引不占位）
        assert!(ac.read_tile(0, 4, 4).unwrap().is_none());
        // 行列交换不应命中
        assert!(ac.read_tile(0, 5, 3).unwrap().is_none());
    }

    /// Conf.xml 最小可用文本（LOD 数 = resolutions 长度）。
    fn tiny_conf(packet: u32, fmt: &str, res: &[f64]) -> String {
        let lods: String = res
            .iter()
            .enumerate()
            .map(|(i, r)| {
                format!(
                    "<LODInfo><LevelID>{i}</LevelID><Scale>{}</Scale><Resolution>{r}</Resolution></LODInfo>",
                    r * 111319.4907932736 / (0.0254 / 96.0)
                )
            })
            .collect();
        format!(
            "<CacheInfo><TileCacheInfo><SpatialReference><WKID>4490</WKID></SpatialReference>\
             <TileOrigin><X>-180</X><Y>90</Y></TileOrigin><TileCols>256</TileCols><TileRows>256</TileRows><DPI>96</DPI>\
             <LODInfos>{lods}</LODInfos></TileCacheInfo>\
             <TileImageInfo><CacheTileFormat>{fmt}</CacheTileFormat></TileImageInfo>\
             <CacheStorageInfo><StorageFormat>esriMapCacheStorageModeCompact</StorageFormat><PacketSize>{packet}</PacketSize></CacheStorageInfo></CacheInfo>"
        )
    }

    #[test]
    fn packet_size_guard_rejects_absurd_values() {
        assert!(check_packet(128).is_ok());
        assert!(check_packet(0).is_err());
        assert!(check_packet(512).is_err());
    }

    #[test]
    fn missing_conf_is_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        // ArcgisCache 未实现 Debug，用 match 取错误而不必为测试加 derive。
        let e = match ArcgisCache::open(dir.path()) {
            Ok(_) => panic!("空目录不应能打开"),
            Err(e) => e,
        };
        assert!(e.contains("Conf.xml"), "{e}");
    }
}
