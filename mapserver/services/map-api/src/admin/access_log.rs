//! Access log & audit: an in-memory ring of recent service requests.
//!
//! Design (manager scope doc §10 P0):
//! * The usage middleware records one entry per service request (and any
//!   non-2xx response on ops paths is skipped — the ring is for *service*
//!   traffic troubleshooting).
//! * Bounded ring (default 4000 entries, ~a few MB worst case) plus
//!   cumulative per-status-class counters, so "在线查询 + 错误统计" works
//!   without any persistence. Data is labeled "since restart" — same policy
//!   as the usage memory ring in files mode.
//! * Queries filter by status class (2xx/3xx/4xx/5xx), service kind/name and
//!   a substring over method/path/query, newest first.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// One logged request.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AccessEntry {
    /// Epoch milliseconds.
    pub ts_ms: u64,
    pub method: String,
    pub path: String,
    /// Query string without `?` (may be empty).
    pub query: String,
    /// Service class from [`crate::admin::usage::classify`] (empty when the
    /// request was not a service route, e.g. an ops endpoint).
    pub kind: String,
    /// Service name (may be `-` / empty for unattributed service hits).
    pub name: String,
    pub status: u16,
    pub duration_ms: u64,
    /// Client IP: `X-Forwarded-For` / `X-Real-IP` when present, else the
    /// peer socket address (or `-` when unknown).
    pub client: String,
    /// Request id (`x-request-id`), chains this entry with the `x-request-id`
    /// echoed to the client and the trace span — lets the access log and the
    /// client's captured id point at the same request.
    pub req_id: String,
}

#[derive(Default)]
struct Inner {
    ring: VecDeque<AccessEntry>,
    /// Cumulative counts per status code since process start.
    status_counts: HashMap<u16, u64>,
    total: u64,
}

pub struct AccessLog {
    inner: Mutex<Inner>,
    cap: usize,
}

/// Status class digit (`200` -> 2, `404` -> 4).
pub fn class_of(status: u16) -> u8 {
    (status / 100) as u8
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl AccessLog {
    /// Ring with the given capacity (entries kept, newest last).
    pub fn new(cap: usize) -> Self {
        AccessLog {
            inner: Mutex::new(Inner::default()),
            cap: cap.clamp(100, 100_000),
        }
    }

    /// Record one request (called from the usage middleware after the
    /// response is produced).
    pub fn record(&self, mut e: AccessEntry) {
        if e.ts_ms == 0 {
            e.ts_ms = now_ms();
        }
        let status = e.status;
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while g.ring.len() >= self.cap {
            g.ring.pop_front();
        }
        g.ring.push_back(e);
        g.total += 1;
        // Cumulative counters never lose information, even for entries the
        // ring will eventually drop.
        *g.status_counts.entry(status).or_default() += 1;
    }

    /// Cumulative totals: (total, per-status-code map).
    pub fn stats(&self) -> (u64, HashMap<u16, u64>) {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        (g.total, g.status_counts.clone())
    }

    /// Newest-first query. `class` filters by status class (2..=5, `None` =
    /// all), `kind`/`name` match exactly (when non-empty), `q` is a
    /// case-insensitive substring over `method path query`.
    pub fn query(
        &self,
        limit: usize,
        class: Option<u8>,
        kind: Option<&str>,
        name: Option<&str>,
        q: Option<&str>,
    ) -> Vec<AccessEntry> {
        let ql = q.map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for e in g.ring.iter().rev() {
            if let Some(c) = class {
                if class_of(e.status) != c {
                    continue;
                }
            }
            if let Some(k) = kind {
                if !k.is_empty() && e.kind != k {
                    continue;
                }
            }
            if let Some(n) = name {
                if !n.is_empty() && e.name != n {
                    continue;
                }
            }
            if !ql.is_empty() {
                let hay = format!("{} {} {}", e.method, e.path, e.query).to_ascii_lowercase();
                if !hay.contains(&ql) {
                    continue;
                }
            }
            out.push(e.clone());
            if out.len() >= limit {
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(status: u16, path: &str, kind: &str, name: &str) -> AccessEntry {
        AccessEntry {
            ts_ms: 0,
            method: "GET".into(),
            path: path.into(),
            query: String::new(),
            kind: kind.into(),
            name: name.into(),
            status,
            duration_ms: 3,
            client: "127.0.0.1".into(),
            req_id: String::new(),
        }
    }

    #[test]
    fn ring_bounds_and_query() {
        let log = AccessLog::new(100);
        for i in 0..250 {
            log.record(entry(200, &format!("/wms?p={i}"), "wms", "world"));
        }
        log.record(entry(404, "/tiles/missing/1/2/3", "xyz", "missing"));
        log.record(entry(500, "/mvt/broken/1/2/3", "mvt", "broken"));

        let (total, counts) = log.stats();
        assert_eq!(total, 252);
        assert_eq!(counts.get(&404), Some(&1));
        assert_eq!(counts.get(&500), Some(&1));

        // Ring keeps only the newest 100.
        let all = log.query(1000, None, None, None, None);
        assert_eq!(all.len(), 100);
        assert_eq!(all[0].path, "/mvt/broken/1/2/3");

        // Filters.
        assert_eq!(log.query(1000, Some(4), None, None, None).len(), 1);
        // Ring of 100 holds wms#152..249 (98) + the 404 + the 500.
        assert_eq!(log.query(1000, Some(2), Some("wms"), None, None).len(), 98);
        assert_eq!(log.query(1000, None, None, Some("missing"), None).len(), 1);
        assert_eq!(log.query(1000, None, None, None, Some("p=249")).len(), 1);
        assert!(log.query(1000, None, None, None, Some("nope")).is_empty());
    }

    #[test]
    fn class_of_basic() {
        assert_eq!(class_of(200), 2);
        assert_eq!(class_of(404), 4);
        assert_eq!(class_of(500), 5);
    }
}
