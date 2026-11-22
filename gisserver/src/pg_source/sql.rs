//! SQL templates for PostGIS-backed layers.
//!
//! * dynamic MVT tiles via `ST_AsMVT` / `ST_AsMVTGeom` / `ST_TileEnvelope`
//! * feature reads via `ST_AsGeoJSON` (WMS / WFS / OGC API paths)
//! * `count(*)` and `ST_Extent` for metadata
//!
//! Every template is assembled from the layer definition on this crate's own
//! terms: identifiers are quoted, literals escaped, and tile queries bind
//! `$1/$2/$3` so the prepared statement stays cached. Spatial predicates
//! follow the documented PostGIS function semantics (see
//! [`tile_index_filter`]); the tile row set is folded into MVT in a CTE.

use crate::pg_source::{PgPool, PgSourceDef};
use bytes::BytesMut;
use serde_json::Value;
use std::fmt::Write as _;
use thiserror::Error;
use tokio_postgres::types::{IsNull, ToSql, Type};

/// Error from a PostGIS query (pool acquisition or the query itself).
#[derive(Debug, Error)]
pub enum QueryError {
    #[error("postgis pool: {0}")]
    Pool(String),
    #[error("postgis query: {0}")]
    Query(String),
}

impl From<tokio_postgres::Error> for QueryError {
    fn from(e: tokio_postgres::Error) -> Self {
        // tokio-postgres's own Display is just "db error"; surface the
        // server-side message + SQLSTATE so handlers and probes can see WHY
        // (e.g. "latitude out of range (SQLSTATE 22003)").
        let msg = match e.as_db_error() {
            Some(db) => format!("{} (SQLSTATE {:?})", db.message(), db.code()),
            None => e.to_string(),
        };
        QueryError::Query(msg)
    }
}

/// Degrees in one full turn; used to scale a tile buffer into EPSG:4326 units.
const DEGREES_PER_FULL_TURN: f64 = 360.0;

/// Bound z/x/y tile address, shared by the geometry envelope and the index
/// predicate so both are built from a single definition.
const ZXY_ARGS: &str = "$1::integer, $2::integer, $3::integer";

/// Web Mercator world width in meters (the 3857 simplify tolerance baseline:
/// tolerance(z) = world / 2^z / extent * tile_units).
const WEB_MERCATOR_WORLD_WIDTH: f64 = 40_075_016.685_578_5;

/// A SQL parameter (owned) so callers can build queries dynamically.
#[derive(Debug)]
pub enum SqlParam {
    F(f64),
    I(i64),
    /// Explicit int4 (e.g. the projected tile path's zoom parameter).
    I32(i32),
    S(String),
}

impl ToSql for SqlParam {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            SqlParam::F(v) => v.to_sql(ty, out),
            SqlParam::I(v) => v.to_sql(ty, out),
            SqlParam::I32(v) => v.to_sql(ty, out),
            SqlParam::S(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(ty: &Type) -> bool {
        <f64 as ToSql>::accepts(ty)
            || <i64 as ToSql>::accepts(ty)
            || <i32 as ToSql>::accepts(ty)
            || <String as ToSql>::accepts(ty)
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Borrow a parameter slice for `tokio_postgres::query`.
pub fn as_params(params: &[SqlParam]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|p| p as &(dyn ToSql + Sync)).collect()
}

/// Quote a SQL identifier (`table` -> `"table"`).
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Quote a SQL string literal (`a'b` -> `'a''b'`).
pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The FROM-clause source: the configured SQL/view expression (used verbatim;
/// admin-trusted, validated by [`PgSourceDef::validate`]) or the quoted
/// `schema.table`.
fn qualified(def: &PgSourceDef) -> String {
    match &def.from {
        Some(f) => f.clone(),
        None => format!("{}.{}", quote_ident(&def.schema), quote_ident(&def.table)),
    }
}

/// The geometry expression for the 3857 tile path, honoring
/// [`PgSourceDef::simplify`]: below `under_zoom` geometry is simplified in
/// EPSG:3857 meters with a per-tile tolerance derived from `$1` (z), so the
/// prepared statement stays shared across zoom levels. Postgres evaluates
/// only the taken `CASE` branch, the other expression is never computed.
fn mercator_geom_expr(def: &PgSourceDef, geom: &str, extent: u32) -> String {
    let base = format!("ST_Transform(ST_CurveToLine({geom}::geometry), 3857)");
    match &def.simplify {
        Some(s) => format!(
            "CASE WHEN $1::integer < {uz} THEN ST_SimplifyPreserveTopology({base}, \
             ({W}::float8 / power(2.0::float8, ($1::integer)::float8)) \
             / {extent}::float8 * {tu}::float8) ELSE {base} END",
            uz = s.under_zoom,
            base = base,
            W = WEB_MERCATOR_WORLD_WIDTH,
            extent = extent,
            tu = s.tile_units,
        ),
        None => base,
    }
}

/// The `id + properties` select list (comma-prefixed), for appending right
/// after a leading select expression. Empty string when the source has
/// neither an id column nor properties.
fn extra_cols(def: &PgSourceDef) -> String {
    let mut cols = String::new();
    if let Some(id) = &def.id_column {
        cols.push_str(&format!(",\n    {}", quote_ident(id)));
    }
    for c in &def.properties {
        cols.push_str(&format!(",\n    {}", quote_ident(c)));
    }
    cols
}

/// Compose the index-friendly geometry predicate for a Web-Mercator tile.
///
/// A non-zero tile buffer widens the search envelope so features straddling
/// the tile edge are still encoded. PostGIS >= 3.1 accepts a `margin`
/// fraction on `ST_TileEnvelope`, but only in EPSG:3857; EPSG:4326 needs the
/// widening in degrees via `ST_Expand` (a degree spans a full turn at z=0,
/// hence the `2^z` divisor). Any other SRID keeps the plain transformed
/// envelope. The predicate always tests the source geometry column, so a GiST
/// index on that column remains usable.
fn tile_index_filter(pool: &PgPool, buffer: u32, extent: u32, srid: i32) -> String {
    let envelope = format!("ST_TileEnvelope({ZXY_ARGS})");
    let transformed = format!("ST_Transform({envelope}, {srid})");
    if buffer == 0 {
        return transformed;
    }

    let margin = f64::from(buffer) / f64::from(extent);
    match srid {
        3857 if pool.envelope_margin_available() => format!(
            "ST_Transform(ST_TileEnvelope({ZXY_ARGS}, margin => {margin}), {srid})"
        ),
        4326 => format!(
            "ST_Expand({transformed}, ({margin} * {DEGREES_PER_FULL_TURN}) / 2^$1::integer)"
        ),
        _ => transformed,
    }
}

/// Build the dynamic MVT tile SQL for a PostGIS layer.
///
/// Parameters: `$1` z, `$2` x, `$3` y (all `::integer`). Clipped feature rows
/// are collected in a CTE and folded into one MVT row afterwards; keeping the
/// geometry predicate on the source column lets PostGIS use its spatial index.
pub fn tile_sql(pool: &PgPool, def: &PgSourceDef, layer_id: &str) -> String {
    let extent = def.extent();
    let buffer = def.buffer();
    let clip = if def.clip_geom() { "true" } else { "false" };
    let geom = quote_ident(&def.geometry_column);
    let cols = extra_cols(def);
    let geom_expr = mercator_geom_expr(def, &geom, extent);
    let spatial_filter = tile_index_filter(pool, buffer, extent, def.srid);
    let limit = def
        .limit
        .map(|v| format!("LIMIT {v}"))
        .unwrap_or_default();

    format!(
        r#"WITH clipped AS (
  SELECT
    ST_AsMVTGeom(
        {geom_expr},
        ST_TileEnvelope({ZXY_ARGS}),
        {extent}, {buffer}, {clip}
    ) AS geom{cols}
  FROM {table}
  WHERE {geom} && {spatial_filter}
  {limit}
)
SELECT ST_AsMVT(tile, {lid}, {extent}, 'geom') FROM clipped AS tile"#,
        geom_expr = geom_expr,
        cols = cols,
        table = qualified(def),
        spatial_filter = spatial_filter,
        limit = limit,
        lid = quote_literal(layer_id),
    )
}

/// Property columns for feature reads: configured list, or (when empty) the
/// caller-supplied discovered list.
fn prop_columns(def: &PgSourceDef) -> Vec<String> {
    def.properties
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
}

/// Build the feature-read SQL (`ST_AsGeoJSON` rows in EPSG:4326).
///
/// * `bbox_4326` — optional envelope filter (intersects via `&&`)
/// * `where_sql` — optional extra predicate (translated OGC filter)
/// * `order_by` — optional `ORDER BY` fragment override (already validated +
///   quoted by [`parse_sortby`]); falls back to the id column
/// * `limit` / `offset` — paging (`-1` = no limit)
///
/// Returns the SQL and the positional parameters it expects.
pub fn feature_sql(
    def: &PgSourceDef,
    bbox_4326: Option<[f64; 4]>,
    where_sql: Option<&str>,
    order_by: Option<&str>,
    limit: i64,
    offset: i64,
) -> (String, Vec<SqlParam>) {
    let geom = quote_ident(&def.geometry_column);
    let mut params: Vec<SqlParam> = Vec::new();
    let mut conds: Vec<String> = Vec::new();

    if let Some([a, b, c, d]) = bbox_4326 {
        params.push(SqlParam::F(a));
        params.push(SqlParam::F(b));
        params.push(SqlParam::F(c));
        params.push(SqlParam::F(d));
        let n = params.len();
        conds.push(format!(
            "{geom} && ST_Transform(ST_MakeEnvelope(${}, ${}, ${}, ${}, 4326), {})",
            n - 3,
            n - 2,
            n - 1,
            n,
            def.srid
        ));
    }
    if let Some(w) = where_sql {
        if !w.is_empty() {
            conds.push(format!("({w})"));
        }
    }
    let where_clause = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };

    let id_expr = def
        .id_column
        .as_ref()
        .map(|c| format!("{} AS id", quote_ident(c)))
        .unwrap_or_else(|| "NULL AS id".to_string());
    let props = prop_columns(def);
    let props_sql = if props.is_empty() {
        String::new()
    } else {
        format!(",\n    {}", props.join(",\n    "))
    };
    let order = match order_by {
        Some(o) if !o.is_empty() => format!("ORDER BY {o}"),
        _ => def
            .id_column
            .as_ref()
            .map(|c| format!("ORDER BY {}", quote_ident(c)))
            .unwrap_or_default(),
    };

    let mut paging = String::new();
    if limit >= 0 {
        params.push(SqlParam::I(limit));
        let _ = write!(paging, "LIMIT ${}", params.len());
        if offset > 0 {
            params.push(SqlParam::I(offset));
            let _ = write!(paging, " OFFSET ${}", params.len());
        }
    }

    let sql = format!(
        "SELECT ST_AsGeoJSON(t.*) FROM (\n  SELECT {id_expr}{props_sql},\n    ST_Transform({geom}, 4326) AS geom\n  FROM {table}\n  {where_clause}\n  {order}\n  {paging}\n) AS t",
        id_expr = id_expr,
        props_sql = props_sql,
        geom = geom,
        table = qualified(def),
        where_clause = where_clause,
        order = order,
        paging = paging,
    );
    (sql, params)
}

/// Count SQL for the same predicate set as [`feature_sql`].
pub fn count_sql(
    def: &PgSourceDef,
    bbox_4326: Option<[f64; 4]>,
    where_sql: Option<&str>,
) -> (String, Vec<SqlParam>) {
    let geom = quote_ident(&def.geometry_column);
    let mut params: Vec<SqlParam> = Vec::new();
    let mut conds: Vec<String> = Vec::new();
    if let Some([a, b, c, d]) = bbox_4326 {
        params.extend([SqlParam::F(a), SqlParam::F(b), SqlParam::F(c), SqlParam::F(d)]);
        conds.push(format!(
            "{geom} && ST_Transform(ST_MakeEnvelope($1, $2, $3, $4, 4326), {})",
            def.srid
        ));
    }
    if let Some(w) = where_sql {
        if !w.is_empty() {
            conds.push(format!("({w})"));
        }
    }
    let where_clause = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    (
        format!(
            "SELECT count(*) FROM {} {}",
            qualified(def),
            where_clause
        ),
        params,
    )
}

/// `ST_Extent` of the (optionally filtered) table, in EPSG:4326.
pub fn bounds_sql(def: &PgSourceDef) -> String {
    // `::text` cast: ST_Extent returns box2d, which tokio-postgres cannot
    // decode into String; the cast yields "BOX(x1 y1,x2 y2)".
    format!(
        "SELECT ST_Extent(ST_Transform({}, 4326))::text FROM {}",
        quote_ident(&def.geometry_column),
        qualified(def)
    )
}

/// Single-feature SQL by id value (`$1` = id compared as text, so any id
/// column type works). Requires `def.id_column`.
pub fn feature_by_id_sql(def: &PgSourceDef) -> (String, Vec<SqlParam>) {
    let geom = quote_ident(&def.geometry_column);
    let id_col = quote_ident(def.id_column.as_deref().unwrap_or("id"));
    let id_expr = def
        .id_column
        .as_ref()
        .map(|c| format!("{} AS id", quote_ident(c)))
        .unwrap_or_else(|| "NULL AS id".to_string());
    let props = prop_columns(def);
    let props_sql = if props.is_empty() {
        String::new()
    } else {
        format!(",\n    {}", props.join(",\n    "))
    };
    let sql = format!(
        "SELECT ST_AsGeoJSON(t.*) FROM (\n  SELECT {id_expr}{props_sql},\n    ST_Transform({geom}, 4326) AS geom\n  FROM {table}\n  WHERE {id_col}::text = $1\n  LIMIT 1\n) AS t",
        id_expr = id_expr,
        props_sql = props_sql,
        geom = geom,
        table = qualified(def),
        id_col = id_col,
    );
    (sql, vec![SqlParam::S(String::new())])
}

/// Point-in-table SQL for GetFeatureInfo (first feature containing the point).
///
/// Parameters: `$1` lon, `$2` lat (EPSG:4326).
pub fn point_feature_sql(def: &PgSourceDef) -> (String, Vec<SqlParam>) {
    let geom = quote_ident(&def.geometry_column);
    let id_expr = def
        .id_column
        .as_ref()
        .map(|c| format!("{} AS id", quote_ident(c)))
        .unwrap_or_else(|| "NULL AS id".to_string());
    let props = prop_columns(def);
    let props_sql = if props.is_empty() {
        String::new()
    } else {
        format!(",\n    {}", props.join(",\n    "))
    };
    let sql = format!(
        "SELECT ST_AsGeoJSON(t.*) FROM (\n  SELECT {id_expr}{props_sql},\n    ST_Transform({geom}, 4326) AS geom\n  FROM {table}\n  WHERE ST_Intersects({geom}, ST_Transform(ST_SetSRID(ST_MakePoint($1, $2), 4326), {srid}))\n  LIMIT 1\n) AS t",
        id_expr = id_expr,
        props_sql = props_sql,
        geom = geom,
        table = qualified(def),
        srid = def.srid,
    );
    (sql, vec![SqlParam::F(0.0), SqlParam::F(0.0)])
}

/// Run the dynamic tile query. `Ok(None)` = empty tile (no features).
pub async fn query_tile(
    pool: &PgPool,
    sql: &str,
    z: u32,
    x: u32,
    y: u32,
) -> Result<Option<Vec<u8>>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let stmt = conn.prepare_cached(sql).await?;
    let row = conn
        .query_opt(&stmt, &[&(z as i32), &(x as i32), &(y as i32)])
        .await?;
    Ok(row.and_then(|r| r.get::<_, Option<Vec<u8>>>(0)))
}

/// Run a feature query, parsing each row's `ST_AsGeoJSON` text.
pub async fn query_features(
    pool: &PgPool,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Value>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let rows = conn.query(sql, params).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let text: String = row.get(0);
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            out.push(v);
        }
    }
    Ok(out)
}

/// Count matching features.
pub async fn query_count(
    pool: &PgPool,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<i64, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let row = conn.query_one(sql, params).await?;
    Ok(row.get::<_, i64>(0))
}

/// Data extent `[minx, miny, maxx, maxy]` (EPSG:4326) or `None` when empty.
pub async fn query_bounds(pool: &PgPool, sql: &str) -> Result<Option<[f64; 4]>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let row = conn.query_one(sql, &[]).await?;
    let text: Option<String> = row.get(0);
    Ok(text.as_deref().and_then(parse_box))
}

/// Parse PostGIS `BOX(x1 y1,x2 y2)` text.
pub fn parse_box(s: &str) -> Option<[f64; 4]> {
    let inner = s.strip_prefix("BOX(")?.strip_suffix(')')?;
    let (a, b) = inner.split_once(',')?;
    let mut ia = a.split_whitespace();
    let mut ib = b.split_whitespace();
    let x1 = ia.next()?.parse().ok()?;
    let y1 = ia.next()?.parse().ok()?;
    let x2 = ib.next()?.parse().ok()?;
    let y2 = ib.next()?.parse().ok()?;
    Some([x1, y1, x2, y2])
}

/// Discover the non-geometry columns of a table (used when the layer config
/// leaves `properties` empty).
pub async fn discover_columns(
    pool: &PgPool,
    schema: &str,
    table: &str,
    geometry_column: &str,
) -> Result<Vec<String>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let rows = conn
        .query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             AND column_name <> $3 ORDER BY ordinal_position",
            &[&schema, &table, &geometry_column],
        )
        .await?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

/// Discover columns **with TileJSON field types** (`Number`/`String`/
/// `Boolean`), used for the `vector_layers` member of PostGIS TileJSON.
pub async fn discover_fields(
    pool: &PgPool,
    schema: &str,
    table: &str,
    geometry_column: &str,
) -> Result<Vec<(String, &'static str)>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let rows = conn
        .query(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             AND column_name <> $3 ORDER BY ordinal_position",
            &[&schema, &table, &geometry_column],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let ty: String = r.get(1);
            let kind = match ty.as_str() {
                "smallint" | "integer" | "bigint" | "decimal" | "numeric" | "real"
                | "double precision" => "Number",
                "boolean" => "Boolean",
                _ => "String",
            };
            (name, kind)
        })
        .collect())
}

/// Build the **projected-CRS** dynamic MVT tile SQL (the "original
/// projection" interface).
///
/// Unlike [`tile_sql`] (which reprojects geometry to 3857 and uses
/// `ST_TileEnvelope`), this template keeps geometry in the source SRID —
/// the tile envelope in map units is computed by
/// [`crate::pg_source::grid::tile_bounds`] and passed as `$1..$4`
/// (`minx, miny, maxx, maxy`). No `ST_Transform` at all: the cheapest and
/// most faithful path for projected data.
pub fn tile_sql_projected(def: &PgSourceDef, layer_id: &str) -> String {
    tile_sql_env(def, layer_id, def.srid)
}

/// Like [`tile_sql_projected`], but the tile envelope is expressed in
/// `out_srid` (the published grid's CRS) instead of the source SRID.
///
/// `out_srid == def.srid` keeps the geometry untouched (the common case: a
/// 4490 layer on a 4490 custom grid). A different grid CRS reprojects the
/// geometry once, so the same source can be cut into any advertised grid.
pub fn tile_sql_env(def: &PgSourceDef, layer_id: &str, out_srid: i32) -> String {
    let extent = def.extent();
    let buffer = def.buffer();
    let clip = if def.clip_geom() { "true" } else { "false" };
    let srid = out_srid;
    let reproject = out_srid != def.srid;
    let geom = quote_ident(&def.geometry_column);

    let limit = def
        .limit
        .map(|v| format!("LIMIT {v}"))
        .unwrap_or_default();
    let cols = extra_cols(def);

    // $5 is the zoom level (SqlParam::I32 from the caller) — the simplify
    // gate needs it because the projected envelope travels as coordinates,
    // not as a tile address.
    let geom_expr = match &def.simplify {
        Some(s) => format!(
            "CASE WHEN $5::integer < {uz} THEN ST_SimplifyPreserveTopology(\
             ST_CurveToLine({geom}::geometry), ($3::float8 - $1::float8) / {extent}::float8 * {tu}::float8) \
             ELSE ST_CurveToLine({geom}::geometry) END",
            uz = s.under_zoom,
            geom = geom,
            extent = extent,
            tu = s.tile_units,
        ),
        None => format!("ST_CurveToLine({geom}::geometry)"),
    };
    // Grid CRS differs from the data SRID: reproject the geometry once. The
    // WHERE filter still tests the *source* geometry against the envelope
    // transformed back into the source SRID, so the GiST index is used.
    //
    // The envelope is widened by the same tile buffer fraction the
    // Web-Mercator path uses (see `tile_index_filter`): `ST_AsMVTGeom` keeps
    // everything inside tile+buffer, but a feature that only reaches into that
    // ring cannot pass an un-widened `&&` test, so it is dropped before
    // encoding. Measured against the 51 192-row Wuhua buildings table on tile
    // 10/1608/368: exact envelope -> 8 950 encoded features, widened -> 9 415,
    // i.e. 465 features (4.9 %) live only in the buffer ring.
    let envelope = format!("ST_MakeEnvelope($1, $2, $3, $4, {srid})");
    let search_box = if buffer == 0 {
        envelope
    } else {
        let margin = f64::from(buffer) / f64::from(extent);
        format!("ST_Expand({envelope}, ($3 - $1) * {margin}, ($4 - $2) * {margin})")
    };
    let (geom_expr, spatial_filter) = if reproject {
        (
            format!("ST_Transform({geom_expr}, {srid})"),
            format!("ST_Transform({search_box}, {src})", src = def.srid),
        )
    } else {
        (geom_expr, search_box)
    };

    format!(
        r#"SELECT ST_AsMVT(tile, {lid}, {extent}, 'geom') FROM (
  SELECT
    ST_AsMVTGeom(
        {geom_expr},
        ST_MakeEnvelope($1, $2, $3, $4, {srid}),
        {extent}, {buffer}, {clip}
    ) AS geom{cols}
  FROM
    {table}
  WHERE
    {geom} && {spatial_filter}
  {limit}
) AS tile"#,
        lid = quote_literal(layer_id),
        geom_expr = geom_expr,
        spatial_filter = spatial_filter,
        cols = cols,
        table = qualified(def),
        geom = geom,
        limit = limit.trim_end(),
    )
}

/// `ST_Extent` of the table **in the source SRID** (no reprojection), for the
/// projected TileJSON bounds.
pub fn bounds_sql_src(def: &PgSourceDef) -> String {
    // `::text` cast: see bounds_sql.
    format!(
        "SELECT ST_Extent({})::text FROM {}",
        quote_ident(&def.geometry_column),
        qualified(def)
    )
}

/// Fast startup bounds (P3): `ST_EstimatedExtent` reads planner statistics in
/// the source SRID, then the envelope is transformed to EPSG:4326 — no full
/// table scan. Returns a NULL box when statistics are missing; callers fall
/// back to [`bounds_sql`]. Table sources only (a `from` subquery has no
/// single stats entry).
pub fn bounds_sql_estimated(def: &PgSourceDef) -> String {
    format!(
        "WITH e AS (SELECT ST_EstimatedExtent({s}, {t}, {g}) AS b) \
         SELECT CASE WHEN b IS NULL THEN NULL ELSE \
         ST_Envelope(ST_Transform(ST_SetSRID(ST_MakeEnvelope(ST_XMin(b), ST_YMin(b), \
         ST_XMax(b), ST_YMax(b), {srid}), {srid}), 4326))::text END FROM e",
        s = quote_literal(&def.schema),
        t = quote_literal(&def.table),
        g = quote_literal(&def.geometry_column),
        srid = def.srid,
    )
}

/// Fast startup count (P3): `pg_class.reltuples` (planner statistics). A
/// result `<= 0` (never analyzed / empty estimate) must fall back to an exact
/// count — the caller decides.
pub fn count_sql_estimated(def: &PgSourceDef) -> String {
    format!(
        "SELECT reltuples::bigint FROM pg_class WHERE oid = to_regclass({})",
        quote_literal(&format!(
            "{}.{}",
            quote_ident(&def.schema),
            quote_ident(&def.table)
        ))
    )
}

/// Exact full-collection count over whatever [`qualified`] resolves to
/// (table or `from` expression).
pub fn count_all_sql(def: &PgSourceDef) -> String {
    format!("SELECT count(*) FROM {}", qualified(def))
}

/// GiST spatial-index coverage check (P3): counts indexes on the table that
/// are `USING gist` and mention the geometry column. `0` at startup means
/// every tile/feature query degenerates into a sequential scan.
pub fn gist_index_sql(def: &PgSourceDef) -> String {
    format!(
        "SELECT count(*) FROM pg_indexes WHERE schemaname = {s} AND tablename = {t} \
         AND indexdef ILIKE '%using gist%' AND indexdef ILIKE '%' || {g} || '%'",
        s = quote_literal(&def.schema),
        t = quote_literal(&def.table),
        g = quote_literal(&def.geometry_column),
    )
}

/// Discover property columns **with TileJSON field types** for a `from`
/// SQL/view layer (P3): `information_schema` cannot see through subqueries,
/// so the row description of `SELECT * FROM <expr> LIMIT 0` is read instead —
/// the prepared result carries column names and Postgres types even with no
/// rows.
pub async fn discover_from_fields(
    pool: &PgPool,
    from_expr: &str,
    geometry_column: &str,
) -> Result<Vec<(String, &'static str)>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let sql = format!("SELECT * FROM {} LIMIT 0", from_expr);
    let stmt = conn.prepare(&sql).await?;
    Ok(stmt
        .columns()
        .iter()
        .filter(|c| c.name() != geometry_column)
        .map(|c| {
            let kind = match c.type_().name() {
                "int2" | "int4" | "int8" | "float4" | "float8" | "numeric" => "Number",
                "bool" => "Boolean",
                _ => "String",
            };
            (c.name().to_string(), kind)
        })
        .collect())
}

/// Translate an OGC API - Features `sortby` value (`prop`, `-prop`, or a
/// comma list) into an `ORDER BY` fragment (P3). Column names are validated
/// against the allowed set (configured properties + id column); anything
/// else is a caller-mapped 400 — never interpolated into SQL.
pub fn parse_sortby(sortby: &str, allowed: &[String]) -> Result<Option<String>, String> {
    let mut parts = Vec::new();
    for raw in sortby.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (col, desc) = match raw.strip_prefix('-') {
            Some(c) => (c, true),
            None => (raw.strip_prefix('+').unwrap_or(raw), false),
        };
        let col = col.trim();
        if !allowed.iter().any(|a| a == col) {
            return Err(format!("column '{col}' is not a published property"));
        }
        parts.push(format!(
            "{} {}",
            quote_ident(col),
            if desc { "DESC" } else { "ASC" }
        ));
    }
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(parts.join(", ")))
}

/// Run a tile query with `SqlParam` parameters (projected envelope path).
/// `Ok(None)` = empty tile.
pub async fn query_tile_env(
    pool: &PgPool,
    sql: &str,
    params: &[SqlParam],
) -> Result<Option<Vec<u8>>, QueryError> {
    let conn = pool.get().await.map_err(|e| QueryError::Pool(e.to_string()))?;
    let stmt = conn.prepare_cached(sql).await?;
    let refs = as_params(params);
    let row = conn.query_opt(&stmt, &refs).await?;
    Ok(row.and_then(|r| r.get::<_, Option<Vec<u8>>>(0)))
}

#[cfg(test)]
mod projected_tests {
    use super::*;
    use crate::pg_source::PgPool;

    #[test]
    fn tile_index_predicate_branches() {
        // `for_tests()` pretends to be PostGIS 3.5, so margins are available.
        let pool = PgPool::for_tests();
        let unbuffered = tile_index_filter(&pool, 0, 4096, 3857);
        assert_eq!(
            unbuffered,
            "ST_Transform(ST_TileEnvelope($1::integer, $2::integer, $3::integer), 3857)"
        );

        let mercator = tile_index_filter(&pool, 64, 4096, 3857);
        assert!(mercator.contains("margin => 0.015625"), "{mercator}");
        assert!(mercator.contains(", 3857)"), "{mercator}");

        let geographic = tile_index_filter(&pool, 64, 4096, 4326);
        assert!(geographic.starts_with("ST_Expand("), "{geographic}");
        assert!(geographic.contains("360"), "{geographic}");
        assert!(geographic.contains("2^$1::integer"), "{geographic}");

        let projected = tile_index_filter(&pool, 64, 4096, 4543);
        assert!(!projected.contains("margin"), "{projected}");
        assert!(!projected.contains("ST_Expand"), "{projected}");
        assert!(projected.ends_with(", 4543)"), "{projected}");
    }

    #[test]
    fn mercator_tile_sql_uses_cte_and_keeps_contract() {
        let def = test_def();
        let sql = tile_sql(&PgPool::for_tests(), &def, "wuhua");
        assert!(sql.starts_with("WITH clipped AS ("), "{sql}");
        assert!(sql.contains("ST_AsMVT(tile, 'wuhua', 4096, 'geom')"), "{sql}");
        assert!(sql.contains("ST_AsMVTGeom("), "{sql}");
        assert!(sql.contains("ST_TileEnvelope($1::integer, $2::integer, $3::integer)"), "{sql}");
        assert!(sql.contains("\"geom\" &&"), "{sql}");
        assert!(sql.contains("FROM \"public\".\"wuhua_buildings\""), "{sql}");
        assert!(sql.contains("FROM clipped AS tile"), "{sql}");
    }

    /// The envelope path must widen its index predicate by the tile buffer,
    /// otherwise features that only reach into the buffer ring never pass the
    /// `&&` test and the `buffer` handed to `ST_AsMVTGeom` is wasted.
    #[test]
    fn envelope_tile_sql_widens_the_index_predicate() {
        let def = test_def();

        // Same CRS as the grid: widen in grid units, no reprojection.
        let same = tile_sql_env(&def, "wuhua", 4543);
        assert!(
            same.contains("ST_Expand(ST_MakeEnvelope($1, $2, $3, $4, 4543), ($3 - $1) * 0.015625, ($4 - $2) * 0.015625)"),
            "{same}"
        );
        // The clip envelope handed to ST_AsMVTGeom stays the exact tile.
        assert!(
            same.contains("ST_AsMVTGeom(\n        ST_CurveToLine(\"geom\"::geometry),\n        ST_MakeEnvelope($1, $2, $3, $4, 4543),\n        4096, 64, true"),
            "{same}"
        );

        // Different grid CRS: widen in grid units *before* transforming back.
        let reproj = tile_sql_env(&def, "wuhua", 4326);
        assert!(reproj.contains("ST_Transform(ST_Expand(ST_MakeEnvelope($1, $2, $3, $4, 4326)"), "{reproj}");
        assert!(reproj.contains(", 4543)"), "{reproj}");

        // buffer = 0 -> exact envelope, no widening anywhere.
        let mut nobuf = test_def();
        nobuf.buffer = Some(0);
        let sql = tile_sql_env(&nobuf, "wuhua", 4543);
        assert!(!sql.contains("ST_Expand"), "{sql}");
        assert!(sql.contains("\"geom\" && ST_MakeEnvelope($1, $2, $3, $4, 4543)"), "{sql}");
    }

    fn test_def() -> PgSourceDef {
        PgSourceDef {
            dsn: "postgres://u:p@localhost:5432/gis".into(),
            schema: "public".into(),
            table: "wuhua_buildings".into(),
            geometry_column: "geom".into(),
            srid: 4543,
            layer_id: None,
            id_column: Some("gid".into()),
            properties: vec!["height".into()],
            minzoom: None,
            maxzoom: None,
            extent: None,
            buffer: None,
            clip_geom: None,
            limit: None,
            bounds: None,
            feature_count: None,
            from: None,
            simplify: None,
        }
    }

    #[test]
    fn from_expr_replaces_schema_table() {
        let mut def = test_def();
        def.from = Some("(SELECT gid, height, geom FROM public.v_tall) AS v".into());
        let sql = tile_sql_projected(&def, "tall");
        assert!(sql.contains("(SELECT gid, height, geom FROM public.v_tall) AS v"));
        assert!(!sql.contains("wuhua_buildings"));
        // count_all / bounds reuse the same FROM resolution.
        assert_eq!(
            count_all_sql(&def),
            "SELECT count(*) FROM (SELECT gid, height, geom FROM public.v_tall) AS v"
        );
        assert!(bounds_sql_src(&def).ends_with("FROM (SELECT gid, height, geom FROM public.v_tall) AS v"));
    }

    #[test]
    fn validate_rejects_from_semicolon_and_bad_tolerance() {
        let mut def = test_def();
        def.from = Some("(SELECT 1) AS v; DROP TABLE x".into());
        assert!(def.validate().is_err());
        def.from = Some("(SELECT 1) AS v".into());
        def.simplify = Some(crate::pg_source::SimplifyOpts { under_zoom: 12, tile_units: -1.0 });
        assert!(def.validate().is_err());
        def.simplify = Some(crate::pg_source::SimplifyOpts { under_zoom: 12, tile_units: 2.0 });
        assert!(def.validate().is_ok());
    }

    #[test]
    fn simplify_enters_both_tile_templates() {
        let mut def = test_def();
        def.simplify = Some(crate::pg_source::SimplifyOpts { under_zoom: 12, tile_units: 2.5 });
        let projected = tile_sql_projected(&def, "w");
        assert!(projected.contains("CASE WHEN $5::integer < 12"));
        assert!(projected.contains("($3::float8 - $1::float8) / 4096::float8 * 2.5::float8"));
        let merc = tile_sql(&PgPool::for_tests(), &def, "w");
        assert!(merc.contains("CASE WHEN $1::integer < 12"));
        assert!(merc.contains("power(2.0::float8, ($1::integer)::float8)"));
        assert!(merc.contains("ST_SimplifyPreserveTopology"));
        // No simplify -> plain expressions, no CASE.
        def.simplify = None;
        assert!(!tile_sql_projected(&def, "w").contains("CASE WHEN"));
    }

    #[test]
    fn parse_sortby_validates_and_orders() {
        let allowed = vec!["height".to_string(), "gid".to_string()];
        assert_eq!(
            parse_sortby("height", &allowed).unwrap().unwrap(),
            "\"height\" ASC"
        );
        assert_eq!(
            parse_sortby("-height", &allowed).unwrap().unwrap(),
            "\"height\" DESC"
        );
        assert_eq!(
            parse_sortby("+height,-gid", &allowed).unwrap().unwrap(),
            "\"height\" ASC, \"gid\" DESC"
        );
        assert!(parse_sortby("  ", &allowed).unwrap().is_none());
        assert!(parse_sortby("geom; drop table x", &allowed).is_err());
        assert!(parse_sortby("secret_col", &allowed).is_err());
    }

    #[test]
    fn estimated_probe_sql_shapes() {
        let def = test_def();
        let b = bounds_sql_estimated(&def);
        assert!(b.contains("ST_EstimatedExtent('public', 'wuhua_buildings', 'geom')"));
        assert!(b.contains("ST_Transform(ST_SetSRID(ST_MakeEnvelope("));
        let c = count_sql_estimated(&def);
        assert_eq!(
            c,
            "SELECT reltuples::bigint FROM pg_class WHERE oid = to_regclass('\"public\".\"wuhua_buildings\"')"
        );
        assert!(gist_index_sql(&def).contains("indexdef ILIKE '%' || 'geom' || '%'"));
        assert_eq!(
            count_all_sql(&def),
            "SELECT count(*) FROM \"public\".\"wuhua_buildings\""
        );
    }

    #[test]
    fn projected_tile_sql_shape() {
        let mut def = test_def();
        def.limit = Some(1000);
        let sql = tile_sql_projected(&def, "wuhua");
        // Geometry stays in the source SRID: no ST_Transform anywhere.
        assert!(!sql.contains("ST_Transform"));
        assert!(sql.contains("ST_MakeEnvelope($1, $2, $3, $4, 4543)"));
        assert!(sql.contains("ST_AsMVT(tile, 'wuhua', 4096, 'geom')"));
        assert!(sql.contains("ST_CurveToLine(\"geom\"::geometry)"));
        assert!(sql.contains("LIMIT 1000"));
    }

    #[test]
    fn projected_tile_sql_no_properties() {
        let mut def = test_def();
        def.properties.clear();
        def.id_column = None;
        let sql = tile_sql_projected(&def, "b");
        assert!(!sql.contains("NULL AS id")); // tile path needs no id alias
        assert!(sql.contains("\"public\".\"wuhua_buildings\""));
        // No limit configured -> no LIMIT clause.
        assert!(!sql.contains("LIMIT"));
    }

    #[test]
    fn bounds_src_sql_shape() {
        let sql = bounds_sql_src(&test_def());
        assert_eq!(
            sql,
            "SELECT ST_Extent(\"geom\")::text FROM \"public\".\"wuhua_buildings\""
        );
    }

    // query_tile_env needs a live pool; exercised end-to-end by the probe.
    #[allow(dead_code)]
    fn _uses_pool(p: &PgPool) {
        let _ = p;
    }
}
