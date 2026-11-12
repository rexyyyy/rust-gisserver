//! OGC API - Features handler: `/ogc/features/v1` (landing, conformance, api,
//! collections, items) — GeoJSON responses with an HTML rendering for
//! `Accept: text/html` clients.

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;
use gisserver::ogcapi_features::{CollectionInfo, html, items as items_doc, page_features, query};

/// The mount point (kept in one place so links and routes cannot drift).
pub const BASE_PATH: &str = "/ogc/features/v1";

fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false)
}

fn json_response(body: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

fn html_response(body: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

fn error_response(e: &gisserver::ogcapi_features::error::OgcError) -> Response {
    (
        e.status,
        [(header::CONTENT_TYPE, "application/json")],
        gisserver::ogcapi_features::error_json(e),
    )
        .into_response()
}

/// The catalog's vector layers as OGC API collections.
///
/// Backed by the cached catalog metadata: the previous implementation
/// deep-cloned every vector FeatureCollection (and re-derived extents /
/// counts / geometry kinds) on **every** request — landing, collections,
/// collection, items and item all called it.
pub fn collections_of(state: &AppState) -> Arc<Vec<CollectionInfo>> {
    Arc::clone(&state.catalog_meta().oaf_collections)
}

/// GET /ogc/features/v1 — landing page.
pub async fn landing(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = crate::handlers::base_url(&state, &headers);
    let base = format!("{base}{BASE_PATH}");
    if wants_html(&headers) {
        return html_response(html::landing_html(&base, collections_of(&state).len()));
    }
    let doc = state.cache.doc(format!("oaf-landing/{base}"), || gisserver::ogcapi_features::landing(&base));
    json_response((*doc).clone())
}

/// GET /ogc/features/v1/conformance
pub async fn conformance(State(state): State<Arc<AppState>>) -> Response {
    // Constant document (no base URL in it): cached under a fixed key.
    let doc = state.cache.doc("oaf-conformance".to_string(), gisserver::ogcapi_features::conformance);
    json_response((*doc).clone())
}

/// GET /ogc/features/v1/api — minimal OpenAPI document.
pub async fn api(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let doc = state.cache.doc(format!("oaf-api/{base}"), || gisserver::ogcapi_features::api_doc(&base));
    json_response((*doc).clone())
}

/// GET /ogc/features/v1/collections
pub async fn collections(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let cols = collections_of(&state);
    if wants_html(&headers) {
        return html_response(html::collections_html(&base, &cols));
    }
    let doc = state.cache.doc(format!("oaf-collections/{base}"), || {
        gisserver::ogcapi_features::collections(&base, &cols)
    });
    json_response((*doc).clone())
}

/// GET /ogc/features/v1/collections/{collectionId}
pub async fn collection(
    State(state): State<Arc<AppState>>,
    Path(collection_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let Some(c) = collections_of(&state)
        .iter()
        .find(|c| c.id == collection_id)
        .cloned()
    else {
        return error_response(&gisserver::ogcapi_features::error::OgcError::not_found(format!(
            "collection '{collection_id}' is not published"
        )));
    };
    if wants_html(&headers) {
        // The HTML view renders the collection plus its first items page.
        let fc = collection_fc(&state, &c.id);
        let q = query::ItemsQuery::default();
        let page = fc.as_ref().map(|fc| page_features(fc, &q));
        let body = match page {
            Some(p) => html::collection_html(&base, &c, &p),
            None => html::collection_html(&base, &c, &empty_page(&q)),
        };
        return html_response(body);
    }
    let doc = state.cache.doc(format!("oaf-collection/{base}/{}", c.id), || {
        gisserver::ogcapi_features::collection(&base, &c)
    });
    json_response((*doc).clone())
}

/// GET /ogc/features/v1/collections/{collectionId}/items[?limit&offset&bbox]
pub async fn items(
    State(state): State<Arc<AppState>>,
    Path(collection_id): Path<String>,
    RawQuery(qs): RawQuery,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    if !collections_of(&state).iter().any(|c| c.id == collection_id) {
        return error_response(&gisserver::ogcapi_features::error::OgcError::not_found(format!(
            "collection '{collection_id}' is not published"
        )));
    }
    let kv = qs
        .as_deref()
        .map(gisserver::wms_protocol::kvp::parse_kvp)
        .unwrap_or_default();
    let q = match query::parse_items_query(&kv) {
        Ok(q) => q,
        Err(e) => return error_response(&e),
    };
    // PostGIS path: bbox / filter / limit / offset are pushed down as SQL; no
    // doc caching (the database is the cache — moka-era page caching would
    // otherwise bypass the live query).
    if state
        .layer(&collection_id)
        .map(|l| l.is_postgis())
        .unwrap_or(false)
    {
        // OGC Filter pushdown (`filter=<xml>` KVP parameter). A malformed or
        // unsupported filter FAILS the request — never silently unfiltered.
        let filter_where = match gisserver::wms_protocol::kvp::lookup(&kv, "filter") {
            Some(xml) if !xml.trim().is_empty() => {
                let geom_col = state
                    .pg_registry()
                    .as_ref()
                    .and_then(|r| r.find(&collection_id))
                    .map(|(_, d)| d.geometry_column.clone())
                    .unwrap_or_default();
                let srid = state
                    .pg_registry()
                    .as_ref()
                    .and_then(|r| r.find(&collection_id))
                    .map(|(_, d)| d.srid)
                    .unwrap_or(4326);
                match gisserver::pg_source::filter::filter_to_where(xml, &geom_col, srid) {
                    Ok(Some(w)) => Some(w),
                    Ok(None) => None,
                    Err(e) => {
                        return error_response(&gisserver::ogcapi_features::error::OgcError::bad_request(
                            format!("invalid filter: {e}"),
                        ))
                    }
                }
            }
            _ => None,
        };
        // OGC API - Features `sortby` (P3): `prop`, `-prop`, or a comma
        // list — validated against the published properties, then pushed
        // down as ORDER BY. A bad column FAILS the request (400), never
        // silently unsorted.
        let sort_order = match gisserver::wms_protocol::kvp::lookup(&kv, "sortby") {
            Some(s) if !s.trim().is_empty() => {
                let def = state
                    .pg_registry()
                    .as_ref()
                    .and_then(|r| r.find(&collection_id))
                    .map(|(_, d)| d.clone());
                let mut allowed: Vec<String> =
                    def.as_ref().map(|d| d.properties.clone()).unwrap_or_default();
                if let Some(id) = def.as_ref().and_then(|d| d.id_column.clone()) {
                    allowed.push(id);
                }
                match gisserver::pg_source::sql::parse_sortby(s, &allowed) {
                    Ok(o) => o,
                    Err(e) => {
                        return error_response(&gisserver::ogcapi_features::error::OgcError::bad_request(
                            format!("invalid sortby: {e}"),
                        ))
                    }
                }
            }
            _ => None,
        };
        return pg_items(
            state,
            &base,
            &collection_id,
            &q,
            filter_where.as_deref(),
            sort_order.as_deref(),
            wants_html(&headers),
        )
        .await;
    }
    if wants_html(&headers) {
        let Some(fc) = collection_fc(&state, &collection_id) else {
            return error_response(&gisserver::ogcapi_features::error::OgcError::internal(format!(
                "collection '{collection_id}' lost its data"
            )));
        };
        let page = page_features(&fc, &q);
        return html_response(html::items_html(&base, &collection_id, &page));
    }
    // Cached rendered page: the key covers every input (base, collection,
    // limit, offset, bbox). A hit skips the FeatureCollection clone + walk
    // that the uncached path paid on every request.
    let bbox_key = q
        .bbox
        .map(|b| format!("{},{},{},{}", b.minx, b.miny, b.maxx, b.maxy))
        .unwrap_or_else(|| "none".to_string());
    let doc = state.cache.doc(
        format!(
            "oaf-items/{base}/{collection_id}/{}/{}/{bbox_key}",
            q.limit, q.offset
        ),
        || match collection_fc(&state, &collection_id) {
            Some(fc) => items_doc(&base, &collection_id, &page_features(&fc, &q)),
            None => gisserver::ogcapi_features::error_json(&gisserver::ogcapi_features::error::OgcError::internal(
                format!("collection '{collection_id}' lost its data"),
            )),
        },
    );
    json_response((*doc).clone())
}

/// GET /ogc/features/v1/collections/{collectionId}/items/{featureId}
pub async fn item(
    State(state): State<Arc<AppState>>,
    Path((collection_id, feature_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let base = format!("{}{}", crate::handlers::base_url(&state, &headers), BASE_PATH);
    let exists = collections_of(&state).iter().any(|c| c.id == collection_id);
    if !exists {
        return error_response(&gisserver::ogcapi_features::error::OgcError::not_found(format!(
            "collection '{collection_id}' is not published"
        )));
    }
    // PostGIS path: per-id SQL lookup (the key-value feature cache only
    // covers in-memory collections).
    if state
        .layer(&collection_id)
        .map(|l| l.is_postgis())
        .unwrap_or(false)
    {
        return pg_item(state, &base, &collection_id, &feature_id, wants_html(&headers)).await;
    }
    // Key-value feature cache: the old code cloned the whole
    // FeatureCollection and linearly scanned it on every single-feature
    // fetch. Misses that stay missing are not negative-cached.
    let Some(feature) = state.cache.feature(&collection_id, &feature_id, || {
        collection_fc(&state, &collection_id)
            .and_then(|fc| gisserver::vector_query::find_by_id(&fc, &feature_id).cloned())
    }) else {
        return error_response(&gisserver::ogcapi_features::error::OgcError::not_found(format!(
            "feature '{feature_id}' not found in collection '{collection_id}'"
        )));
    };
    if wants_html(&headers) {
        return html_response(html::item_html(&base, &collection_id, &feature, &feature_id));
    }
    json_response(gisserver::ogcapi_features::item(&base, &collection_id, &feature, &feature_id))
}

/// The FeatureCollection behind a collection id (owned clone; the documents
/// borrow from it, handlers run per-request so the clone is the simplest
/// correct lifetime story).
fn collection_fc(state: &AppState, id: &str) -> Option<serde_json::Value> {
    state
        .layer(id)
        .filter(|l| l.is_vector())
        .and_then(|l| l.vector_fc.clone())
}

/// OGC API - Features items over a PostGIS layer: bbox / filter / limit /
/// offset as SQL predicates, `numberMatched` from a server-side `count(*)`,
/// `sortby` as an ORDER BY fragment (validated upstream).
async fn pg_items(
    state: Arc<AppState>,
    base: &str,
    collection_id: &str,
    q: &gisserver::ogcapi_features::query::ItemsQuery,
    filter_where: Option<&str>,
    order_by: Option<&str>,
    html: bool,
) -> Response {
    let Some((pool, def)) = state
        .pg_registry()
        .as_ref()
        .and_then(|r| r.find(collection_id))
        .map(|(p, d)| (p, d.clone()))
    else {
        return error_response(&gisserver::ogcapi_features::error::OgcError::internal(
            "postgis layer has no pool",
        ));
    };
    let bbox = q.bbox.map(|b| [b.minx, b.miny, b.maxx, b.maxy]);
    // Clamp the page limit with the source `limit` (full-table protection).
    let limit = q.limit as i64;
    let limit = def.limit.map(|l| limit.min(l as i64)).unwrap_or(limit);
    let (sql, qp) = gisserver::pg_source::sql::feature_sql(
        &def,
        bbox,
        filter_where,
        order_by,
        limit,
        q.offset as i64,
    );
    let rows = match gisserver::pg_source::sql::query_features(&pool, &sql, &gisserver::pg_source::sql::as_params(&qp))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return error_response(&gisserver::ogcapi_features::error::OgcError::internal(format!(
                "postgis query: {e}"
            )))
        }
    };
    let feats: Vec<serde_json::Value> =
        rows.into_iter().map(crate::handlers::ensure_feature_id).collect();
    let (csql, cqp) = gisserver::pg_source::sql::count_sql(&def, bbox, filter_where);
    let matched = gisserver::pg_source::sql::query_count(&pool, &csql, &gisserver::pg_source::sql::as_params(&cqp))
        .await
        .unwrap_or(feats.len() as i64) as usize;
    let returned = feats.len();
    let next_offset = (q.offset + returned < matched).then_some(q.offset + returned);
    let page = gisserver::ogcapi_features::ItemsPage {
        matched,
        returned,
        features: feats.iter().collect(),
        next_offset,
        query: q.clone(),
    };
    if html {
        html_response(gisserver::ogcapi_features::html::items_html(base, collection_id, &page))
    } else {
        json_response(gisserver::ogcapi_features::items(base, collection_id, &page))
    }
}

/// OGC API - Features single item over a PostGIS layer (per-id SQL lookup).
async fn pg_item(
    state: Arc<AppState>,
    base: &str,
    collection_id: &str,
    feature_id: &str,
    html: bool,
) -> Response {
    use gisserver::pg_source::sql::SqlParam;
    let Some((pool, def)) = state
        .pg_registry()
        .as_ref()
        .and_then(|r| r.find(collection_id))
        .map(|(p, d)| (p, d.clone()))
    else {
        return error_response(&gisserver::ogcapi_features::error::OgcError::internal(
            "postgis layer has no pool",
        ));
    };
    let (sql, _) = gisserver::pg_source::sql::feature_by_id_sql(&def);
    let qp = vec![SqlParam::S(feature_id.to_string())];
    let rows = match gisserver::pg_source::sql::query_features(&pool, &sql, &gisserver::pg_source::sql::as_params(&qp))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return error_response(&gisserver::ogcapi_features::error::OgcError::internal(format!(
                "postgis query: {e}"
            )))
        }
    };
    let Some(feature) = rows.into_iter().next().map(crate::handlers::ensure_feature_id) else {
        return error_response(&gisserver::ogcapi_features::error::OgcError::not_found(format!(
            "feature '{feature_id}' not found in collection '{collection_id}'"
        )));
    };
    if html {
        html_response(gisserver::ogcapi_features::html::item_html(
            base,
            collection_id,
            &feature,
            feature_id,
        ))
    } else {
        json_response(gisserver::ogcapi_features::item(base, collection_id, &feature, feature_id))
    }
}

/// An empty page for degenerate collections (keeps HTML views renderable).
/// The empty feature vec needs no borrow, so `'static` is fine here.
fn empty_page(q: &query::ItemsQuery) -> gisserver::ogcapi_features::ItemsPage<'static> {
    gisserver::ogcapi_features::ItemsPage {
        matched: 0,
        returned: 0,
        features: Vec::new(),
        next_offset: None,
        query: q.clone(),
    }
}
