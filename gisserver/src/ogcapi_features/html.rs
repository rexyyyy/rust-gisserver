//! Minimal HTML renderings of the OGC API - Features resources (the `html`
//! conformance class): landing, collections, items and item pages.
//!
//! Hand-rolled strings (no templating dependency): the pages are read-only
//! views over the same data the JSON responses carry.

use serde_json::Value;

use crate::ogcapi_features::{CollectionInfo, ItemsPage};

fn page(title: &str, base: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\n\
         <title>{title} - rust-gisserver OGC API - Features</title>\n\
         <style>\n\
         body{{font-family:sans-serif;margin:2rem;color:#222}}\n\
         table{{border-collapse:collapse;margin:1rem 0}}\n\
         th,td{{border:1px solid #999;padding:.3rem .6rem;text-align:left}}\n\
         th{{background:#e8e8e8}}\n\
         a{{color:#06c}}\n\
         </style></head><body>\n\
         <h1>{title}</h1>\n\
         <p><a href=\"{base}\">landing</a> | <a href=\"{base}/conformance\">conformance</a> | \
         <a href=\"{base}/collections\">collections</a> | \
         <a href=\"{base}/api\">api</a></p>\n{body}\n\
         <hr><p>rust-gisserver OGC API - Features 1.0.1</p>\n</body></html>"
    )
}

/// Landing page.
pub fn landing_html(base: &str, collection_count: usize) -> String {
    let body = format!("<p>{collection_count} collection(s) published.</p>");
    page("OGC API - Features", base, &body)
}

/// Collections list.
pub fn collections_html(base: &str, cols: &[CollectionInfo]) -> String {
    let mut rows = String::new();
    for c in cols {
        rows.push_str(&format!(
            "<tr><td><a href=\"{base}/collections/{id}\">{id}</a></td><td>{t}</td><td>{n}</td></tr>\n",
            id = c.id,
            t = c.title,
            n = c.count,
        ));
    }
    let body = format!(
        "<table><tr><th>collection</th><th>title</th><th>features</th></tr>\n{rows}</table>"
    );
    page("Collections", base, &body)
}

/// One collection + its items page (first page inline for convenience).
pub fn collection_html(base: &str, c: &CollectionInfo, items_page: &ItemsPage<'_>) -> String {
    let items_url = format!("{base}/collections/{}/items", c.id);
    let extent = c
        .extent
        .map(|b| format!("[{:.4}, {:.4}, {:.4}, {:.4}]", b.minx, b.miny, b.maxx, b.maxy))
        .unwrap_or_else(|| "n/a".into());
    let body = format!(
        "<p>{desc}</p>\n\
         <p>extent: {extent} · features matched: {n}</p>\n\
         <p><a href=\"{items_url}\">items (JSON)</a> · \
         <a href=\"{items_url}?limit={limit}\">first page</a></p>\n\
         {table}",
        desc = c.description,
        extent = extent,
        n = items_page.matched,
        items_url = items_url,
        limit = items_page.query.limit,
        table = items_table(&items_page.features),
    );
    page(&c.title, base, &body)
}

/// Items page with next/prev links.
pub fn items_html(base: &str, collection_id: &str, items_page: &ItemsPage<'_>) -> String {
    let self_url = format!("{base}/collections/{collection_id}/items");
    let mut nav = String::new();
    if items_page.query.offset > 0 {
        let prev = items_page.query.offset.saturating_sub(items_page.query.limit);
        nav.push_str(&format!(
            "<a href=\"{self_url}?limit={}&offset={}\">prev</a> ",
            items_page.query.limit, prev
        ));
    }
    nav.push_str(&format!(
        "page offset {} · matched {}",
        items_page.query.offset, items_page.matched
    ));
    if let Some(next) = items_page.next_offset {
        nav.push_str(&format!(
            " · <a href=\"{self_url}?limit={}&offset={}\">next</a>",
            items_page.query.limit, next
        ));
    }
    let body = format!(
        "<p>{nav}</p>\n{table}",
        nav = nav,
        table = items_table(&items_page.features)
    );
    page(&format!("items of {collection_id}"), base, &body)
}

/// One feature.
pub fn item_html(base: &str, collection_id: &str, feature: &Value, feature_id: &str) -> String {
    let props = feature
        .get("properties")
        .cloned()
        .unwrap_or(Value::Null);
    let body = format!(
        "<p><a href=\"{base}/collections/{collection_id}/items\">back to items</a></p>\n\
         <pre>{props}</pre>",
        props = serde_json::to_string_pretty(&props).unwrap_or_default(),
    );
    page(&format!("feature {feature_id}"), base, &body)
}

fn items_table(features: &[&Value]) -> String {
    let mut rows = String::new();
    for f in features {
        let id = crate::ogcapi_features::vector_id(f);
        let geom = f
            .get("geometry")
            .and_then(|g| g.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("null");
        let props = serde_json::to_string(f.get("properties").unwrap_or(&Value::Null))
            .unwrap_or_default();
        rows.push_str(&format!(
            "<tr><td>{id}</td><td>{geom}</td><td><code>{props}</code></td></tr>\n"
        ));
    }
    format!("<table><tr><th>featureId</th><th>geometry</th><th>properties</th></tr>\n{rows}</table>")
}
