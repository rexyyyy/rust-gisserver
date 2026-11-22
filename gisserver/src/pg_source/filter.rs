//! OGC Filter XML → SQL WHERE translation (filter pushdown).
//!
//! Supports the comparison operators, `PropertyIsBetween`,
//! `PropertyIsLike`, `PropertyIsNull`, `And`/`Or`/`Not`, and the spatial
//! predicates `BBOX`, `DWithin` and `Intersects`. Anything outside this
//! subset is reported as an error (the caller must fail the request rather
//! than silently return unfiltered data).
//!
//! The translator is self-contained: a ~100-line namespace-tolerant XML
//! reader plus the recursive predicate translation. Numeric literals are
//! emitted as canonical numbers (injection-safe); geometry literals are
//! built with `ST_MakeEnvelope` / `ST_MakePoint` / `ST_GeomFromText` from
//! parsed coordinates.

use crate::pg_source::sql::quote_ident;

/// A parsed XML element (namespace prefixes kept on names, callers compare
/// local names).
#[derive(Debug, Clone, PartialEq)]
pub struct XmlElem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlElem>,
    pub text: String,
}

impl XmlElem {
    /// Local name (drops any `prefix:`).
    pub fn local(&self) -> &str {
        match self.name.split_once(':') {
            Some((_, rest)) => rest,
            None => self.name.as_str(),
        }
    }

    /// First child with this local name.
    pub fn child(&self, local: &str) -> Option<&XmlElem> {
        self.children.iter().find(|c| c.local() == local)
    }

    /// All children with this local name.
    pub fn children_named(&self, local: &str) -> Vec<&XmlElem> {
        self.children.iter().filter(|c| c.local() == local).collect()
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.split(':').next_back() == Some(name) || k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Parse a small XML document into a tree. Tolerant of the shapes OGC
/// filter documents use: single root element, nested elements, attributes,
/// text content. No DTD/entities/comments/CDATA.
pub fn parse_xml(s: &str) -> Option<XmlElem> {
    let mut stack: Vec<XmlElem> = Vec::new();
    let mut root: Option<XmlElem> = None;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if s[i..].starts_with("<!--") {
                i = s[i..].find("-->").map(|p| i + p + 3).unwrap_or(bytes.len());
                continue;
            }
            if s[i..].starts_with("<?") {
                i = s[i..].find("?>").map(|p| i + p + 2).unwrap_or(bytes.len());
                continue;
            }
            let end = s[i..].find('>').map(|p| i + p + 1)?;
            let tag = &s[i + 1..end - 1];
            if let Some(name) = tag.strip_prefix('/') {
                // closing tag
                let name = name.trim();
                if let Some(el) = stack.pop() {
                    debug_assert_eq!(el.name, name);
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(el),
                        None => {
                            if root.is_none() {
                                root = Some(el);
                            }
                        }
                    }
                }
            } else if let Some(name) = tag.strip_suffix('/') {
                // self-closing
                let (name, attrs) = split_tag(name.trim());
                let el = XmlElem {
                    name,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => {
                        if root.is_none() {
                            root = Some(el);
                        }
                    }
                }
            } else {
                let (name, attrs) = split_tag(tag.trim());
                stack.push(XmlElem {
                    name,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            i = end;
        } else {
            // text run
            let next = s[i..].find('<').map(|p| i + p).unwrap_or(bytes.len());
            let text = &s[i..next];
            if !text.trim().is_empty() {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(text.trim());
                }
            }
            i = next;
        }
    }
    root
}

/// Split `tag attr="v" attr2='v2'` into (name, attrs).
fn split_tag(tag: &str) -> (String, Vec<(String, String)>) {
    let mut parts = tag.split_whitespace();
    let name = parts.next().unwrap_or("").to_string();
    let mut attrs = Vec::new();
    for p in parts {
        if let Some((k, v)) = p.split_once('=') {
            let v = v.trim_matches(|c| c == '"' || c == '\'');
            attrs.push((k.to_string(), v.to_string()));
        }
    }
    (name, attrs)
}

/// Translate an OGC Filter document into a SQL predicate for `geom_col`
/// (table SRID `srid`). `Ok(None)` when the document carries no filter.
///
/// `Err` for malformed XML or unsupported operators — the caller must
/// surface the error, never fall back to an unfiltered query.
pub fn filter_to_where(xml: &str, geom_col: &str, srid: i32) -> Result<Option<String>, String> {
    let root = parse_xml(xml).ok_or_else(|| "malformed filter XML".to_string())?;
    let filter = if root.local() == "Filter" {
        &root
    } else {
        // tolerate <wfs:GetFeature><Filter>… shapes by searching for Filter
        root.child("Filter")
            .ok_or_else(|| "filter document has no <Filter>".to_string())?
    };
    if filter.children.is_empty() {
        return Ok(None);
    }
    Ok(Some(translate(filter, geom_col, srid)?))
}

fn translate(el: &XmlElem, geom_col: &str, srid: i32) -> Result<String, String> {
    match el.local() {
        "Filter" => match el.children.len() {
            0 => Ok("TRUE".to_string()),
            1 => translate(&el.children[0], geom_col, srid),
            _ => Err("Filter must contain exactly one operator".to_string()),
        },
        "And" | "Or" => {
            let parts: Result<Vec<String>, String> = el
                .children
                .iter()
                .map(|c| translate(c, geom_col, srid))
                .collect();
            let parts = parts?;
            if parts.is_empty() {
                return Err("empty And/Or".to_string());
            }
            let joiner = if el.local() == "And" { " AND " } else { " OR " };
            Ok(format!("({})", parts.join(joiner)))
        }
        "Not" => {
            let child = el
                .children
                .first()
                .ok_or_else(|| "Not without child".to_string())?;
            Ok(format!("NOT ({})", translate(child, geom_col, srid)?))
        }
        "PropertyIsEqualTo" | "PropertyIsNotEqualTo" | "PropertyIsLessThan"
        | "PropertyIsLessThanOrEqualTo" | "PropertyIsGreaterThan"
        | "PropertyIsGreaterThanOrEqualTo" => {
            let (prop, lit) = comparison_operands(el)?;
            let col = quote_ident(&prop);
            let op = match el.local() {
                "PropertyIsEqualTo" => "=",
                "PropertyIsNotEqualTo" => "<>",
                "PropertyIsLessThan" => "<",
                "PropertyIsLessThanOrEqualTo" => "<=",
                "PropertyIsGreaterThan" => ">",
                _ => ">=",
            };
            Ok(format!("{col} {op} {lit}"))
        }
        "PropertyIsBetween" => {
            let prop = el
                .child("PropertyName")
                .map(|e| e.text.clone())
                .ok_or_else(|| "PropertyIsBetween without PropertyName".to_string())?;
            let bounds = el.child("LowerBound").and_then(|e| e.child("Literal"));
            let upper = el.child("UpperBound").and_then(|e| e.child("Literal"));
            let (lo, hi) = match (bounds, upper) {
                (Some(a), Some(b)) => (a.text.trim(), b.text.trim()),
                _ => return Err("PropertyIsBetween without bounds".to_string()),
            };
            let col = quote_ident(&prop);
            Ok(format!(
                "{col} BETWEEN {} AND {}",
                literal_sql(lo),
                literal_sql(hi)
            ))
        }
        "PropertyIsLike" => {
            let prop = el
                .child("PropertyName")
                .map(|e| e.text.clone())
                .ok_or_else(|| "PropertyIsLike without PropertyName".to_string())?;
            let pattern = el
                .child("Literal")
                .map(|e| e.text.clone())
                .ok_or_else(|| "PropertyIsLike without Literal".to_string())?;
            let wild = el.attr("wildCard").unwrap_or("*");
            let single = el.attr("singleChar").unwrap_or("?");
            let escape = el.attr("escapeChar").unwrap_or("\\");
            let sql_pattern = like_to_sql(&pattern, wild, single, escape);
            let col = quote_ident(&prop);
            Ok(format!(
                "{col} LIKE '{}' ESCAPE '{}'",
                sql_pattern.replace('\'', "''"),
                escape.replace('\'', "''")
            ))
        }
        "PropertyIsNull" => {
            let prop = el
                .child("PropertyName")
                .map(|e| e.text.clone())
                .ok_or_else(|| "PropertyIsNull without PropertyName".to_string())?;
            Ok(format!("{} IS NULL", quote_ident(&prop)))
        }
        "BBOX" => {
            let prop = el
                .child("PropertyName")
                .map(|e| e.text.clone())
                .unwrap_or_else(|| geom_col.to_string());
            let coords = bbox_coords(el)?;
            let col = quote_ident(&prop);
            Ok(format!(
                "{col} && ST_Transform(ST_MakeEnvelope({}, {}, {}, {}, 4326), {})",
                coords[0], coords[1], coords[2], coords[3], srid
            ))
        }
        "DWithin" | "Intersects" => spatial(el, geom_col, srid),
        other => Err(format!("unsupported filter operator '{other}'")),
    }
}

/// Extract `(PropertyName, Literal)` from a comparison operator.
fn comparison_operands(el: &XmlElem) -> Result<(String, String), String> {
    let prop = el
        .child("PropertyName")
        .map(|e| e.text.trim().to_string())
        .ok_or_else(|| format!("{} without PropertyName", el.local()))?;
    // direct <Literal> child (also covers Function-less <Literal> forms)
    let lit = el
        .children
        .iter()
        .find(|c| c.local() == "Literal")
        .map(|e| e.text.trim().to_string())
        .ok_or_else(|| format!("{} without Literal", el.local()))?;
    Ok((prop, literal_sql(&lit)))
}

/// Render a literal as SQL: numbers bare, strings quoted.
fn literal_sql(v: &str) -> String {
    let v = v.trim();
    if v.eq_ignore_ascii_case("null") {
        return "NULL".to_string();
    }
    if let Ok(n) = v.parse::<f64>() {
        if n.is_finite() {
            return format!("{n}");
        }
    }
    format!("'{}'", v.replace('\'', "''"))
}

/// Convert an OGC `PropertyIsLike` pattern into SQL LIKE syntax.
fn like_to_sql(pattern: &str, wild: &str, single: &str, escape: &str) -> String {
    let mut out = String::new();
    for ch in pattern.chars() {
        if escape.chars().any(|e| e == ch) {
            out.push(ch); // keep escape as-is (SQL ESCAPE char)
            continue;
        }
        if wild.chars().any(|w| w == ch) {
            out.push('%');
        } else if single.chars().any(|s| s == ch) {
            out.push('_');
        } else if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out
}

/// BBOX coordinates from `<gml:Box><gml:coordinates>x1,y1 x2,y2` or
/// `<gml:Envelope><gml:lowerCorner>` / `<gml:upperCorner>`.
fn bbox_coords(el: &XmlElem) -> Result<[f64; 4], String> {
    for child in &el.children {
        if let Some(c) = child.child("coordinates") {
            let nums = parse_coord_list(&c.text)?;
            if nums.len() >= 4 {
                return Ok([nums[0], nums[1], nums[2], nums[3]]);
            }
        }
        if child.local() == "Envelope" || child.local() == "Box" {
            let lo = child.child("lowerCorner").map(|e| e.text.clone());
            let hi = child.child("upperCorner").map(|e| e.text.clone());
            if let (Some(lo), Some(hi)) = (lo, hi) {
                let l = parse_coord_list(&lo)?;
                let h = parse_coord_list(&hi)?;
                if l.len() >= 2 && h.len() >= 2 {
                    return Ok([l[0], l[1], h[0], h[1]]);
                }
            }
        }
        // direct coordinates child of BBOX
        if child.local() == "coordinates" {
            let nums = parse_coord_list(&child.text)?;
            if nums.len() >= 4 {
                return Ok([nums[0], nums[1], nums[2], nums[3]]);
            }
        }
    }
    Err("BBOX without usable gml:Box/Envelope coordinates".to_string())
}

/// Parse `x1,y1 x2,y2 …` (gml:coordinates) or `x1 y1 x2 y2` (gml:pos).
fn parse_coord_list(s: &str) -> Result<Vec<f64>, String> {
    let t = s.trim().replace(',', " ");
    let mut out = Vec::new();
    for tok in t.split_whitespace() {
        let v: f64 = tok
            .parse()
            .map_err(|_| format!("bad coordinate '{tok}'"))?;
        out.push(v);
    }
    Ok(out)
}

/// Translate `<DWithin>` / `<Intersects>`.
fn spatial(el: &XmlElem, geom_col: &str, srid: i32) -> Result<String, String> {
    let prop = el
        .child("PropertyName")
        .map(|e| e.text.trim().to_string())
        .unwrap_or_else(|| geom_col.to_string());
    let col = quote_ident(&prop);
    let gml = el
        .children
        .iter()
        .find(|c| c.local() != "PropertyName" && c.local() != "Distance")
        .ok_or_else(|| format!("{} without geometry", el.local()))?;
    let geom_sql = gml_to_sql(gml, srid)?;
    match el.local() {
        "DWithin" => {
            let dist = el
                .child("Distance")
                .map(|e| e.text.trim().to_string())
                .ok_or_else(|| "DWithin without Distance".to_string())?;
            let units = el.child("Distance").and_then(|e| e.attr("units"));
            let meters = match units {
                // http://schemas.opengis.net/gml/3.2.0/uom: metre vs degrees
                Some(u) if u.ends_with("degree") => {
                    let d: f64 = dist
                        .parse()
                        .map_err(|_| "bad DWithin distance".to_string())?;
                    format!("{}", d * 111320.0)
                }
                _ => literal_sql(&dist),
            };
            Ok(format!(
                "ST_DWithin({col}, {geom_sql}, {meters})"
            ))
        }
        _ => Ok(format!("ST_Intersects({col}, {geom_sql})")),
    }
}

/// Render a gml geometry literal as SQL (in EPSG:4326 input, transformed to
/// the table SRID).
fn gml_to_sql(el: &XmlElem, srid: i32) -> Result<String, String> {
    match el.local() {
        "Point" => {
            let (x, y) = point_coords(el)?;
            Ok(format!(
                "ST_Transform(ST_SetSRID(ST_MakePoint({x}, {y}), 4326), {srid})"
            ))
        }
        "Box" | "Envelope" => {
            let coords = bbox_coords(el)?;
            Ok(format!(
                "ST_Transform(ST_MakeEnvelope({}, {}, {}, {}, 4326), {})",
                coords[0], coords[1], coords[2], coords[3], srid
            ))
        }
        "Polygon" | "Surface" => {
            let rings = polygon_rings(el)?;
            if rings.is_empty() {
                return Err("polygon without rings".to_string());
            }
            let wkt = rings
                .iter()
                .map(|r| {
                    format!(
                        "({})",
                        r.iter()
                            .map(|(x, y)| format!("{x} {y}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "ST_Transform(ST_SetSRID(ST_GeomFromText('POLYGON({wkt})'), 4326), {srid})"
            ))
        }
        other => Err(format!("unsupported gml geometry '{other}'")),
    }
}

fn point_coords(el: &XmlElem) -> Result<(f64, f64), String> {
    if let Some(pos) = el.child("pos") {
        let v = parse_coord_list(&pos.text)?;
        if v.len() >= 2 {
            return Ok((v[0], v[1]));
        }
    }
    if let Some(c) = el.child("coordinates") {
        let v = parse_coord_list(&c.text)?;
        if v.len() >= 2 {
            return Ok((v[0], v[1]));
        }
    }
    Err("point without coordinates".to_string())
}

/// Extract polygon rings (outer + holes) as coordinate lists.
fn polygon_rings(el: &XmlElem) -> Result<Vec<Vec<(f64, f64)>>, String> {
    let mut rings = Vec::new();
    // <gml:exterior><gml:LinearRing><gml:posList> or <gml:coordinates>
    for ring in el
        .children_named("exterior")
        .into_iter()
        .chain(el.children_named("interior"))
    {
        if let Some(lr) = ring.child("LinearRing") {
            rings.push(ring_coords(lr)?);
        }
    }
    if rings.is_empty() {
        // bare <gml:LinearRing> children
        for lr in el.children_named("LinearRing") {
            rings.push(ring_coords(lr)?);
        }
    }
    Ok(rings)
}

fn ring_coords(lr: &XmlElem) -> Result<Vec<(f64, f64)>, String> {
    if let Some(pl) = lr.child("posList") {
        let nums = parse_coord_list(&pl.text)?;
        if nums.len() >= 4 {
            return Ok(nums.chunks(2).map(|c| (c[0], c[1])).collect());
        }
    }
    if let Some(c) = lr.child("coordinates") {
        let nums = parse_coord_list(&c.text)?;
        if nums.len() >= 4 {
            return Ok(nums.chunks(2).map(|c| (c[0], c[1])).collect());
        }
    }
    Err("LinearRing without coordinates".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaced_filter_tree() {
        let xml = r#"<wfs:GetFilter xmlns:wfs="http://www.opengis.net/wfs">
          <ogc:Filter xmlns:ogc="http://www.opengis.net/ogc">
            <ogc:PropertyIsGreaterThan>
              <ogc:PropertyName>height</ogc:PropertyName>
              <ogc:Literal>50</ogc:Literal>
            </ogc:PropertyIsGreaterThan>
          </ogc:Filter>
        </wfs:GetFilter>"#;
        let root = parse_xml(xml).unwrap();
        let f = root.child("Filter").unwrap();
        assert_eq!(f.local(), "Filter");
    }

    #[test]
    fn comparison_pushdown() {
        let xml = r#"<Filter><PropertyIsGreaterThan>
          <PropertyName>height</PropertyName><Literal>50</Literal>
        </PropertyIsGreaterThan></Filter>"#;
        let w = filter_to_where(xml, "geom", 4543).unwrap().unwrap();
        assert_eq!(w, "\"height\" > 50");
    }

    #[test]
    fn string_comparison_is_quoted() {
        let xml = r#"<Filter><PropertyIsEqualTo>
          <PropertyName>name</PropertyName><Literal>O'Brien</Literal>
        </PropertyIsEqualTo></Filter>"#;
        let w = filter_to_where(xml, "geom", 4326).unwrap().unwrap();
        assert_eq!(w, "\"name\" = 'O''Brien'");
    }

    #[test]
    fn between_and_like() {
        let xml = r#"<Filter><And>
          <PropertyIsBetween>
            <PropertyName>height</PropertyName>
            <LowerBound><Literal>10</Literal></LowerBound>
            <UpperBound><Literal>20</Literal></UpperBound>
          </PropertyIsBetween>
          <PropertyIsLike wildCard="*" singleChar="?" escapeChar="!">
            <PropertyName>name</PropertyName><Literal>a!*b?c</Literal>
          </PropertyIsLike>
        </And></Filter>"#;
        let w = filter_to_where(xml, "geom", 4326).unwrap().unwrap();
        assert_eq!(
            w,
            "(\"height\" BETWEEN 10 AND 20 AND \"name\" LIKE 'a!%b_c' ESCAPE '!')"
        );
    }

    #[test]
    fn is_null_and_not() {
        let xml = r#"<Filter><Not><PropertyIsNull>
          <PropertyName>name</PropertyName>
        </PropertyIsNull></Not></Filter>"#;
        let w = filter_to_where(xml, "geom", 4326).unwrap().unwrap();
        assert_eq!(w, "NOT (\"name\" IS NULL)");
    }

    #[test]
    fn bbox_pushdown() {
        let xml = r#"<Filter><BBOX><PropertyName>geom</PropertyName>
          <gml:Box srsName="EPSG:4326">
            <gml:coordinates>102.6,25.2 102.7,25.3</gml:coordinates>
          </gml:Box>
        </BBOX></Filter>"#;
        let w = filter_to_where(xml, "geom", 4543).unwrap().unwrap();
        assert_eq!(
            w,
            "\"geom\" && ST_Transform(ST_MakeEnvelope(102.6, 25.2, 102.7, 25.3, 4326), 4543)"
        );
    }

    #[test]
    fn dwithin_and_intersects() {
        let xml = r#"<Filter><DWithin><PropertyName>geom</PropertyName>
          <gml:Point><gml:pos>102.66 25.22</gml:pos></gml:Point>
          <Distance units="m">100</Distance>
        </DWithin></Filter>"#;
        let w = filter_to_where(xml, "geom", 4543).unwrap().unwrap();
        assert_eq!(
            w,
            "ST_DWithin(\"geom\", ST_Transform(ST_SetSRID(ST_MakePoint(102.66, 25.22), 4326), 4543), 100)"
        );
        let xml2 = r#"<Filter><Intersects><PropertyName>geom</PropertyName>
          <gml:Polygon><gml:exterior><gml:LinearRing>
            <gml:posList>102.6 25.2 102.7 25.2 102.7 25.3 102.6 25.2</gml:posList>
          </gml:LinearRing></gml:exterior></gml:Polygon>
        </Intersects></Filter>"#;
        let w2 = filter_to_where(xml2, "geom", 4543).unwrap().unwrap();
        assert!(w2.starts_with("ST_Intersects(\"geom\", ST_Transform(ST_SetSRID(ST_GeomFromText('POLYGON(("));
        assert!(w2.contains("102.6 25.2, 102.7 25.2, 102.7 25.3, 102.6 25.2"));
    }

    #[test]
    fn unsupported_operator_errors() {
        let xml = r#"<Filter><ResourceId rid="fid1"/></Filter>"#;
        assert!(filter_to_where(xml, "geom", 4326).is_err());
    }

    #[test]
    fn empty_filter_is_none() {
        let xml = "<Filter/>";
        assert_eq!(filter_to_where(xml, "geom", 4326).unwrap(), None);
    }
}
