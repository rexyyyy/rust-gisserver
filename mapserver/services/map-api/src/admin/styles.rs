//! Style library: SLD files under `<catalog.root>/styles` managed through the
//! admin surface (manager scope doc §10 P1, lightweight by decision — no
//! online editor, QGIS is the editing tool).
//!
//! * `GET    /admin/styles`              — list (parsed summary + usage)
//! * `GET    /admin/styles/:name/content`— raw SLD text
//! * `GET    /admin/styles/:name/legend.png` — standalone legend preview
//! * `POST   /admin/styles/:name`        — upload (raw XML body), validated
//! * `DELETE /admin/styles/:name`        — remove
//!
//! Uploads are validated with the same SLD parser the renderer uses, so a
//! style that lands here is guaranteed to parse at serve time. Layer configs
//! reference library styles by path (`sld: styles/xxx.sld` relative to the
//! server working directory); the listing reports which configured layers use
//! each file so the UI can warn before deletion.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use renderer::sld::SldDocument;

use crate::state::AppState;

use super::http::{authorize, json};

/// Styles directory (`<catalog.root>/styles`).
fn styles_dir(state: &AppState) -> PathBuf {
    FsPath::new(&state.config.catalog.root).join("styles")
}

/// Sanitized file name guard: `*.sld`, no directories, no traversal.
fn safe_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 100 {
        return None;
    }
    if !n.ends_with(".sld") && !n.ends_with(".SLD") {
        return None;
    }
    if !n
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return None;
    }
    if n.contains("..") {
        return None;
    }
    Some(n.to_ascii_lowercase())
}

fn rule_count(doc: &SldDocument) -> usize {
    doc.feature_type_styles.iter().map(|f| f.rules.len()).sum()
}

/// Which configured layers reference this SLD file (by file name).
fn used_by(state: &AppState, file: &str) -> Vec<String> {
    let want = file.to_ascii_lowercase();
    state
        .config
        .layers
        .iter()
        .filter(|l| {
            l.sld
                .as_deref()
                .map(|p| {
                    FsPath::new(p)
                        .file_name()
                        .map(|f| f.to_string_lossy().to_ascii_lowercase() == want)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .map(|l| l.name.clone())
        .collect()
}

/// `GET /admin/styles` — library listing with parsed summaries.
pub async fn list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let dir = styles_dir(&state);
    let mut styles: Vec<serde_json::Value> = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|e| e.to_string_lossy().to_ascii_lowercase() == "sld")
                        .unwrap_or(false)
                })
                .collect();
            files.sort();
            for f in files {
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let (size, modified) = match std::fs::metadata(&f) {
                    Ok(m) => (
                        m.len(),
                        m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    ),
                    Err(_) => (0, 0),
                };
                let (style_name, layer_name, rules, parse_ok) =
                    match std::fs::read_to_string(&f).ok().as_deref().map(SldDocument::parse) {
                        Some(Some(doc)) => (
                            doc.style_name.clone(),
                            doc.layer_name.clone(),
                            rule_count(&doc),
                            true,
                        ),
                        _ => (None, None, 0, false),
                    };
                styles.push(serde_json::json!({
                    "name": name,
                    "size": size,
                    "modified": modified,
                    "style_name": style_name,
                    "layer_name": layer_name,
                    "rules": rules,
                    "parse_ok": parse_ok,
                    "used_by": used_by(&state, &name),
                }));
            }
        }
        Err(_) => {} // directory absent: empty library
    }
    json(
        StatusCode::OK,
        serde_json::json!({
            "dir": dir.display().to_string(),
            "count": styles.len(),
            "styles": styles,
        }),
    )
}

/// `GET /admin/styles/:name/content` — raw SLD text (for viewer / download).
pub async fn content(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let Some(name) = safe_name(&name) else {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid style file name (use <name>.sld, ascii only)"}),
        );
    };
    let path = styles_dir(&state).join(&name);
    match std::fs::read_to_string(&path) {
        Ok(text) => json(StatusCode::OK, serde_json::json!({"name": name, "content": text})),
        Err(e) => json(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("read {name}: {e}")}),
        ),
    }
}

/// `POST /admin/styles/:name` — upload / replace one SLD (raw XML body).
/// The document is parsed BEFORE it lands on disk; an unparsable upload is a
/// 400 and nothing is written.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let Some(name) = safe_name(&name) else {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid style file name (use <name>.sld, ascii only)"}),
        );
    };
    let text = match std::str::from_utf8(&body) {
        Ok(t) => t,
        Err(_) => {
            return json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "body is not valid UTF-8"}),
            )
        }
    };
    let Some(doc) = SldDocument::parse(text) else {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "not a parsable SLD document (root StyledLayerDescriptor expected)"}),
        );
    };
    if doc.feature_type_styles.is_empty() {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "SLD has no FeatureTypeStyle; it would never render"}),
        );
    }
    let dir = styles_dir(&state);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("create {}: {e}", dir.display())}),
        );
    }
    let path = dir.join(&name);
    if let Err(e) = std::fs::write(&path, text) {
        return json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("write {name}: {e}")}),
        );
    }
    json(
        StatusCode::OK,
        serde_json::json!({
            "ok": true,
            "name": name,
            "rules": rule_count(&doc),
            "bytes": text.len(),
            "used_by": used_by(&state, &name),
        }),
    )
}

/// `DELETE /admin/styles/:name` — remove one SLD from the library.
pub async fn remove(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let Some(name) = safe_name(&name) else {
        return json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid style file name"}),
        );
    };
    let path = styles_dir(&state).join(&name);
    match std::fs::remove_file(&path) {
        Ok(()) => json(StatusCode::OK, serde_json::json!({"ok": true, "name": name})),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("style '{name}' not found")}),
        ),
        Err(e) => json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("delete {name}: {e}")}),
        ),
    }
}

/// `GET /admin/styles/:name/legend.png?w=&h=` — standalone legend preview
/// rendered from the SLD only (no data source involved). Reuses the WMS
/// GetLegendGraphic renderer with a synthetic layer.
pub async fn legend(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    RawQuery(qs): RawQuery,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let Some(name) = safe_name(&name) else {
        return json(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid style file name"}));
    };
    let path = styles_dir(&state).join(&name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return json(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": format!("style '{name}' not found")}),
        );
    };
    let Some(doc) = SldDocument::parse(&text) else {
        return json(
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!({"error": "style no longer parses"}),
        );
    };
    let kv = qs
        .as_deref()
        .map(wms_protocol::kvp::parse_kvp)
        .unwrap_or_default();
    let get = |k: &str| -> Option<u32> {
        kv.iter()
            .find(|(key, _)| key == k)
            .and_then(|(_, v)| v.parse().ok())
    };
    let w = get("WIDTH").unwrap_or(24).clamp(8, 512);
    let h = get("HEIGHT").unwrap_or(if doc.feature_type_styles.len() > 1 { 96 } else { 24 }).clamp(8, 512);

    let rt = crate::layer::LayerRuntime {
        def: wms_common::layer::Layer {
            name: "style-preview".into(),
            title: String::new(),
            source: wms_common::layer::SourceKind::Vector { path: String::new() },
            src_crs: "EPSG:4326".into(),
            style: None,
            sld: None,
            grids: vec![],
            tile_px: 256,
            extent: None,
        },
        style: None,
        sld: Some(doc),
        cog: None,
        source: None,
        source_uri: String::new(),
        vector_fc: None,
        pmtiles: None,
        mbtiles: None,
        arcgis: None,
    };
    match crate::legend::legend_png(&rt, w, h, None) {
        Ok(png) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "no-store")],
            png,
        )
            .into_response(),
        Err(e) => json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("render legend: {e}")}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK_SLD: &str = r#"<StyledLayerDescriptor><NamedLayer><Name>roads</Name>
      <UserStyle><Name>main</Name><FeatureTypeStyle>
        <Rule><Name>r1</Name><LineSymbolizer><Stroke>
          <SvgParameter name="stroke">#ff0000</SvgParameter></Stroke></LineSymbolizer></Rule>
        <Rule><Name>r2</Name><LineSymbolizer><Stroke>
          <SvgParameter name="stroke">#00ff00</SvgParameter></Stroke></LineSymbolizer></Rule>
      </FeatureTypeStyle></UserStyle></NamedLayer></StyledLayerDescriptor>"#;

    #[test]
    fn safe_name_rules() {
        assert_eq!(safe_name("roads.sld").as_deref(), Some("roads.sld"));
        assert_eq!(safe_name("Roads-2.SLD").as_deref(), Some("roads-2.sld"));
        assert!(safe_name("../evil.sld").is_none());
        assert!(safe_name("no-ext").is_none());
        assert!(safe_name("a/b.sld").is_none());
        assert!(safe_name("中文名.sld").is_none());
    }

    #[test]
    fn rule_count_counts_all_fts() {
        let doc = SldDocument::parse(OK_SLD).unwrap();
        assert_eq!(rule_count(&doc), 2);
        assert_eq!(doc.style_name.as_deref(), Some("main"));
        assert_eq!(doc.layer_name.as_deref(), Some("roads"));
    }
}
