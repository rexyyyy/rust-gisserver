//! GIS 能力底座 · 路径编码检测与归一化（语言区域驱动）。
//!
//! 基于字符频度的编码概率打分实现：GBK/BIG5 频度表用于字节序列打分，
//! EUC-KR / EUC-TW / Shift_JIS / CP932 按同构打分法实现。
//!
//! **探测集合由语言区域（locale）决定**，UTF-8/ASCII 恒为公共候选：
//!
//! | 地区 | 传统编码候选 |
//! |---|---|
//! | 英文（En） | 无（只认 UTF-8/ASCII） |
//! | 简体中文（ZhCn，**当前部署默认**） | GBK |
//! | 繁体中文（ZhTw） | BIG5 + EUC-TW |
//! | 韩语（Ko） | EUC-KR |
//! | 日语（Ja） | Shift_JIS（含 CP932 扩展区识别） |
//!
//! 场景：Windows 下路径可能以本地传统编码字节形态流入系统（旧工具链、cmd 复制、
//! 外部清单文件、DBF 属性等）。GDAL 窄字符接口按 UTF-8 解释路径，编码不对就会
//! "No such file or directory"。本 crate 在路径交给 GDAL 前做归一化：
//!
//! * [`fix_path`]：修复两类 mojibake 字符串
//!   - 形态 1："传统编码字节被逐字节误读为 Latin-1"（全串字符 ≤ U+00FF，
//!     如 `¹¤¾ß` 实为 GBK 的 `工具`）；
//!   - 形态 2："UTF-8 字节被 GBK 误读"（字符串本身含 CJK 但语义是乱码，
//!     如 `娴嬭瘯` 实为 `测试`：其 GBK 字节恰好还原出合法 UTF-8）。
//! * [`fix_path_bytes`]：原始字节入口，先试 UTF-8，失败按区域传统编码解码。
//! * [`dbf`]：shp 属性表（DBF）编码检测（.cpg / LDID / 字节采样打分）。
//!
//! 判定保守：正常 UTF-8 路径原样返回；无法确信时不改动。
//!
//! > 同步源：gis-wt 主仓 `rust_back_apiserver/crates/gis-encoding`（两仓各持一份，
//!   修改探测逻辑后须双向同步）。

mod tables;

pub mod dbf;

use tables::{BIG5_FREQ, GB_FREQ, GBK_FREQ};

// Windows 系统 ANSI 代码页（kernel32）。仅当显式开启 `win-acp` feature 时编译，
// 默认关闭 → core 零 FFI / 零 unsafe。区域首选由 `.env` 的 `GIS_ENCODING_REGION`
// 显式声明（见 `config/env/README.md`）。
#[cfg(all(windows, feature = "win-acp"))]
extern "system" {
    fn GetACP() -> u32;
}

/// 语言区域：决定要探测的传统编码集合（UTF-8/ASCII 恒为公共候选）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 英文地区：只认 UTF-8/ASCII，不做多字节探测。
    En,
    /// 简体中文地区（当前部署默认）：GBK。
    ZhCn,
    /// 繁体中文地区：BIG5 + EUC-TW。
    ZhTw,
    /// 韩语地区：EUC-KR。
    Ko,
    /// 日语地区：Shift_JIS / CP932。
    Ja,
}

impl Region {
    /// 从 locale 字符串解析（POSIX 命名，如 `zh_CN.UTF-8` / `zh_TW` / `ko_KR` /
    /// `ja_JP` / `en_US`；Windows 风格 `Chinese(R)` 等也可识别）。
    pub fn from_locale(s: &str) -> Region {
        let l = s.to_ascii_lowercase();
        // 顺序敏感：zh_tw/zh_hk/zh_hant 必须先于 zh 判断
        if ["zh_tw", "zh-tw", "zh_hk", "zh-hk", "zh_hant", "zh-hant", "cht"]
            .iter()
            .any(|k| l.contains(k))
        {
            Region::ZhTw
        } else if l.starts_with("zh") || l.contains("chinese") || l.contains("chs") {
            Region::ZhCn
        } else if l.starts_with("ko") || l.contains("korean") {
            Region::Ko
        } else if l.starts_with("ja") || l.contains("japanese") {
            Region::Ja
        } else if l.starts_with("en") {
            Region::En
        } else {
            // 未识别 → 当前部署默认：简体中文地区
            Region::ZhCn
        }
    }

    /// 从环境推断区域，优先级（`config/env/README.md` 为权威说明）：
    ///
    /// 1. `GIS_ENCODING_REGION` —— 推荐，严格取值见 [`Region::parse`]；
    /// 2. `GIS_ENCODING_LOCALE` —— 旧名兼容，接受 `zh_CN.UTF-8` 等 POSIX / Windows 写法；
    /// 3. `win-acp` feature 开启时，Windows `GetACP()` 作为兜底；
    /// 4. 兜底 `zh-CN`（保持既有部署默认）。
    ///
    /// **不再读取** `LANG` / `LC_ALL` / `LC_CTYPE`：它们是进程 locale，
    /// 不代表磁盘路径字节原本使用的传统编码。
    pub fn from_env() -> Region {
        if let Ok(v) = std::env::var("GIS_ENCODING_REGION") {
            if let Some(r) = Region::parse(v.trim()) {
                return r;
            }
        }
        if let Ok(v) = std::env::var("GIS_ENCODING_LOCALE") {
            let v = v.trim();
            if !v.is_empty() {
                return Region::from_locale(v);
            }
        }
        fallback_region()
    }

    /// 严格解析地区取值（用于 `GIS_ENCODING_REGION`）。未知值返回 `None`。
    ///
    /// 取值（大小写 / 连字符不敏感）：`zh-CN` / `zh-TW` / `en` / `ko` / `ja`，
    /// 以及常见同义写法（`zh`、`chs`、`zh_hk`、`ko-kr`、`ja-jp` 等）。
    pub fn parse(s: &str) -> Option<Region> {
        let k = s.trim().to_ascii_lowercase().replace('_', "-");
        match k.as_str() {
            "zh-cn" | "zh" | "chs" | "chinese" => Some(Region::ZhCn),
            "zh-tw" | "zh-hk" | "zh-hant" | "cht" => Some(Region::ZhTw),
            "ko" | "ko-kr" | "korean" => Some(Region::Ko),
            "ja" | "ja-jp" | "japanese" => Some(Region::Ja),
            "en" | "en-us" | "english" => Some(Region::En),
            _ => None,
        }
    }

    /// 规范化取值（与 `GIS_ENCODING_REGION` 取值一致），用于写回 / 校验。
    pub fn as_str(self) -> &'static str {
        match self {
            Region::En => "en",
            Region::ZhCn => "zh-CN",
            Region::ZhTw => "zh-TW",
            Region::Ko => "ko",
            Region::Ja => "ja",
        }
    }

    /// 从 Windows ANSI 代码页映射区域。
    pub fn from_acp(acp: u32) -> Region {
        match acp {
            936 => Region::ZhCn,   // 简体中文 GBK
            950 | 951 => Region::ZhTw, // 繁体中文 Big5
            949 => Region::Ko,     // 韩语 Wansung/EUC-KR
            932 => Region::Ja,     // 日语 Shift_JIS(CP932)
            _ => Region::En,       // 65001(UTF-8)/1252 等西文页：不探测传统编码
        }
    }

    /// 本地区需要探测的传统编码（优先级从高到低）。
    pub fn legacy(self) -> &'static [Legacy] {
        match self {
            Region::En => &[],
            Region::ZhCn => &[Legacy::Gbk],
            Region::ZhTw => &[Legacy::Big5, Legacy::EucTw],
            Region::Ko => &[Legacy::EucKr],
            Region::Ja => &[Legacy::Sjis],
        }
    }
}

/// `from_env` 的兜底区域：默认 `zh-CN`；仅当显式开启 `win-acp` feature 时，
/// 在 Windows 上改用系统 ANSI 代码页 `GetACP()`（保留旧 Windows 兼容行为，可选启用）。
#[cfg(all(windows, feature = "win-acp"))]
fn fallback_region() -> Region {
    Region::from_acp(unsafe { GetACP() })
}

#[cfg(not(all(windows, feature = "win-acp")))]
fn fallback_region() -> Region {
    Region::ZhCn
}

/// 区域传统编码候选（不含公共候选 UTF-8/ASCII）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Legacy {
    Gbk,
    Big5,
    EucTw,
    EucKr,
    Sjis,
}

/// 检测结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// 纯 ASCII（< 0x80），任何编码下语义一致。
    Ascii,
    /// 合法 UTF-8。
    Utf8,
    /// GBK/GB2312 双字节序列（简体中文地区）。
    Gbk,
    /// Big5（繁体中文地区）。
    Big5,
    /// EUC-TW（繁体中文地区，CNS 11643，SS2 四字节序列特征）。
    EucTw,
    /// EUC-KR / CP949（韩语地区）。
    EucKr,
    /// Shift_JIS（日语地区）。
    ShiftJis,
    /// CP932（Shift_JIS 的 Windows 扩展，含 NEC/IBM 扩展区字节）。
    Cp932,
    /// 无法判定。
    Other,
}

impl Encoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Ascii => "ASCII",
            Encoding::Utf8 => "UTF-8",
            Encoding::Gbk => "GBK",
            Encoding::Big5 => "BIG5",
            Encoding::EucTw => "EUC-TW",
            Encoding::EucKr => "EUC-KR",
            Encoding::ShiftJis => "SHIFT_JIS",
            Encoding::Cp932 => "CP932",
            Encoding::Other => "OTHER",
        }
    }
}

/// 对原始字节做编码判定（按当前环境区域探测，见 [`Region::from_env`]）。
pub fn detect(bytes: &[u8]) -> Encoding {
    detect_for(bytes, Region::from_env())
}

/// 对原始字节做编码判定（显式指定区域）。
///
/// 探测顺序：ASCII → UTF-8 → 区域传统编码。GBK 与 Big5 合法字节区间高度重叠，
/// 因此 ZhCn/ZhTw 互为对方打分的**否决 guard**（分数不高者不算数，防止跨区误判），
/// 但结果只会落在本区域候选内。
pub fn detect_for(bytes: &[u8], region: Region) -> Encoding {
    if bytes.is_empty() || bytes.iter().all(|&b| b < 0x80) {
        return Encoding::Ascii;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Encoding::Utf8;
    }
    match region {
        // 英文地区：不做多字节探测
        Region::En => Encoding::Other,
        Region::ZhCn => {
            let gbk = gbk_probability(bytes);
            let big5 = big5_probability(bytes);
            if gbk > 50 && gbk >= big5 {
                Encoding::Gbk
            } else {
                Encoding::Other
            }
        }
        Region::ZhTw => {
            // EUC-TW 2 字节区与 GB2312 完全重叠，仅凭 SS2 四字节序列签名判定
            if euc_tw_ss2_count(bytes) >= 2 {
                return Encoding::EucTw;
            }
            let big5 = big5_probability(bytes);
            let gbk = gbk_probability(bytes);
            if big5 > 50 && big5 > gbk {
                Encoding::Big5
            } else {
                Encoding::Other
            }
        }
        Region::Ko => {
            if euc_kr_probability(bytes) > 50 {
                Encoding::EucKr
            } else {
                Encoding::Other
            }
        }
        Region::Ja => {
            if shift_jis_probability(bytes) > 50 {
                if has_cp932_extension(bytes) {
                    Encoding::Cp932
                } else {
                    Encoding::ShiftJis
                }
            } else {
                Encoding::Other
            }
        }
    }
}

/// GBK 可能性打分（0-100），见 `GBKProbability`。
/// 范围校验 + 频度表加权；对短字符串（路径）同样有效。
pub fn gbk_probability(rawtext: &[u8]) -> i32 {
    let n = rawtext.len();
    let mut dbchars: f32 = 1.0;
    let mut gbchars: f32 = 1.0;
    let mut gbfreq: i64 = 0;
    let mut totalfreq: i64 = 1;

    let mut i = 0usize;
    while i + 1 < n {
        let b0 = rawtext[i] as i32;
        let b1 = rawtext[i + 1] as i32;
        if b0 < 0x80 {
            // ASCII，跳过（C# 原文为 sbyte >= 0，u8 域等价于 < 0x80）
        } else {
            dbchars += 1.0;
            // GB2312 区：0xA1-0xF7 / 0xA1-0xFE
            if (0xA1..=0xF7).contains(&b0) && (0xA1..=0xFE).contains(&b1) {
                gbchars += 1.0;
                totalfreq += 500;
                let row = (b0 - 0xA1) as usize;
                let column = (b1 - 0xA1) as usize;
                let f = GB_FREQ[row][column] as i64;
                if f != 0 {
                    gbfreq += f;
                } else if (15..55).contains(&row) {
                    gbfreq += 200;
                }
            }
            // GBK 扩展区：0x81-0xFE / (0x80-0xFE | 0x40-0x7E)
            else if (0x81..=0xFE).contains(&b0)
                && ((0x80..=0xFE).contains(&b1) || (0x40..=0x7E).contains(&b1))
            {
                gbchars += 1.0;
                totalfreq += 500;
                let row = (b0 - 0x81) as usize;
                let column = if (0x40..=0x7E).contains(&b1) {
                    (b1 - 0x40) as usize
                } else {
                    // C# sbyte 域的 `+256-0x80`（-128..-2 → 0..126）在 u8 域等价于 `- 0x80`
                    (b1 - 0x80) as usize
                };
                gbfreq += GBK_FREQ[row][column] as i64;
            }
            i += 1;
        }
        i += 1;
    }

    let rangeval = 50.0 * (gbchars / dbchars);
    let freqval = 50.0 * (gbfreq as f32 / totalfreq as f32);
    (rangeval + freqval) as i32 - 1
}

/// Big5 可能性打分（0-100），见 `BIG5Probability`（早期版本中被停用，
/// 此处按原算法放开）。lead 0xA1-0xF9，trail 0x40-0x7E 或 0xA1-0xFE。
pub fn big5_probability(rawtext: &[u8]) -> i32 {
    let n = rawtext.len();
    let mut dbchars: f32 = 1.0;
    let mut bfchars: f32 = 1.0;
    let mut bffreq: i64 = 0;
    let mut totalfreq: i64 = 1;

    let mut i = 0usize;
    while i + 1 < n {
        let b0 = rawtext[i];
        let b1 = rawtext[i + 1];
        if b0 < 0x80 {
            // ASCII，跳过
        } else {
            dbchars += 1.0;
            if (0xA1..=0xF9).contains(&b0)
                && ((0x40..=0x7E).contains(&b1) || (0xA1..=0xFE).contains(&b1))
            {
                bfchars += 1.0;
                totalfreq += 500;
                let row = (b0 - 0xA1) as usize;
                let column = if (0x40..=0x7E).contains(&b1) {
                    (b1 - 0x40) as usize
                } else {
                    // C# sbyte 域的 `+256-0x61` 在 u8 域等价于 `-0x61`（trail≥0xA1 不下溢）
                    (b1 - 0x61) as usize
                };
                let f = BIG5_FREQ[row][column] as i64;
                if f != 0 {
                    bffreq += f;
                } else if (3..=37).contains(&row) {
                    bffreq += 200;
                }
            }
            i += 1;
        }
        i += 1;
    }

    let rangeval = 50.0 * (bfchars / dbchars);
    let freqval = 50.0 * (bffreq as f32 / totalfreq as f32);
    (rangeval + freqval) as i32
}

/// 通用解码脚本占比：用指定编码解码，返回"命中期望文字集的字符数 / 非 ASCII 字符数"。
/// 解码报错（非法序列）返回 None。
fn decode_script_ratio(
    bytes: &[u8],
    enc: &'static encoding_rs::Encoding,
    expected: fn(char) -> bool,
) -> Option<f32> {
    let (text, _, had_errors) = enc.decode(bytes);
    if had_errors {
        return None;
    }
    let mut non_ascii = 0usize;
    let mut hits = 0usize;
    for c in text.chars() {
        if !c.is_ascii() {
            non_ascii += 1;
            if expected(c) {
                hits += 1;
            }
        }
    }
    if non_ascii == 0 {
        return None;
    }
    Some(hits as f32 / non_ascii as f32)
}

fn is_hangul(c: char) -> bool {
    matches!(c as u32,
        0xAC00..=0xD7A3 // Hangul 音节
        | 0x1100..=0x11FF // Hangul Jamo
        | 0x3130..=0x318F // Hangul 兼容字母
    )
}

fn is_japanese(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x309F // 平假名
        | 0x30A0..=0x30FF // 片假名
        | 0x31F0..=0x31FF // 片假名扩展
        | 0xFF61..=0xFF9F // 半角片假名
        | 0x4E00..=0x9FFF // 汉字（日文汉字属 CJK 统一区）
        | 0x3000..=0x303F // 全角标点
    )
}

/// EUC-KR / CP949 可能性打分（0-100）。按同构打分法实现：
/// 结构分（合法双字节占比）+ 脚本分（解码后 Hangul 占比）。
/// lead 0x81-0xFE，trail 0x41-0xFE（不含 0x7F）。
pub fn euc_kr_probability(rawtext: &[u8]) -> i32 {
    let n = rawtext.len();
    let mut dbchars: f32 = 1.0;
    let mut ok: f32 = 1.0;
    let mut i = 0usize;
    while i + 1 < n {
        let b0 = rawtext[i];
        let b1 = rawtext[i + 1];
        if b0 >= 0x80 {
            dbchars += 1.0;
            if (0x81..=0xFE).contains(&b0) && (0x41..=0xFE).contains(&b1) && b1 != 0x7F {
                ok += 1.0;
            }
            i += 1;
        }
        i += 1;
    }
    let rangeval = 50.0 * (ok / dbchars);
    let scriptval = decode_script_ratio(rawtext, encoding_rs::EUC_KR, is_hangul)
        .map(|r| 50.0 * r)
        .unwrap_or(0.0);
    (rangeval + scriptval) as i32
}

/// Shift_JIS / CP932 可能性打分（0-100）。按同构打分法实现：
/// 结构分（合法双字节占比）+ 脚本分（解码后假名/日文汉字占比）。
/// lead 0x81-0x9F / 0xE0-0xFC（CP932 IBM 扩展），trail 0x40-0x7E / 0x80-0xFC（不含 0x7F）。
/// 注意：单字节 0xA1-0xDF（半角片假名）不计入结构分——GBK 中文大量落在该区，
/// 计入会把简体中文误判成 Shift_JIS。
pub fn shift_jis_probability(rawtext: &[u8]) -> i32 {
    let n = rawtext.len();
    let mut dbchars: f32 = 1.0;
    let mut ok: f32 = 1.0;
    let mut i = 0usize;
    while i + 1 < n {
        let b0 = rawtext[i];
        let b1 = rawtext[i + 1];
        if b0 >= 0x80 {
            dbchars += 1.0;
            let lead_ok = (0x81..=0x9F).contains(&b0) || (0xE0..=0xFC).contains(&b0);
            let trail_ok = ((0x40..=0x7E).contains(&b1) || (0x80..=0xFC).contains(&b1))
                && b1 != 0x7F;
            if lead_ok && trail_ok {
                ok += 1.0;
                i += 1; // 双字节成对，跳过 trail
            }
        }
        i += 1;
    }
    let rangeval = 50.0 * (ok / dbchars);
    let scriptval = decode_script_ratio(rawtext, encoding_rs::SHIFT_JIS, is_japanese)
        .map(|r| 50.0 * r)
        .unwrap_or(0.0);
    (rangeval + scriptval) as i32
}

/// 是否含 CP932 独有的 NEC/IBM 扩展区字节（Shift_JIS(JIS X 0208) 不使用）：
/// NEC 選択 0x87-0x88 行、NEC IBM 0xEB-0xEE 行、IBM 拡張 0xFA-0xFC 行。
pub fn has_cp932_extension(rawtext: &[u8]) -> bool {
    let n = rawtext.len();
    let mut i = 0usize;
    while i + 1 < n {
        let b0 = rawtext[i];
        if b0 < 0x80 {
            i += 1;
            continue;
        }
        let b1 = rawtext[i + 1];
        let hit = matches!(b0, 0x87..=0x88 | 0xEB..=0xEE | 0xFA..=0xFC)
            && (0x40..=0xFC).contains(&b1)
            && b1 != 0x7F;
        if hit {
            return true;
        }
        i += 2;
    }
    false
}

/// EUC-TW 的 SS2 四字节序列计数：`0x8E + 面号(0xA1-0xB0) + 0xA1-0xFE + 0xA1-0xFE`。
/// EUC-TW 的 2 字节区与 GB2312 完全重叠、无法凭字节区分，SS2 序列是其唯一可靠
/// 签名（GBK/Big5 正常文本几乎不会出现连续的该模式）。
pub fn euc_tw_ss2_count(rawtext: &[u8]) -> usize {
    let n = rawtext.len();
    let mut i = 0usize;
    let mut count = 0usize;
    while i + 3 < n {
        if rawtext[i] == 0x8E
            && (0xA1..=0xB0).contains(&rawtext[i + 1])
            && (0xA1..=0xFE).contains(&rawtext[i + 2])
            && (0xA1..=0xFE).contains(&rawtext[i + 3])
        {
            count += 1;
            i += 4;
        } else {
            i += 1;
        }
    }
    count
}

/// 判断字符串是否含 CJK/韩文/假名字符（已正常解码的标志）。
pub fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32,
            0x4E00..=0x9FFF   // CJK 统一表意
            | 0x3400..=0x4DBF // 扩展 A
            | 0x3000..=0x303F // CJK 标点
            | 0xFF00..=0xFFEF // 全角字符
            | 0x20000..=0x2A6DF // 扩展 B
            | 0xAC00..=0xD7A3 // Hangul 音节（韩语地区）
            | 0x3040..=0x30FF // 平假名/片假名（日语地区）
        )
    })
}

/// 路径归一化（字符串入口，按当前环境区域探测，见 [`Region::from_env`]）。
pub fn fix_path(s: &str) -> String {
    fix_path_for(s, Region::from_env())
}

/// 路径归一化（显式指定区域）。
///
/// * 纯 ASCII → 原样返回（已正确）。
/// * 含 CJK/Hangul/假名 → 先试形态 2（UTF-8 被 GBK 误读，如 `娴嬭瘯`→`测试`），
///   不成立则原样返回。
/// * 全串字符 ≤ U+00FF → 形态 1（传统编码→Latin-1），按字节还原并按**区域候选
///   编码**（打分高者优先）尝试解码；解码成功且含 CJK/Hangul/假名才采纳，
///   否则原样返回。EUC-TW 无解码器（encoding_rs 不支持），跳过。
pub fn fix_path_for(s: &str, region: Region) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    if contains_cjk(s) {
        // 形态 2：字符串本身是 CJK 乱码（UTF-8 字节被 GBK 误读的结果）
        if let Some(fixed) = fix_utf8_read_as_gbk(s) {
            return fixed;
        }
        return s.to_string();
    }
    // 形态 1：全部字符都在 Latin-1 范围内才可能是一次无损的逐字节误读
    let mut bytes = Vec::with_capacity(s.len());
    let all_latin1 = s.chars().all(|c| (c as u32) <= 0xFF);
    if !all_latin1 {
        return s.to_string();
    }
    bytes.extend(s.chars().map(|c| c as u8));

    // 还原出的字节流若本身是合法 UTF-8，说明是被 UTF-8 通道吞掉后再吐出来的，
    // 原样返回即可（不会走到这里，因为 contains_cjk 已拦截，但保守保留判断）。
    if let Ok(u) = std::str::from_utf8(&bytes) {
        if contains_cjk(u) {
            return u.to_string();
        }
    }

    // 区域候选编码，按打分高者优先尝试解码；解码成功且含 CJK/Hangul/假名才采纳
    let mut cands: Vec<(i32, &'static encoding_rs::Encoding)> = region
        .legacy()
        .iter()
        .filter_map(|l| {
            let (score, enc) = match l {
                Legacy::Gbk => (gbk_probability(&bytes), encoding_rs::GBK),
                Legacy::Big5 => (big5_probability(&bytes), encoding_rs::BIG5),
                Legacy::EucKr => (euc_kr_probability(&bytes), encoding_rs::EUC_KR),
                Legacy::Sjis => (shift_jis_probability(&bytes), encoding_rs::SHIFT_JIS),
                Legacy::EucTw => return None, // encoding_rs 不支持 EUC-TW
            };
            Some((-score, enc))
        })
        .collect();
    cands.sort_by_key(|(neg_score, _)| *neg_score);
    for (_, enc) in &cands {
        let (decoded, _, had_errors) = enc.decode(&bytes);
        if !had_errors && contains_cjk(&decoded) {
            return decoded.into_owned();
        }
    }
    s.to_string()
}

/// 形态 2 修复：UTF-8 字节被 GBK 误读（字符串含 CJK 但是乱码）。
///
/// 判定：字符串可无损 GBK 编码，且编码结果恰好是合法 UTF-8、解码后含 CJK。
/// 保守约束：字节数 ≥ 6（至少 3 个汉字的量级才可能构成混淆），且结果与原文不同。
/// 正常中文（如 `工具`）GBK 字节解码为 UTF-8 得到的是非 CJK 字符，不会误伤。
fn fix_utf8_read_as_gbk(s: &str) -> Option<String> {
    let (bytes, _, had_errors) = encoding_rs::GBK.encode(s);
    if had_errors || bytes.len() < 6 {
        return None;
    }
    let decoded = std::str::from_utf8(&bytes).ok()?;
    if decoded == s || !contains_cjk(decoded) {
        return None;
    }
    Some(decoded.to_string())
}

/// 路径归一化（原始字节入口，供 FFI/OS API 侧使用；按当前环境区域）。
///
/// 合法 UTF-8 → 走 [`fix_path`] 逻辑；否则按区域传统编码依次解码（全部失败
/// 回退 GBK 替换解码，保证总有输出）。
pub fn fix_path_bytes(bytes: &[u8]) -> String {
    fix_path_bytes_for(bytes, Region::from_env())
}

/// 路径归一化（原始字节 + 显式区域）。
pub fn fix_path_bytes_for(bytes: &[u8], region: Region) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => fix_path_for(s, region),
        Err(_) => {
            // 非 UTF-8 字节：按区域候选依次解码，取首个无错结果；否则 GBK 替换解码兜底
            let cands: Vec<&'static encoding_rs::Encoding> = region
                .legacy()
                .iter()
                .filter_map(|l| match l {
                    Legacy::Gbk => Some(encoding_rs::GBK),
                    Legacy::Big5 => Some(encoding_rs::BIG5),
                    Legacy::EucKr => Some(encoding_rs::EUC_KR),
                    Legacy::Sjis => Some(encoding_rs::SHIFT_JIS),
                    Legacy::EucTw => None,
                })
                .collect();
            for enc in &cands {
                let (decoded, _, had_errors) = enc.decode(bytes);
                if !had_errors {
                    return decoded.into_owned();
                }
            }
            let (decoded, _, _) = encoding_rs::GBK.decode(bytes);
            decoded.into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_ascii_utf8() {
        assert_eq!(detect(b"C:/data/layer.shp"), Encoding::Ascii);
        assert_eq!(detect("G:/数据/昆明.shp".as_bytes()), Encoding::Utf8);
    }

    #[test]
    fn detect_gbk_bytes() {
        // "工具" 的 GBK 字节（显式指定简体中文区域，测试不依赖机器 locale）
        let gbk_bytes = [0xB9u8, 0xA4, 0xBE, 0xDF];
        assert_eq!(detect_for(&gbk_bytes, Region::ZhCn), Encoding::Gbk);
        // "昆明市五华区" 的 GBK
        let (enc, _, _) = encoding_rs::GBK.encode("昆明市五华区");
        assert_eq!(detect_for(&enc, Region::ZhCn), Encoding::Gbk);
    }

    #[test]
    fn fix_mojibake_latin1() {
        // GBK 字节 0xB9 0xA4 0xBE 0xDF 被误读为 Latin-1 → "¹¤¾ß"
        let moji: String = [0xB9u8, 0xA4, 0xBE, 0xDF]
            .iter()
            .map(|&b| b as char)
            .collect();
        assert_eq!(moji, "\u{B9}\u{A4}\u{BE}\u{DF}");
        assert_eq!(fix_path_for(&moji, Region::ZhCn), "工具");
        assert_eq!(
            fix_path_for(&format!("G:/{}A/{}.gdb", moji, moji), Region::ZhCn),
            "G:/工具A/工具.gdb"
        );
    }

    #[test]
    fn fix_real_world_path() {
        // "G:/net/CLGT/数据成果(实验)" 的 GBK 字节被逐字节 Latin-1 误读
        let orig = r"G:/net/CLGT/数据成果(实验)/数据成果(实验)/530100昆明市CSFWJZ2024.gdb";
        let gbk = encoding_rs::GBK.encode(orig).0;
        let moji: String = gbk.iter().map(|&b| b as char).collect();
        assert_ne!(moji, orig);
        assert_eq!(fix_path_for(&moji, Region::ZhCn), orig);
    }

    #[test]
    fn fix_keeps_correct_paths() {
        assert_eq!(fix_path("G:/工具/ArcGIS中国工具2/data/my.gdb"), "G:/工具/ArcGIS中国工具2/data/my.gdb");
        assert_eq!(fix_path("D:/plain/path.shp"), "D:/plain/path.shp");
        // 含非 Latin-1 的非 CJK 字符（如西里尔/emoji）不冒险改写
        assert_eq!(fix_path("D:/data/путь/ток.shp"), "D:/data/путь/ток.shp");
    }

    #[test]
    fn fix_path_bytes_both_ways() {
        assert_eq!(fix_path_bytes_for("G:/数据/a.gdb".as_bytes(), Region::ZhCn), "G:/数据/a.gdb");
        let gbk = encoding_rs::GBK.encode("G:/数据/a.gdb").0;
        assert_eq!(fix_path_bytes_for(&gbk, Region::ZhCn), "G:/数据/a.gdb");
    }

    #[test]
    fn fix_form2_real() {
        // 构造：原文本 UTF-8 字节 → 按 GBK 解码 → 得到 CJK 乱码字符串
        fn make_moji(orig: &str) -> String {
            let (d, _, _) = encoding_rs::GBK.decode(orig.as_bytes());
            d.into_owned()
        }
        let moji = make_moji("D:/测试/文件.gdb");
        assert!(contains_cjk(&moji));
        assert_eq!(fix_path(&moji), "D:/测试/文件.gdb");

        // 正常中文路径不受影响（GBK 字节按 UTF-8 解不出 CJK）
        assert_eq!(fix_path("G:/工具/ArcGIS中国工具2/data/my.gdb"),
                   "G:/工具/ArcGIS中国工具2/data/my.gdb");
        assert_eq!(fix_path("D:/昆明市盘龙区/建筑物.shp"),
                   "D:/昆明市盘龙区/建筑物.shp");
    }

    #[test]
    fn detect_big5_vs_gbk_by_region() {
        // 繁体 BIG5："臺北市大地圖" —— 简体中文区域不返回 BIG5（区域外候选），
        // 繁体中文区域判 Big5
        let (big5, _, had_err) = encoding_rs::BIG5.encode("臺北市大地圖");
        assert!(!had_err);
        assert_ne!(detect_for(&big5, Region::ZhCn), Encoding::Big5);
        assert_eq!(detect_for(&big5, Region::ZhTw), Encoding::Big5);
        // 简体 GBK 在简体区域仍判 GBK（不因区域模型改动而回归）
        let (gbk, _, _) = encoding_rs::GBK.encode("昆明市盘龙区建筑物");
        assert_eq!(detect_for(&gbk, Region::ZhCn), Encoding::Gbk);
    }

    #[test]
    fn detect_euc_kr_ko_region() {
        let (kr, _, had_err) = encoding_rs::EUC_KR.encode("서울특별시 강남구");
        assert!(!had_err);
        assert_eq!(detect_for(&kr, Region::Ko), Encoding::EucKr);
        // 简体中文区域不探测 EUC-KR（区域外候选）
        assert_ne!(detect_for(&kr, Region::ZhCn), Encoding::EucKr);
    }

    #[test]
    fn detect_shift_jis_ja_region() {
        let (sj, _, had_err) = encoding_rs::SHIFT_JIS.encode("東京都新宿区渋谷区");
        assert!(!had_err);
        assert_eq!(detect_for(&sj, Region::Ja), Encoding::ShiftJis);
        // 简体中文区域不探测 Shift_JIS
        assert_ne!(detect_for(&sj, Region::ZhCn), Encoding::ShiftJis);
    }

    #[test]
    fn detect_cp932_extension_bytes() {
        // CP932 NEC 扩展区：① = 0x87 0x41（Shift_JIS JIS X 0208 不使用该行）
        let bytes = [0x87u8, 0x41, 0x93, 0x8C, 0x8B, 0x9E]; // "①東京"
        assert!(has_cp932_extension(&bytes));
        assert_eq!(detect_for(&bytes, Region::Ja), Encoding::Cp932);
        // 无扩展区的普通日文仍是 SHIFT_JIS（上面用例已覆盖）
    }

    #[test]
    fn detect_euc_tw_by_ss2_signature() {
        // EUC-TW SS2 四字节序列：0x8E + 面号(0xA1-0xB0) + 0xA1-0xFE × 2
        let bytes = [0x8Eu8, 0xA2, 0xA1, 0xA1, 0x8E, 0xA2, 0xA1, 0xA2];
        assert_eq!(euc_tw_ss2_count(&bytes), 2);
        assert_eq!(detect_for(&bytes, Region::ZhTw), Encoding::EucTw);
        // 常规简体 GBK 文本不含 SS2 序列，不会被误判
        let (gbk, _, _) = encoding_rs::GBK.encode("昆明市盘龙区建筑物");
        assert_eq!(euc_tw_ss2_count(&gbk), 0);
    }

    #[test]
    fn detect_en_region_no_legacy_probe() {
        // 英文地区只认 UTF-8/ASCII，GBK 字节判 Other
        let (gbk, _, _) = encoding_rs::GBK.encode("昆明市盘龙区建筑物");
        assert_eq!(detect_for(&gbk, Region::En), Encoding::Other);
        assert_eq!(detect_for(b"plain ascii path", Region::En), Encoding::Ascii);
        assert_eq!(
            detect_for("G:/数据/a.gdb".as_bytes(), Region::En),
            Encoding::Utf8
        );
    }

    #[test]
    fn region_from_locale() {
        assert_eq!(Region::from_locale("zh_CN.UTF-8"), Region::ZhCn);
        assert_eq!(Region::from_locale("zh_TW.Big5"), Region::ZhTw);
        assert_eq!(Region::from_locale("zh_HK"), Region::ZhTw);
        assert_eq!(Region::from_locale("ko_KR"), Region::Ko);
        assert_eq!(Region::from_locale("ja_JP.UTF-8"), Region::Ja);
        assert_eq!(Region::from_locale("en_US.UTF-8"), Region::En);
        assert_eq!(Region::from_locale("Japanese_Japan.932"), Region::Ja);
        // 未识别 → 当前部署默认简体中文
        assert_eq!(Region::from_locale("fr_FR"), Region::ZhCn);
    }

    #[test]
    fn fix_mojibake_big5() {
        // Big5 路径的 Latin-1 误读形态："地圖" 的 Big5 字节 a6 61 b9 cf
        // —— 按区域模型属繁体中文地区
        let moji: String = [0xA6u8, 0x61, 0xB9, 0xCF].iter().map(|&b| b as char).collect();
        assert_eq!(fix_path_for(&moji, Region::ZhTw), "地圖");
    }

    #[test]
    fn fix_mojibake_euc_kr() {
        // 韩语地区："서울" 的 EUC-KR 字节 bc ad bf ef 被 Latin-1 误读
        let (kr, _, _) = encoding_rs::EUC_KR.encode("서울");
        let moji: String = kr.iter().map(|&b| b as char).collect();
        assert_eq!(fix_path_for(&moji, Region::Ko), "서울");
    }

    #[test]
    fn gbk_score_shape() {
        // ASCII 路径 GBK 分数应低（不误判）
        assert!(gbk_probability(b"C:/data/layer.shp") <= 50);
        // 中文 GBK 路径分数应高
        let (enc, _, _) = encoding_rs::GBK.encode("昆明市盘龙区建筑物");
        assert!(gbk_probability(&enc) > 50);
    }
}
