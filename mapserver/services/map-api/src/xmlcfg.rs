//! XML ↔ JSON bridge: read XML config as `serde_json::Value`, so existing serde structs
//! (`Layer` / `GridDef` / `ServerConfig` / `PgSourceDef` /
//! `Style`）。
//!
//! # XML conventions
//!
//! 1. **Attributes → same-name keys**: `<source type="cog" path="a.tif"/>` →
//!    `{"type":"cog","path":"a.tif"}`. `source` uses an internal tag enum
//!    (`#[serde(tag = "type")]`), so it matches naturally.
//! 2. **Scalar text → auto type inference**: number / `true` / `false` / `null` / string.
//! 3. **Child elements with `id` attr → map keyed by id**:
//!    `<custom_grids><item id="CGCS2000Geo">…</item></custom_grids>` →
//!    `{"CGCS2000Geo": {...}}` (`id` itself is stripped).
//! 4. **All children are `<item>` → array**: `<grids><item>A</item><item>B</item></grids>`
//!    → `["A","B"]`. `<item>` is the **dedicated marker** for arrays — in XML, 'array'
//!    and 'single-key object' look the same (parent + same-tag children); this convention
//!    lets us restore them unambiguously. Array fields (`grids` / `properties` / `resolutions`
//!    / `palette`) and tuple fields (`origin` / `extent`) all use this path:
//!    `<origin><item>-180</item><item>90</item></origin>` → `[-180, 90]`。
//! 5. **Otherwise → object keyed by tag name**; repeated same-tag children become array.
//!
//! The reverse ([`to_xml`]) follows the same rules, so XML written out can be
//! read back identically (see round-trip tests in this module).
//!
//! The parser is hand-written: config XML is server-generated with only elements, attrs,
//! and text — no namespaces, no DTD; no extra dependencies.

use serde_json::{Map, Value};

/// An XML element.
#[derive(Debug, Default, Clone)]
pub struct Element {
    /// Element name (tag).
    pub name: String,
    /// Attribute list, preserving document order.
    pub attrs: Vec<(String, String)>,
    /// Child elements.
    pub children: Vec<Element>,
    /// Text content (entities decoded).
    pub text: String,
}

/// Parse XML text, return JSON value for the **root element**.
///
/// On failure returns a Chinese error message (caller supplies file path).
pub fn parse_to_value(text: &str) -> Result<Value, String> {
    // Expand env vars when reading config: passwords / data_root etc. use `${VAR}` or
    // `${VAR:-default}` placeholders — plaintext never stored in DB or baked into images (P0-3 / P0-4).
    let expanded = expand_env(text);
    let mut p = Parser::new(&expanded);
    p.skip_misc()?;
    if p.i >= p.b.len() {
        return Err("XML 内容为空".to_string());
    }
    let root = p.parse_element()?;
    p.skip_misc()?;
    Ok(element_to_value(&root))
}

/// Validate XML syntax (no semantic parsing). Returns Ok if syntax is correct.
pub fn validate(text: &str) -> Result<(), String> {
    parse_to_value(text).map(|_| ())
}

/// Expand env vars: replace `${NAME}` / `${NAME:-default}` with the env var value.
///
/// - `${NAME}`: replace with value if set, else empty string.
/// - `${NAME:-default}`: use 'default' if var is unset.
/// - Var name only allows `[A-Za-z0-9_]`; `${}` with other chars is kept as-is.
///
/// 用于配置文件读入路径：DB 口令、data_root 等以占位书写，
/// 明文既不入库也不烘焙进镜像，改由部署环境注入（P0-3 / P0-4）。
pub fn expand_env(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("${") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        match after.find('}') {
            Some(end) => {
                let inner = &after[..end];
                let (name, default) = match inner.find(":-") {
                    Some(p) => (&inner[..p], Some(&inner[p + 2..])),
                    None => (inner, None),
                };
                let valid = !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if valid {
                    match std::env::var(name) {
                        Ok(v) => out.push_str(&v),
                        Err(_) => out.push_str(default.unwrap_or("")),
                    }
                } else {
                    // Invalid placeholder: write back `${...}` as-is
                    out.push_str("${");
                    out.push_str(inner);
                    out.push('}');
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unclosed: keep remainder as-is (including `${` prefix)
                out.push_str("${");
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// XML escape (shared by text and attribute values).
pub fn escape(s: &str) -> String {
    escape_text(s)
}

/// Write JSON value as XML text (follows the same conventions as [`parse_to_value`]).
///
/// `root` is the root element name.
pub fn to_xml(value: &Value, root: &str) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    match value {
        Value::Object(map) => write_object(&mut out, root, map, &[], 0),
        Value::Array(items) => {
            for item in items {
                write_value(&mut out, "item", item, 0);
            }
        }
        scalar => {
            out.push_str(&format!(
                "<{root}>{}</{root}>\n",
                escape_text(&scalar_to_string(scalar))
            ));
        }
    }
    out
}

// ---------------------------------------------------------------- Parser

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s,
            b: s.as_bytes(),
            i: 0,
        }
    }

    fn err<T>(&self, msg: impl std::fmt::Display) -> Result<T, String> {
        Err(format!("XML 解析失败（偏移 {}）：{msg}", self.i))
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn starts_with(&self, pat: &str) -> bool {
        self.s[self.i..].starts_with(pat)
    }

    fn find_from(&self, pat: &str, from: usize) -> Option<usize> {
        self.s[from..].find(pat).map(|p| p + from)
    }

    /// Skip XML declarations, comments, DOCTYPE, processing instructions.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.i >= self.b.len() {
                return Ok(());
            }
            if self.starts_with("<?") {
                match self.find_from("?>", self.i + 2) {
                    Some(p) => self.i = p + 2,
                    None => return self.err("XML 声明未闭合"),
                }
                continue;
            }
            if self.starts_with("<!--") {
                match self.find_from("-->", self.i + 4) {
                    Some(p) => self.i = p + 3,
                    None => return self.err("注释未闭合"),
                }
                continue;
            }
            if self.starts_with("<!") {
                match self.find_from(">", self.i + 2) {
                    Some(p) => self.i = p + 1,
                    None => return self.err("DOCTYPE 未闭合"),
                }
                continue;
            }
            return Ok(());
        }
    }

    /// Parse one element (`self.i` points to `<` on entry).
    fn parse_element(&mut self) -> Result<Element, String> {
        self.skip_ws();
        if self.i >= self.b.len() || self.b[self.i] != b'<' {
            return self.err("期望元素起始 '<'");
        }
        self.i += 1;
        let name = self.parse_name()?;
        if name.is_empty() {
            return self.err("元素名为空");
        }
        let mut attrs: Vec<(String, String)> = Vec::new();
        let self_closing = loop {
            self.skip_ws();
            if self.i >= self.b.len() {
                return self.err(format!("元素 <{name}> 未闭合"));
            }
            match self.b[self.i] {
                b'>' => {
                    self.i += 1;
                    break false;
                }
                b'/' => {
                    if self.i + 1 < self.b.len() && self.b[self.i + 1] == b'>' {
                        self.i += 2;
                        break true;
                    }
                    return self.err(format!("元素 <{name}> 的 '/' 后应为 '>'"));
                }
                _ => {
                    let a_name = self.parse_name()?;
                    if a_name.is_empty() {
                        return self.err(format!("元素 <{name}> 的属性名无效"));
                    }
                    self.skip_ws();
                    if self.i >= self.b.len() || self.b[self.i] != b'=' {
                        return self.err(format!("属性 {a_name} 缺少 '='"));
                    }
                    self.i += 1;
                    self.skip_ws();
                    if self.i >= self.b.len() {
                        return self.err(format!("属性 {a_name} 的值缺失"));
                    }
                    let q = self.b[self.i];
                    if q != b'"' && q != b'\'' {
                        return self.err(format!("属性 {a_name} 的值必须加引号"));
                    }
                    self.i += 1;
                    let start = self.i;
                    while self.i < self.b.len() && self.b[self.i] != q {
                        self.i += 1;
                    }
                    if self.i >= self.b.len() {
                        return self.err(format!("属性 {a_name} 的值未闭合"));
                    }
                    let raw = &self.s[start..self.i];
                    self.i += 1;
                    attrs.push((a_name, decode_entities(raw)));
                }
            }
        };

        let mut el = Element {
            name,
            attrs,
            children: Vec::new(),
            text: String::new(),
        };
        if self_closing {
            return Ok(el);
        }

        // Element content
        loop {
            if self.i >= self.b.len() {
                return self.err(format!("元素 <{}> 未闭合", el.name));
            }
            if self.b[self.i] == b'<' {
                if self.starts_with("</") {
                    self.i += 2;
                    let close = self.parse_name()?;
                    self.skip_ws();
                    if self.i >= self.b.len() || self.b[self.i] != b'>' {
                        return self.err(format!("结束标签 </{close}> 未闭合"));
                    }
                    self.i += 1;
                    if close != el.name {
                        return self.err(format!("标签不匹配：<{}> 对 </{}>", el.name, close));
                    }
                    break;
                }
                if self.starts_with("<!--") {
                    match self.find_from("-->", self.i + 4) {
                        Some(p) => {
                            self.i = p + 3;
                            continue;
                        }
                        None => return self.err("注释未闭合"),
                    }
                }
                if self.starts_with("<![CDATA[") {
                    match self.find_from("]]>", self.i + 9) {
                        Some(p) => {
                            el.text.push_str(&self.s[self.i + 9..p]);
                            self.i = p + 3;
                            continue;
                        }
                        None => return self.err("CDATA 未闭合"),
                    }
                }
                if self.starts_with("<?") {
                    match self.find_from("?>", self.i + 2) {
                        Some(p) => {
                            self.i = p + 2;
                            continue;
                        }
                        None => return self.err("处理指令未闭合"),
                    }
                }
                let child = self.parse_element()?;
                el.children.push(child);
                continue;
            }
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b'<' {
                self.i += 1;
            }
            el.text.push_str(&decode_entities(&self.s[start..self.i]));
        }
        Ok(el)
    }

    fn parse_name(&mut self) -> Result<String, String> {
        let start = self.i;
        while self.i < self.b.len() && is_name_char(self.b[self.i]) {
            self.i += 1;
        }
        Ok(self.s[start..self.i].to_string())
    }
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':')
}

/// Decode XML entities (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;` `&#NN;` `&#xNN;`).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let mut handled = false;
        if let Some(semi) = rest.find(';') {
            let ent = &rest[1..semi];
            let decoded = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = ent.strip_prefix('#') {
                        dec.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            if let Some(c) = decoded {
                out.push(c);
                rest = &rest[semi + 1..];
                handled = true;
            }
        }
        if !handled {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- Element → JSON

fn element_to_value(el: &Element) -> Value {
    let mut obj = Map::new();
    for (k, v) in &el.attrs {
        obj.insert(k.clone(), coerce(v));
    }

    if el.children.is_empty() {
        let t = el.text.trim();
        if el.attrs.is_empty() {
            return coerce(t);
        }
        if !t.is_empty() {
            obj.insert("value".to_string(), coerce(t));
        }
        return Value::Object(obj);
    }

    // Rule A: all children have `id` attr → map keyed by id.
    if el
        .children
        .iter()
        .all(|c| c.attrs.iter().any(|(k, _)| k == "id"))
    {
        let mut m = Map::new();
        for c in &el.children {
            let id = c
                .attrs
                .iter()
                .find(|(k, _)| k == "id")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            // `<item id="X">text</item>` → value is the text itself; else the element object (id stripped).
            let v = if c.children.is_empty() && c.attrs.len() == 1 {
                coerce(c.text.trim())
            } else {
                let mut v = element_to_value(c);
                if let Value::Object(o) = &mut v {
                    o.remove("id");
                }
                v
            };
            m.insert(id, v);
        }
        for (k, v) in obj {
            m.entry(k).or_insert(v);
        }
        return Value::Object(m);
    }

    // Rule B: all children are `<item>` → array (generic list marker).
    //
    // `<item>` is the dedicated array marker, to distinguish 'array' from 'single-key object' —
    // both look the same in XML (parent + same-tag children); only this convention lets us
    // restore them unambiguously. E.g. `<palette><item>...</item>x9</palette>` is an array, while
    // `<server><disk_cache/></server>` is an object.
    if el.children.iter().all(|c| c.name == "item") {
        return Value::Array(el.children.iter().map(element_to_value).collect());
    }

    // Rule C: object keyed by tag name; repeated same-tag children merge into an array.
    let mut tags: Vec<&str> = Vec::new();
    for c in &el.children {
        if !tags.contains(&c.name.as_str()) {
            tags.push(&c.name);
        }
    }
    for tag in tags {
        let group: Vec<&Element> = el.children.iter().filter(|c| c.name == tag).collect();
        let val = if group.len() == 1 {
            element_to_value(group[0])
        } else {
            Value::Array(group.iter().map(|c| element_to_value(c)).collect())
        };
        obj.insert(tag.to_string(), val);
    }
    Value::Object(obj)
}

/// Scalar text → JSON value (number / bool / null / string).
fn coerce(s: &str) -> Value {
    let t = s.trim();
    if t.is_empty() {
        return Value::String(String::new());
    }
    match t {
        "null" | "nil" => return Value::Null,
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = t.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = t.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(t.to_string())
}

// ---------------------------------------------------------------- JSON → XML

fn write_value(out: &mut String, name: &str, value: &Value, level: usize) {
    match value {
        Value::Object(map) => write_object(out, name, map, &[], level),
        Value::Array(items) => {
            indent(out, level);
            out.push_str(&format!("<{name}>\n"));
            for item in items {
                write_value(out, "item", item, level + 1);
            }
            indent(out, level);
            out.push_str(&format!("</{name}>\n"));
        }
        scalar => {
            indent(out, level);
            out.push_str(&format!(
                "<{name}>{}</{name}>\n",
                escape_text(&scalar_to_string(scalar))
            ));
        }
    }
}

/// Write an object element: scalar members → attrs; object/array members → children.
///
/// `extra_attrs`: additional attrs (e.g. the `id` of a map entry).
fn write_object(
    out: &mut String,
    name: &str,
    map: &Map<String, Value>,
    extra_attrs: &[(String, String)],
    level: usize,
) {
    let mut attrs = String::new();
    for (k, v) in extra_attrs {
        attrs.push_str(&format!(" {}=\"{}\"", escape_attr(k), escape_attr(v)));
    }
    let mut kids: Vec<(&String, &Value)> = Vec::new();
    for (k, v) in map {
        match v {
            Value::Object(m) if !m.is_empty() => kids.push((k, v)),
            Value::Object(_) => {}
            Value::Array(a) if !a.is_empty() => kids.push((k, v)),
            Value::Array(_) => {}
            Value::Null => {}
            _ => attrs.push_str(&format!(
                " {}=\"{}\"",
                escape_attr(k),
                escape_attr(&scalar_to_string(v))
            )),
        }
    }
    if kids.is_empty() {
        indent(out, level);
        out.push_str(&format!("<{name}{attrs}/>\n"));
        return;
    }
    indent(out, level);
    out.push_str(&format!("<{name}{attrs}>\n"));
    for (k, v) in kids {
        write_member(out, k, v, level + 1);
    }
    indent(out, level);
    out.push_str(&format!("</{name}>\n"));
}

/// Write an object member (distinguishes 'map' vs 'plain object' shapes).
fn write_member(out: &mut String, name: &str, value: &Value, level: usize) {
    match value {
        Value::Object(map) => {
            // Map shape: all values are objects → `<name><item id="key">...</item></name>`.
            if !map.is_empty()
                && map
                    .values()
                    .all(|v| matches!(v, Value::Object(m) if !m.is_empty()))
            {
                indent(out, level);
                out.push_str(&format!("<{name}>\n"));
                for (k, v) in map {
                    if let Value::Object(m) = v {
                        write_object(
                            out,
                            "item",
                            m,
                            &[("id".to_string(), k.clone())],
                            level + 1,
                        );
                    }
                }
                indent(out, level);
                out.push_str(&format!("</{name}>\n"));
            } else {
                write_object(out, name, map, &[], level);
            }
        }
        Value::Array(items) => {
            indent(out, level);
            out.push_str(&format!("<{name}>\n"));
            for item in items {
                write_value(out, "item", item, level + 1);
            }
            indent(out, level);
            out.push_str(&format!("</{name}>\n"));
        }
        _ => write_value(out, name, value, level),
    }
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 属性映射为同名键() {
        let v = parse_to_value(r#"<source type="cog" path="a.tif"/>"#).unwrap();
        assert_eq!(v["type"], "cog");
        assert_eq!(v["path"], "a.tif");
    }

    #[test]
    fn 标量按类型推断() {
        let v = parse_to_value(r#"<a n="8080" f="0.5" b="true" z="null" s="EPSG:4326"/>"#).unwrap();
        assert_eq!(v["n"], 8080);
        assert_eq!(v["f"], 0.5);
        assert_eq!(v["b"], true);
        assert!(v["z"].is_null());
        assert_eq!(v["s"], "EPSG:4326");
    }

    #[test]
    fn 同标签标量子元素成数组() {
        let v = parse_to_value(r#"<grids><item>A</item><item>B</item></grids>"#).unwrap();
        assert_eq!(v, serde_json::json!(["A", "B"]));
    }

    #[test]
    fn 元组字段还原为数组() {
        let v = parse_to_value(
            r#"<g><origin><item>-180</item><item>90</item></origin><extent><item>-180</item><item>-90</item><item>180</item><item>90</item></extent></g>"#,
        )
        .unwrap();
        assert_eq!(v["origin"], serde_json::json!([-180, 90]));
        assert_eq!(v["extent"], serde_json::json!([-180, -90, 180, 90]));
    }

    #[test]
    fn 带id的子元素成映射() {
        let v = parse_to_value(
            r#"<custom_grids><item id="CGCS2000Geo" crs="EPSG:4490"><tile_px>256</tile_px></item></custom_grids>"#,
        )
        .unwrap();
        assert_eq!(v["CGCS2000Geo"]["crs"], "EPSG:4490");
        assert_eq!(v["CGCS2000Geo"]["tile_px"], 256);
        assert!(v["CGCS2000Geo"].get("id").is_none());
    }

    #[test]
    fn 嵌套对象与同名元素合并() {
        let v = parse_to_value(
            r#"<config><server host="0.0.0.0"><disk_cache dir="cache/foyer"/></server><a>1</a><a>2</a></config>"#,
        )
        .unwrap();
        assert_eq!(v["server"]["host"], "0.0.0.0");
        assert_eq!(v["server"]["disk_cache"]["dir"], "cache/foyer");
        assert_eq!(v["a"], serde_json::json!([1, 2]));
    }

    #[test]
    fn 注释声明与实体被正确处理() {
        let v = parse_to_value(
            "<?xml version=\"1.0\"?><!-- 说明 --><a b=\"x &amp; y\">t &lt; u</a>",
        )
        .unwrap();
        assert_eq!(v["b"], "x & y");
        // When an element has both attrs and text, text goes under the `value` key.
        assert_eq!(v["value"], "t < u");
        // Plain text elements are scalars directly.
        let s = parse_to_value("<a>t &lt; u</a>").unwrap();
        assert_eq!(s.as_str().unwrap(), "t < u");
    }

    #[test]
    fn 数组的数组还原为二维数组() {
        // Regression: 'array of arrays' like `palette` must restore as 2D array,
        // not `{"item": [...]}`.
        let v = parse_to_value(
            r#"<palette><item><item>8</item><item>29</item><item>88</item></item><item><item>37</item><item>52</item><item>148</item></item></palette>"#,
        )
        .unwrap();
        assert_eq!(v, serde_json::json!([[8, 29, 88], [37, 52, 148]]));
    }

    #[test]
    fn 单个子元素的对象保持对象形态() {
        // Regression: single child that is an object must not degrade into an array.
        let v = parse_to_value(r#"<server host="0.0.0.0"><disk_cache dir="cache/foyer"/></server>"#)
            .unwrap();
        assert_eq!(v["host"], "0.0.0.0");
        assert_eq!(v["disk_cache"]["dir"], "cache/foyer");
    }

    #[test]
    fn 带属性的元素不会退化成数组() {
        // Regression: when an element has its own attrs, scalar children must stay as 'key → value',
        // not become arrays (else attrs like id/crs get silently dropped).
        let v = parse_to_value(r#"<item id="X" crs="EPSG:4490"><tile_px>256</tile_px></item>"#)
            .unwrap();
        assert_eq!(v["id"], "X");
        assert_eq!(v["crs"], "EPSG:4490");
        assert_eq!(v["tile_px"], 256);
    }

    #[test]
    fn 语法错误被拒绝() {
        assert!(parse_to_value("<a><b></a>").is_err());
        assert!(parse_to_value("<a").is_err());
        assert!(parse_to_value("").is_err());
    }

    #[test]
    fn 往返一致() {
        let src = serde_json::json!({
            "name": "wh",
            "title": "wh2013 COG",
            "source": {"type": "cog", "path": "E:\\data\\a.tif"},
            "src_crs": "EPSG:4326",
            "grids": ["WebMercatorQuad", "CGCS2000Geo"],
            "tile_px": 256,
            "custom_grids": {
                "CGCS2000Geo": {
                    "crs": "EPSG:4490",
                    "origin": [-180.0, 90.0],
                    "extent": [-180.0, -90.0, 180.0, 90.0],
                    "tile_px": 256,
                    "resolutions": [0.703125, 0.3515625]
                }
            }
        });
        let xml = to_xml(&src, "layer");
        let back = parse_to_value(&xml).unwrap();
        assert_eq!(src, back, "XML 往返后应与原 JSON 完全一致\n{xml}");
    }

    #[test]
    fn 空值与省略() {
        // null members are not written to XML; on read-back the key is absent (serde default fills).
        let src = serde_json::json!({"a": 1, "b": null});
        let xml = to_xml(&src, "x");
        assert!(!xml.contains("b="));
        let back = parse_to_value(&xml).unwrap();
        assert_eq!(back["a"], 1);
        assert!(back.get("b").is_none());
    }
}
