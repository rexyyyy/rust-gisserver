//! XML ↔ JSON 桥接：把 XML 配置文件读成 `serde_json::Value`，从而**零改动**复用
//! 既有的 serde 结构体（`Layer` / `GridDef` / `ServerConfig` / `PgSourceDef` /
//! `Style`）。
//!
//! # XML 约定
//!
//! 1. **属性 → 同名键**：`<source type="cog" path="a.tif"/>` →
//!    `{"type":"cog","path":"a.tif"}`。`source` 用的是内部标签枚举
//!    （`#[serde(tag = "type")]`），因此天然匹配。
//! 2. **标量文本 → 自动类型推断**：数字 / `true` / `false` / `null` / 字符串。
//! 3. **带 `id` 属性的子元素 → 以 id 为键的映射**：
//!    `<custom_grids><item id="CGCS2000Geo">…</item></custom_grids>` →
//!    `{"CGCS2000Geo": {…}}`（`id` 本身被剔除）。
//! 4. **子元素全是 `<item>` → 数组**：`<grids><item>A</item><item>B</item></grids>`
//!    → `["A","B"]`。`<item>` 是数组的**专用标记**——XML 里「数组」与「单键对象」
//!    形状相同（都是一个父元素带若干同标签子元素），只有靠这个约定才能无歧义
//!    地还原。数组字段（`grids` / `properties` / `resolutions` / `palette`）与元组
//!    字段（`origin` / `extent`）都走这条：
//!    `<origin><item>-180</item><item>90</item></origin>` → `[-180, 90]`。
//! 5. **其余情况 → 以标签为键的对象**；同名子元素出现多次时合并为数组。
//!
//! 反向（[`to_xml`]）沿用同一套约定，保证写出的 XML 能被 [`parse_to_value`]
//! 原样读回（见模块内的往返测试）。
//!
//! 解析器是手写的：配置 XML 由服务端自己生成、只含元素/属性/文本，无命名空间、
//! 无 DTD；不引入额外依赖。

use serde_json::{Map, Value};

/// 一个 XML 元素。
#[derive(Debug, Default, Clone)]
pub struct Element {
    /// 元素名（标签）。
    pub name: String,
    /// 属性列表，保持文档顺序。
    pub attrs: Vec<(String, String)>,
    /// 子元素。
    pub children: Vec<Element>,
    /// 文本内容（已解实体）。
    pub text: String,
}

/// 解析 XML 文本，返回**根元素**对应的 JSON 值。
///
/// 失败时返回中文错误信息（文件路径由调用方补上）。
pub fn parse_to_value(text: &str) -> Result<Value, String> {
    // 配置文件读入即做环境变量展开：口令 / data_root 等以 `${VAR}` 或
    // `${VAR:-默认}` 写占位，明文不入库、也不烘焙进镜像（P0-3 / P0-4）。
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

/// 校验 XML 语法（不解析语义）。返回 Ok 表示语法正确。
pub fn validate(text: &str) -> Result<(), String> {
    parse_to_value(text).map(|_| ())
}

/// 环境变量展开：把 `${NAME}` / `${NAME:-默认值}` 替换为对应环境变量。
///
/// - `${NAME}`：变量存在则替换为其值，否则替换为空串。
/// - `${NAME:-默认}`：变量未设置时使用「默认」。
/// - 变量名只允许 `[A-Za-z0-9_]`；`${}` 内含其它字符（如普通文本）原样保留。
///
/// 用于配置文件读入路径：DB 口令、JWT 密钥、data_root 等以占位书写，
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
                    // 非法占位：原样回填 `${...}`
                    out.push_str("${");
                    out.push_str(inner);
                    out.push('}');
                }
                rest = &after[end + 1..];
            }
            None => {
                // 未闭合：原样保留剩余（含 `${` 前缀）
                out.push_str("${");
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// XML 转义（文本 / 属性值通用）。
pub fn escape(s: &str) -> String {
    escape_text(s)
}

/// 把 JSON 值写成 XML 文本（沿用 [`parse_to_value`] 的约定）。
///
/// `root` 为根元素名。
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

// ---------------------------------------------------------------- 解析器

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

    /// 跳过 XML 声明、注释、DOCTYPE、处理指令。
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

    /// 解析一个元素（调用时 `self.i` 指向 `<`）。
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

        // 元素内容
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

/// 解码 XML 实体（`&amp;` `&lt;` `&gt;` `&quot;` `&apos;` `&#NN;` `&#xNN;`）。
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

// ---------------------------------------------------------------- 元素 → JSON

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

    // 规则 A：子元素都带 `id` 属性 → 以 id 为键的映射。
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
            // `<item id="X">文本</item>` → 映射值就是文本本身；否则取元素对象（去掉 id）。
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

    // 规则 B：子元素全是 `<item>` → 数组（通用列表标记）。
    //
    // `<item>` 是数组的专用标记，用来区分「数组」与「单键对象」——两者在 XML 里
    // 形状相同（都是一个父元素带若干同标签子元素），只有靠这个约定才能无歧义
    // 地还原。例如 `<palette><item>…</item>×9</palette>` 是数组，而
    // `<server><disk_cache/></server>` 是对象。
    if el.children.iter().all(|c| c.name == "item") {
        return Value::Array(el.children.iter().map(element_to_value).collect());
    }

    // 规则 C：以标签为键的对象；同名子元素多次出现合并为数组。
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

/// 标量文本 → JSON 值（数字 / 布尔 / null / 字符串）。
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

/// 写一个对象元素：标量成员 → 属性；对象/数组成员 → 子元素。
///
/// `extra_attrs` 额外附加的属性（如映射条目的 `id`）。
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

/// 写一个对象成员（区分“映射”与“普通对象”两种形态）。
fn write_member(out: &mut String, name: &str, value: &Value, level: usize) {
    match value {
        Value::Object(map) => {
            // 映射形态：值全是对象 → `<name><item id="key">…</item></name>`。
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
    fn 环境变量展开默认值与字面量() {
        // 未设置的变量：${NAME} → 空，${NAME:-默认} → 默认
        assert_eq!(expand_env("a=${R_MGR_NO_SUCH_VAR_XYZ}b"), "a=b");
        assert_eq!(expand_env("h=${R_MGR_NO_SUCH_VAR_XYZ:-localhost}"), "h=localhost");
        // 非法占位（名字含空格）原样保留
        assert_eq!(expand_env("keep $ {a} lit"), "keep $ {a} lit");
        // 未闭合原样保留
        assert_eq!(expand_env("tail ${OPEN"), "tail ${OPEN");
        // parse_to_value 入口即展开：未设置的口令 → 空串
        let v = parse_to_value(r#"<admin dsn="pg://${R_MGR_NO_SUCH_VAR_XYZ}@h/db"/>"#).unwrap();
        assert_eq!(v["dsn"], "pg://@h/db");
    }

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
        // 元素同时有属性与文本时，文本放在 `value` 键下。
        assert_eq!(v["value"], "t < u");
        // 纯文本元素直接是标量。
        let s = parse_to_value("<a>t &lt; u</a>").unwrap();
        assert_eq!(s.as_str().unwrap(), "t < u");
    }

    #[test]
    fn 数组的数组还原为二维数组() {
        // 回归：`palette` 这类「数组的数组」必须还原成二维数组，
        // 而不是 `{"item": [...]}`。
        let v = parse_to_value(
            r#"<palette><item><item>8</item><item>29</item><item>88</item></item><item><item>37</item><item>52</item><item>148</item></item></palette>"#,
        )
        .unwrap();
        assert_eq!(v, serde_json::json!([[8, 29, 88], [37, 52, 148]]));
    }

    #[test]
    fn 单个子元素的对象保持对象形态() {
        // 回归：只有一个子元素且该子元素是对象时，不能退化成数组。
        let v = parse_to_value(r#"<server host="0.0.0.0"><disk_cache dir="cache/foyer"/></server>"#)
            .unwrap();
        assert_eq!(v["host"], "0.0.0.0");
        assert_eq!(v["disk_cache"]["dir"], "cache/foyer");
    }

    #[test]
    fn 带属性的元素不会退化成数组() {
        // 回归：元素自带属性时，其标量子元素必须以「键 → 值」保留，不能变成数组
        // （否则 id/crs 这类属性会被悄悄丢掉）。
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
        // null 成员不写入 XML，读回时该键缺失（serde default 兜底）
        let src = serde_json::json!({"a": 1, "b": null});
        let xml = to_xml(&src, "x");
        assert!(!xml.contains("b="));
        let back = parse_to_value(&xml).unwrap();
        assert_eq!(back["a"], 1);
        assert!(back.get("b").is_none());
    }
}
