//! Request usage metering: per-service-class + service-name counters, a
//! periodic PG flush and the `/api/usage` aggregation query.
//!
//! Design (manager design doc §9.4):
//! * The middleware counts every **service** request (tiles, caps, features)
//!   by `(kind, name)`; admin/api/health/viewer paths are skipped.
//! * Counters are cheap (single mutex, touched once per request) and keep an
//!   hourly ring in memory (400h ≈ 16 days) so `/api/usage` works even in
//!   files mode (clearly labeled "since restart").
//! * With a PG registry, deltas are flushed every `admin.usage_flush_secs`
//!   (default 30s) into `sys_service_usage_hour` — batched, one upsert per
//!   (hour, kind, name), never per request.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Extension, Request, State};
use axum::http::Uri;
use tower_http::request_id::RequestId;
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

/// How many hours of history the in-memory ring retains.
const KEEP_HOURS: i64 = 400;

#[derive(Default)]
struct UsageInner {
    /// (kind, name) -> (epoch-hour -> hits)
    hist: HashMap<(String, String), HashMap<i64, u64>>,
    /// All-time totals since process start.
    total: HashMap<(String, String), u64>,
    /// Snapshot of what has already been flushed to PG (delta base).
    flushed: HashMap<(String, String, i64), u64>,
}

pub struct Usage {
    inner: Mutex<UsageInner>,
}

impl Default for Usage {
    fn default() -> Self {
        Usage {
            inner: Mutex::new(UsageInner::default()),
        }
    }
}

fn now_hour() -> i64 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) / 3600) as i64
}

impl Usage {
    fn record(&self, kind: &str, name: &str) {
        let h = now_hour();
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *g.hist
            .entry((kind.to_string(), name.to_string()))
            .or_default()
            .entry(h)
            .or_insert(0) += 1;
        *g.total
            .entry((kind.to_string(), name.to_string()))
            .or_insert(0) += 1;
    }

    /// One (kind, name) aggregated over the last `hours` hours (memory ring).
    fn window_rows(&self, hours: i64, by_kind: bool) -> Vec<(String, String, i64)> {
        let from = now_hour() - hours + 1;
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut acc: HashMap<(String, String), u64> = HashMap::new();
        for ((kind, name), hours_map) in &g.hist {
            let key = if by_kind {
                (kind.clone(), String::new())
            } else {
                (kind.clone(), name.clone())
            };
            let e = acc.entry(key).or_insert(0);
            for (h, hits) in hours_map {
                if *h >= from {
                    *e += hits;
                }
            }
        }
        let mut rows: Vec<(String, String, i64)> = acc
            .into_iter()
            .map(|((k, n), v)| (k, n, v as i64))
            .collect();
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        rows
    }

    /// Compute the un-flushed delta (per hour bucket) and mark it flushed.
    /// Also prunes the ring.
    fn take_delta(&self) -> Vec<(i64, String, String, i64)> {
        let now_h = now_hour();
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // Prune first (single borrow, no closures touching `flushed`).
        for hours in g.hist.values_mut() {
            hours.retain(|h, _| now_h - *h <= KEEP_HOURS);
        }
        g.hist.retain(|_, hours| !hours.is_empty());
        // Snapshot hist (owned) so `flushed` can be mutated freely below.
        let snap: Vec<((String, String), Vec<(i64, u64)>)> = g
            .hist
            .iter()
            .map(|((k, n), hours)| {
                (
                    (k.clone(), n.clone()),
                    hours.iter().map(|(h, c)| (*h, *c)).collect(),
                )
            })
            .collect();
        let mut out = Vec::new();
        for ((kind, name), hours) in snap {
            for (h, hits) in hours {
                let base = g
                    .flushed
                    .get(&(kind.clone(), name.clone(), h))
                    .copied()
                    .unwrap_or(0);
                let d = hits as i64 - base as i64;
                if d > 0 {
                    out.push((h, kind.clone(), name.clone(), d));
                }
                g.flushed.insert((kind.clone(), name.clone(), h), hits);
            }
        }
        out
    }

    /// All-time totals (for the /admin/services listing).
    pub fn totals(&self) -> Vec<(String, String, u64)> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut v: Vec<(String, String, u64)> =
            g.total.iter().map(|((k, n), c)| (k.clone(), n.clone(), *c)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v
    }
}

/// Which service did this request hit? Best-effort `(kind, name)` from the
/// path (and, for KVP endpoints, the query string). `None` = don't count.
pub fn classify(uri: &Uri) -> Option<(String, String)> {
    let segs: Vec<&str> = uri.path().split('/').filter(|s| !s.is_empty()).collect();
    let s0 = *segs.first()?;
    // Never counted: ops/admin surfaces.
    if matches!(
        s0,
        "admin" | "api" | "health" | "ready" | "viewer" | "cesium" | "favicon.ico" | "tileMatrixSets"
    ) {
        return None;
    }
    let query = uri.query().unwrap_or("");
    let kvp = (!query.is_empty()).then(|| gisserver::wms_protocol::kvp::parse_kvp(query));
    let param = |key: &str| -> Option<String> {
        kvp.as_ref().and_then(|kv| {
            kv.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        })
    };
    let seg = |i: usize| -> Option<String> {
        segs.get(i).map(|s| s.to_string())
    };
    let (kind, name) = match s0 {
        "wms" => ("wms", param("LAYERS").unwrap_or_else(|| "-".into())),
        "wfs" => ("wfs", param("TYPENAME").unwrap_or_else(|| "-".into())),
        "wmts" => {
            if segs.get(1) == Some(&"rest") {
                ("wmts", seg(3).unwrap_or_else(|| "-".into()))
            } else {
                ("wmts", param("LAYER").unwrap_or_else(|| "-".into()))
            }
        }
        "tms" => ("tms", seg(3)?),
        "tiles" => ("xyz", seg(1)?),
        "mvt" => ("mvt", seg(1)?),
        "pmtiles" => ("pmtiles", seg(1)?),
        "cog" => ("cog", seg(1)?),
        "mbtiles" => ("mbtiles", seg(1)?),
        "3dtiles" => ("3dtiles", seg(1).unwrap_or_else(|| "(list)".into())),
        "ogc" => match segs.get(1).copied() {
            Some("features") => ("oaf", seg(4).unwrap_or_else(|| "(catalog)".into())),
            Some("tiles") => ("oat", seg(4).unwrap_or_else(|| "(catalog)".into())),
            _ => return None,
        },
        _ => return None,
    };
    // Tile-addressing names may carry a `.png` / `.pbf` suffix.
    let name = name.split('.').next().unwrap_or(&name).to_string();
    Some((kind.to_string(), name))
}

/// axum middleware: count service requests AND write one access-log entry per
/// request (status code, duration, client IP) into the audit ring.
///
/// The peer address is `Option`-extracted: the server enables connect info via
/// `into_make_service_with_connect_info`, but tests / one-shot harnesses do
/// not, and a hard extractor would 500 those.
///
/// axum 0.8 note: `Option<T>` no longer has a blanket `FromRequestParts` impl
/// (it now requires `T: OptionalFromRequestParts`), and `ConnectInfo` only
/// implements the latter via its `Extension` representation. `ConnectInfo` is
/// injected into the request extensions by
/// `into_make_service_with_connect_info` as `Extension(ConnectInfo(..))`,
/// so extracting `Option<Extension<ConnectInfo<SocketAddr>>>` is exactly
/// equivalent to the old `Option<ConnectInfo<SocketAddr>>` and stays optional.
pub async fn track(
    State(state): State<Arc<AppState>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    req: Request,
    next: Next,
) -> Response {
    let cls = classify(req.uri());
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    // Client IP: proxy headers first (reverse-proxy deployments), then the
    // socket peer.
    let client = req
        .headers()
        .get("x-forwarded-for")
        .or_else(|| req.headers().get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or(v).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            peer.map(|c| c.0.0.ip().to_string())
                .unwrap_or_else(|| "-".to_string())
        });
    if let Some((kind, name)) = &cls {
        state.admin.usage.record(kind, name);
    }
    // 请求 ID：由最外层 SetRequestIdLayer 写入请求扩展，与回传前端的 `x-request-id`
    // 一致，使访问日志与客户端捕获的 id 指向同一次请求。必须在 next.run 消费 req 前读取。
    let req_id = req
        .extensions()
        .get::<RequestId>()
        .and_then(|r| r.header_value().to_str().ok())
        .unwrap_or_default()
        .to_string();
    let start = Instant::now();
    let resp = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let (kind, name) = cls.unwrap_or_default();
    state.admin.access_log.record(crate::admin::access_log::AccessEntry {
        ts_ms: 0, // filled by the ring
        method,
        path,
        query,
        kind,
        name,
        status: resp.status().as_u16(),
        duration_ms,
        client,
        req_id,
    });
    resp
}

/// Periodic flush task: push in-memory deltas into the PG registry.
pub fn spawn_flush(state: Arc<AppState>) {
    let secs = state.config.admin.usage_flush_secs;
    if secs == 0 {
        tracing::info!("usage flush disabled (admin.usage_flush_secs = 0)");
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(secs));
        tick.tick().await; // first tick completes immediately
        loop {
            tick.tick().await;
            let delta = state.admin.usage.take_delta();
            if delta.is_empty() {
                continue;
            }
            if let Err(e) = state.admin.registry.add_usage(&delta).await {
                tracing::warn!("usage flush failed ({} rows kept in memory): {e}", delta.len());
            }
        }
    });
}

/// Query usage: PG history when available, memory ring otherwise.
pub async fn query(
    state: &AppState,
    hours: i64,
    by_kind: bool,
) -> (Vec<(String, String, i64)>, &'static str) {
    if state.admin.registry.mode() == "pg" {
        let to = now_hour();
        let from = to - hours + 1;
        match state.admin.registry.query_usage(from, to, by_kind).await {
            Ok(rows) => return (rows, "pg"),
            Err(e) => tracing::warn!("usage PG query failed, falling back to memory: {e}"),
        }
    }
    (state.admin.usage.window_rows(hours, by_kind), "memory")
}
