//! Service registry — the durable half of the manager integration
//! (`docs/manager-scope-analysis-2026-10-26.md` §9.1: XML is the carrier,
//! the registry is the ledger).
//!
//! Two backends, chosen once at startup:
//!
//! * **PG** (`admin.dsn` configured): `sys_service` table. Every catalog scan
//!   UPSERTs service rows (kind/title/source) but **never touches `state`** —
//!   management state (stopped/paused/deleted) survives restarts and XML
//!   edits. Usage counters land in `sys_service_usage_hour` (hour buckets).
//! * **Files** (no DSN, or DSN connect failed): a sidecar XML
//!   `<catalog.root>/service-state.xml` mapping service name → state. Scan
//!   results are not persisted (the XML files ARE the scan result); only the
//!   management states are. Usage history stays in memory (lost on restart).
//!
//! Service states: `active` (serving) / `stopped` (persistently disabled,
//! survives restart) / `paused` (temporarily disabled, auto-resumes on
//! restart — identical to stopped at runtime, different intent metadata) /
//! `deleted` (marked for the trash bin; never served until restored).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pg_source::PgPool;

/// All valid service states.
pub const STATES: [&str; 4] = ["active", "stopped", "paused", "deleted"];

/// True when this state means "do not serve".
pub fn is_off(state: &str) -> bool {
    state != "active"
}

/// One catalog scan entry (a service as found on disk).
#[derive(Clone, Debug)]
pub struct ServiceScan {
    pub name: String,
    /// cog | vector | pmtiles | mbtiles | postgis | 3dtiles
    pub kind: String,
    pub title: String,
    /// Data location (path / schema.table), informational.
    pub source: String,
}

/// Durable registry handle.
pub enum Registry {
    /// PostgreSQL `sys_service` / `sys_service_usage_hour`.
    Pg(PgPool),
    /// Sidecar JSON next to the catalog (files mode).
    Files(PathBuf),
}

impl Registry {
    /// Open the registry per config: PG when `admin.dsn` is set and
    /// connectable, file sidecar otherwise. Never fails — a broken DSN
    /// degrades to files mode with a warning (the server must start).
    pub async fn open(config: &crate::config::ServerConfig) -> Registry {
        if let Some(dsn) = &config.admin.dsn {
            // new_plain: the registry DB (manager 库) usually has NO PostGIS
            // extension — skip the postgis_version probe.
            match PgPool::new_plain(dsn, 2).await {
                Ok(pool) => match Self::ensure_schema(&pool).await {
                    Ok(()) => {
                        tracing::info!(dsn = %pool.id(), "service registry: PostgreSQL (sys_service)");
                        return Registry::Pg(pool);
                    }
                    Err(e) => tracing::warn!(
                        "registry schema setup failed ({e}); falling back to file sidecar"
                    ),
                },
                Err(e) => tracing::warn!("registry DSN unreachable ({e}); file sidecar mode"),
            }
        }
        let path = Self::sidecar_path(config);
        tracing::warn!(
            path = %path.display(),
            "service registry: file sidecar — 该模式不支持多实例/集群部署（各节点的管理态会互相覆盖、计量不入库）；集群请配置 admin.dsn 走 PostgreSQL 模式"
        );
        Registry::Files(path)
    }

    fn sidecar_path(config: &crate::config::ServerConfig) -> PathBuf {
        Path::new(&config.catalog.root).join("service-state.xml")
    }

    /// Files-mode registry (test/compat constructor): sidecar JSON next to
    /// the catalog, no DSN probing.
    pub fn open_files(config: &crate::config::ServerConfig) -> Registry {
        Registry::Files(Self::sidecar_path(config))
    }

    async fn ensure_schema(pool: &PgPool) -> anyhow::Result<()> {
        let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        c.execute(
            "create table if not exists sys_service (
                name       text primary key,
                kind       text not null,
                title      text not null default '',
                source     text not null default '',
                state      text not null default 'active',
                updated_at timestamptz not null default now()
            )",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("sys_service: {e}"))?;
        c.execute(
            "create table if not exists sys_service_usage_hour (
                bucket bigint not null,
                kind   text   not null,
                name   text   not null,
                hits   bigint not null default 0,
                primary key (bucket, kind, name)
            )",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("sys_service_usage_hour: {e}"))?;
        Ok(())
    }

    /// Registry mode label (diagnostics / API responses).
    pub fn mode(&self) -> &'static str {
        match self {
            Registry::Pg(_) => "pg",
            Registry::Files(_) => "files",
        }
    }

    /// Upsert the catalog scan. Management state is intentionally NOT
    /// touched: an XML edit or a restart must never resurrect a deleted
    /// service or forget a stopped one.
    pub async fn upsert_services(&self, recs: &[ServiceScan]) -> anyhow::Result<()> {
        match self {
            Registry::Pg(pool) => {
                let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                for r in recs {
                    c.execute(
                        "insert into sys_service (name, kind, title, source, updated_at)
                         values ($1, $2, $3, $4, now())
                         on conflict (name) do update set
                           kind = excluded.kind,
                           title = excluded.title,
                           source = excluded.source,
                           updated_at = now()",
                        &[&r.name, &r.kind, &r.title, &r.source],
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("upsert {}: {e}", r.name))?;
                }
                Ok(())
            }
            Registry::Files(_) => Ok(()), // scan results live in the XML files
        }
    }

    /// All persisted management states keyed by service name.
    pub async fn fetch_states(&self) -> anyhow::Result<BTreeMap<String, String>> {
        match self {
            Registry::Pg(pool) => {
                let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                let rows = c
                    .query("select name, state from sys_service", &[])
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let mut out = BTreeMap::new();
                for r in rows {
                    let name: String = r.get(0);
                    let state: String = r.get(1);
                    out.insert(name, state);
                }
                Ok(out)
            }
            Registry::Files(path) => {
                let text = match std::fs::read_to_string(path) {
                    Ok(t) => t,
                    Err(_) => return Ok(BTreeMap::new()), // no sidecar yet
                };
                let v = sidecar_from_xml(&text)
                    .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
                Ok(v)
            }
        }
    }

    /// Persist one service's management state (creating the row when new).
    pub async fn set_state(
        &self,
        name: &str,
        kind: &str,
        title: &str,
        source: &str,
        state: &str,
    ) -> anyhow::Result<()> {
        if !STATES.contains(&state) {
            anyhow::bail!("invalid state '{state}' (use active|stopped|paused|deleted)");
        }
        match self {
            Registry::Pg(pool) => {
                let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                c.execute(
                    "insert into sys_service (name, kind, title, source, state, updated_at)
                     values ($1, $2, $3, $4, $5, now())
                     on conflict (name) do update set
                       state = excluded.state,
                       updated_at = now()",
                    &[&name, &kind, &title, &source, &state],
                )
                .await
                .map_err(|e| anyhow::anyhow!("set_state {name}: {e}"))?;
                Ok(())
            }
            Registry::Files(path) => {
                let mut v: BTreeMap<String, String> = match std::fs::read_to_string(path) {
                    Ok(t) => sidecar_from_xml(&t)
                        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?,
                    Err(_) => BTreeMap::new(),
                };
                if state == "active" {
                    v.remove(name); // default state: keep the sidecar minimal
                } else {
                    v.insert(name.to_string(), state.to_string());
                }
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if v.is_empty() {
                    // Everything back to default: drop the sidecar entirely.
                    match std::fs::remove_file(path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(anyhow::anyhow!("remove {}: {e}", path.display()))
                        }
                    }
                } else {
                    // Atomic replace: write to a temp file, then rename over
                    // the target. A concurrent reader (another process on a
                    // shared catalog root) then sees either the old or the
                    // new file, never a torn one. `std::fs::rename` replaces
                    // an existing destination on both Windows and Unix.
                    let tmp = path.with_extension("xml.tmp");
                    std::fs::write(&tmp, sidecar_to_xml(&v))
                        .map_err(|e| anyhow::anyhow!("write {}: {e}", tmp.display()))?;
                    std::fs::rename(&tmp, path)
                        .map_err(|e| anyhow::anyhow!("rename {}: {e}", path.display()))?;
                }
                Ok(())
            }
        }
    }

    /// 彻底删除一个服务的持久化管理态（硬删除后清理：配置文件已移除，
    /// 登记簿行也应随之消失，否则它会以「deleted 回收站」形态残留并干扰
    /// 后续重扫 /admin/services 列表）。幂等：本就没有该服务态时直接成功。
    pub async fn remove(&self, name: &str) -> anyhow::Result<()> {
        match self {
            Registry::Pg(pool) => {
                let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                c.execute(
                    "delete from sys_service where name = $1",
                    &[&name],
                )
                .await
                .map_err(|e| anyhow::anyhow!("remove {name}: {e}"))?;
                Ok(())
            }
            Registry::Files(path) => {
                let mut v: BTreeMap<String, String> = match std::fs::read_to_string(path) {
                    Ok(t) => sidecar_from_xml(&t)
                        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?,
                    Err(_) => return Ok(()), // no sidecar yet
                };
                if v.remove(name).is_none() {
                    return Ok(()); // 本来就没有，幂等
                }
                write_sidecar_atomic(path, &v)
            }
        }
    }

    /// 重命名服务的持久化管理态（配置文件已改名后，登记簿行随之更名，使
    /// stopped/paused/deleted 等非默认态不丢失）。仅当旧名存在非默认态时才
    /// 需要迁移（默认态 active 根本不会出现在 sidecar 中）。
    pub async fn rename(&self, name: &str, newname: &str) -> anyhow::Result<()> {
        match self {
            Registry::Pg(pool) => {
                let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
                c.execute(
                    "update sys_service set name = $2 where name = $1",
                    &[&name, &newname],
                )
                .await
                .map_err(|e| anyhow::anyhow!("rename {name}->{newname}: {e}"))?;
                Ok(())
            }
            Registry::Files(path) => {
                let mut v: BTreeMap<String, String> = match std::fs::read_to_string(path) {
                    Ok(t) => sidecar_from_xml(&t)
                        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?,
                    Err(_) => return Ok(()),
                };
                if let Some(state) = v.remove(name) {
                    v.insert(newname.to_string(), state);
                }
                write_sidecar_atomic(path, &v)
            }
        }
    }

    /// Flush usage deltas into the PG hour-bucket table. Files mode: no-op
    /// (history lives in memory only and is lost on restart).
    pub async fn add_usage(
        &self,
        rows: &[(i64, String, String, i64)], // (epoch hour, kind, name, hits)
    ) -> anyhow::Result<()> {
        let pool = match self {
            Registry::Pg(p) => p,
            Registry::Files(_) => return Ok(()),
        };
        let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        for (bucket, kind, name, hits) in rows {
            c.execute(
                "insert into sys_service_usage_hour (bucket, kind, name, hits)
                 values ($1, $2, $3, $4)
                 on conflict (bucket, kind, name) do update set
                   hits = sys_service_usage_hour.hits + excluded.hits",
                &[bucket, kind, name, hits],
            )
            .await
            .map_err(|e| anyhow::anyhow!("usage upsert: {e}"))?;
        }
        Ok(())
    }

    /// Aggregated usage over `[from_hour, to_hour]`, grouped by kind+name or
    /// kind only. Returns `Err` in files mode (caller falls back to memory).
    pub async fn query_usage(
        &self,
        from_hour: i64,
        to_hour: i64,
        by_kind: bool,
    ) -> anyhow::Result<Vec<(String, String, i64)>> {
        let pool = match self {
            Registry::Pg(p) => p,
            Registry::Files(_) => anyhow::bail!("files mode has no usage history"),
        };
        let c = pool.get().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let sql = if by_kind {
            "select kind, '' as name, sum(hits)::bigint as hits
               from sys_service_usage_hour
              where bucket between $1 and $2
              group by kind order by hits desc"
        } else {
            "select kind, name, sum(hits)::bigint as hits
               from sys_service_usage_hour
              where bucket between $1 and $2
              group by kind, name order by hits desc"
        };
        let rows = c
            .query(sql, &[&from_hour, &to_hour])
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let kind: String = r.get(0);
            let name: String = r.get(1);
            let hits: i64 = r.get(2);
            out.push((kind, name, hits));
        }
        Ok(out)
    }
}

/// sidecar 文件（XML）格式：
///
/// ```xml
/// <states>
///   <state id="wh">stopped</state>
/// </states>
/// ```
///
/// 只记录非默认状态（`active` 不落盘）。
fn sidecar_to_xml(v: &BTreeMap<String, String>) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<states>\n");
    for (name, state) in v {
        out.push_str(&format!(
            "  <state id=\"{}\">{}</state>\n",
            crate::xmlcfg::escape(name),
            crate::xmlcfg::escape(state)
        ));
    }
    out.push_str("</states>\n");
    out
}

/// 解析 sidecar XML → 服务名 → 状态。
fn sidecar_from_xml(text: &str) -> Result<BTreeMap<String, String>, String> {
    let v = crate::xmlcfg::parse_to_value(text)?;
    let mut out = BTreeMap::new();
    if let Some(states) = v.as_object() {
        for (name, state) in states {
            out.insert(name.clone(), state.as_str().unwrap_or_default().to_string());
        }
    }
    Ok(out)
}

/// 原子写 sidecar：临时文件 + rename 覆盖（见 `set_state` 注释）。`v` 为空时
/// 直接删除 sidecar（全部回到默认态），与 `set_state` 的清理逻辑一致。
fn write_sidecar_atomic(path: &Path, v: &BTreeMap<String, String>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if v.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(anyhow::anyhow!("remove {}: {e}", path.display())),
        }
        return Ok(());
    }
    let tmp = path.with_extension("xml.tmp");
    std::fs::write(&tmp, sidecar_to_xml(v))
        .map_err(|e| anyhow::anyhow!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("rename {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files 模式 sidecar 的 remove / rename 往返：模拟落盘、删除、更名，
    /// 校验 fetch_states 结果并清理临时文件。
    #[tokio::test]
    async fn files_registry_remove_and_rename() {
        let dir = std::env::temp_dir().join(format!("rms_reg_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("service-state.xml");
        let _ = std::fs::remove_file(&path);

        // 预置两个非默认态条目。
        let mut seed = BTreeMap::new();
        seed.insert("a".to_string(), "stopped".to_string());
        seed.insert("b".to_string(), "paused".to_string());
        std::fs::write(&path, sidecar_to_xml(&seed)).unwrap();

        let reg = Registry::Files(path.clone());
        // remove("a")
        reg.remove("a").await.unwrap();
        let v = reg.fetch_states().await.unwrap();
        assert!(!v.contains_key("a"), "a 应已被删除");
        assert_eq!(v.get("b").map(|s| s.as_str()), Some("paused"));

        // rename("b" -> "c")
        reg.rename("b", "c").await.unwrap();
        let v = reg.fetch_states().await.unwrap();
        assert!(!v.contains_key("b"), "b 应已更名");
        assert_eq!(v.get("c").map(|s| s.as_str()), Some("paused"));

        // 幂等：删除不存在的键不应报错。
        reg.remove("nope").await.unwrap();

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
