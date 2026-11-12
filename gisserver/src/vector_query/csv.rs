//! CSV encoding for `GetFeature` / items (`text/csv`).

use serde_json::Value;

/// Encode selected features as CSV (RFC 4180 quoting).
/// `columns` fixes the attribute order; an empty list uses the first feature's
/// keys.
pub fn csv(features: &[&Value], columns: &[String]) -> String {
    let cols: Vec<String> = if columns.is_empty() {
        features
            .first()
            .and_then(|f| f.get("properties"))
            .and_then(|p| p.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    } else {
        columns.to_vec()
    };
    let mut out = String::new();
    if !cols.is_empty() {
        out.push_str(&cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    for f in features {
        let props = f.get("properties").and_then(|p| p.as_object());
        let row: Vec<String> = cols
            .iter()
            .map(|c| {
                props
                    .and_then(|p| p.get(c))
                    .map(cell)
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// CSV cell: numbers/booleans bare, strings quoted when needed, JSON for the
/// rest.
fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote(s),
        other => quote(&other.to_string()),
    }
}

/// Quote a field when it contains a delimiter/quote/newline (RFC 4180).
fn quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn header_and_rows() {
        let fc = json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"name":"a, b","pop":10},"geometry":{"type":"Point","coordinates":[0,0]}},
            {"type":"Feature","properties":{"name":"plain","pop":20.5},"geometry":{"type":"Point","coordinates":[1,1]}}
        ]});
        let fs: Vec<&Value> = fc["features"].as_array().unwrap().iter().collect();
        let out = csv(&fs, &["name".into(), "pop".into()]);
        assert_eq!(out, "name,pop\n\"a, b\",10\nplain,20.5\n");
    }

    #[test]
    fn explicit_columns_override() {
        let fc = json!({"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{"a":1,"b":2},"geometry":null}
        ]});
        let fs: Vec<&Value> = fc["features"].as_array().unwrap().iter().collect();
        let out = csv(&fs, &["b".into()]);
        assert_eq!(out, "b\n2\n");
    }
}
