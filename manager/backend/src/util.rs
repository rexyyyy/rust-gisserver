//! 查询条件/参数构造助手：动态 SQL 的参数占位符管理。
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::types::ToSql;

/// 动态查询构造器：累积 `WHERE` 条件与位置参数。
pub struct Qb {
    conds: Vec<String>,
    params: Vec<Box<dyn ToSql + Sync + Send>>,
}

impl Default for Qb {
    fn default() -> Self {
        Self::new()
    }
}

impl Qb {
    pub fn new() -> Self {
        Self { conds: Vec::new(), params: Vec::new() }
    }

    /// 追加一个参数，返回可直接拼进 SQL 的 `$n` 占位符。
    pub fn p<T: ToSql + Sync + Send + 'static>(&mut self, v: T) -> String {
        self.params.push(Box::new(v));
        format!("${}", self.params.len())
    }

    /// 追加条件（条件串里用 `{}` 填占位符，由调用方 `format!` 先算好）。
    pub fn when<T: ToSql + Sync + Send + 'static>(&mut self, cond: &str, v: Option<T>) {
        if let Some(v) = v {
            let ph = self.p(v);
            self.conds.push(cond.replace("{}", &ph));
        }
    }

    /// 追加模糊匹配条件。
    pub fn like(&mut self, col: &str, v: Option<&str>) {
        if let Some(v) = v {
            if !v.is_empty() {
                let ph = self.p(format!("%{v}%"));
                self.conds.push(format!("{col} LIKE {ph}"));
            }
        }
    }

    /// 追加固定条件（不带参数）。
    pub fn raw(&mut self, cond: &str) {
        self.conds.push(cond.to_string());
    }

    pub fn where_sql(&self) -> String {
        if self.conds.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conds.join(" AND "))
        }
    }

    pub fn refs(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.params.iter().map(|b| b.as_ref() as &(dyn ToSql + Sync)).collect()
    }
}

// ---------- JSON 取值（兼容前端 camelCase 字段） ----------

pub fn jstr(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| match x {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn jopt_str(v: &Value, key: &str) -> Option<String> {
    let s = jstr(v, key);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn jopt_i64(v: &Value, key: &str) -> Option<i64> {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok().filter(|_| !s.trim().is_empty()),
        _ => None,
    }
}

/// 取 int4 列的数值（PG 的 integer 列必须以 Rust i32 传参）。
pub fn jopt_i32(v: &Value, key: &str) -> Option<i32> {
    jopt_i64(v, key).map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

pub fn jopt_bool(v: &Value, key: &str) -> Option<bool> {
    match v.get(key) {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => Some(n.as_i64().unwrap_or(0) != 0),
        Some(Value::String(s)) => Some(s == "true" || s == "1"),
        _ => None,
    }
}

pub fn jarr_i64(v: &Value, key: &str) -> Vec<i64> {
    match v.get(key) {
        Some(Value::Array(arr)) => arr.iter().filter_map(|x| x.as_i64()).collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => s
            .split(',')
            .filter_map(|p| p.trim().parse::<i64>().ok())
            .collect(),
        Some(Value::Number(n)) => n.as_i64().into_iter().collect(),
        _ => Vec::new(),
    }
}

// ---------- 位置参数收集 ----------

/// 参数包：持有 `Box<dyn ToSql + Send + Sync>`。
///
/// 有两点必要：
/// 1. 允许把 `jstr(&body, "x")` 这类**临时值**移交所有权后再取引用（否则 E0716）；
/// 2. `+ Send` 保证 handler 的 Future 是 Send（否则 axum 的 `Handler` 不满足）。
#[derive(Default)]
pub struct Pv {
    inner: Vec<Box<dyn ToSql + Sync + Send>>,
}

impl Pv {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push<T: ToSql + Sync + Send + 'static>(&mut self, v: T) -> &mut Self {
        self.inner.push(Box::new(v));
        self
    }
    pub fn refs(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.inner.iter().map(|b| b.as_ref() as &(dyn ToSql + Sync)).collect()
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// `pv![a, b, c]` → 收集为 `Pv`，用于 `query(&pool, sql, &pv.refs())`。
#[macro_export]
macro_rules! pv {
    ($($v:expr),* $(,)?) => {{
        let mut _pv = $crate::util::Pv::new();
        $( _pv.push($v); )*
        _pv
    }};
}

// ---------- 查询串取值 ----------

pub fn qstr(q: &HashMap<String, String>, key: &str) -> String {
    q.get(key).cloned().unwrap_or_default()
}
pub fn qopt(q: &HashMap<String, String>, key: &str) -> Option<String> {
    q.get(key).filter(|s| !s.trim().is_empty()).cloned()
}
pub fn qopt_i64(q: &HashMap<String, String>, key: &str) -> Option<i64> {
    q.get(key).and_then(|s| s.trim().parse::<i64>().ok())
}

/// `pageNum / pageSize` → `(limit, offset)`；缺省 1/10，最大 1000。
pub fn page_of(q: &HashMap<String, String>) -> (i64, i64) {
    let mut n = qopt_i64(q, "pageNum").unwrap_or(1).max(1);
    let mut size = qopt_i64(q, "pageSize").unwrap_or(10).max(1);
    if size > 1000 {
        size = 1000;
    }
    if n > 100_000 {
        n = 100_000;
    }
    (size, (n - 1) * size)
}
