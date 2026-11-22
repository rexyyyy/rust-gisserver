//! 极简 XML 扫描器：ArcGIS 缓存的 `Conf.xml` / `conf.cdi` 语法面很窄（元素 +
//! 属性 + 文本，无命名空间处理需求、无 DTD、无混合内容语义），因此这里**不引入
//! 任何 XML 依赖**，只解析出这棵元素树所需的结构。
//!
//! 与 `map-api::xmlcfg` 的分工：xmlcfg 服务本仓库**自己生成**的配置 XML（键名
//! 与 serde 字段严格对应，需要 JSON 桥）；本模块服务**第三方产出**的 ArcGIS XML
//! （标签大小写、重复元素、属性噪声都需要按 GIS 语义读取），两者互不复用。

/// 一个 XML 元素。
#[derive(Debug, Clone, Default)]
pub struct Elem {
    /// 标签名（保留原文大小写，匹配一律走 [`Elem::child`] 的不敏感比较）。
    pub name: String,
    /// 属性（本模块的调用方目前不读属性，仅保留结构完整性）。
    pub attrs: Vec<(String, String)>,
    /// 子元素（文档顺序）。
    pub children: Vec<Elem>,
    /// 直接文本内容（已解实体；子元素之间的文本会被拼接）。
    pub text: String,
}

impl Elem {
    /// 第一个标签匹配的子元素（大小写不敏感）。
    pub fn child(&self, name: &str) -> Option<&Elem> {
        self.children.iter().find(|c| eq_name(&c.name, name))
    }

    /// 所有标签匹配的子元素（文档顺序）。
    pub fn children_named(&self, name: &str) -> Vec<&Elem> {
        self.children.iter().filter(|c| eq_name(&c.name, name)).collect()
    }

    /// 子元素文本（trim 后）。空串视为 `None`，让 `?` 链能直接表达「缺失」。
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim()).filter(|t| !t.is_empty())
    }

    pub fn f64_of(&self, name: &str) -> Option<f64> {
        self.text_of(name).and_then(|t| t.parse().ok())
    }

    pub fn u32_of(&self, name: &str) -> Option<u32> {
        self.text_of(name).and_then(|t| t.parse().ok())
    }

    /// 深度优先查找第一个匹配标签的后代（ArcGIS 把 `SpatialReference` 嵌在好几层里）。
    pub fn find(&self, name: &str) -> Option<&Elem> {
        for c in &self.children {
            if eq_name(&c.name, name) {
                return Some(c);
            }
            if let Some(d) = c.find(name) {
                return Some(d);
            }
        }
        None
    }
}

fn eq_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// 解析 XML 文本，返回根元素。
pub fn parse(text: &str) -> Result<Elem, String> {
    let mut p = Parser { b: text.as_bytes(), i: 0, src: text };
    p.skip_misc()?;
    if p.i >= p.b.len() {
        return Err("XML 内容为空".to_string());
    }
    let root = p.element()?;
    p.skip_misc()?;
    Ok(root)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    /// 跳过空白、`<?xml ... ?>` 声明、`<!-- 注释 -->` 与 `<!DOCTYPE/!ENTITY` 声明。
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.ws();
            if self.starts("<?") {
                self.until("?>")?;
            } else if self.starts("<!--") {
                self.until("-->")?;
            } else if self.starts("<!") {
                // DTD / 声明：本模块不消费其语义，整段跳过。
                self.until(">")?;
            } else {
                return Ok(());
            }
        }
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn starts(&self, s: &str) -> bool {
        self.src[self.i..].starts_with(s)
    }

    /// 跳到下一个 `s` 并把它整体消费掉。
    fn until(&mut self, s: &str) -> Result<(), String> {
        match self.src[self.i..].find(s) {
            Some(off) => {
                self.i += off + s.len();
                Ok(())
            }
            None => Err(format!("缺少结束标记 {s}")),
        }
    }

    fn element(&mut self) -> Result<Elem, String> {
        if self.b.get(self.i) != Some(&b'<') {
            return Err("期望 '<' 开始的元素".to_string());
        }
        self.i += 1;
        let mut el = Elem::default();
        el.name = self.name()?;
        loop {
            self.ws();
            match self.b.get(self.i) {
                None => return Err("元素未闭合".to_string()),
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    // 自闭合：无子元素、无文本。
                    return Ok(el);
                }
                Some(_) => {
                    let (k, v) = self.attr()?;
                    el.attrs.push((k, v));
                }
            }
        }
        // 内容：文本与子元素交替，直到 </name>。
        loop {
            if self.starts("</") {
                self.i += 2;
                let close = self.name()?;
                self.ws();
                if self.b.get(self.i) != Some(&b'>') {
                    return Err(format!("</{close}> 未闭合"));
                }
                self.i += 1;
                if !eq_name(&close, &el.name) {
                    return Err(format!("标签不匹配：<{}> … </{close}>", el.name));
                }
                return Ok(el);
            }
            if self.starts("<!") {
                self.skip_misc()?;
                continue;
            }
            if self.starts("<?") {
                self.until("?>")?;
                continue;
            }
            if self.starts("<") {
                let c = self.element()?;
                el.text.push_str(c.text.trim());
                el.children.push(c);
                continue;
            }
            el.text.push_str(&self.text_run()?);
        }
    }

    /// 元素名 / 属性名：ASCII 标识符，允许 ArcGIS 的 `xsi:type` 冒号与下划线。
    fn name(&mut self) -> Result<String, String> {
        let start = self.i;
        while self.i < self.b.len() {
            let c = self.b[self.i] as char;
            if c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '-' || c == '.' {
                self.i += 1;
            } else {
                break;
            }
        }
        if start == self.i {
            return Err("期望名称".to_string());
        }
        Ok(self.src[start..self.i].to_string())
    }

    fn attr(&mut self) -> Result<(String, String), String> {
        let k = self.name()?;
        self.ws();
        if self.b.get(self.i) != Some(&b'=') {
            return Err(format!("属性 {k} 缺少 '='"));
        }
        self.i += 1;
        self.ws();
        let quote = *self.b.get(self.i).ok_or("属性值缺失")? as char;
        if quote != '"' && quote != '\'' {
            return Err(format!("属性 {k} 的值未加引号"));
        }
        self.i += 1;
        let end = self.src[self.i..]
            .find(quote)
            .ok_or_else(|| format!("属性 {k} 的引号未闭合"))?;
        let raw = &self.src[self.i..self.i + end];
        self.i += end + 1;
        Ok((k, unescape(raw)))
    }

    /// 一段字符文本（直到下一个 '<'），已解实体。
    fn text_run(&mut self) -> Result<String, String> {
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i] != b'<' {
            self.i += 1;
        }
        Ok(unescape(&self.src[start..self.i]))
    }
}

/// 解析五个内置实体与 `&#..;` 数字实体；其余原样保留。
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let Some(end) = after.find(';') else {
            out.push('&');
            rest = after;
            continue;
        };
        let ent = &after[..end];
        match ent {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                        out.push(c);
                    }
                } else if let Some(dec) = ent.strip_prefix('#') {
                    if let Some(c) = dec.parse::<u32>().ok().and_then(char::from_u32) {
                        out.push(c);
                    }
                } else {
                    out.push('&');
                    out.push_str(ent);
                    out.push(';');
                }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_scalar_and_repeat() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<CacheInfo xsi:type='typens:CacheInfo'>
  <TileCacheInfo><TileCols>256</TileCols><DPI>96</DPI>
    <LODInfos><LODInfo><LevelID>0</LevelID><Resolution>0.7039</Resolution></LODInfo>
    <LODInfo><LevelID>1</LevelID><Resolution>0.3519</Resolution></LODInfo></LODInfos>
    <TileOrigin><X>-180</X><Y>90</Y></TileOrigin>
  </TileCacheInfo>
</CacheInfo>"#;
        let root = parse(xml).unwrap();
        let tci = root.child("tilecacheinfo").unwrap();
        assert_eq!(tci.u32_of("TileCols"), Some(256));
        assert_eq!(tci.f64_of("DPI"), Some(96.0));
        assert_eq!(tci.children_named("LODInfo").len(), 0);
        let lods = tci.child("LODInfos").unwrap();
        assert_eq!(lods.children_named("LODInfo").len(), 2);
        assert_eq!(lods.children_named("LODInfo")[1].f64_of("Resolution"), Some(0.3519));
        assert_eq!(tci.child("TileOrigin").unwrap().f64_of("X"), Some(-180.0));
        // 属性带实体、大小写不敏感查找、深度查找
        assert_eq!(root.attrs[0].0, "xsi:type");
        assert!(root.find("WKID").is_none());
    }

    #[test]
    fn decodes_entities_and_skips_comment() {
        let xml = "<!-- c --><R><WKT>GEOGCS[&quot;S&quot;,UNIT[&quot;Degree&quot;,0.0174532925199433]]</WKT><N>8.9e-009</N></R>";
        let root = parse(xml).unwrap();
        assert!(root.text_of("WKT").unwrap().contains("\"S\""));
        assert_eq!(root.f64_of("N"), Some(8.9e-9));
    }

    #[test]
    fn mismatched_tag_is_an_error() {
        assert!(parse("<A><B></A>").is_err());
        assert!(parse("").is_err());
    }
}
