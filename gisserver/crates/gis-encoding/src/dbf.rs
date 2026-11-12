//! shp 属性表（DBF）编码检测（语言区域驱动）。
//!
//! 背景：DBF 的编码声明有三级——`.cpg` 旁文件 > DBF 头 LDID 字节 > 默认假设。
//! 各国生产的 shp 常见缺 `.cpg` 且 LDID 为 0x00/0x57（GDAL 按 CP1252 解释），
//! 非 ASCII 属性/字段名直接乱码。GDAL 可用 `SHAPE_ENCODING` 配置强制指定编码，
//! 本模块在交给 GDAL 前检测并给出建议值。
//!
//! 检测策略（保守，候选由区域决定，见 [`crate::Region`]）：
//! 1. 有 `.cpg` → 交给 GDAL，不干预（`suggested = None`）；
//! 2. 无 `.cpg` → 读 DBF 字段描述符里的字符型字段定义 + 前若干条记录的
//!    字符字段字节做采样：合法 UTF-8 且含 CJK/Hangul/假名 → `UTF-8`；
//!    否则按区域传统编码打分过阈值才建议，其余不干预。
//!    EUC-TW 因 GDAL 侧重编码支持不确定，不做建议（只参与 detect）。

use std::path::Path;

use super::{contains_cjk, Region};

/// 检测结果。
#[derive(Debug, Clone)]
pub struct ShpEncodingHint {
    /// `.cpg` 文件内容（trim 后），None 表示不存在。
    pub cpg: Option<String>,
    /// DBF 头偏移 29 的语言驱动 ID（0 表示未声明）。
    pub ldid: u8,
    /// 建议 GDAL 使用的编码（None = 不干预，按 GDAL 默认逻辑）。
    pub suggested: Option<&'static str>,
    /// 供 `shp2pgsql -W` 使用的等价编码名（None = 不干预）。
    ///
    /// 与 [`Self::suggested`] 分开的原因：各工具认的编码名不统一——
    /// * GDAL `SHAPE_ENCODING`：`GBK` / `BIG5` / `EUC-KR` / `CP932` / `SHIFT_JIS`；
    /// * `shp2pgsql -W`（iconv 名）：`GBK` / `GB18030` / `BIG5` / `EUC-KR`，
    ///   **不接受** `EUC_KR` / `CP932` / `SHIFT_JIS` / `SJIS`。
    ///
    /// 另注：`shp2pgsql` 完全不读 `.cpg`（实测：有 `.cpg` 仍按 UTF-8 解析并报错），
    /// 所以这条链路必须显式传 `-W`，不能依赖 GDAL 的 `.cpg` 优先逻辑。
    pub suggested_shp2pgsql: Option<&'static str>,
}

/// 把内部检测用的编码名映射为 GDAL `SHAPE_ENCODING` / `shp2pgsql -W` 认的名字。
///
/// 返回 `(gdal_name, shp2pgsql_name)`；`None` 表示该编码不建议对外干预。
fn encoding_aliases(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "UTF-8" => Some(("UTF-8", "UTF-8")),
        "GBK" => Some(("GBK", "GBK")),
        "BIG5" => Some(("BIG5", "BIG5")),
        // iconv 认 "EUC-KR"，不认 "EUC_KR"
        "EUC-KR" => Some(("EUC-KR", "EUC-KR")),
        // GDAL 认 "CP932"；iconv 侧用 "SHIFT_JIS" 家族名最稳妥
        "CP932" => Some(("CP932", "SHIFT_JIS")),
        "SHIFT_JIS" => Some(("SHIFT_JIS", "SHIFT_JIS")),
        _ => None,
    }
}

/// 从采样字节给出建议编码（纯函数，便于单测；候选由区域决定）。
pub fn suggest_from_sample(sample: &[u8], region: Region) -> Option<&'static str> {
    if sample.iter().all(|&b| b < 0x80) {
        return None; // 纯 ASCII，无需干预
    }
    if let Ok(s) = std::str::from_utf8(sample) {
        if contains_cjk(s) {
            return Some("UTF-8");
        }
        // 合法 UTF-8 但无 CJK/Hangul/假名（如西文），不动
        return None;
    }
    match region {
        // 英文地区：不做传统编码探测
        Region::En => None,
        Region::ZhCn => {
            // Big5 与 GBK 区间重叠，跨区分数只作否决 guard
            let gbk = super::gbk_probability(sample);
            let big5 = super::big5_probability(sample);
            (gbk >= 60 && gbk >= big5).then_some("GBK")
        }
        Region::ZhTw => {
            // EUC-TW 无法经 GDAL SHAPE_ENCODING 可靠重编码，不干预
            if super::euc_tw_ss2_count(sample) >= 2 {
                return None;
            }
            let big5 = super::big5_probability(sample);
            let gbk = super::gbk_probability(sample);
            (big5 >= 60 && big5 > gbk).then_some("BIG5")
        }
        Region::Ko => {
            let kr = super::euc_kr_probability(sample);
            (kr >= 60).then_some("EUC-KR")
        }
        Region::Ja => {
            let sj = super::shift_jis_probability(sample);
            if sj < 60 {
                return None;
            }
            // 含 NEC/IBM 扩展区字节 → CP932（Windows 标准日文编码）
            if super::has_cp932_extension(sample) {
                Some("CP932")
            } else {
                Some("SHIFT_JIS")
            }
        }
    }
}

/// 检测一份 shp（或直接给 .dbf 路径）的属性编码（按当前环境区域）。
/// IO/解析失败一律返回 Err，由调用方决定忽略。
pub fn detect_shp(shp_or_dbf: &Path) -> std::io::Result<ShpEncodingHint> {
    detect_shp_for(shp_or_dbf, Region::from_env())
}

/// 检测一份 shp（或直接给 .dbf 路径）的属性编码（显式指定区域）。
pub fn detect_shp_for(shp_or_dbf: &Path, region: Region) -> std::io::Result<ShpEncodingHint> {
    let stem = shp_or_dbf
        .parent()
        .unwrap_or(Path::new("."))
        .join(shp_or_dbf.file_stem().ok_or_else(|| not_found("no file stem"))?);

    let dbf_path = find_sibling(&stem, &["dbf", "DBF"])?;
    let cpg_path = find_sibling(&stem, &["cpg", "CPG"])?;

    let cpg = match cpg_path {
        Some(p) => Some(std::fs::read_to_string(&p)?.trim().to_string()),
        None => None,
    };

    let mut hint = ShpEncodingHint {
        cpg,
        ldid: 0,
        suggested: None,
        suggested_shp2pgsql: None,
    };
    let Some(dbf_path) = dbf_path else {
        return Ok(hint);
    };

    let data = std::fs::read(&dbf_path)?;
    if data.len() < 32 {
        return Err(not_found("dbf too small"));
    }
    hint.ldid = data[29];
    let header_size = u16::from_le_bytes([data[8], data[9]]) as usize;
    let record_size = u16::from_le_bytes([data[10], data[11]]) as usize;
    if header_size < 33 || record_size == 0 || data.len() <= header_size {
        return Ok(hint); // 结构异常，不干预
    }

    // 字段描述符：偏移 32 起，每项 32 字节，0x0D 结束
    struct CharField {
        offset: usize, // 记录内偏移（含 1 字节删除标志之后）
        len: usize,
    }
    let mut char_fields: Vec<CharField> = Vec::new();
    let mut name_sample: Vec<u8> = Vec::new();
    let mut pos = 32usize;
    let mut rec_off = 1usize; // 记录首字节是删除标志
    while pos + 32 <= data.len() && data[pos] != 0x0D {
        let ftype = data[pos + 11];
        let flen = data[pos + 16] as usize;
        if ftype == b'C' && flen > 0 {
            char_fields.push(CharField { offset: rec_off, len: flen });
            // 字段名也采样（中文字段名常见；10 字节名可能截断双字节，打分可容忍）
            name_sample.extend_from_slice(&data[pos..pos + 11]);
        }
        rec_off += flen;
        pos += 32;
    }
    if char_fields.is_empty() {
        return Ok(hint);
    }

    // 采样前 100 条记录的字符字段内容（上限 4KB）
    let mut sample: Vec<u8> = name_sample;
    let mut rec = header_size;
    for _ in 0..100 {
        if rec + record_size > data.len() {
            break;
        }
        for f in &char_fields {
            let end = (rec + f.offset + f.len).min(data.len());
            if rec + f.offset < end {
                sample.extend_from_slice(&data[rec + f.offset..end]);
            }
        }
        rec += record_size;
        if sample.len() > 4096 {
            break;
        }
    }

    // GDAL 链路：`.cpg` 存在时 GDAL 自行读取，且其优先级高于 `SHAPE_ENCODING`——
    // 若此时再设 SHAPE_ENCODING 反而会覆盖成错误编码（实测），故不干预。
    hint.suggested = if hint.cpg.is_some() {
        None
    } else {
        suggest_from_sample(&sample, region)
    };
    // shp2pgsql 链路：它根本不读 `.cpg`（实测：有 .cpg 仍按 UTF-8 解析并报错），
    // 必须显式传 `-W`。取值优先级：`.cpg` 声明（生产方权威）→ 采样判定。
    hint.suggested_shp2pgsql = hint
        .cpg
        .as_deref()
        .and_then(cpg_to_iconv_name)
        .or_else(|| suggest_from_sample(&sample, region).and_then(|n| encoding_aliases(n).map(|(_, s)| s)));
    Ok(hint)
}

/// `.cpg` 内容 → `shp2pgsql -W` 认的 iconv 编码名。
/// 只认白名单里的名字，避免把任意文本当编码名传给外部工具。
///
/// 也用于「调用方显式指定编码」的场景（此时把用户输入当作编码名做同样归一）。
pub fn cpg_to_iconv_name(cpg: &str) -> Option<&'static str> {
    let n = cpg.trim().to_ascii_uppercase();
    Some(match n.as_str() {
        "UTF-8" | "UTF8" => "UTF-8",
        "GBK" | "GB2312" | "CP936" => "GBK",
        "GB18030" => "GB18030",
        "BIG5" | "BIG-5" | "CP950" => "BIG5",
        "EUC-KR" | "EUC_KR" | "CP949" => "EUC-KR",
        "SHIFT_JIS" | "SHIFT-JIS" | "SJIS" | "CP932" => "SHIFT_JIS",
        _ => return None,
    })
}

fn find_sibling(stem: &Path, exts: &[&str]) -> std::io::Result<Option<std::path::PathBuf>> {
    for e in exts {
        let p = stem.with_extension(e);
        if p.is_file() {
            return Ok(Some(p));
        }
    }
    // 大小写兜底：逐目录项比对（覆盖 DBF / Dbf 等形态）
    if let Some(dir) = stem.parent() {
        let stem_lc = stem.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if exts.iter().any(|e| name == format!("{}.{}", stem_lc, e)) {
                    return Ok(Some(entry.path()));
                }
            }
        }
    }
    Ok(None)
}

fn not_found(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_ascii_noop() {
        assert_eq!(suggest_from_sample(b"hello world 123", Region::ZhCn), None);
        assert_eq!(suggest_from_sample(b"", Region::ZhCn), None);
    }

    #[test]
    fn suggest_gbk() {
        let (enc, _, _) = encoding_rs::GBK.encode("昆明市五华区地名地址");
        assert_eq!(suggest_from_sample(&enc, Region::ZhCn), Some("GBK"));
    }

    #[test]
    fn suggest_utf8() {
        assert_eq!(
            suggest_from_sample("昆明市五华区地名地址".as_bytes(), Region::ZhCn),
            Some("UTF-8")
        );
    }

    #[test]
    fn suggest_european_not_gbk() {
        // 西文 CP1252 文本不应被误判为 GBK
        assert_eq!(suggest_from_sample(b"Caf\xe9 r\xe9sum\xe9 na\xefve", Region::ZhCn), None);
    }

    #[test]
    fn suggest_big5() {
        let (enc, _, had_err) = encoding_rs::BIG5.encode("臺北市大地圖測站資料");
        assert!(!had_err);
        assert_eq!(suggest_from_sample(&enc, Region::ZhTw), Some("BIG5"));
        // 简体中文区域不探测 Big5（区域外候选）
        assert_eq!(suggest_from_sample(&enc, Region::ZhCn), None);
    }

    #[test]
    fn suggest_euc_kr() {
        let (enc, _, had_err) = encoding_rs::EUC_KR.encode("서울특별시 강남구 도로명주소");
        assert!(!had_err);
        assert_eq!(suggest_from_sample(&enc, Region::Ko), Some("EUC-KR"));
        // 区域模型的固有局限：韩文字节在简体中文区域会被 GBK 认领（GB2312 区重叠）
        assert_eq!(suggest_from_sample(&enc, Region::ZhCn), Some("GBK"));
    }

    #[test]
    fn suggest_shift_jis_and_cp932() {
        let (sj, _, had_err) = encoding_rs::SHIFT_JIS.encode("東京都新宿区地名");
        assert!(!had_err);
        assert_eq!(suggest_from_sample(&sj, Region::Ja), Some("SHIFT_JIS"));
        // 含 NEC 扩展区字节（① = 0x87 0x41）→ 建议 CP932
        let mut with_ext = vec![0x87u8, 0x41];
        with_ext.extend_from_slice(&sj);
        assert_eq!(suggest_from_sample(&with_ext, Region::Ja), Some("CP932"));
        assert_eq!(suggest_from_sample(&sj, Region::ZhCn), None);
    }

    #[test]
    fn detect_synthetic_dbf() {
        // 手工构造最小 DBF：32B 头 + 1 个 C 字段描述符 + 0x0D + 2 条记录
        let mut dbf: Vec<u8> = vec![0x03; 32];
        dbf[4..8].copy_from_slice(&2u32.to_le_bytes()); // 记录数
        let header_size = 32 + 32 + 1;
        dbf[8..10].copy_from_slice(&(header_size as u16).to_le_bytes());
        dbf[10..12].copy_from_slice(&13u16.to_le_bytes()); // 记录大小：1 + 12
        dbf[29] = 0x57; // LDID
        // 字段描述符：名字(11) 类型(1) ... 长度@16
        let mut desc = [0u8; 32];
        desc[..4].copy_from_slice("DZMC".as_bytes());
        desc[11] = b'C';
        desc[16] = 12; // 字段长 12（容纳"昆明市盘龙区"完整 GBK）
        dbf.extend_from_slice(&desc);
        dbf.push(0x0D);
        // 记录：删除标志 + "昆明市盘龙区" 的 GBK（12 字节）
        let (gbk, _, _) = encoding_rs::GBK.encode("昆明市盘龙区");
        assert_eq!(gbk.len(), 12);
        for _ in 0..2 {
            dbf.push(b' '); // 未删除
            dbf.extend_from_slice(&gbk);
        }
        let dir = std::env::temp_dir().join("gis_enc_test");
        let _ = std::fs::remove_dir_all(&dir); // 清掉上次运行的残留（t.cpg 等）
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.shp");
        std::fs::write(dir.join("t.dbf"), &dbf).unwrap();
        // 无 .cpg（显式指定简体中文区域，测试不依赖机器 locale）
        let h = detect_shp_for(&p, Region::ZhCn).unwrap();
        assert_eq!(h.cpg, None);
        assert_eq!(h.ldid, 0x57);
        assert_eq!(h.suggested, Some("GBK"));

        // 写入 .cpg 后应不干预
        std::fs::write(dir.join("t.cpg"), "GBK\n").unwrap();
        let h2 = detect_shp(&p).unwrap();
        assert_eq!(h2.cpg.as_deref(), Some("GBK"));
        assert_eq!(h2.suggested, None);
    }

    /// 构造一份最小 DBF：32B 头 + N 个 C 字段描述符 + 0x0D + 若干记录。
    /// `fields` 为 (字段名, 字段内容) 列表，名字与内容都按 `enc` 编码。
    fn build_dbf(enc: &'static encoding_rs::Encoding, fields: &[(&str, &str)], rows: usize) -> Vec<u8> {
        let mut dbf: Vec<u8> = vec![0x03; 32];
        dbf[4..8].copy_from_slice(&(rows as u32).to_le_bytes());
        dbf[29] = 0x00; // LDID：未声明编码
        let header_size = 32 + 32 * fields.len() + 1;
        dbf[8..10].copy_from_slice(&(header_size as u16).to_le_bytes());
        let mut rec_len = 1usize;
        for (name, _) in fields {
            let (_, _, _) = enc.encode(name);
            rec_len += 32; // 占位，稍后按实际长度修正
        }
        let _ = rec_len;
        let mut descs: Vec<u8> = Vec::new();
        let mut record_size = 1usize;
        for (name, content) in fields {
            let (nb, _, _) = enc.encode(name);
            let (cb, _, _) = enc.encode(content);
            let mut desc = [0u8; 32];
            let n = nb.len().min(10);
            desc[..n].copy_from_slice(&nb[..n]);
            desc[11] = b'C';
            desc[16] = cb.len() as u8;
            descs.extend_from_slice(&desc);
            record_size += cb.len();
        }
        dbf[10..12].copy_from_slice(&(record_size as u16).to_le_bytes());
        dbf.extend_from_slice(&descs);
        dbf.push(0x0D);
        for _ in 0..rows {
            dbf.push(b' ');
            for (_, content) in fields {
                let (cb, _, _) = enc.encode(content);
                dbf.extend_from_slice(&cb);
            }
        }
        dbf
    }

    #[test]
    fn real_gbk_dbf_two_channels() {
        // 真实场景：字段名与内容都是 GBK，无 .cpg、LDID=0
        let dbf = build_dbf(
            encoding_rs::GBK,
            &[("站名", "昆明市级行政中心"), ("地址", "呈贡区锦绣大街1号")],
            3,
        );
        let dir = std::env::temp_dir().join("gis_enc_test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.shp");
        std::fs::write(dir.join("t.dbf"), &dbf).unwrap();

        let h = detect_shp_for(&p, Region::ZhCn).unwrap();
        // GDAL 链路：无 .cpg → 建议 SHAPE_ENCODING=GBK
        assert_eq!(h.suggested, Some("GBK"));
        // shp2pgsql 链路：同样给出 GBK（iconv 名）
        assert_eq!(h.suggested_shp2pgsql, Some("GBK"));
        assert_eq!(h.cpg, None);
        assert_eq!(h.ldid, 0);
    }

    #[test]
    fn real_gbk_dbf_with_cpg() {
        // 有 .cpg 时两条链路行为不同：
        //  - GDAL：.cpg 优先于 SHAPE_ENCODING，故不干预（suggested=None）
        //  - shp2pgsql：它根本不读 .cpg，必须显式给 -W
        let dbf = build_dbf(encoding_rs::GBK, &[("站名", "翠湖公园")], 2);
        let dir = std::env::temp_dir().join("gis_enc_test3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.shp");
        std::fs::write(dir.join("t.dbf"), &dbf).unwrap();
        std::fs::write(dir.join("t.cpg"), "GBK\n").unwrap();

        let h = detect_shp_for(&p, Region::ZhCn).unwrap();
        assert_eq!(h.cpg.as_deref(), Some("GBK"));
        assert_eq!(h.suggested, None, "有 .cpg 时不干预 GDAL");
        assert_eq!(h.suggested_shp2pgsql, Some("GBK"), "shp2pgsql 仍需 -W");
    }

    #[test]
    fn encoding_aliases_shp2pgsql_names() {
        // 实测 shp2pgsql -W 只认 iconv 名：EUC_KR/CP932/SHIFT_JIS 会被拒
        assert_eq!(encoding_aliases("EUC-KR"), Some(("EUC-KR", "EUC-KR")));
        assert_eq!(encoding_aliases("CP932"), Some(("CP932", "SHIFT_JIS")));
        assert_eq!(encoding_aliases("SHIFT_JIS"), Some(("SHIFT_JIS", "SHIFT_JIS")));
        assert_eq!(encoding_aliases("GBK"), Some(("GBK", "GBK")));
        assert_eq!(encoding_aliases("BIG5"), Some(("BIG5", "BIG5")));
        assert_eq!(encoding_aliases("UTF-8"), Some(("UTF-8", "UTF-8")));
        assert_eq!(encoding_aliases("EUC-TW"), None);
    }

    #[test]
    fn cpg_to_iconv_name_white_list() {
        assert_eq!(cpg_to_iconv_name("GBK"), Some("GBK"));
        assert_eq!(cpg_to_iconv_name("utf-8\n"), Some("UTF-8"));
        assert_eq!(cpg_to_iconv_name("GB2312"), Some("GBK"));
        assert_eq!(cpg_to_iconv_name("CP936"), Some("GBK"));
        assert_eq!(cpg_to_iconv_name("CP950"), Some("BIG5"));
        assert_eq!(cpg_to_iconv_name("EUC_KR"), Some("EUC-KR"));
        assert_eq!(cpg_to_iconv_name("CP932"), Some("SHIFT_JIS"));
        // 白名单外的名字不传给外部工具
        assert_eq!(cpg_to_iconv_name("whatever"), None);
    }
}
