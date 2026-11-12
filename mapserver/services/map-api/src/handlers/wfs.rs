//! WFS handler: `/wfs` (GET KVP only) — GetCapabilities, DescribeFeatureType
//! and GetFeature over the catalog's `vector` (GeoJSON) layers.

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::layer::vector_bounds;
use crate::state::AppState;
use wfs_protocol::caps::{self, CapFeatureType};
use wfs_protocol::dft;
use wfs_protocol::error::{WfsError, exception_report_xml, service_exception_xml};
use wfs_protocol::getfeature::{
    OutputFormat, feature_collection_csv, feature_collection_json, feature_collection_xml,
    resolve_features, resolve_features_owned,
};

/// GET /wfs?service=WFS&request=...
pub async fn handle(
    State(state): State<Arc<AppState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let query = query.unwrap_or_default();
    let kv = wms_protocol::kvp::parse_kvp(&query);
    let get = |k: &str| wms_protocol::kvp::lookup(&kv, k);

    // WFS 2.0 negotiates via AcceptVersions; 1.x clients send VERSION=.
    // Honor both (a VERSION= value that is not servable falls back to the
    // negotiated default, which is the lenient behavior clients expect).
    let version = caps::negotiate_version(get("version").or_else(|| get("acceptVersions")));
    let is20 = version.starts_with("2.");

    let request = match get("request") {
        Some(r) => r,
        None => {
            return error_response(&WfsError::missing("request"), is20);
        }
    };

    match request {
        "GetCapabilities" => {
            let base = crate::handlers::base_url(&state, &headers);
            let xml = state.cache.doc(format!("wfs-caps/{version}/{base}"), || {
                caps::capabilities_xml(&base, &version, &cap_feature_types(&state))
            });
            crate::handlers::xml_response((*xml).clone())
        }
        "DescribeFeatureType" => {
            let Some(type_name) = get("typeName").or_else(|| get("typeNames")) else {
                return error_response(&WfsError::missing("typeName"), is20);
            };
            let base = crate::handlers::base_url(&state, &headers);
            let bare = bare_type_name(type_name);
            let Some(layer) = state.layer(&bare) else {
                return error_response(&WfsError::unknown_type(&bare), is20);
            };
            // PostGIS layers have no in-memory FeatureCollection: the schema
            // is derived from the discovered property columns (values null —
            // types fall back to xsd:string, matching a schema-less source).
            if layer.vector_fc.is_none() && layer.is_postgis() {
                let pg_reg = state.pg_registry();
                let Some((_, pg_def)) = pg_reg.as_ref().and_then(|r| r.find(&bare)) else {
                    return error_response(
                        &WfsError::unknown_type(&bare),
                        is20,
                    );
                };
                let mut props = serde_json::Map::new();
                for c in &pg_def.properties {
                    props.insert(c.clone(), serde_json::Value::Null);
                }
                let fc = serde_json::json!({
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature",
                        "properties": props,
                        "geometry": serde_json::Value::Null,
                    }],
                });
                let xml = state.cache.doc(format!("wfs-dft/{base}/{bare}"), || {
                    dft::describe_feature_type_xml(&base, &bare, &fc)
                });
                return crate::handlers::xml_response((*xml).clone());
            }
            let Some(fc) = layer.vector_fc.as_ref() else {
                return error_response(
                    &WfsError::unknown_type(&bare),
                    is20,
                );
            };
            // Schema is derived from the (immutable) FeatureCollection:
            // render once per (base, typeName) per TTL window.
            let xml = state.cache.doc(format!("wfs-dft/{base}/{bare}"), || {
                dft::describe_feature_type_xml(&base, &bare, fc)
            });
            crate::handlers::xml_response((*xml).clone())
        }
        "GetFeature" => {
            let feature_layers: Vec<String> = state
                .layers
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|l| l.is_queryable())
                .map(|l| l.name().to_string())
                .collect();
            let params = match wfs_protocol::getfeature::parse_get_feature(&version, &kv, &feature_layers)
            {
                Ok(p) => p,
                Err(e) => return error_response(&e, is20),
            };
            // TYPENAMES may carry a namespace prefix (`ws:places`).
            let type_name = bare_type_name(&params.type_names[0]).to_string();
            let Some(layer) = state.layer(&type_name) else {
                return error_response(&WfsError::unknown_type(&type_name), is20);
            };
            // PostGIS path: features are QUERIED (bbox / id / OGC-Filter -> SQL).
            if layer.vector_fc.is_none() && layer.is_postgis() {
                return pg_get_feature(state, &params, &type_name, &kv, is20).await;
            }
            let Some(fc) = layer.vector_fc.as_ref() else {
                return error_response(&WfsError::unknown_type(&type_name), is20);
            };
            match params.output_format {
                OutputFormat::Gml => {
                    let (matched, sel) = resolve_features(fc, &params);
                    // 把请求的原始 CRS 一路带到输出：宣告的 srsName 与坐标书写序
                    // 必须由同一个判定得出，否则严格按声明解析的客户端会读成转置。
                    let xml = feature_collection_xml(
                        &version,
                        &type_name,
                        &sel,
                        matched,
                        params.start_index,
                        &params.srs_name,
                    );
                    (
                        StatusCode::OK,
                        [("content-type", "application/gml+xml; charset=utf-8")],
                        xml,
                    )
                        .into_response()
                }
                OutputFormat::Json => {
                    let (matched, owned) = resolve_features_owned(fc, &params);
                    let refs: Vec<&serde_json::Value> = owned.iter().collect();
                    let body = feature_collection_json(&refs, matched);
                    json_response(body)
                }
                OutputFormat::Csv => {
                    let (_, owned) = resolve_features_owned(fc, &params);
                    let refs: Vec<&serde_json::Value> = owned.iter().collect();
                    let body = feature_collection_csv(&refs, &params.property_names);
                    (
                        StatusCode::OK,
                        [("content-type", "text/csv; charset=utf-8")],
                        body,
                    )
                        .into_response()
                }
            }
        }
        other => error_response(&WfsError::unsupported(other), is20),
    }
}

/// `ws:places` -> `places` (r-mapserver layers are flat, no namespaces).
fn bare_type_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).trim().to_string()
}

/// The catalog's feature layers (file vector + PostGIS) as WFS feature types.
fn cap_feature_types(state: &AppState) -> Vec<CapFeatureType> {
    state
        .layers
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|l| l.is_queryable())
        .map(|l| {
            // PostGIS extents come from the startup ST_Extent probe (4326).
            let bounds = l
                .bounds()
                .or_else(|| l.vector_fc.as_ref().and_then(vector_bounds))
                .or_else(|| {
                    state
                        .pg_registry()
                        .as_ref()
                        .and_then(|r| r.find(l.name()))
                        .and_then(|(_, d)| d.bounds)
                        .map(|b| wms_common::Bbox::new(b[0], b[1], b[2], b[3]))
                });
            CapFeatureType {
                name: l.name().to_string(),
                title: if l.def.title.is_empty() {
                    l.name().to_string()
                } else {
                    l.def.title.clone()
                },
                bounds_4326: bounds,
                default_crs_urn: "urn:ogc:def:crs:EPSG::4326".to_string(),
            }
        })
        .collect()
}

/// WFS GetFeature over a PostGIS layer: ids / bbox / OGC-Filter are pushed
/// down as SQL; GML / GeoJSON / CSV output reuses the protocol renderers.
///
/// Unbounded queries are capped (`count` when given, else 10 000 rows) and
/// `numberMatched` always comes from a server-side `count(*)`.
async fn pg_get_feature(
    state: Arc<AppState>,
    params: &wfs_protocol::getfeature::GetFeatureParams,
    type_name: &str,
    kv: &[(String, String)],
    is20: bool,
) -> Response {
    use pg_source::sql::SqlParam;
    let Some((pool, def)) = state
        .pg_registry()
        .as_ref()
        .and_then(|r| r.find(type_name))
        .map(|(p, d)| (p, d.clone()))
    else {
        return error_response(&WfsError::new("NoApplicableCode", "postgis layer has no pool"), is20);
    };

    // OGC Filter pushdown (`FILTER=<xml>` KVP parameter). A malformed or
    // unsupported filter FAILS the request — never silently unfiltered.
    let where_sql = match wms_protocol::kvp::lookup(kv, "filter") {
        Some(xml) if !xml.trim().is_empty() => {
            match pg_source::filter::filter_to_where(xml, &def.geometry_column, def.srid) {
                Ok(Some(w)) => Some(w),
                Ok(None) => None,
                Err(e) => return error_response(&WfsError::invalid("filter", e), is20),
            }
        }
        _ => None,
    };
    let bbox = params
        .bbox
        .map(|b| [b.minx, b.miny, b.maxx, b.maxy]);

    // featureId selection: precise per-id SQL lookups.
    let selected: Vec<serde_json::Value> = if !params.feature_ids.is_empty() {
        let (sql, _) = pg_source::sql::feature_by_id_sql(&def);
        let mut found = Vec::new();
        for id in &params.feature_ids {
            let qp = vec![SqlParam::S(id.clone())];
            let refs = pg_source::sql::as_params(&qp);
            match pg_source::sql::query_features(&pool, &sql, &refs).await {
                Ok(rows) => found.extend(rows.into_iter().map(crate::handlers::ensure_feature_id)),
                Err(e) => {
                    return error_response(&WfsError::new("NoApplicableCode", format!("postgis query: {e}")), is20)
                }
            }
        }
        found
            .into_iter()
            .skip(params.start_index)
            .take(params.count.unwrap_or(usize::MAX))
            .collect()
    } else {
        // bbox / filter query with server-side paging.
        let limit = params.count.map(|c| c as i64).unwrap_or(10_000);
        // Clamp with the source `limit` (full-table protection).
        let limit = def.limit.map(|l| limit.min(l as i64)).unwrap_or(limit);
        let (sql, qp) =
            pg_source::sql::feature_sql(&def, bbox, where_sql.as_deref(), None, limit, params.start_index as i64);
        let rows = match pg_source::sql::query_features(&pool, &sql, &pg_source::sql::as_params(&qp))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return error_response(&WfsError::new("NoApplicableCode", format!("postgis query: {e}")), is20)
            }
        };
        if params.result_type == wfs_protocol::getfeature::ResultType::Hits {
            Vec::new()
        } else {
            rows.into_iter().map(crate::handlers::ensure_feature_id).collect()
        }
    };

    // numberMatched: server-side count over the same predicate.
    let (csql, cqp) = pg_source::sql::count_sql(&def, bbox, where_sql.as_deref());
    let matched = if !params.feature_ids.is_empty() {
        // per-id lookups already know the total
        params.feature_ids.len() as i64
    } else {
        match pg_source::sql::query_count(&pool, &csql, &pg_source::sql::as_params(&cqp)).await {
            Ok(c) => c,
            Err(_) => selected.len() as i64,
        }
    } as usize;

    // propertyName projection (owned values change shape).
    let selected: Vec<serde_json::Value> = if params.property_names.is_empty() {
        selected
    } else {
        selected
            .into_iter()
            .map(|f| vector_query::project_properties(&f, &params.property_names))
            .collect()
    };

    match params.output_format {
        wfs_protocol::getfeature::OutputFormat::Gml => {
            let sel: Vec<&serde_json::Value> = selected.iter().collect();
            let xml = wfs_protocol::getfeature::feature_collection_xml(
                &params.version,
                type_name,
                &sel,
                matched,
                params.start_index,
                &params.srs_name,
            );
            (
                StatusCode::OK,
                [("content-type", "application/gml+xml; charset=utf-8")],
                xml,
            )
                .into_response()
        }
        wfs_protocol::getfeature::OutputFormat::Json => {
            let refs: Vec<&serde_json::Value> = selected.iter().collect();
            let body = wfs_protocol::getfeature::feature_collection_json(&refs, matched);
            json_response(body)
        }
        wfs_protocol::getfeature::OutputFormat::Csv => {
            let refs: Vec<&serde_json::Value> = selected.iter().collect();
            let body =
                wfs_protocol::getfeature::feature_collection_csv(&refs, &params.property_names);
            (
                StatusCode::OK,
                [("content-type", "text/csv; charset=utf-8")],
                body,
            )
                .into_response()
        }
    }
}

fn json_response(body: String) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        body,
    )
        .into_response()
}

fn error_response(err: &WfsError, is20: bool) -> Response {
    let (status, body) = (err.status(), if is20 {
        exception_report_xml(err)
    } else {
        service_exception_xml(err)
    });
    (
        status,
        [("content-type", "text/xml; charset=utf-8")],
        body,
    )
        .into_response()
}
