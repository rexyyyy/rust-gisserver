//! Connection pool with a startup version pre-flight.
//!
//! `ST_TileEnvelope` and its `margin` argument only exist from certain
//! PostGIS releases onwards, so the pool records which SQL features the
//! server supports before any tile template is assembled. A plain pool
//! variant skips the PostGIS probe for metadata-only connections.

use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use semver::Version;
use std::fmt;
use thiserror::Error;
use tokio_postgres::NoTls;

/// Minimum PostgreSQL version required by the supported PostGIS versions.
const MIN_POSTGRES_VERSION: Version = Version::new(11, 0, 0);
/// Minimum PostGIS version: `ST_TileEnvelope` was added in 3.0.0.
const MIN_POSTGIS_VERSION: Version = Version::new(3, 0, 0);
/// First PostGIS release that accepts `margin` on `ST_TileEnvelope`.
const MARGIN_SINCE_POSTGIS: Version = Version::new(3, 1, 0);

/// Whether the connected server can take the `margin` argument; older
/// servers need the widened-envelope fallback in the tile predicate.
fn has_envelope_margin(v: &Version) -> bool {
    *v >= MARGIN_SINCE_POSTGIS
}
/// PostGIS >= 3.5 fixed missing geometry on some zoom levels.
const RECOMMENDED_POSTGIS_VERSION: Version = Version::new(3, 5, 0);

#[derive(Debug, Error)]
pub enum PgPoolError {
    #[error("invalid postgres connection string '{dsn}': {source}")]
    BadDsn {
        dsn: String,
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("cannot build postgres pool '{id}': {source}")]
    PoolBuild {
        id: String,
        #[source]
        source: deadpool_postgres::BuildError,
    },
    #[error("cannot connect to postgres '{id}': {source}")]
    Connect {
        id: String,
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("version probe failed on '{id}': {source}")]
    Probe {
        id: String,
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("postgresql {current} is too old for postgis; need >= {minimum}")]
    PostgresTooOld { current: String, minimum: String },
    #[error("postgis {current} is too old (ST_TileEnvelope needs >= {minimum})")]
    PostgisTooOld { current: String, minimum: String },
}

/// A shared PostGIS connection pool. `Clone` is cheap (inner `Arc`).
pub struct PgPool {
    id: String,
    pool: Pool,
    postgis_version: Version,
    /// Whether `ST_TileEnvelope` accepts the `margin` argument.
    margin_capable: bool,
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgPool")
            .field("id", &self.id)
            .field("postgis", &self.postgis_version)
            .finish()
    }
}

impl PgPool {
    /// Build a pool handle from a DSN (shared by `new` / `new_plain`).
    fn build_pool(dsn: &str, max_size: usize) -> Result<(String, Pool), PgPoolError> {
        let config = dsn
            .parse::<tokio_postgres::Config>()
            .map_err(|source| PgPoolError::BadDsn {
                dsn: dsn.to_string(),
                source,
            })?;
        let id = pool_id(&config);
        let mgr = Manager::from_config(config, NoTls, ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        let pool = Pool::builder(mgr)
            .max_size(max_size.max(1))
            .build()
            .map_err(|source| PgPoolError::PoolBuild {
                id: id.clone(),
                source,
            })?;
        Ok((id, pool))
    }

    /// Create a pool WITHOUT the PostGIS probe, for plain-SQL use on
    /// databases that may not have PostGIS at all (service registry,
    /// metadata tables). Version-gated SQL features stay disabled — never
    /// use this pool for tile SQL.
    pub async fn new_plain(dsn: &str, max_size: usize) -> Result<Self, PgPoolError> {
        let (id, pool) = Self::build_pool(dsn, max_size)?;
        // One probe connection to fail fast on an unreachable server.
        let this = Self {
            id,
            pool,
            postgis_version: Version::new(0, 0, 0),
            margin_capable: false,
        };
        let conn = this.get().await?;
        let pg_ver = probe_version(&conn, "SHOW server_version_num").await?;
        let pg_version = parse_version_num(&pg_ver);
        if pg_version < MIN_POSTGRES_VERSION {
            return Err(PgPoolError::PostgresTooOld {
                current: pg_ver,
                minimum: MIN_POSTGRES_VERSION.to_string(),
            });
        }
        Ok(this)
    }

    /// Create a pool from a `postgres://user:pass@host:port/db` DSN.
    ///
    /// TLS is not used (`NoTls`): PostGIS tile serving is expected on a
    /// trusted network segment. The pool is verified immediately: one
    /// connection is taken and the server/PostGIS versions probed.
    pub async fn new(dsn: &str, max_size: usize) -> Result<Self, PgPoolError> {
        let (id, pool) = Self::build_pool(dsn, max_size)?;
        let mut this = Self {
            id,
            pool,
            postgis_version: Version::new(0, 0, 0),
            margin_capable: false,
        };
        let conn = this.get().await?;
        let pg_ver = probe_version(&conn, "SHOW server_version_num").await?;
        let pg_version = parse_version_num(&pg_ver);
        if pg_version < MIN_POSTGRES_VERSION {
            return Err(PgPoolError::PostgresTooOld {
                current: pg_ver,
                minimum: MIN_POSTGRES_VERSION.to_string(),
            });
        }
        let postgis_ver = probe_version(&conn, "SELECT postgis_version()").await?;
        // `postgis_version()` returns e.g. "3.5 USE_GEOS=1 USE_PROJ=1
        // USE_STATS=1" -- a bare "3.5" has no patch component, which semver
        // rejects; pad to major.minor.patch before parsing.
        let postgis_version = parse_postgis_version(&postgis_ver);
        this.postgis_version = postgis_version.clone();
        if postgis_version < MIN_POSTGIS_VERSION {
            return Err(PgPoolError::PostgisTooOld {
                current: postgis_ver,
                minimum: MIN_POSTGIS_VERSION.to_string(),
            });
        }
        this.margin_capable = has_envelope_margin(&postgis_version);
        if postgis_version < RECOMMENDED_POSTGIS_VERSION {
            tracing::warn!(
                pool.id = %this.id,
                postgis.version = %postgis_version,
                "postgis < {RECOMMENDED_POSTGIS_VERSION} may hide geometry on some zoom levels; consider upgrading"
            );
        }
        tracing::info!(
            pool.id = %this.id,
            postgres.version = %pg_version,
            postgis.version = %postgis_version,
            "postgis pool ready"
        );
        Ok(this)
    }

    /// A connection from the pool (waits when the pool is saturated).
    pub async fn get(&self) -> Result<Object, PgPoolError> {
        self.pool.get().await.map_err(|source| PgPoolError::Connect {
            id: self.id.clone(),
            source,
        })
    }

    /// Stable pool identifier (`host:port/db`), for logs and stats.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether `ST_TileEnvelope` accepts the `margin` argument (PostGIS >= 3.1).
    pub fn envelope_margin_available(&self) -> bool {
        self.margin_capable
    }

    pub fn postgis_version(&self) -> &Version {
        &self.postgis_version
    }

    /// Pool status snapshot (idle / size) for the cache stats endpoint.
    pub fn status(&self) -> deadpool_postgres::Status {
        self.pool.status()
    }

    /// Never-connected pool handle for pure-SQL-template unit tests: deadpool
    /// connects lazily (build never dials), so only `tile_sql`'s
    /// the tile predicate consults this flag — preset to PostGIS 3.5.
    #[cfg(test)]
    pub(crate) fn for_tests() -> PgPool {
        let config: tokio_postgres::Config = "postgres://u:p@127.0.0.1:1/gis"
            .parse()
            .expect("static test dsn");
        let mgr = Manager::from_config(config, NoTls, ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        Self {
            id: "unit-test".to_string(),
            pool: Pool::builder(mgr)
                .build()
                .expect("deadpool build never dials"),
            postgis_version: Version::new(3, 5, 2),
            margin_capable: true,
        }
    }
}

async fn probe_version(conn: &Object, sql: &str) -> Result<String, PgPoolError> {
    let row = conn
        .query_one(sql, &[])
        .await
        .map_err(|source| PgPoolError::Probe {
            id: String::new(),
            source,
        })?;
    Ok(row.get::<_, String>(0))
}

/// Parse the PostGIS version string ("3.5 USE_GEOS=1 ...", "3.5.2", "3.4.1").
/// Two-component versions get `.0` appended; unparseable input yields 0.0.0
/// (which correctly fails the minimum-version gate).
fn parse_postgis_version(s: &str) -> Version {
    let core = s.split_whitespace().next().unwrap_or("");
    let mut parts: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    while parts.len() < 3 {
        parts.push(0);
    }
    parts.truncate(3);
    Version::new(parts[0], parts[1], parts[2])
}

/// `SHOW server_version_num` yields e.g. `160007`.
fn parse_version_num(s: &str) -> Version {
    let n: u64 = s.parse().unwrap_or(0);
    if n >= 100000 {
        Version::new((n / 10000) as u64, (n / 100 % 100) as u64, (n % 100) as u64)
    } else {
        Version::new((n / 10000) as u64, (n / 100 % 100) as u64, 0)
    }
}

/// `host:port/db` — never includes credentials (safe for logs).
fn pool_id(config: &tokio_postgres::Config) -> String {
    let host = config
        .get_hosts()
        .first()
        .map(|h| match h {
            tokio_postgres::config::Host::Tcp(s) => s.clone(),
            #[cfg(unix)]
            tokio_postgres::config::Host::Unix(p) => p.display().to_string(),
        })
        .unwrap_or_else(|| "localhost".to_string());
    let port = config
        .get_ports()
        .first()
        .copied()
        .unwrap_or(5432)
        .to_string();
    let db = config.get_dbname().unwrap_or("postgres").to_string();
    let user = config.get_user().unwrap_or("postgres").to_string();
    format!("{user}@{host}:{port}/{db}")
}
