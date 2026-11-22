//! 3D Tiles 服务注册表：从配置目录（`config/3dtiles/*.xml`）加载服务定义，
//! **一个 XML 配置文件就是一个 3D Tiles 服务**。
//!
//! XML 格式（UTF-8，元素大小写不敏感）：
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <TilesetService>
//!   <Name>kmczc</Name>
//!   <Title>kmczc 倾斜摄影（西安80 34度带 EPSG:2358）</Title>
//!   <Root>D:\data\3dtiles\kmczc_3dtiles</Root>
//!   <!-- 可选，缺省 tileset.json -->
//!   <Entry>tileset.json</Entry>
//! </TilesetService>
//! ```
//!
//! 解析器是零依赖的迷你实现（跳过注释/声明、支持基础实体解码），只面向
//! 这种简单 schema；不需要完整 XML 解析器。

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// 一个 3D Tiles 服务：指向一个 3D Tiles 数据集根目录（内含 tileset.json）。
#[derive(Clone, Debug)]
pub struct TilesetService {
    /// 服务名（URL 段，`/3dtiles/{name}/...`）。
    pub name: String,
    /// 展示标题。
    pub title: String,
    /// 3D Tiles 数据集根目录（含 tileset.json 与瓦片文件）。
    pub root: PathBuf,
    /// 入口 json 文件名（相对 root），缺省 `tileset.json`。
    pub entry: String,
    /// 来源 XML 路径（诊断用）。
    pub source_xml: PathBuf,
    /// 数据可用性：false = Root/Entry 校验失败（多机测试换机时数据目录
    /// 可能本机不存在）。不可用的服务仍在服务目录可见（登记/体检），
    /// 但不进 serving 集、`/3dtiles/{name}` 请求返回 404 + 原因。
    pub available: bool,
    /// 不可用原因（available = false 时给出人类可读说明）。
    pub unavailable_reason: Option<String>,
}

/// 全部已加载的 3D Tiles 服务（按服务名索引）。
#[derive(Clone, Debug, Default)]
pub struct TilesetRegistry {
    services: BTreeMap<String, TilesetService>,
}

impl TilesetRegistry {
    /// 扫描 `dir` 下所有 `*.xml` 配置并解析为服务定义。单个文件失败只记录
    /// 错误并跳过，不影响其它服务。
    pub fn load_dir(dir: &Path) -> (TilesetRegistry, Vec<String>) {
        let mut reg = TilesetRegistry::default();
        let mut errors = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            errors.push(format!("3D Tiles 配置目录不存在：{}", dir.display()));
            return (reg, errors);
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map(|e| e.eq_ignore_ascii_case("xml"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        for f in files {
            match std::fs::read_to_string(&f) {
                Ok(text) => match parse_tileset_xml(&text, &f) {
                    Ok(mut svc) => {
                        if reg.services.contains_key(&svc.name) {
                            errors.push(format!(
                                "{}: 服务名 '{}' 与其它配置重复，已跳过",
                                f.display(),
                                svc.name
                            ));
                            continue;
                        }
                        // 数据存在性校验：多机测试/换机部署时 XML 指向的
                        // 数据目录可能本机不存在。服务定义照常注册（目录
                        // 可见、可体检），但标记为不可用——不进 serving 集。
                        let entry_path = svc.root.join(&svc.entry);
                        if !svc.root.is_dir() {
                            let reason = format!(
                                "数据目录指向 {}，但该目录不存在",
                                svc.root.display()
                            );
                            errors.push(format!("{}: 服务 '{}': {reason}", f.display(), svc.name));
                            svc.available = false;
                            svc.unavailable_reason = Some(reason);
                        } else if !entry_path.is_file() {
                            let reason = format!(
                                "数据目录 {} 缺少入口文件 {}",
                                svc.root.display(),
                                svc.entry
                            );
                            errors.push(format!("{}: 服务 '{}': {reason}", f.display(), svc.name));
                            svc.available = false;
                            svc.unavailable_reason = Some(reason);
                        } else {
                            tracing::info!(
                                name = %svc.name,
                                root = %svc.root.display(),
                                source = %f.display(),
                                "3D Tiles service"
                            );
                        }
                        reg.services.insert(svc.name.clone(), svc);
                    }
                    Err(e) => errors.push(format!("{}: {e}", f.display())),
                },
                Err(e) => errors.push(format!("读取 {}: {e}", f.display())),
            }
        }
        (reg, errors)
    }

    pub fn get(&self, name: &str) -> Option<&TilesetService> {
        self.services.get(name)
    }

    /// Keep only services passing `keep` (registry-state filtering on
    /// startup / hot reload).
    pub fn retain_by(&mut self, keep: impl Fn(&str) -> bool) {
        self.services.retain(|name, _| keep(name));
    }

    pub fn list(&self) -> Vec<&TilesetService> {
        self.services.values().collect()
    }

    pub fn len(&self) -> usize {
        self.services.len()
    }

    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}

/// 解析单个 3D Tiles 服务 XML。
pub(crate) fn parse_tileset_xml(text: &str, path: &Path) -> Result<TilesetService, String> {
    let el = collect_elements(text);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "tileset".to_string());
    let name = el
        .get("name")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or(stem);
    let name = sanitize_name(&name);
    let root = el
        .get("root")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少 <Root> 元素（3D Tiles 数据集根目录）".to_string())?;
    let root = PathBuf::from(expand_env(root));
    let entry = el
        .get("entry")
        .or_else(|| el.get("tilesetjson"))
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "tileset.json".to_string());
    let title = el
        .get("title")
        .cloned()
        .unwrap_or_else(|| name.clone());
    Ok(TilesetService {
        name,
        title,
        root,
        entry,
        source_xml: path.to_path_buf(),
        available: true,
        unavailable_reason: None,
    })
}

/// 服务名只保留 URL 安全字符：`[A-Za-z0-9_-]`，其余替换为 `_`。
fn sanitize_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("tileset");
    }
    out
}

/// 展开 `%VAR%`（Windows）与 `${VAR}` / `$VAR` 环境变量引用。
fn expand_env(s: &str) -> String {
    let mut out = s.to_string();
    // ${VAR}
    while let (Some(a), Some(b)) = (out.find("${"), out.find('}')) {
        if a < b {
            let var = &out[a + 2..b];
            let val = std::env::var(var).unwrap_or_default();
            out.replace_range(a..=b, &val);
        } else {
            break;
        }
    }
    // %VAR%
    loop {
        let Some(a) = out.find('%') else { break };
        let Some(rel) = out[a + 1..].find('%') else { break };
        let b = a + 1 + rel;
        let var = &out[a + 1..b];
        if var.is_empty() {
            break;
        }
        let val = std::env::var(var).unwrap_or_default();
        out.replace_range(a..=b, &val);
    }
    out
}

/// 迷你 XML 元素收集器：返回「元素名(小写) → 首个出现元素的文本」。
/// 跳过 `<?...?>` / `<!--...-->` / `<!...>`，忽略属性，不解 CDATA。
fn collect_elements(doc: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let b = doc.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    while i < n {
        let lt = match b[i..].iter().position(|&c| c == b'<') {
            Some(p) => i + p,
            None => break,
        };
        i = lt;
        let rest = &doc[i..];
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(p) => {
                    i += p + 3;
                    continue;
                }
                None => break,
            }
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            match rest.find('>') {
                Some(p) => {
                    i += p + 1;
                    continue;
                }
                None => break,
            }
        }
        // 标签名
        let name_start = i + 1;
        if name_start >= n {
            break;
        }
        let first = b[name_start];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            i += 1;
            continue;
        }
        let mut j = name_start;
        while j < n
            && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b':' || b[j] == b'-')
        {
            j += 1;
        }
        let name = doc[name_start..j].to_ascii_lowercase();
        // 开标签结束 '>'
        let gt = match doc[j..].find('>') {
            Some(p) => j + p,
            None => break,
        };
        if gt > 0 && b[gt - 1] == b'/' {
            // 自闭合
            i = gt + 1;
            continue;
        }
        // 大小写不敏感地查找本元素的闭标签 </name>（跳过中间出现的其它标签）
        let mut found: Option<(usize, usize)> = None; // (close_name_start, close_tag_end)
        let mut k = gt + 1;
        while let Some(rel) = doc[k..].find("</") {
            let cs = k + rel + 2;
            if cs >= n {
                break;
            }
            let mut m = cs;
            while m < n
                && (b[m].is_ascii_alphanumeric() || b[m] == b'_' || b[m] == b':' || b[m] == b'-')
            {
                m += 1;
            }
            let cname = doc[cs..m].to_ascii_lowercase();
            let Some(g2rel) = doc[m..].find('>') else { break };
            let g2 = m + g2rel;
            if cname == name {
                found = Some((cs, g2));
                break;
            }
            k = g2 + 1;
        }
        match found {
            Some((cs, _g2)) => {
                let text = doc[gt + 1..cs - 2].trim();
                out.entry(name)
                    .or_insert_with(|| decode_entities(text));
                // 继续从开标签之后扫描，让子元素也能被收集
                //（close 标签自身会在主循环中被当作非法名跳过）
                i = gt + 1;
            }
            None => {
                i = gt + 1;
            }
        }
    }
    out
}

/// 基础实体解码：命名实体 + 十进制/十六进制数字引用（按 char 遍历，保持
/// UTF-8 多字节内容如中文标题不被破坏）。
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        if chars[i] == '&' {
            // 从 '&' 开始找 ';'（限制在 10 个 char 内，避免吞掉大段文本）
            let semi = (i + 1..n.min(i + 11)).find(|&k| chars[k] == ';');
            match semi {
                Some(k) => {
                    let ent: String = chars[i + 1..k].iter().collect();
                    let decoded = match ent.as_str() {
                        "amp" => Some('&'.to_string()),
                        "lt" => Some('<'.to_string()),
                        "gt" => Some('>'.to_string()),
                        "quot" => Some('"'.to_string()),
                        "apos" => Some('\''.to_string()),
                        _ => {
                            if let Some(hex) =
                                ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X"))
                            {
                                u32::from_str_radix(hex, 16)
                                    .ok()
                                    .and_then(char::from_u32)
                                    .map(|c| c.to_string())
                            } else if let Some(dec) = ent.strip_prefix('#') {
                                dec.parse::<u32>()
                                    .ok()
                                    .and_then(char::from_u32)
                                    .map(|c| c.to_string())
                            } else {
                                None
                            }
                        }
                    };
                    match decoded {
                        Some(v) => {
                            out.push_str(&v);
                            i = k + 1;
                        }
                        None => {
                            out.push('&');
                            i += 1;
                        }
                    }
                }
                None => {
                    out.push('&');
                    i += 1;
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// 请求的相对路径是否安全（拒绝目录穿越成分：`..` / 绝对路径前缀）。
pub fn safe_rel_path(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches(['/', '\\']);
    if rel.is_empty() {
        return None;
    }
    let p = Path::new(rel);
    let bad = p
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)));
    if bad {
        None
    } else {
        Some(p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_xml() {
        let doc = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- 一个 XML 一个服务 -->
<TilesetService>
  <Name>kmczc</Name>
  <Title>kmczc 倾斜摄影（西安80 34度带 EPSG:2358）</Title>
  <Root>D:\data\3dtiles\kmczc_3dtiles</Root>
</TilesetService>"#;
        let svc = parse_tileset_xml(doc, Path::new("config/3dtiles/kmczc.xml")).unwrap();
        assert_eq!(svc.name, "kmczc");
        assert_eq!(svc.root, PathBuf::from(r"D:\data\3dtiles\kmczc_3dtiles"));
        assert_eq!(svc.entry, "tileset.json");
        assert!(svc.title.contains("西安80"));
    }

    #[test]
    fn parse_defaults_and_entities() {
        let doc = "<TilesetService><Root>C:&amp;/data/a b</Root></TilesetService>";
        let svc = parse_tileset_xml(doc, Path::new("x.xml")).unwrap();
        assert_eq!(svc.name, "x"); // 取文件名 stem
        assert_eq!(svc.root, PathBuf::from("C:&/data/a b"));
        assert_eq!(svc.entry, "tileset.json");
    }

    #[test]
    fn missing_root_is_error() {
        let doc = "<TilesetService><Name>a</Name></TilesetService>";
        assert!(parse_tileset_xml(doc, Path::new("a.xml")).is_err());
    }

    #[test]
    fn sanitize_url_safe() {
        assert_eq!(sanitize_name("a b/c"), "a_b_c");
        assert_eq!(sanitize_name(""), "tileset");
    }

    #[test]
    fn rejects_traversal() {
        assert!(safe_rel_path("../secret.json").is_none());
        assert!(safe_rel_path("a/../../b").is_none());
        assert_eq!(
            safe_rel_path("Tile_1/tile.b3dm").unwrap(),
            PathBuf::from("Tile_1/tile.b3dm")
        );
        assert!(safe_rel_path("").is_none());
    }
}
