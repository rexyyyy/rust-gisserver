//! ArcGIS Tile Package（`.tpk` / `.tpkx`）解压层。
//!
//! # 它是什么
//!
//! `.tpk` 本质是一个 ZIP 容器，内部就是一套标准的 ArcGIS 切片缓存目录：
//!
//! ```text
//! <stem>.tpk  (ZIP)
//!  └─ v101/Layers/conf.xml       TileCacheInfo / TileOrigin / LODInfos
//!     v101/Layers/conf.cdi        数据范围
//!     v101/Layers/_alllayers/L{zz}/R{rrrr}C{cccc}.bundle[.bundlx]
//!     esriinfo/...                包元数据（本层不读）
//! ```
//!
//! `.tpkx`（ArcGIS Pro Compact Tile Package）是另一种骨架：
//!
//! ```text
//! <stem>.tpkx  (ZIP)
//!  └─ root.json                  瓦片方案（等价 Conf.xml，字段名不同）
//!     tile/L{zz}/R{rrrr}C{cccc}.bundle
//!     iteminfo.json / thumbnail.png
//! ```
//!
//! 两者解开的**瓦片字节布局相同**（紧凑缓存 v2），差异只在**目录布局**与
//! **配置文件**：前者 `_alllayers` + `Conf.xml`，后者 `tile` + `root.json`。
//! 这个差异由 [`crate::arcgis_cache::definition`] 的定义记录吸收（`tile_root`
//! 字段），因此**读瓦片的逻辑一行都不用改**——本层只负责把包解开成目录。
//!
//! # 两种解压落点
//!
//! * [`ensure_dir`]：**自动**解压到进程级缓存目录（`cache/tpk`），供「直接把
//!   `.tpk` 当图层源」的场景使用（幂等：源包更新才重解）；
//! * [`unpack_to`]：**解压到调用方指定的目录**，供「导入」功能使用——落点由
//!   用户决定（`{data_root}/arcgis_tpk/explode_tpk/<name>_tpk/`）。
//!
//! # 为什么解压而非 ZIP 内直读
//!
//! 现有 [`crate::arcgis_cache::bundle`] 的读瓦片全靠文件系统随机读（`File` + `seek` +
//! `read_exact_at`，并对 `.bundle` 索引表做缓存）。ZIP entry 若是 deflate
//! 压缩，随机读就得整段解压，等于重造一套带缓存的 reader；而 TPK 体量可大可小、
//! 结构又与目录完全同形，**解压一次复用久经考验的目录路径**是风险最低的选择。
//! 解压以（源文件路径 + 大小 + mtime）为幂等令牌：同一 `.tpk` 多次 `open`（启动
//! 网格注入、运行时出图、管理面体检各一次）只真正解压一次，源包更新则自动重解。
//!
//! # ZIP 解析
//!
//! TPK 只用 `store`（0）与 `deflate`（8）两种方法，本模块自解析中央目录并逐条
//! 解出，不引入重型 `zip` crate（与 COG/TIFF 容器自解析的工作区风格一致）；
//! 顺带处理 zip64（大 TPK 的中央目录偏移 / 单条大小超 4 GiB 时用 0x0001 额外字段）。

use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use flate2::read::DeflateDecoder;

/// 解压根目录（注入的目录即解压根，包落在 `<cache_root>/<stem>-<hash>`）。
/// map-api 启动时用 [`set_cache_root`] 注入服务级缓存目录（`cache/tpk`）；
/// 未注入（如单独跑本 crate 的测试）退回系统临时目录下的 `tpk`，功能一致只是落点不同。
static CACHE_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// 注入解压根目录；仅第一次调用生效（`OnceLock` 语义）。
pub fn set_cache_root(dir: PathBuf) {
    let _ = CACHE_ROOT.set(dir);
}

fn cache_root() -> PathBuf {
    // 注入的目录就是解压根（map-api 传 `cache/tpk`），不再追加一层；
    // 未注入（单独跑本 crate 的测试）才退回临时目录下的 tpk 子目录。
    CACHE_ROOT
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("tpk"))
}

/// 包类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    /// `.tpk`：ArcGIS Server 紧凑缓存包（`Conf.xml` + `_alllayers/`）。
    Tpk,
    /// `.tpkx`：ArcGIS Pro Compact Tile Package（`root.json` + `tile/`）。
    Tpkx,
}

impl PackageKind {
    /// 包对应的配置文件名。
    pub fn config_file(&self) -> &'static str {
        match self {
            PackageKind::Tpk => "conf.xml",
            PackageKind::Tpkx => "root.json",
        }
    }
}

/// `path` 是否是一个 `.tpk` 包（大小写不敏感、且必须是已存在的普通文件）。
pub fn is_tpk(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("tpk"))
            .unwrap_or(false)
}

/// `path` 是否是一个 `.tpkx` 包（大小写不敏感、且必须是已存在的普通文件）。
pub fn is_tpkx(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("tpkx"))
            .unwrap_or(false)
}

/// `path` 是否是任一形态的 ArcGIS 缓存包。
pub fn is_package(path: &Path) -> bool {
    is_tpk(path) || is_tpkx(path)
}

/// 判定包类型（非包返回 `None`）。
pub fn package_kind(path: &Path) -> Option<PackageKind> {
    if is_tpk(path) {
        Some(PackageKind::Tpk)
    } else if is_tpkx(path) {
        Some(PackageKind::Tpkx)
    } else {
        None
    }
}

/// 规范化「arcgis 源路径」→ 一个真正的缓存目录：
///
/// * 普通目录 / 非包路径：原样返回（现有行为完全不变）。
/// * `.tpk` 文件：幂等解压后返回其中含 `conf.xml` 的目录（如 `…/v101/Layers`）；
/// * `.tpkx` 文件：幂等解压后返回其中含 `root.json` 的目录。
///
/// 这是打开路径（[`crate::arcgis_cache::conf::CacheDocs::load`]）与管理面体检共用的唯一切点，
/// 两侧因此对包与目录的表现一致，不会出图正常、体检却报 critical。
pub fn ensure_dir(path: &Path) -> Result<PathBuf, String> {
    if !is_package(path) {
        return Ok(path.to_path_buf());
    }
    let token = source_token(path)?;
    let dest = staging_dir(path);
    let stamp = dest.join(".extract_ok");
    // 命中且令牌匹配 → 复用既有解压，避免重复 58MB+ 落盘。
    if let Ok(have) = std::fs::read_to_string(&stamp) {
        if let Some(dir) = recorded_conf_dir(&have, &token, &dest) {
            return Ok(dir);
        }
    }
    // 解压到同级临时目录，再原子改名发布；并发/半程崩溃都不会留下半成品被复用。
    std::fs::create_dir_all(dest.parent().unwrap_or(&dest))
        .map_err(|e| format!("创建解压根 {} 失败: {e}", dest.parent().unwrap_or(&dest).display()))?;
    let tmp = unique_tmp(&dest);
    let conf_rel = match extract(path, &tmp) {
        Ok(rel) => rel,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(err);
        }
    };
    let _ = std::fs::remove_dir_all(&dest);
    match std::fs::rename(&tmp, &dest) {
        Ok(()) => {}
        Err(_) => {
            // 另一进程刚发布了同一版本：直接复用；否则清理临时目录并报错。
            let _ = std::fs::remove_dir_all(&tmp);
            if let Ok(have) = std::fs::read_to_string(&stamp) {
                if let Some(dir) = recorded_conf_dir(&have, &token, &dest) {
                    return Ok(dir);
                }
            }
            return Err(format!("发布解压目录失败: {}", dest.display()));
        }
    }
    std::fs::write(&stamp, format!("{token}\n{}", conf_rel.display()))
        .map_err(|e| format!("写解压标记 {}: {e}", stamp.display()))?;
    Ok(dest.join(conf_rel))
}

/// 解包结果：落点目录 + 包类型 + 配置文件所在目录。
#[derive(Clone, Debug)]
pub struct UnpackResult {
    /// 实际落点目录（`dest` 本身）。
    pub dir: PathBuf,
    /// 包类型。
    pub kind: PackageKind,
    /// 配置文件（`conf.xml` / `root.json`）的完整路径。
    pub config_file: PathBuf,
    /// 解出的条目数。
    pub entries: usize,
}

/// 把 `.tpk` / `.tpkx` **解压到指定目录**（导入功能用）。
///
/// 与 [`ensure_dir`] 的差别只在落点：这里由调用方决定（管理面给的是
/// `{data_root}/arcgis_tpk/explode_tpk/<name>_tpk/`），且**不做幂等缓存**——
/// 每次调用都真解一遍（导入是显式动作，源包变了就该重解）。
///
/// # 落点即缓存目录
///
/// 解压不是原样铺开，而是**把「配置文件所在目录」的内容提升到落点根**，
/// 使落点目录本身成为一份标准的 ArcGIS 缓存目录：
///
/// | 包 | 原始骨架 | 落点 |
/// |---|---|---|
/// | `.tpk` | `v101/图层/{conf.xml,conf.cdi,_alllayers/}` | `{name}_tpk/{conf.xml,conf.cdi,_alllayers/}` |
/// | `.tpkx` | `{root.json,tile/}` | `{name}_tpk/{root.json,tile/}` |
///
/// `.tpk` 多出的 `v101/图层` 两层若原样保留，落点就不是可识别的缓存目录
/// （定位只认同级的 `conf.xml` 与瓦片根目录），后续登记成数据源会失败。
/// 包内其余元数据（`esriinfo/`、`servicedescriptions/`、`*.lyr`）移入
/// `_package/`：既保留出处，又不干扰缓存目录的识别。
///
/// 解压同样走「临时目录 + 原子改名」，因此中途失败不会留下半成品目录。
/// 目标目录已存在时先清空再发布（同名重新导入 = 覆盖）。
pub fn unpack_to(src: &Path, dest: &Path) -> Result<UnpackResult, String> {
    let kind = package_kind(src).ok_or_else(|| {
        format!(
            "{} 不是 ArcGIS 缓存包（扩展名应为 .tpk 或 .tpkx）",
            src.display()
        )
    })?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    let tmp = unique_tmp(dest);
    let conf_rel = match extract(src, &tmp) {
        Ok(rel) => rel,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(err);
        }
    };
    // 提升缓存根：让落点目录自身即可被识别的 ArcGIS 缓存目录。
    let conf_rel = match promote_cache_root(&tmp, &conf_rel) {
        Ok(r) => r,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(err);
        }
    };
    let entries = count_entries(&tmp);
    // 目标已存在 → 改名让开，再删旧的，最后发布新的。
    let old = unique_tmp(dest).with_extension("old");
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &old).map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp);
            format!("移动旧目录 {} 失败: {e}", dest.display())
        })?;
    }
    if std::fs::rename(&tmp, dest).is_err() {
        // 发布失败 → 把旧目录挪回来，不让调用方丢数据。
        if had_old {
            let _ = std::fs::rename(&old, dest);
        }
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("发布解压目录失败: {}", dest.display()));
    }
    if had_old {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(UnpackResult {
        dir: dest.to_path_buf(),
        kind,
        config_file: dest.join(&conf_rel),
        entries,
    })
}

/// 把「配置文件所在目录」的内容提升到 `root` 根，使 `root` 本身即缓存目录。
///
/// `conf_rel` 是**配置文件所在目录**相对 `root` 的路径（`.tpk` 为 `v101/图层`，
/// `.tpkx` 为空即包根本身）。返回配置文件相对 `root` 的路径（提升后恒为根下的
/// 一个文件名）。
///
/// `.tpk` 的骨架是 `v101/图层/…`（两层），提升后 `root` 下直接是
/// `conf.xml` + `conf.cdi` + `_alllayers/`；`.tpkx` 的 `root.json` 本就在包根，
/// 无需提升。包内其余内容移入 `_package/`。
fn promote_cache_root(root: &Path, conf_rel: &Path) -> Result<PathBuf, String> {
    let cfg_dir = if conf_rel.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(conf_rel)
    };
    if !cfg_dir.is_dir() {
        return Err(format!("配置文件目录不存在：{}", cfg_dir.display()));
    }
    // 配置文件名（`conf.xml` / `root.json`，磁盘实际大小写为准）。
    let conf_name = find_config_file(&cfg_dir).ok_or_else(|| {
        format!(
            "{} 下未找到 conf.xml 或 root.json，不是 ArcGIS 缓存包",
            cfg_dir.display()
        )
    })?;

    // 配置目录不在根上（.tpk）：把缓存内容提升到根。
    if cfg_dir != root {
        // 1) 缓存内容上提（同名冲突：旧的先删）。
        let mut promoted: Vec<std::ffi::OsString> = Vec::new();
        for e in std::fs::read_dir(&cfg_dir)
            .map_err(|e| format!("读 {}: {e}", cfg_dir.display()))?
        {
            let e = e.map_err(|e| format!("读 {}: {e}", cfg_dir.display()))?;
            let name = e.file_name();
            let from = e.path();
            let target = root.join(&name);
            if target.exists() {
                remove_any(&target)?;
            }
            std::fs::rename(&from, &target)
                .map_err(|e| format!("移动 {} 到 {} 失败: {e}", from.display(), target.display()))?;
            promoted.push(name);
        }
        // 2) 清掉空掉的配置目录及其空祖先（`v101/图层` → `v101`）。
        let _ = std::fs::remove_dir_all(&cfg_dir);
        let mut cur = cfg_dir.parent().map(|p| p.to_path_buf());
        while let Some(dir) = cur {
            if dir == root || !dir.starts_with(root) {
                break;
            }
            match std::fs::read_dir(&dir) {
                Ok(mut rd) => {
                    if rd.next().is_some() {
                        break; // 还有内容（如 `v101/图层.lyr`），留待整体搬走
                    }
                }
                Err(_) => break,
            }
            if std::fs::remove_dir(&dir).is_err() {
                break;
            }
            cur = dir.parent().map(|p| p.to_path_buf());
        }
        // 3) 其余顶层内容（包元数据等）移入 `_package/`。
        let pkg = root.join("_package");
        let tops: Vec<std::ffi::OsString> = std::fs::read_dir(root)
            .map_err(|e| format!("读 {}: {e}", root.display()))?
            .flatten()
            .map(|e| e.file_name())
            .collect();
        for name in tops {
            if promoted.iter().any(|p| p == &name) || name == "_package" {
                continue;
            }
            std::fs::create_dir_all(&pkg)
                .map_err(|e| format!("建目录 {}: {e}", pkg.display()))?;
            let from = root.join(&name);
            let to = pkg.join(&name);
            if to.exists() {
                remove_any(&to)?;
            }
            std::fs::rename(&from, &to).map_err(|e| {
                format!("移动 {} 到 {} 失败: {e}", from.display(), to.display())
            })?;
        }
    }

    Ok(PathBuf::from(conf_name))
}

/// 在目录里找配置文件（`conf.xml` / `root.json`，不区分大小写），返回磁盘实际文件名。
fn find_config_file(dir: &Path) -> Option<std::ffi::OsString> {
    let rd = std::fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let n = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if n == "conf.xml" || n == "root.json" {
            return Some(e.file_name());
        }
    }
    None
}

/// 删除文件或目录（目录递归）。
fn remove_any(p: &Path) -> Result<(), String> {
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| format!("删除目录 {}: {e}", p.display()))
    } else {
        std::fs::remove_file(p).map_err(|e| format!("删除文件 {}: {e}", p.display()))
    }
}

/// 统计解压目录里的文件数（用于给管理面一个粗略的体量反馈）。
fn count_entries(dir: &Path) -> usize {
    fn walk(d: &Path, n: &mut usize) {
        let Ok(rd) = std::fs::read_dir(d) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, n);
            } else {
                *n += 1;
            }
        }
    }
    let mut n = 0usize;
    walk(dir, &mut n);
    n
}

/// 幂等令牌：源文件绝对路径 + 字节数 + 修改时间（纳秒）的短指纹。
fn source_token(path: &Path) -> Result<String, String> {
    let canon = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    let md = std::fs::metadata(path).map_err(|e| format!("读 {} 元数据失败: {e}", path.display()))?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(format!("{canon}|{}|{mtime}", md.len()))
}

/// 由源路径派生的稳定解压目录：`<cache_root>/<stem>-<hash16>`。
fn staging_dir(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tpk".to_string());
    let safe: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let token = source_token(path).unwrap_or_default();
    let hash = fnv1a64(token.as_bytes());
    cache_root().join(format!("{safe}-{hash:016x}"))
}

/// 从标记文件内容里取回「与当前令牌匹配的 conf 目录」；令牌不符视为过期，返回 None。
fn recorded_conf_dir(have: &str, token: &str, dest: &Path) -> Option<PathBuf> {
    let mut lines = have.splitn(2, '\n');
    let stored = lines.next()?;
    let rel = lines.next()?;
    if stored != token {
        return None;
    }
    let dir = dest.join(rel);
    dir.is_dir().then_some(dir)
}

/// 生成一个全局唯一的临时解压目录名（放在最终目录的父级下，便于 rename）。
///
/// 仅用 pid 不够：同进程内两个并发 `ensure_dir`（如启动注入与首个出图请求同时
/// 落到同一 `.tpk` 的首解压）会拿到同一个临时目录并互相覆盖，因此再叠加
/// 进程内单调序号与纳秒时间戳。
fn unique_tmp(dest: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tpk".to_string());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    parent.join(format!(".tmp-{name}-{}-{nanos:x}-{seq:x}", std::process::id()))
}

// ---- ZIP 解析 --------------------------------------------------------------

/// 单条中央目录记录。
struct Entry {
    method: u16,
    comp_size: u64,
    uncomp_size: u64,
    local_off: u64,
    name: String,
}

/// 解出中央目录结束记录（EOCD），必要时跟进 zip64 EOCD。
fn read_eocd(f: &mut std::fs::File, total: u64) -> Result<(u64, u64), String> {
    // EOCD 定长 22B，最多再跟 65535B 注释；在文件尾部这段窗口里反向找签名。
    let window = (65535 + 22).min(total as usize);
    if window < 22 {
        return Err("文件过小，不是有效 ZIP/TPK".to_string());
    }
    let start = total as usize - window;
    f.seek(SeekFrom::Start(start as u64)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; window];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    let sig = [0x50, 0x4b, 0x05, 0x06]; // PK\x05\x06
    let mut pos = None;
    let mut i = buf.len() - 22;
    loop {
        if buf[i..i + 4] == sig {
            pos = Some(i);
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    let p = pos.ok_or("未找到 ZIP 中央目录结束记录（EOCD），文件可能损坏")?;
    let e = &buf[p..p + 22];
    let mut count = u16::from_le_bytes([e[10], e[11]]) as u64;
    let mut cd_offset = u32::from_le_bytes([e[16], e[17], e[18], e[19]]) as u64;
    // zip64：任一基准字段全 F → 用紧邻 EOCD 之前的定位器跳到 zip64 EOCD 记录。
    if count == 0xFFFF || cd_offset == 0xFFFFFFFF {
        if p >= 20 && buf[p - 20..p - 16] == [0x50, 0x4b, 0x06, 0x07] {
            let loc = &buf[p - 20..p];
            let z_off = u64::from_le_bytes(le8(&loc[8..16]));
            f.seek(SeekFrom::Start(z_off)).map_err(|err| err.to_string())?;
            let mut zr = [0u8; 56];
            f.read_exact(&mut zr).map_err(|err| err.to_string())?;
            if zr[0..4] == [0x50, 0x4b, 0x06, 0x06] {
                count = u64::from_le_bytes(le8(&zr[24..32]));
                cd_offset = u64::from_le_bytes(le8(&zr[48..56]));
            }
        }
    }
    Ok((count, cd_offset))
}

/// 遍历中央目录，返回全部条目（含 zip64 额外字段修正后的真实大小 / 偏移）。
fn read_entries(f: &mut std::fs::File, count: u64, cd_offset: u64) -> Result<Vec<Entry>, String> {
    f.seek(SeekFrom::Start(cd_offset))
        .map_err(|e| format!("定位中央目录 {cd_offset}: {e}"))?;
    let mut out = Vec::with_capacity(count.min(1_000_000) as usize);
    let mut head = [0u8; 46];
    for _ in 0..count {
        f.read_exact(&mut head).map_err(|e| format!("读中央目录头: {e}"))?;
        if head[0..4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err("中央目录签名错误，ZIP 结构损坏".to_string());
        }
        let method = u16::from_le_bytes([head[10], head[11]]);
        let mut comp = u32::from_le_bytes([head[20], head[21], head[22], head[23]]) as u64;
        let mut uncomp = u32::from_le_bytes([head[24], head[25], head[26], head[27]]) as u64;
        let name_len = u16::from_le_bytes([head[28], head[29]]) as usize;
        let extra_len = u16::from_le_bytes([head[30], head[31]]) as usize;
        let comment_len = u16::from_le_bytes([head[32], head[33]]) as usize;
        let mut local_off = u32::from_le_bytes([head[42], head[43], head[44], head[45]]) as u64;
        let mut name = vec![0u8; name_len];
        f.read_exact(&mut name).map_err(|e| e.to_string())?;
        let mut extra = vec![0u8; extra_len];
        if extra_len > 0 {
            f.read_exact(&mut extra).map_err(|e| e.to_string())?;
        }
        if comment_len > 0 {
            f.seek(SeekFrom::Current(comment_len as i64)).map_err(|e| e.to_string())?;
        }
        // zip64 额外字段（0x0001）：按 uncomp/comp/local_off 中出现 0xFFFFFFFF 者补真值。
        if uncomp == 0xFFFFFFFF || comp == 0xFFFFFFFF || local_off == 0xFFFFFFFF {
            apply_zip64_extra(&extra, &mut uncomp, &mut comp, &mut local_off);
        }
        out.push(Entry {
            method,
            comp_size: comp,
            uncomp_size: uncomp,
            local_off,
            name: String::from_utf8_lossy(&name).into_owned(),
        });
    }
    Ok(out)
}

/// 解析条目的 zip64 扩展头（id 0x0001），把被 0xFFFFFFFF 占位的字段替换成 8 字节真值。
fn apply_zip64_extra(extra: &[u8], uncomp: &mut u64, comp: &mut u64, local_off: &mut u64) {
    let mut i = 0;
    while i + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[i], extra[i + 1]]);
        let sz = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        let body = &extra[i + 4..(i + 4 + sz).min(extra.len())];
        if id == 0x0001 {
            // 字段顺序固定为 [uncomp][comp][local_off][disk]，仅写实际被占位者之前的顺序值。
            let mut off = 0usize;
            let mut next = || -> Option<u64> {
                if off + 8 <= body.len() {
                    let v = u64::from_le_bytes(le8(&body[off..off + 8]));
                    off += 8;
                    Some(v)
                } else {
                    None
                }
            };
            // zip64 里三者的存在性由各自基准是否 0xFFFFFFFF 决定，但值一律按此顺序排布。
            let base_uncomp = *uncomp == 0xFFFFFFFF;
            let base_comp = *comp == 0xFFFFFFFF;
            let base_off = *local_off == 0xFFFFFFFF;
            if base_uncomp {
                if let Some(v) = next() {
                    *uncomp = v;
                }
            }
            if base_comp {
                if let Some(v) = next() {
                    *comp = v;
                }
            }
            if base_off {
                if let Some(v) = next() {
                    *local_off = v;
                }
            }
            return;
        }
        i += 4 + sz;
    }
}

/// 取某条目的数据区起点：本地头 30B + 本地 name/extra 长（可能与中央目录不同）。
fn data_offset(f: &mut std::fs::File, local_off: u64) -> Result<u64, String> {
    f.seek(SeekFrom::Start(local_off))
        .map_err(|e| format!("定位本地头 {local_off}: {e}"))?;
    let mut lh = [0u8; 30];
    f.read_exact(&mut lh).map_err(|e| e.to_string())?;
    if lh[0..4] != [0x50, 0x4b, 0x03, 0x04] {
        return Err("本地文件头签名错误，ZIP 结构损坏".to_string());
    }
    let name_len = u16::from_le_bytes([lh[26], lh[27]]) as u64;
    let extra_len = u16::from_le_bytes([lh[28], lh[29]]) as u64;
    Ok(local_off + 30 + name_len + extra_len)
}

/// 单个 deflate 条目解压后的体积上限（1 GiB）。真缓存的 bundle 远小于此，
/// 越线说明中央目录声明不可信（损坏或 zip bomb），直接失败而不是先分配再炸内存。
const DECOMP_LIMIT: u64 = 1 << 30;

/// 把 `.tpk` / `.tpkx` 解到 `dest`，返回含配置文件的目录相对 `dest` 的路径。
///
/// 配置文件两种：`conf.xml`（`.tpk`，位于 `v101/<图层>/`）与 `root.json`
/// （`.tpkx`，位于包根）。两者都认，找到即返回其所在目录。
fn extract(tpk: &Path, dest: &Path) -> Result<PathBuf, String> {
    let mut f = std::fs::File::open(tpk).map_err(|e| format!("打开 {}: {e}", tpk.display()))?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let (count, cd_offset) = read_eocd(&mut f, total)?;
    let entries = read_entries(&mut f, count, cd_offset)?;

    let mut conf_dir: Option<PathBuf> = None;
    for e in entries {
        let rel = match sanitize(&e.name) {
            Some(r) => r,
            None => continue, // 目录项 / 非法名 → 跳过（目录会在建文件时按需创建）
        };
        let target = dest.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("建目录 {}: {err}", parent.display()))?;
        }
        let start = data_offset(&mut f, e.local_off)?;
        f.seek(SeekFrom::Start(start))
            .map_err(|err| format!("seek {}: {err}", e.name))?;
        match e.method {
            0 => {
                // store：原样拷。
                let mut out = std::fs::File::create(&target)
                    .map_err(|err| format!("建 {}: {err}", target.display()))?;
                copy_n(&mut f, &mut out, e.comp_size)
                    .map_err(|err| format!("写 {}: {err}", target.display()))?;
            }
            8 => {
                // deflate：读出压缩块再整段解压（bundle 会被落盘，后续随机读走文件系统）。
                if e.comp_size > total.saturating_sub(start) {
                    return Err(format!(
                        "{}: 压缩块长度 {} 超过文件剩余 {} 字节，中央目录不可信",
                        e.name,
                        e.comp_size,
                        total.saturating_sub(start)
                    ));
                }
                if e.uncomp_size > DECOMP_LIMIT {
                    return Err(format!(
                        "{}: 声明解压后大小 {} 超过单条目上限 {DECOMP_LIMIT}",
                        e.name, e.uncomp_size
                    ));
                }
                let mut comp = vec![0u8; e.comp_size as usize];
                f.read_exact(&mut comp)
                    .map_err(|err| format!("读压缩块 {}: {err}", e.name))?;
                let mut dec = DeflateDecoder::new(Cursor::new(comp));
                let mut buf = Vec::with_capacity(e.uncomp_size.min(64 * 1024 * 1024) as usize);
                // 声明值可能谎报，解压长度以 take() 硬截断后再复核。
                (&mut dec).take(DECOMP_LIMIT + 1)
                    .read_to_end(&mut buf)
                    .map_err(|err| format!("inflate {}: {err}", e.name))?;
                if buf.len() as u64 > DECOMP_LIMIT {
                    return Err(format!(
                        "{}: 实际解压 {} 字节，超过单条目上限 {DECOMP_LIMIT}",
                        e.name,
                        buf.len()
                    ));
                }
                std::fs::write(&target, &buf)
                    .map_err(|err| format!("写 {}: {err}", target.display()))?;
            }
            m => return Err(format!("{}: 不支持的压缩方法 {m}（仅 store/deflate）", e.name)),
        }
        // 配置文件（conf.xml / root.json）所在目录即「缓存根」：
        // .tpk 是 v101/<图层>/（内含 _alllayers/），.tpkx 是包根（内含 tile/）。
        if conf_dir.is_none() {
            let base = e.name.rsplit(['/', '\\']).next().unwrap_or("");
            if base.eq_ignore_ascii_case("conf.xml") || base.eq_ignore_ascii_case("root.json") {
                conf_dir = Some(rel.parent().map(|p| p.to_path_buf()).unwrap_or_default());
            }
        }
    }
    conf_dir.ok_or_else(|| {
        format!(
            "{} 内未找到 conf.xml 或 root.json，不是 ArcGIS 缓存包",
            tpk.display()
        )
    })
}

/// 把 ZIP 条目名安全映射为相对路径：拒绝绝对路径与 `..`（zip-slip），目录项返回 None。
fn sanitize(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.is_empty() || name.ends_with('/') || name.starts_with('/') {
        return None; // 目录占位 / 绝对路径
    }
    let mut p = PathBuf::new();
    for c in name.split('/') {
        match c {
            "" | "." => continue,
            ".." => return None, // 越界，丢弃
            seg => p.push(seg),
        }
    }
    // 只保留普通字符组件，挡开 Windows 盘符/怪异名。
    if p.components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        (!p.as_os_str().is_empty()).then_some(p)
    } else {
        None
    }
}

fn copy_n<R: Read, W: Write>(r: &mut R, w: &mut W, n: u64) -> std::io::Result<()> {
    let mut left = n;
    let mut chunk = [0u8; 64 * 1024];
    while left > 0 {
        let want = left.min(chunk.len() as u64) as usize;
        let got = r.read(&mut chunk[..want])?;
        if got == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ZIP 数据块提前结束",
            ));
        }
        w.write_all(&chunk[..got])?;
        left -= got as u64;
    }
    Ok(())
}

fn le8(b: &[u8]) -> [u8; 8] {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    a
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手写最小 ZIP（store + deflate），够喂给本模块的解析器。
    /// CRC 一律填 0——`extract` 不校验 CRC，填真值只会让测试更难读。
    fn build_zip(items: &[(&str, u16, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, method, data) in items {
            let payload = match *method {
                0 => data.to_vec(),
                8 => {
                    use flate2::write::DeflateEncoder;
                    let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                    enc.write_all(data).unwrap();
                    enc.finish().unwrap()
                }
                m => panic!("测试不构造方法 {m}"),
            };
            let local_off = out.len() as u64;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20u8, 0, 0, 0]); // version needed / flags
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0u8; 4]); // time + date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&payload);

            cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            cd.extend_from_slice(&[20u8, 0, 20, 0]); // made-by + needed
            cd.extend_from_slice(&[0u8, 0]); // flags
            cd.extend_from_slice(&method.to_le_bytes());
            cd.extend_from_slice(&[0u8; 4]); // time + date
            cd.extend_from_slice(&0u32.to_le_bytes()); // crc
            cd.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
            cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0u8; 6]); // extra + comment + disk-start
            cd.extend_from_slice(&[0u8; 6]); // internal + external attrs
            cd.extend_from_slice(&(local_off as u32).to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u64;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // disks
        let n = items.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    /// 篡改中央目录里最后一条记录声明的 comp/uncomp 大小，模拟损坏或恶意 ZIP 头。
    /// 反向找签名：本地数据（deflate 流）理论上可能撞出同一 4 字节串。
    fn forge_sizes(zip: &mut [u8], comp: u32, uncomp: u32) {
        let pos = zip
            .windows(4)
            .rposition(|w| w == [0x50, 0x4b, 0x01, 0x02])
            .expect("中央目录签名");
        zip[pos + 20..pos + 24].copy_from_slice(&comp.to_le_bytes());
        zip[pos + 24..pos + 28].copy_from_slice(&uncomp.to_le_bytes());
    }

    fn write_and_extract(dir: &Path, zip: &[u8]) -> Result<PathBuf, String> {
        let tpk = dir.join("fake.tpk");
        std::fs::write(&tpk, zip).unwrap();
        let dest = dir.join("out");
        std::fs::create_dir_all(&dest).unwrap();
        extract(&tpk, &dest)
    }

    /// deflate 分支：真数据全是 store，这条覆盖「解得出且内容逐字节一致」。
    #[test]
    fn extract_decodes_deflate_and_store() {
        let tmp = tempfile::tempdir().unwrap();
        let conf = r#"<?xml version="1.0" ?><TileCacheInfo><TileCols>256</TileCols></TileCacheInfo>"#;
        let bundle = vec![0xABu8; 4096];
        let zip = build_zip(&[
            ("v101/Layers/conf.xml", 8, conf.as_bytes()),
            ("v101/Layers/_alllayers/L00/R0000C0000.bundle", 8, &bundle),
            ("v101/root.xml", 0, b"store passthrough"),
        ]);
        let rel = write_and_extract(tmp.path(), &zip).expect("deflate 包应解出 conf 目录");
        assert_eq!(rel, PathBuf::from("v101").join("Layers"));
        let dest = tmp.path().join("out");
        assert_eq!(std::fs::read(dest.join(rel).join("conf.xml")).unwrap(), conf.as_bytes());
        assert_eq!(
            std::fs::read(dest.join("v101/Layers/_alllayers/L00/R0000C0000.bundle")).unwrap(),
            bundle
        );
        assert_eq!(std::fs::read(dest.join("v101/root.xml")).unwrap(), b"store passthrough");
    }

    /// 中央目录谎报解压后大小（> 单条目上限）→ 报错，不先分配再 OOM。
    #[test]
    fn extract_rejects_bogus_declared_sizes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut zip = build_zip(&[("v101/Layers/conf.xml", 8, b"<TileCacheInfo/>")]);
        forge_sizes(&mut zip, 12, 0xE000_0000); // 声称 3.5 GiB，实际压缩块 12B
        let err = write_and_extract(tmp.path(), &zip).unwrap_err();
        assert!(err.contains("单条目上限"), "报错应来自大小上限：{err}");
    }

    /// 中央目录谎报压缩块长度（超出文件剩余）→ 报错，不越界大分配。
    #[test]
    fn extract_rejects_comp_block_beyond_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut zip = build_zip(&[("v101/Layers/conf.xml", 8, b"<TileCacheInfo/>")]);
        forge_sizes(&mut zip, 0xFFFF_FFF0, 64);
        let err = write_and_extract(tmp.path(), &zip).unwrap_err();
        assert!(err.contains("中央目录不可信"), "报错应来自越界校验：{err}");
    }

    /// 同进程并发首解压不能撞同一个临时目录（仅 pid 时会）。
    #[test]
    fn unique_tmp_never_repeats_within_process() {
        let dest = Path::new("cache/tpk/pkg-0000000000000001");
        let a = unique_tmp(dest);
        let b = unique_tmp(dest);
        assert_ne!(a, b);
        assert_eq!(a.parent(), dest.parent());
        assert!(a.file_name().unwrap().to_string_lossy().starts_with(".tmp-pkg-"));
    }

    /// 注入的 cache_root 就是解压根：包落在其直接子级，不再套一层 `tpk/`。
    #[test]
    fn staging_dir_is_direct_child_of_cache_root() {
        let root = cache_root();
        let dest = staging_dir(Path::new("some/where/longjie2en.tpk"));
        assert_eq!(dest.parent(), Some(root.as_path()), "不应出现 {}/tpk/…", root.display());
        assert!(dest
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("longjie2en-"));
    }
}
