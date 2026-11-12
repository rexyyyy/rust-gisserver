//! SLD (Styled Layer Descriptor) document model — OGC SLD 1.0 / SE 1.1 subset.
//!
//! The parser is dependency-free (hand-rolled lite XML tree) and lifts the
//! full hierarchy the rasterizer needs:
//!
//! ```text
//! StyledLayerDescriptor
//!  └ NamedLayer (Name)
//!     └ UserStyle
//!        └ FeatureTypeStyle
//!           └ Rule (Name/Title, Min/MaxScaleDenominator, Filter|ElseFilter)
//!              ├ PolygonSymbolizer (Fill, Stroke)
//!              ├ LineSymbolizer   (Stroke)
//!              ├ PointSymbolizer  (Graphic → Mark|ExternalGraphic, Size)
//!              ├ TextSymbolizer   (Label, Font, Fill, Halo, LabelPlacement)
//!              └ RasterSymbolizer (parsed, not applied — see docs)
//! ```
//!
//! **Rule evaluation**: rules are evaluated in
//! document order; *every* rule whose filter matches (and whose scale window
//! contains the request scale) contributes its symbolizers, so symbolizers
//! stack (e.g. a line rule over a fill rule). `<ElseFilter/>` matches only
//! when no earlier rule in the same `FeatureTypeStyle` matched.
//!
//! Supported filter operators: `PropertyIsEqualTo/NotEqualTo/LessThan/
//! LessThanOrEqualTo/GreaterThan/GreaterThanOrEqualTo`, `PropertyIsBetween`,
//! `PropertyIsLike`, `PropertyIsNull`, `And`, `Or`, `Not`. Spatial filters
//! (`BBOX`, `Intersects`, `DWithin`, …) parse to [`Filter::Unsupported`] and
//! never match (the rule is skipped, with a one-time warning).
//!
//! Deliberate boundaries:
//! `ExternalGraphic` marks (no PNG decoder), `RasterSymbolizer` color maps
//! (raster palettes live in the JSON `Style`), geometry expressions
//! (`<Geometry>`), recoding `<ogc:Function>` operands, and uom-aware sizes
//! (all lengths are pixels).

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json};

// ---------------------------------------------------------------------------
// Public model
// ---------------------------------------------------------------------------

/// A parsed SLD document (one user style; the first `UserStyle` wins if a
/// document carries several).
#[derive(Clone, Debug, Default)]
pub struct SldDocument {
    /// `<NamedLayer><Name>` when present.
    pub layer_name: Option<String>,
    /// `<UserStyle><Name>` when present.
    pub style_name: Option<String>,
    pub feature_type_styles: Vec<FeatureTypeStyle>,
}

/// One `<FeatureTypeStyle>`: an ordered rule list.
#[derive(Clone, Debug, Default)]
pub struct FeatureTypeStyle {
    pub rules: Vec<Rule>,
}

/// One `<Rule>`.
#[derive(Clone, Debug, Default)]
pub struct Rule {
    pub name: Option<String>,
    pub title: Option<String>,
    /// Applies when `scale >= min_scale` (zoomed in past this denominator).
    pub min_scale: Option<f64>,
    /// Applies when `scale <= max_scale` (zoomed out before this).
    pub max_scale: Option<f64>,
    /// `None` = matches everything (subject to the scale window).
    pub filter: Option<Filter>,
    pub symbolizers: Vec<Symbolizer>,
}

impl Rule {
    /// Scale-window check (SLD: applies when min <= scale <= max).
    pub fn applies_at_scale(&self, scale: f64) -> bool {
        if let Some(m) = self.min_scale {
            if scale < m - 1e-9 {
                return false;
            }
        }
        if let Some(m) = self.max_scale {
            if scale > m + 1e-9 {
                return false;
            }
        }
        true
    }
}

/// The four symbolizer kinds we parse. `Raster` is recorded so callers can
/// detect its presence, but the rasterizer ignores it.
#[derive(Clone, Debug)]
pub enum Symbolizer {
    Polygon(PolygonSymbolizer),
    Line(LineSymbolizer),
    Point(PointSymbolizer),
    Text(TextSymbolizer),
    Raster,
}

/// `<PolygonSymbolizer>`: fill + outline.
#[derive(Clone, Debug)]
pub struct PolygonSymbolizer {
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub stroke_width: f64,
    /// `stroke-dasharray` in pixels (empty = solid).
    pub dash: Vec<f64>,
}

/// `<LineSymbolizer>`.
#[derive(Clone, Debug)]
pub struct LineSymbolizer {
    pub stroke: [u8; 4],
    pub stroke_width: f64,
    pub dash: Vec<f64>,
}

/// `<PointSymbolizer>`: a well-known mark.
#[derive(Clone, Debug)]
pub struct PointSymbolizer {
    pub mark: MarkKind,
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    /// SLD graphic size (mark diameter) in pixels.
    pub size: f64,
    pub stroke_width: f64,
}

/// `<TextSymbolizer>`: label + font + halo.
#[derive(Clone, Debug)]
pub struct TextSymbolizer {
    pub label: LabelExpr,
    pub font_size: f64,
    pub bold: bool,
    pub italic: bool,
    pub fill: [u8; 4],
    pub halo_fill: Option<[u8; 4]>,
    pub halo_radius: f64,
    /// AnchorPoint (0..1 fractions of the text box).
    pub anchor: (f64, f64),
    /// Displacement in pixels.
    pub displacement: (f64, f64),
}

/// SLD well-known mark names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Circle,
    Square,
    Triangle,
    Star,
    Cross,
    X,
    Arrow,
    Hexagon,
    Pentagon,
}

impl MarkKind {
    /// SLD `<WellKnownName>` (case-insensitive). `hexagon`/`pentagon` are
    /// Extension names rendered as regular polygons.
    pub fn from_wellknown(name: &str) -> Option<MarkKind> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "circle" => MarkKind::Circle,
            "square" => MarkKind::Square,
            "triangle" => MarkKind::Triangle,
            "star" => MarkKind::Star,
            "cross" => MarkKind::Cross,
            "x" => MarkKind::X,
            "arrow" => MarkKind::Arrow,
            "hexagon" => MarkKind::Hexagon,
            "pentagon" => MarkKind::Pentagon,
            _ => return None,
        })
    }

    /// Number of sides for regular-polygon marks (0 = not a polygon).
    pub fn sides(self) -> u32 {
        match self {
            MarkKind::Hexagon => 6,
            MarkKind::Pentagon => 5,
            _ => 0,
        }
    }
}

/// A `<Label>`: a property reference or a literal string.
#[derive(Clone, Debug)]
pub enum LabelExpr {
    Property(String),
    Literal(String),
}

impl LabelExpr {
    /// Resolve against feature properties (missing property → empty label).
    pub fn resolve(&self, props: &Map<String, Json>) -> String {
        match self {
            LabelExpr::Property(p) => props
                .get(p)
                .map(|v| match v {
                    Json::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default(),
            LabelExpr::Literal(s) => s.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// A filter expression (subset). `Unsupported` marks a parsed-but-unusable
/// filter (spatial / function operands); such rules never match.
#[derive(Clone, Debug)]
pub enum Filter {
    Comparison(Comparison, Operand, Operand),
    Between(String, Operand, Operand),
    Like {
        property: String,
        pattern: String,
        wildcard: char,
        single: char,
        escape: char,
    },
    IsNull(String),
    And(Box<Filter>, Box<Filter>),
    Or(Box<Filter>, Box<Filter>),
    Not(Box<Filter>),
    /// `<ElseFilter/>`: matches iff no earlier rule in the same
    /// FeatureTypeStyle matched (resolved by the caller).
    Else,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    NotEq,
    Less,
    LessOrEq,
    Greater,
    GreaterOrEq,
}

/// A filter operand: property reference or literal.
#[derive(Clone, Debug)]
pub enum Operand {
    Property(String),
    Literal(Value),
}

/// A typed literal / property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(s) => s.trim().parse().ok(),
        }
    }
    /// Borrowed for strings, owned for numbers (no allocation for the common
    /// string-compare path).
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Value::Num(n) => std::borrow::Cow::Owned(n.to_string()),
            Value::Str(s) => std::borrow::Cow::Borrowed(s.as_str()),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Num(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s}"),
        }
    }
}

impl Filter {
    /// Evaluate against feature properties. `Else` always returns `true` here;
    /// the caller resolves else-semantics across a rule list.
    pub fn matches(&self, props: &Map<String, Json>) -> bool {
        match self {
            Filter::Comparison(op, a, b) => {
                let (Some(va), Some(vb)) = (resolve_operand(a, props), resolve_operand(b, props))
                else {
                    return false;
                };
                let Some(ord) = compare_values(&va, &vb) else {
                    return false;
                };
                match op {
                    Comparison::Eq => ord == std::cmp::Ordering::Equal,
                    Comparison::NotEq => ord != std::cmp::Ordering::Equal,
                    Comparison::Less => ord == std::cmp::Ordering::Less,
                    Comparison::LessOrEq => ord != std::cmp::Ordering::Greater,
                    Comparison::Greater => ord == std::cmp::Ordering::Greater,
                    Comparison::GreaterOrEq => ord != std::cmp::Ordering::Less,
                }
            }
            Filter::Between(prop, lo, hi) => {
                let Some(v) = resolve_operand(&Operand::Property(prop.clone()), props) else {
                    return false;
                };
                let (Some(lo), Some(hi)) = (
                    resolve_operand(lo, props).and_then(|x| x.as_f64()),
                    resolve_operand(hi, props).and_then(|x| x.as_f64()),
                ) else {
                    return false;
                };
                let Some(x) = v.as_f64() else { return false };
                x >= lo.min(hi) && x <= lo.max(hi)
            }
            Filter::Like {
                property,
                pattern,
                wildcard,
                single,
                escape,
            } => {
                let Some(v) = resolve_operand(&Operand::Property(property.clone()), props) else {
                    return false;
                };
                like_match(&v.as_str(), pattern, *wildcard, *single, *escape)
            }
            Filter::IsNull(prop) => props
                .get(prop)
                .map(|v| v.is_null())
                .unwrap_or(true),
            Filter::And(a, b) => a.matches(props) && b.matches(props),
            Filter::Or(a, b) => a.matches(props) || b.matches(props),
            Filter::Not(a) => !a.matches(props),
            Filter::Else => true,
            Filter::Unsupported => false,
        }
    }
}

fn resolve_operand(op: &Operand, props: &Map<String, Json>) -> Option<Value> {
    match op {
        Operand::Literal(v) => Some(v.clone()),
        Operand::Property(p) => props.get(p).and_then(json_to_value),
    }
}

fn json_to_value(v: &Json) -> Option<Value> {
    match v {
        Json::Number(n) => n.as_f64().map(Value::Num),
        Json::String(s) => Some(Value::Str(s.clone())),
        Json::Bool(b) => Some(Value::Str(b.to_string())),
        _ => None,
    }
}

/// Numeric compare when both sides are numbers, else string compare.
fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x.partial_cmp(y),
        _ => Some(a.as_str().cmp(&b.as_str())),
    }
}

/// SLD `PropertyIsLike` wildcard matcher (`*` any run, `?` one char,
/// `escape` prefixes a literal wildcard).
fn like_match(text: &str, pattern: &str, wildcard: char, single: char, escape: char) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    like_rec(&t, 0, &p, 0, wildcard, single, escape)
}

fn like_rec(
    t: &[char],
    mut ti: usize,
    p: &[char],
    mut pi: usize,
    wildcard: char,
    single: char,
    escape: char,
) -> bool {
    while pi < p.len() {
        let pc = p[pi];
        if pc == escape && pi + 1 < p.len() {
            // Escaped literal: must match the next pattern char exactly.
            if ti >= t.len() || t[ti] != p[pi + 1] {
                return false;
            }
            ti += 1;
            pi += 2;
            continue;
        }
        if pc == wildcard {
            // Try to match the rest of the pattern at every suffix.
            pi += 1;
            if pi == p.len() {
                return true; // trailing '*'
            }
            while ti <= t.len() {
                if like_rec(t, ti, p, pi, wildcard, single, escape) {
                    return true;
                }
                if ti == t.len() {
                    break;
                }
                ti += 1;
            }
            return false;
        }
        if ti >= t.len() {
            return false;
        }
        if pc == single || pc == t[ti] {
            ti += 1;
            pi += 1;
        } else {
            return false;
        }
    }
    ti == t.len()
}

// ---------------------------------------------------------------------------
// Document evaluation
// ---------------------------------------------------------------------------

impl SldDocument {
    /// Parse an SLD document. Returns `None` when the document is not an SLD
    /// (no `FeatureTypeStyle` found).
    pub fn parse(doc: &str) -> Option<SldDocument> {
        let root = parse_root(doc)?;
        if !local(&root.name).eq_ignore_ascii_case("StyledLayerDescriptor") {
            return None;
        }
        let mut out = SldDocument::default();
        let mut fts_nodes: Vec<&Node> = Vec::new();
        for child in &root.children {
            let ln = local(&child.name).to_ascii_lowercase();
            match ln.as_str() {
                "namedlayer" => {
                    if out.layer_name.is_none() {
                        out.layer_name = child_leaf(child, "name");
                    }
                    for sub in &child.children {
                        let sl = local(&sub.name).to_ascii_lowercase();
                        match sl.as_str() {
                            "userstyle" => {
                                if out.style_name.is_none() {
                                    out.style_name = child_leaf(sub, "name");
                                }
                                for fts in &sub.children {
                                    if local(&fts.name).eq_ignore_ascii_case("FeatureTypeStyle") {
                                        fts_nodes.push(fts);
                                    }
                                }
                            }
                            "featuretypestyle" => fts_nodes.push(sub),
                            _ => {}
                        }
                    }
                }
                "userstyle" => {
                    if out.style_name.is_none() {
                        out.style_name = child_leaf(child, "name");
                    }
                    for fts in &child.children {
                        if local(&fts.name).eq_ignore_ascii_case("FeatureTypeStyle") {
                            fts_nodes.push(fts);
                        }
                    }
                }
                "featuretypestyle" => fts_nodes.push(child),
                _ => {}
            }
        }
        if fts_nodes.is_empty() {
            return None;
        }
        out.feature_type_styles = fts_nodes.iter().filter_map(|n| parse_fts(n)).collect();
        if out.feature_type_styles.is_empty() {
            return None;
        }
        Some(out)
    }

    /// Rules that apply to a feature at the given scale denominator, in
    /// document order (all matching rules stack;
    /// `ElseFilter` matches only when no earlier rule in the same
    /// FeatureTypeStyle matched).
    pub fn matching_rules(&self, props: &Map<String, Json>, scale: f64) -> Vec<&Rule> {
        let mut out = Vec::new();
        for fts in &self.feature_type_styles {
            let mut any_matched = false;
            for rule in &fts.rules {
                if !rule.applies_at_scale(scale) {
                    continue;
                }
                let matched = match &rule.filter {
                    None => true,
                    Some(Filter::Else) => !any_matched,
                    Some(f) => f.matches(props),
                };
                if matched {
                    any_matched = true;
                    out.push(rule);
                }
            }
        }
        out
    }

    /// Symbolizers of the first rule that matches (legend convenience).
    pub fn first_matching_symbolizers(
        &self,
        props: &Map<String, Json>,
        scale: f64,
    ) -> Vec<&Symbolizer> {
        self.matching_rules(props, scale)
            .into_iter()
            .flat_map(|r| r.symbolizers.iter())
            .collect()
    }
}

fn parse_fts(node: &Node) -> Option<FeatureTypeStyle> {
    let mut rules = Vec::new();
    for child in &node.children {
        if local(&child.name).eq_ignore_ascii_case("Rule") {
            rules.push(parse_rule(child));
        }
    }
    if rules.is_empty() {
        return None;
    }
    Some(FeatureTypeStyle { rules })
}

fn parse_rule(node: &Node) -> Rule {
    let mut rule = Rule::default();
    for child in &node.children {
        let ln = local(&child.name).to_ascii_lowercase();
        match ln.as_str() {
            "name" => rule.name = Some(child.text.clone()).filter(|s| !s.is_empty()),
            "title" => rule.title = Some(child.text.clone()).filter(|s| !s.is_empty()),
            "minscaledenominator" => {
                rule.min_scale = child.text.trim().parse::<f64>().ok();
            }
            "maxscaledenominator" => {
                rule.max_scale = child.text.trim().parse::<f64>().ok();
            }
            "filter" => rule.filter = Some(parse_filter(child)),
            "elsefilter" => rule.filter = Some(Filter::Else),
            "polygonsymbolizer" => {
                if let Some(s) = parse_polygon(child) {
                    rule.symbolizers.push(Symbolizer::Polygon(s));
                }
            }
            "linesymbolizer" => {
                if let Some(s) = parse_line(child) {
                    rule.symbolizers.push(Symbolizer::Line(s));
                }
            }
            "pointsymbolizer" => {
                if let Some(s) = parse_point(child) {
                    rule.symbolizers.push(Symbolizer::Point(s));
                }
            }
            "textsymbolizer" => {
                if let Some(s) = parse_text(child) {
                    rule.symbolizers.push(Symbolizer::Text(s));
                }
            }
            "rastersymbolizer" => rule.symbolizers.push(Symbolizer::Raster),
            _ => {}
        }
    }
    rule
}

fn parse_polygon(node: &Node) -> Option<PolygonSymbolizer> {
    let fill_node = child_elem(node, "Fill");
    let stroke_node = child_elem(node, "Stroke");
    let fill = fill_node
        .and_then(|f| svg_param(f, "fill"))
        .and_then(parse_color)
        .unwrap_or([128, 128, 128, 255]);
    let fill_op = fill_node
        .and_then(|f| svg_param(f, "fill-opacity"))
        .and_then(parse_unit)
        .unwrap_or(1.0);
    let stroke = stroke_node
        .and_then(|s| svg_param(s, "stroke"))
        .and_then(parse_color)
        .unwrap_or([0, 0, 0, 255]);
    let stroke_op = stroke_node
        .and_then(|s| svg_param(s, "stroke-opacity"))
        .and_then(parse_unit)
        .unwrap_or(1.0);
    let stroke_width = stroke_node
        .and_then(|s| svg_param(s, "stroke-width"))
        .and_then(parse_f64)
        .unwrap_or(1.0);
    let dash = stroke_node
        .and_then(|s| svg_param(s, "stroke-dasharray"))
        .map(|s| parse_dasharray(s))
        .unwrap_or_default();
    Some(PolygonSymbolizer {
        fill: with_alpha(fill, fill_op),
        stroke: with_alpha(stroke, stroke_op),
        stroke_width,
        dash,
    })
}

fn parse_line(node: &Node) -> Option<LineSymbolizer> {
    let stroke_node = child_elem(node, "Stroke")?;
    let stroke = svg_param(stroke_node, "stroke")
        .and_then(parse_color)
        .unwrap_or([0, 0, 0, 255]);
    let stroke_op = svg_param(stroke_node, "stroke-opacity")
        .and_then(parse_unit)
        .unwrap_or(1.0);
    let stroke_width = svg_param(stroke_node, "stroke-width")
        .and_then(parse_f64)
        .unwrap_or(1.0);
    let dash = svg_param(stroke_node, "stroke-dasharray")
        .map(parse_dasharray)
        .unwrap_or_default();
    Some(LineSymbolizer {
        stroke: with_alpha(stroke, stroke_op),
        stroke_width,
        dash,
    })
}

fn parse_point(node: &Node) -> Option<PointSymbolizer> {
    let graphic = child_elem(node, "Graphic")?;
    let size = child_elem(graphic, "Size")
        .or_else(|| child_elem(graphic, "size"))
        .and_then(|s| s.text.trim().parse::<f64>().ok())
        .unwrap_or(6.0);
    // ExternalGraphic is a documented boundary: fall back to a circle.
    let mark_node = child_elem(graphic, "Mark");
    let mark = mark_node
        .and_then(|m| child_elem(m, "WellKnownName"))
        .and_then(|w| MarkKind::from_wellknown(&w.text))
        .unwrap_or(MarkKind::Circle);
    let fill = mark_node
        .and_then(|m| child_elem(m, "Fill"))
        .and_then(|f| svg_param(f, "fill"))
        .and_then(parse_color)
        .unwrap_or([200, 60, 60, 255]);
    let fill_op = mark_node
        .and_then(|m| child_elem(m, "Fill"))
        .and_then(|f| svg_param(f, "fill-opacity"))
        .and_then(parse_unit)
        .unwrap_or(1.0);
    let stroke = mark_node
        .and_then(|m| child_elem(m, "Stroke"))
        .and_then(|s| svg_param(s, "stroke"))
        .and_then(parse_color)
        .unwrap_or([0, 0, 0, 255]);
    let stroke_op = mark_node
        .and_then(|m| child_elem(m, "Stroke"))
        .and_then(|s| svg_param(s, "stroke-opacity"))
        .and_then(parse_unit)
        .unwrap_or(1.0);
    let stroke_width = mark_node
        .and_then(|m| child_elem(m, "Stroke"))
        .and_then(|s| svg_param(s, "stroke-width"))
        .and_then(parse_f64)
        .unwrap_or(1.0);
    Some(PointSymbolizer {
        mark,
        fill: with_alpha(fill, fill_op),
        stroke: with_alpha(stroke, stroke_op),
        size,
        stroke_width,
    })
}

fn parse_text(node: &Node) -> Option<TextSymbolizer> {
    // Label: <Label><ogc:PropertyName>name</...></Label> or <ogc:Literal>
    let label_node = child_elem(node, "Label")?;
    let prop = find_descendant(label_node, "PropertyName");
    let lit = find_descendant(label_node, "Literal");
    let label = if let Some(p) = prop {
        LabelExpr::Property(p.text.clone())
    } else if let Some(l) = lit {
        LabelExpr::Literal(l.text.clone())
    } else {
        LabelExpr::Literal(label_node.text.clone())
    };
    let font = child_elem(node, "Font");
    let font_size = font
        .and_then(|f| svg_param(f, "font-size"))
        .and_then(parse_f64)
        .unwrap_or(10.0);
    let bold = font
        .and_then(|f| svg_param(f, "font-weight"))
        .map(|w| w.eq_ignore_ascii_case("bold"))
        .unwrap_or(false);
    let italic = font
        .and_then(|f| svg_param(f, "font-style"))
        .map(|w| w.eq_ignore_ascii_case("italic"))
        .unwrap_or(false);
    let fill = child_elem(node, "Fill")
        .and_then(|f| svg_param(f, "fill"))
        .and_then(parse_color)
        .unwrap_or([0, 0, 0, 255]);
    let fill_op = child_elem(node, "Fill")
        .and_then(|f| svg_param(f, "fill-opacity"))
        .and_then(parse_unit)
        .unwrap_or(1.0);
    let halo_node = child_elem(node, "Halo");
    let halo_fill = halo_node
        .and_then(|h| child_elem(h, "Fill"))
        .and_then(|f| svg_param(f, "fill"))
        .and_then(parse_color);
    let halo_radius = halo_node
        .and_then(|h| child_elem(h, "Radius"))
        .and_then(|r| r.text.trim().parse::<f64>().ok())
        .unwrap_or(1.0);
    // LabelPlacement → PointPlacement → AnchorPoint/Displacement.
    let mut anchor = (0.5, 0.5);
    let mut displacement = (0.0, 0.0);
    if let Some(placement) = child_elem(node, "LabelPlacement") {
        if let Some(pp) = child_elem(placement, "PointPlacement") {
            if let Some(ap) = child_elem(pp, "AnchorPoint") {
                anchor = (
                    child_elem(ap, "AnchorPointX")
                        .and_then(|x| x.text.trim().parse().ok())
                        .unwrap_or(0.5),
                    child_elem(ap, "AnchorPointY")
                        .and_then(|y| y.text.trim().parse().ok())
                        .unwrap_or(0.5),
                );
            }
            if let Some(d) = child_elem(pp, "Displacement") {
                displacement = (
                    child_elem(d, "DisplacementX")
                        .and_then(|x| x.text.trim().parse().ok())
                        .unwrap_or(0.0),
                    child_elem(d, "DisplacementY")
                        .and_then(|y| y.text.trim().parse().ok())
                        .unwrap_or(0.0),
                );
            }
        }
    }
    Some(TextSymbolizer {
        label,
        font_size,
        bold,
        italic,
        fill: with_alpha(fill, fill_op),
        halo_fill: halo_fill.map(|c| with_alpha(c, 1.0)),
        halo_radius,
        anchor,
        displacement,
    })
}

// ---------------------------------------------------------------------------
// Filter parsing
// ---------------------------------------------------------------------------

fn parse_filter(node: &Node) -> Filter {
    // <ogc:Filter> wraps one operator element.
    let Some(op) = node.children.first() else {
        return Filter::Unsupported;
    };
    parse_filter_node(op)
}

fn parse_filter_node(node: &Node) -> Filter {
    let ln = local(&node.name).to_ascii_lowercase();
    match ln.as_str() {
        "and" | "or" if node.children.len() == 2 => {
            let a = Box::new(parse_filter_node(&node.children[0]));
            let b = Box::new(parse_filter_node(&node.children[1]));
            if ln == "and" {
                Filter::And(a, b)
            } else {
                Filter::Or(a, b)
            }
        }
        "not" if node.children.len() == 1 => {
            Filter::Not(Box::new(parse_filter_node(&node.children[0])))
        }
        "propertyisequalto" | "propertyisnotequalto" | "propertyislessthan"
        | "propertyislessthanorequalto" | "propertyisgreaterthan"
        | "propertyisgreaterthanorequalto" => {
            let op = match ln.as_str() {
                "propertyisequalto" => Comparison::Eq,
                "propertyisnotequalto" => Comparison::NotEq,
                "propertyislessthan" => Comparison::Less,
                "propertyislessthanorequalto" => Comparison::LessOrEq,
                "propertyisgreaterthan" => Comparison::Greater,
                _ => Comparison::GreaterOrEq,
            };
            let mut operands = Vec::new();
            for child in &node.children {
                if let Some(o) = parse_operand(child) {
                    operands.push(o);
                }
            }
            if operands.len() == 2 {
                Filter::Comparison(op, operands.remove(0), operands.remove(0))
            } else {
                Filter::Unsupported
            }
        }
        "propertyisbetween" => {
            let prop = find_descendant(node, "PropertyName").map(|p| p.text.clone());
            let lo = child_elem(node, "LowerBoundary").and_then(|b| b.children.first());
            let hi = child_elem(node, "UpperBoundary").and_then(|b| b.children.first());
            match (prop, lo, hi) {
                (Some(p), Some(lo), Some(hi)) => {
                    let lo = parse_operand(lo).unwrap_or(Operand::Literal(Value::Str(String::new())));
                    let hi = parse_operand(hi).unwrap_or(Operand::Literal(Value::Str(String::new())));
                    Filter::Between(p, lo, hi)
                }
                _ => Filter::Unsupported,
            }
        }
        "propertyislike" => {
            let prop = find_descendant(node, "PropertyName").map(|p| p.text.clone());
            let lit = find_descendant(node, "Literal").map(|l| l.text.clone());
            match (prop, lit) {
                (Some(p), Some(pattern)) => Filter::Like {
                    property: p,
                    pattern,
                    wildcard: node
                        .attr("wildCard")
                        .and_then(|c| c.chars().next())
                        .unwrap_or('*'),
                    single: node
                        .attr("singleChar")
                        .and_then(|c| c.chars().next())
                        .unwrap_or('?'),
                    escape: node.attr("escape").and_then(|c| c.chars().next()).unwrap_or('!'),
                },
                _ => Filter::Unsupported,
            }
        }
        "propertyisnull" => find_descendant(node, "PropertyName")
            .map(|p| Filter::IsNull(p.text.clone()))
            .unwrap_or(Filter::Unsupported),
        // Spatial filters / functions: parsed, never match.
        _ => Filter::Unsupported,
    }
}

fn parse_operand(node: &Node) -> Option<Operand> {
    let ln = local(&node.name).to_ascii_lowercase();
    match ln.as_str() {
        "propertyname" => Some(Operand::Property(node.text.clone())),
        "literal" => {
            let t = node.text.trim();
            if let Ok(n) = t.parse::<f64>() {
                Some(Operand::Literal(Value::Num(n)))
            } else {
                Some(Operand::Literal(Value::Str(node.text.clone())))
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Value parsing (colors / units / dash arrays)
// ---------------------------------------------------------------------------

/// `#rgb` | `#rrggbb` | `#rrggbbaa` | `rgb()`/`rgba()` | CSS names → rgba.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        let digits: Vec<u32> = hex.chars().filter_map(|c| c.to_digit(16)).collect();
        match digits.len() {
            3 => {
                return Some([
                    (digits[0] * 17) as u8,
                    (digits[1] * 17) as u8,
                    (digits[2] * 17) as u8,
                    255,
                ])
            }
            6 | 8 => {
                let r = digits[0] * 16 + digits[1];
                let g = digits[2] * 16 + digits[3];
                let b = digits[4] * 16 + digits[5];
                let a = if digits.len() == 8 {
                    digits[6] * 16 + digits[7]
                } else {
                    255
                };
                return Some([r as u8, g as u8, b as u8, a as u8]);
            }
            _ => return None,
        }
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("rgb(") || lower.starts_with("rgba(") {
        let inner = lower
            .trim_start_matches("rgba")
            .trim_start_matches("rgb")
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .to_string();
        let parts: Vec<&str> = inner
            .split([',', ' '])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() >= 3 {
            let r = parts[0].parse::<f64>().ok()? as u8;
            let g = parts[1].parse::<f64>().ok()? as u8;
            let b = parts[2].parse::<f64>().ok()? as u8;
            let a = if parts.len() >= 4 {
                (parts[3].parse::<f64>().ok()? * 255.0).round().clamp(0.0, 255.0) as u8
            } else {
                255
            };
            return Some([r, g, b, a]);
        }
        return None;
    }
    let named: [u8; 4] = match lower.as_str() {
        "black" => [0, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        "red" => [255, 0, 0, 255],
        "darkred" => [139, 0, 0, 255],
        "green" => [0, 128, 0, 255],
        "darkgreen" => [0, 100, 0, 255],
        "lightgreen" => [144, 238, 144, 255],
        "blue" => [0, 0, 255, 255],
        "darkblue" => [0, 0, 139, 255],
        "lightblue" => [173, 216, 230, 255],
        "yellow" => [255, 255, 0, 255],
        "orange" => [255, 165, 0, 255],
        "purple" => [128, 0, 128, 255],
        "gray" | "grey" => [128, 128, 128, 255],
        "lightgray" | "lightgrey" => [211, 211, 211, 255],
        "darkgray" | "darkgrey" => [169, 169, 169, 255],
        "cyan" => [0, 255, 255, 255],
        "magenta" => [255, 0, 255, 255],
        "brown" => [165, 42, 42, 255],
        "pink" => [255, 192, 203, 255],
        "transparent" => [0, 0, 0, 0],
        _ => return None,
    };
    Some(named)
}

/// Parse a 0..=1 unit (opacity); tolerant of "1", "0.6", "60%".
pub fn parse_unit(s: &str) -> Option<f64> {
    let t = s.trim();
    let has_pct = t.ends_with('%');
    let t = t.trim_end_matches('%');
    let v: f64 = t.parse().ok()?;
    let v = if has_pct { v / 100.0 } else { v };
    Some(v.clamp(0.0, 1.0))
}

/// Parse a plain number, tolerating trailing units ("2px").
pub fn parse_f64(s: &str) -> Option<f64> {
    let digits: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    digits.parse::<f64>().ok()
}

/// `stroke-dasharray` "4 2" | "4,2" → [4.0, 2.0] (pixels).
pub fn parse_dasharray(s: &str) -> Vec<f64> {
    s.split([',', ' '])
        .filter_map(|p| p.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect()
}

fn with_alpha(rgb: [u8; 4], opacity: f64) -> [u8; 4] {
    let a = (rgb[3] as f64 * opacity).round().clamp(0.0, 255.0) as u8;
    [rgb[0], rgb[1], rgb[2], a]
}

// ---------------------------------------------------------------------------
// Lite XML tree
// ---------------------------------------------------------------------------

struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn new(name: String, attrs: Vec<(String, String)>) -> Node {
        Node {
            name,
            attrs,
            children: Vec::new(),
            text: String::new(),
        }
    }

    fn leaf(name: String, attrs: Vec<(String, String)>) -> Node {
        Node {
            name,
            attrs,
            children: Vec::new(),
            text: String::new(),
        }
    }

    /// Attribute value by (case-insensitive) name.
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Local name of a possibly-prefixed tag (`se:Rule` → `rule`-preserving case;
/// callers compare case-insensitively).
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// First direct child element with the given local name (case-insensitive).
fn child_elem<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
    node.children
        .iter()
        .find(|c| local(&c.name).eq_ignore_ascii_case(name))
}

/// First direct child *leaf* (no element children) with the given local name.
fn child_leaf(node: &Node, name: &str) -> Option<String> {
    child_elem(node, name).map(|c| c.text.clone())
}

/// Depth-first search for the first element with the given local name.
fn find_descendant<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
    for c in &node.children {
        if local(&c.name).eq_ignore_ascii_case(name) {
            return Some(c);
        }
        if let Some(found) = find_descendant(c, name) {
            return Some(found);
        }
    }
    None
}

/// Recursive search for a `SvgParameter` / `CssParameter` with `name=key`
/// (case-insensitive), returning its text.
fn svg_param<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    for c in &node.children {
        let ln = local(&c.name);
        if (ln.eq_ignore_ascii_case("SvgParameter") || ln.eq_ignore_ascii_case("CssParameter"))
            && c.attr("name").map(|v| v.eq_ignore_ascii_case(key)).unwrap_or(false)
        {
            return Some(c.text.as_str());
        }
        if let Some(found) = svg_param(c, key) {
            return Some(found);
        }
    }
    None
}

/// Parse the document root element (skipping declaration/comments/doctype).
fn parse_root(doc: &str) -> Option<Node> {
    let mut i = 0usize;
    loop {
        let rel = doc[i..].find('<')?;
        let lt = i + rel;
        let rest = &doc[lt..];
        if rest.starts_with("<!--") {
            i = lt + rest.find("-->")? + 3;
            continue;
        }
        if rest.starts_with("<?") {
            i = lt + rest.find('>')? + 1;
            continue;
        }
        if rest.starts_with("<!") {
            i = lt + rest.find('>')? + 1;
            continue;
        }
        let (node, _) = parse_element(doc, lt)?;
        return Some(node);
    }
}

/// Parse one element starting at the `<` at `start`; returns the node and the
/// byte index just past its close tag.
fn parse_element(doc: &str, start: usize) -> Option<(Node, usize)> {
    let bytes = doc.as_bytes();
    if bytes.get(start) != Some(&b'<') {
        return None;
    }
    let mut i = start + 1;
    // Tag name.
    let name_start = i;
    while i < bytes.len()
        && !matches!(
            bytes[i],
            b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r'
        )
    {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let raw_name = doc[name_start..i].to_string();
    if raw_name.is_empty() {
        return None;
    }
    // Attributes.
    let mut attrs: Vec<(String, String)> = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        if bytes[i] == b'/' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                return Some((Node::leaf(raw_name, attrs), i + 2));
            }
            return None;
        }
        // Attribute name.
        let a_start = i;
        while i < bytes.len()
            && !matches!(
                bytes[i],
                b'=' | b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r'
            )
        {
            i += 1;
        }
        let a_name = doc[a_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue; // valueless attribute: skip
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            continue;
        }
        let quote = bytes[i];
        i += 1;
        let v_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        attrs.push((a_name, doc[v_start..i].to_string()));
        i += 1;
    }
    // Children / text until the matching close tag.
    let mut node = Node::new(raw_name.clone(), attrs);
    let mut text = String::new();
    loop {
        let rel = doc[i..].find('<')?;
        let lt = i + rel;
        if lt > i {
            text.push_str(&doc[i..lt]);
        }
        let rest = &doc[lt..];
        if rest.starts_with("<!--") {
            i = lt + rest.find("-->")? + 3;
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i = lt + rest.find('>')? + 1;
            continue;
        }
        if rest.starts_with("</") {
            let gt = rest.find('>')?;
            let close_name = rest[2..gt].trim();
            if !close_name.eq_ignore_ascii_case(&raw_name) {
                return None; // mismatched nesting
            }
            node.text = unescape(text.trim());
            return Some((node, lt + gt + 1));
        }
        let (child, next) = parse_element(doc, lt)?;
        node.children.push(child);
        i = next;
    }
}

/// Resolve the five XML entities (+ numeric refs).
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let end = after.find(';');
        let Some(end) = end else {
            out.push('&');
            rest = after;
            continue;
        };
        let ent = &after[..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                if let Some(hex) = ent.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = ent.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match ch {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(ent);
                out.push(';');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Rule bookkeeping for one-time spatial-filter warnings (avoids log spam on
/// every tile render).
pub static SPATIAL_FILTER_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Mark a rule's unsupported filter as warned; returns true the first time.
pub fn warn_spatial_once() -> bool {
    !SPATIAL_FILTER_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed)
}

/// Convenience: properties map type alias used across the crate boundary.
pub type Props = BTreeMap<String, Json>;

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, Json)]) -> Map<String, Json> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    // ---- XML tree ----

    #[test]
    fn xml_tree_nested_and_self_closing() {
        let doc = r#"<?xml version="1.0"?>
        <!-- a comment -->
        <root xmlns:se="urn:x">
          <a attr="1" other='2'/>
          <b>text &amp; more</b>
          <se:c><d>deep</d></se:c>
        </root>"#;
        let root = parse_root(doc).unwrap();
        assert_eq!(local(&root.name), "root");
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[0].attr("attr"), Some("1"));
        assert_eq!(root.children[0].attr("OTHER"), Some("2"));
        assert_eq!(root.children[1].text, "text & more");
        assert_eq!(local(&root.children[2].name), "c");
        assert_eq!(root.children[2].children[0].text, "deep");
    }

    #[test]
    fn xml_tree_rejects_non_sld() {
        assert!(parse_root("<notsld/>").is_some());
        assert!(SldDocument::parse("<notsld/>").is_none());
        assert!(SldDocument::parse("not xml at all").is_none());
    }

    // ---- Full document parse ----

    const SAMPLE_SLD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<StyledLayerDescriptor version="1.1.0"
    xmlns="http://www.opengis.net/sld"
    xmlns:se="http://www.opengis.net/se"
    xmlns:ogc="http://www.opengis.net/ogc">
  <NamedLayer>
    <se:Name>places</se:Name>
    <UserStyle>
      <se:Name>cities</se:Name>
      <se:FeatureTypeStyle>
        <se:Rule>
          <se:Name>big</se:Name>
          <ogc:Filter>
            <ogc:PropertyIsGreaterThan>
              <ogc:PropertyName>pop</ogc:PropertyName>
              <ogc:Literal>8000000</ogc:Literal>
            </ogc:PropertyIsGreaterThan>
          </ogc:Filter>
          <se:PointSymbolizer>
            <se:Graphic>
              <se:Mark>
                <se:WellKnownName>star</se:WellKnownName>
                <se:Fill><se:SvgParameter name="fill">#ffcc00</se:SvgParameter></se:Fill>
                <se:Stroke><se:SvgParameter name="stroke">#7a5200</se:SvgParameter></se:Stroke>
              </se:Mark>
              <se:Size>14</se:Size>
            </se:Graphic>
          </se:PointSymbolizer>
          <se:TextSymbolizer>
            <se:Label><ogc:PropertyName>name</ogc:PropertyName></se:Label>
            <se:Font>
              <se:SvgParameter name="font-size">11</se:SvgParameter>
              <se:SvgParameter name="font-weight">bold</se:SvgParameter>
            </se:Font>
            <se:Fill><se:SvgParameter name="fill">#202020</se:SvgParameter></se:Fill>
            <se:Halo><se:Radius>1.5</se:Radius><se:Fill><se:SvgParameter name="fill">white</se:SvgParameter></se:Fill></se:Halo>
          </se:TextSymbolizer>
        </se:Rule>
        <se:Rule>
          <se:Name>small</se:Name>
          <ogc:Filter>
            <ogc:PropertyIsLessThanOrEqualTo>
              <ogc:PropertyName>pop</ogc:PropertyName>
              <ogc:Literal>8000000</ogc:Literal>
            </ogc:PropertyIsLessThanOrEqualTo>
          </ogc:Filter>
          <se:PointSymbolizer>
            <se:Graphic>
              <se:Mark><se:WellKnownName>circle</se:WellKnownName>
                <se:Fill><se:SvgParameter name="fill">#3388cc</se:SvgParameter></se:Fill>
              </se:Mark>
              <se:Size>8</se:Size>
            </se:Graphic>
          </se:PointSymbolizer>
        </se:Rule>
      </se:FeatureTypeStyle>
    </UserStyle>
  </NamedLayer>
</StyledLayerDescriptor>"#;

    #[test]
    fn parses_full_document() {
        let doc = SldDocument::parse(SAMPLE_SLD).unwrap();
        assert_eq!(doc.layer_name.as_deref(), Some("places"));
        assert_eq!(doc.style_name.as_deref(), Some("cities"));
        assert_eq!(doc.feature_type_styles.len(), 1);
        let rules = &doc.feature_type_styles[0].rules;
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name.as_deref(), Some("big"));
        assert_eq!(rules[1].name.as_deref(), Some("small"));
        // Rule 0: filter + star mark + text.
        match &rules[0].symbolizers[0] {
            Symbolizer::Point(p) => {
                assert_eq!(p.mark, MarkKind::Star);
                assert_eq!(p.size, 14.0);
                assert_eq!(p.fill, [255, 204, 0, 255]);
                assert_eq!(p.stroke, [122, 82, 0, 255]);
            }
            other => panic!("expected Point, got {other:?}"),
        }
        match &rules[0].symbolizers[1] {
            Symbolizer::Text(t) => {
                assert!(matches!(t.label, LabelExpr::Property(ref p) if p == "name"));
                assert_eq!(t.font_size, 11.0);
                assert!(t.bold);
                assert!(!t.italic);
                assert_eq!(t.fill, [32, 32, 32, 255]);
                assert_eq!(t.halo_fill, Some([255, 255, 255, 255]));
                assert_eq!(t.halo_radius, 1.5);
            }
            other => panic!("expected Text, got {other:?}"),
        }
        // Rule 1: circle.
        match &rules[1].symbolizers[0] {
            Symbolizer::Point(p) => {
                assert_eq!(p.mark, MarkKind::Circle);
                assert_eq!(p.size, 8.0);
            }
            other => panic!("expected Point, got {other:?}"),
        }
    }

    #[test]
    fn rule_evaluation_stacks_all_matching_rules() {
        let doc = SldDocument::parse(SAMPLE_SLD).unwrap();
        let big = props(&[("name", Json::from("Kunming")), ("pop", Json::from(8460088.0))]);
        let small = props(&[("name", Json::from("Dali")), ("pop", Json::from(400000.0))]);
        let scale = 100000.0;
        let big_rules = doc.matching_rules(&big, scale);
        assert_eq!(big_rules.len(), 1);
        assert_eq!(big_rules[0].name.as_deref(), Some("big"));
        let small_rules = doc.matching_rules(&small, scale);
        assert_eq!(small_rules.len(), 1);
        assert_eq!(small_rules[0].name.as_deref(), Some("small"));
    }

    #[test]
    fn scale_window_filters_rules() {
        let sld = r#"<StyledLayerDescriptor><NamedLayer><UserStyle><FeatureTypeStyle>
          <Rule><MinScaleDenominator>10000</MinScaleDenominator>
            <PointSymbolizer><Graphic><Mark><WellKnownName>circle</WellKnownName></Mark></Graphic></PointSymbolizer>
          </Rule>
          <Rule><MaxScaleDenominator>50000</MaxScaleDenominator>
            <PointSymbolizer><Graphic><Mark><WellKnownName>square</WellKnownName></Mark></Graphic></PointSymbolizer>
          </Rule>
        </FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let p = props(&[]);
        // Scale 5000 (more zoomed in than MinScaleDenominator=10000):
        // the min-scale rule does NOT apply (scale >= min required); only the
        // max-scale rule (scale <= 50000) matches.
        let r = doc.matching_rules(&p, 5000.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].max_scale, Some(50000.0));
        // Scale 200000 (more zoomed out than MaxScaleDenominator=50000):
        // only the min-scale rule (scale >= 10000) matches.
        let r = doc.matching_rules(&p, 200000.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].min_scale, Some(10000.0));
        // Between both: both match (stack).
        let r = doc.matching_rules(&p, 30000.0);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn else_filter_matches_only_when_no_rule_matched() {
        let sld = r#"<StyledLayerDescriptor><NamedLayer><UserStyle><FeatureTypeStyle>
          <Rule><ogc:Filter xmlns:ogc="http://www.opengis.net/ogc">
            <ogc:PropertyIsEqualTo><ogc:PropertyName>kind</ogc:PropertyName><ogc:Literal>city</ogc:Literal></ogc:PropertyIsEqualTo>
          </ogc:Filter>
          <PointSymbolizer><Graphic><Mark><WellKnownName>star</WellKnownName></Mark></Graphic></PointSymbolizer></Rule>
          <Rule><ElseFilter/>
          <PointSymbolizer><Graphic><Mark><WellKnownName>circle</WellKnownName></Mark></Graphic></PointSymbolizer></Rule>
        </FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let city = props(&[("kind", Json::from("city"))]);
        let town = props(&[("kind", Json::from("town"))]);
        let r = doc.matching_rules(&city, 1000.0);
        assert_eq!(r.len(), 1);
        match &r[0].symbolizers[0] {
            Symbolizer::Point(p) => assert_eq!(p.mark, MarkKind::Star),
            _ => panic!(),
        }
        let r = doc.matching_rules(&town, 1000.0);
        assert_eq!(r.len(), 1);
        match &r[0].symbolizers[0] {
            Symbolizer::Point(p) => assert_eq!(p.mark, MarkKind::Circle),
            _ => panic!(),
        }
    }

    #[test]
    fn bare_feature_type_style_without_named_layer() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle><Rule>
          <LineSymbolizer><Stroke>
            <ogc:CssParameter name="stroke">#112233</ogc:CssParameter>
            <ogc:CssParameter name="stroke-width">2.5</ogc:CssParameter>
            <ogc:CssParameter name="stroke-dasharray">6 3</ogc:CssParameter>
          </Stroke></LineSymbolizer>
        </Rule></FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        match &doc.feature_type_styles[0].rules[0].symbolizers[0] {
            Symbolizer::Line(l) => {
                assert_eq!(l.stroke, [17, 34, 51, 255]);
                assert_eq!(l.stroke_width, 2.5);
                assert_eq!(l.dash, vec![6.0, 3.0]);
            }
            other => panic!("expected Line, got {other:?}"),
        }
    }

    #[test]
    fn polygon_symbolizer_full_params() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle><Rule>
          <PolygonSymbolizer>
            <Fill><ogc:CssParameter name="fill">#2d6edc</ogc:CssParameter>
              <ogc:CssParameter name="fill-opacity">0.35</ogc:CssParameter></Fill>
            <Stroke><ogc:CssParameter name="stroke">#123a70</ogc:CssParameter>
              <ogc:CssParameter name="stroke-width">1.5</ogc:CssParameter>
              <ogc:CssParameter name="stroke-opacity">0.8</ogc:CssParameter></Stroke>
          </PolygonSymbolizer>
        </Rule></FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        match &doc.feature_type_styles[0].rules[0].symbolizers[0] {
            Symbolizer::Polygon(p) => {
                assert_eq!(p.fill, [45, 110, 220, 89]); // 0.35*255≈89
                assert_eq!(p.stroke, [18, 58, 112, 204]); // 0.8*255≈204
                assert_eq!(p.stroke_width, 1.5);
            }
            other => panic!("expected Polygon, got {other:?}"),
        }
    }

    #[test]
    fn raster_symbolizer_parsed_but_ignored() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle><Rule>
          <RasterSymbolizer><Opacity>0.5</Opacity></RasterSymbolizer>
        </Rule></FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        assert!(matches!(
            doc.feature_type_styles[0].rules[0].symbolizers[0],
            Symbolizer::Raster
        ));
    }

    // ---- Filters ----

    #[test]
    fn filter_comparisons_numeric_and_string() {
        let p = props(&[("pop", Json::from(8460088.0)), ("kind", Json::from("city"))]);
        let gt = Filter::Comparison(
            Comparison::Greater,
            Operand::Property("pop".into()),
            Operand::Literal(Value::Num(8000000.0)),
        );
        assert!(gt.matches(&p));
        let le = Filter::Comparison(
            Comparison::LessOrEq,
            Operand::Property("pop".into()),
            Operand::Literal(Value::Num(8000000.0)),
        );
        assert!(!le.matches(&p));
        let eq = Filter::Comparison(
            Comparison::Eq,
            Operand::Property("kind".into()),
            Operand::Literal(Value::Str("city".into())),
        );
        assert!(eq.matches(&p));
        // Missing property never matches.
        let neq = Filter::Comparison(
            Comparison::Eq,
            Operand::Property("nope".into()),
            Operand::Literal(Value::Str("x".into())),
        );
        assert!(!neq.matches(&p));
    }

    #[test]
    fn filter_between_like_isnull_logic() {
        let p = props(&[
            ("pop", Json::from(5000000.0)),
            ("name", Json::from("Kunming")),
            ("kind", Json::Null),
        ]);
        let between = Filter::Between(
            "pop".into(),
            Operand::Literal(Value::Num(1000000.0)),
            Operand::Literal(Value::Num(9000000.0)),
        );
        assert!(between.matches(&p));
        let like = Filter::Like {
            property: "name".into(),
            pattern: "Kun*".into(),
            wildcard: '*',
            single: '?',
            escape: '!',
        };
        assert!(like.matches(&p));
        let like2 = Filter::Like {
            property: "name".into(),
            pattern: "?unming".into(),
            wildcard: '*',
            single: '?',
            escape: '!',
        };
        assert!(like2.matches(&p));
        let like3 = Filter::Like {
            property: "name".into(),
            pattern: "Chengdu".into(),
            wildcard: '*',
            single: '?',
            escape: '!',
        };
        assert!(!like3.matches(&p));
        assert!(Filter::IsNull("kind".into()).matches(&p));
        assert!(!Filter::IsNull("name".into()).matches(&p));
        assert!(Filter::IsNull("missing".into()).matches(&p));
        let and = Filter::And(
            Box::new(between.clone()),
            Box::new(Filter::IsNull("name".into())),
        );
        assert!(!and.matches(&p));
        let not = Filter::Not(Box::new(Filter::IsNull("name".into())));
        assert!(not.matches(&p));
    }

    #[test]
    fn spatial_filter_never_matches() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle><Rule>
          <ogc:Filter xmlns:ogc="http://www.opengis.net/ogc">
            <ogc:BBOX><ogc:PropertyName>geom</ogc:PropertyName>
              <gml:Envelope xmlns:gml="http://www.opengis.net/gml">
                <gml:lowerCorner>0 0</gml:lowerCorner><gml:upperCorner>1 1</gml:upperCorner>
              </gml:Envelope></ogc:BBOX>
          </ogc:Filter>
          <PointSymbolizer><Graphic><Mark><WellKnownName>circle</WellKnownName></Mark></Graphic></PointSymbolizer>
        </Rule></FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        let r = doc.matching_rules(&props(&[]), 1000.0);
        assert_eq!(r.len(), 0, "spatial filter must never match");
    }

    // ---- Values ----

    #[test]
    fn color_and_dash_parsing() {
        assert_eq!(parse_color("#abc"), Some([170, 187, 204, 255]));
        assert_eq!(parse_color("#aabbccdd"), Some([170, 187, 204, 221]));
        assert_eq!(parse_color("rgb(1,2,3)"), Some([1, 2, 3, 255]));
        assert_eq!(parse_color("rgba(1,2,3,0.5)"), Some([1, 2, 3, 128]));
        assert_eq!(parse_color("darkblue"), Some([0, 0, 139, 255]));
        assert_eq!(parse_color("nonsense"), None);
        assert_eq!(parse_unit("60%"), Some(0.6));
        assert_eq!(parse_unit("0.6"), Some(0.6));
        assert_eq!(parse_unit("2"), Some(1.0)); // clamped
        assert_eq!(parse_f64("2.5px"), Some(2.5));
        assert_eq!(parse_dasharray("4 2"), vec![4.0, 2.0]);
        assert_eq!(parse_dasharray("4,2,1"), vec![4.0, 2.0, 1.0]);
        assert_eq!(parse_dasharray("solid"), Vec::<f64>::new());
    }

    #[test]
    fn like_wildcards() {
        assert!(like_match("Kunming", "Kun*", '*', '?', '!'));
        assert!(like_match("Kunming", "*ming", '*', '?', '!'));
        assert!(like_match("Kunming", "K?nming", '*', '?', '!'));
        assert!(!like_match("Kunming", "kun*", '*', '?', '!'));
        assert!(like_match("a*b", "a!*b", '*', '?', '!'));
        assert!(!like_match("axb", "a!*b", '*', '?', '!'));
        assert!(like_match("", "*", '*', '?', '!'));
        assert!(!like_match("", "?", '*', '?', '!'));
    }

    #[test]
    fn text_placement_defaults_and_overrides() {
        let sld = r#"<StyledLayerDescriptor><FeatureTypeStyle><Rule>
          <TextSymbolizer>
            <Label><ogc:Literal>Hi</ogc:Literal></Label>
            <LabelPlacement><PointPlacement>
              <AnchorPoint><AnchorPointX>0</AnchorPointX><AnchorPointY>0.5</AnchorPointY></AnchorPoint>
              <Displacement><DisplacementX>5</DisplacementX><DisplacementY>-3</DisplacementY></Displacement>
            </PointPlacement></LabelPlacement>
          </TextSymbolizer>
        </Rule></FeatureTypeStyle></StyledLayerDescriptor>"#;
        let doc = SldDocument::parse(sld).unwrap();
        match &doc.feature_type_styles[0].rules[0].symbolizers[0] {
            Symbolizer::Text(t) => {
                assert!(matches!(t.label, LabelExpr::Literal(ref s) if s == "Hi"));
                assert_eq!(t.anchor, (0.0, 0.5));
                assert_eq!(t.displacement, (5.0, -3.0));
                assert_eq!(t.font_size, 10.0); // default
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }
}
