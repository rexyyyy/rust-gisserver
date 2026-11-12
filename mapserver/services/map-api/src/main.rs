//! map-api CLI: `serve` (the HTTP server), plus fixture tooling:
//! `cog-create` (gradient sample COG) and `pmtiles-pack` (pack a tile
//! directory into a `.pmtiles` archive).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::signal;

use clap::{Parser, Subcommand};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "map-api")]
#[command(about = "rust-gisserver: WMS / WMTS / TMS / WFS / OGC API-Features / OGC API-Tiles / MVT / PMTiles / MBTiles / COG tile services")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the HTTP map server.
    Serve(ServeArgs),
    /// Import the XML catalog into the shared DB (`sys_published_layer`).
    CatalogImport(CatalogImportArgs),
    /// Generate a gradient sample COG (fixtures).
    CogCreate(CogCreateArgs),
    /// Pack a directory of `z/x/y.{pbf,png}` tiles into a PMTiles archive.
    PmtilesPack(PmtilesPackArgs),
}

#[derive(clap::Args, Debug)]
struct CatalogImportArgs {
    /// Path to the XML config (`config/mapserver.xml`).
    #[arg(short, long, default_value = "config/mapserver.xml")]
    config: PathBuf,

    /// Target DB DSN. Defaults to `catalog.dsn` / `admin.dsn` from the config.
    #[arg(long)]
    dsn: Option<String>,

    /// Workspace owning the imported services (created if absent).
    #[arg(long, default_value = "mapserver")]
    workspace: String,

    /// Dry run: scan and report, do not write to the DB.
    #[arg(long)]
    dry_run: bool,

    /// Log level: trace/debug/info/warn/error.
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[derive(clap::Args, Debug)]
struct ServeArgs {
    /// Path to the XML config (`config/mapserver.xml`).
    #[arg(short, long, default_value = "config/mapserver.xml")]
    config: PathBuf,

    /// Override the listen address (`host:port`).
    #[arg(short, long)]
    listen: Option<SocketAddr>,

    /// Log level: trace/debug/info/warn/error.
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[derive(clap::Args, Debug)]
struct CogCreateArgs {
    #[arg(long, default_value_t = 1024)]
    width: u32,
    #[arg(long, default_value_t = 512)]
    height: u32,
    #[arg(long, default_value_t = 128)]
    tile: u32,
    /// West edge (degrees).
    #[arg(long, default_value_t = 100.0)]
    minx: f64,
    /// North edge (degrees).
    #[arg(long, default_value_t = 40.0)]
    maxy: f64,
    /// East edge (degrees).
    #[arg(long, default_value_t = 108.0)]
    maxx: f64,
    /// South edge (degrees).
    #[arg(long, default_value_t = 36.0)]
    miny: f64,
    /// Output path.
    #[arg(short, long, default_value = "fixtures/sample.tif")]
    output: PathBuf,
}

#[derive(clap::Args, Debug)]
struct PmtilesPackArgs {
    /// Directory containing `z/x/y.{pbf|png}` tiles.
    #[arg(long)]
    input: PathBuf,
    /// Output archive path.
    #[arg(short, long)]
    output: PathBuf,
    /// Tile type: mvt or png.
    #[arg(long, default_value = "mvt")]
    tile_type: String,
    /// Tile compression: none or gzip.
    #[arg(long, default_value = "none")]
    compression: String,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve(args) => run_serve(&args),
        Cmd::CatalogImport(args) => run_catalog_import(&args),
        Cmd::CogCreate(args) => run_cog_create(&args),
        Cmd::PmtilesPack(args) => run_pmtiles_pack(&args),
    }
}

fn init_logging(level: &str) {
    let lvl = match level.to_ascii_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };
    let sub = FmtSubscriber::builder().with_max_level(lvl).finish();
    let _ = tracing::subscriber::set_global_default(sub);
}

fn run_serve(args: &ServeArgs) -> anyhow::Result<()> {
    init_logging(&args.log_level);
    let text = std::fs::read_to_string(&args.config)
        .map_err(|e| anyhow::anyhow!("read config {}: {e}", args.config.display()))?;
    let config_value = map_api::xmlcfg::parse_to_value(&text)
        .map_err(|e| anyhow::anyhow!("parse config {}: {e}", args.config.display()))?;
    let mut config: map_api::config::ServerConfig = serde_json::from_value(config_value)
        .map_err(|e| anyhow::anyhow!("parse config {}: {e}", args.config.display()))?;

    // `.tpk`（ZIP 缓存包）解压落点：必须在 collect_layers() 触发 arcgis 网格
    // 注入（内部会 ArcgisCache::open→CacheDocs::load→tpk::ensure_dir）之前设好，
    // 否则退回系统临时目录。进程级一次注入，后续重读/刷新沿用。
    arcgis_cache::tpk::set_cache_root(config.tpk_cache_root());

    // Layer catalog: merge legacy inline `layers:` with one-file-per-layer
    // XML documents under `<catalog.root>/<type>/` (cog/vector/pmtiles/mbtiles).
    // Layer files may also define custom grids via `custom_grids:` — merged
    // into `config.grids` (first definition wins).
    //
    // `catalog.backend = "db"`: 图层定义不再来自磁盘目录，而来自共享库
    // `sys_published_layer.service_doc`（manager 为唯一写入方）。此处的
    // collect_layers() 会返回内联层，随后被库内容整体替换。
    let (layers, extra_grids, warnings) = config.collect_layers();
    for w in &warnings {
        tracing::warn!("{w}");
    }
    config.grids.extend(extra_grids);
    config.layers = layers;

    let addr = args
        .listen
        .unwrap_or_else(|| format!("{}:{}", config.server.host, config.server.port).parse().unwrap());
    info!(config = %args.config.display(), address = %addr, "starting map-api");

    // PostGIS pools and the foyer output cache (memory + optional disk tier)
    // must be built inside a tokio context. They are built on the SAME runtime
    // that serves: foyer's hybrid-cache storage binds its background tasks to
    // the runtime that built it — building on a temporary runtime and then
    // dropping it stranded those tasks and made every cold `output.get` hang
    // forever when `server.disk_cache` was configured.
    let runtime = tokio::runtime::Runtime::new()?;
    let state = {
        let mut cfg = config.clone();
        let config_path = args.config.clone();
        runtime.block_on(async move {
            // DB 目录后端：从共享库载入图层定义（唯一真理源），磁盘目录不再扫描。
            let catalog_pool = if cfg.catalog.backend == map_api::config::CatalogBackend::Db {
                let dsn = cfg
                    .catalog
                    .dsn
                    .clone()
                    .or_else(|| cfg.admin.dsn.clone())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "catalog.backend=db 需要配置 catalog.dsn（或 admin.dsn）"
                        )
                    })?;
                // 不做 PostGIS 探针：目录库只读普通表。
                let pool = Arc::new(
                    pg_source::PgPool::new_plain(&dsn, 2)
                        .await
                        .map_err(|e| anyhow::anyhow!("目录库连接失败：{e}"))?,
                );
                let (db_layers, db_grids, db_warnings) =
                    map_api::db_catalog::load_catalog(&pool).await?;
                for w in &db_warnings {
                    tracing::warn!("{w}");
                }
                tracing::info!(
                    layers = db_layers.len(),
                    grids = db_grids.len(),
                    "catalog loaded from database (sys_published_layer.service_doc)"
                );
                cfg.grids.extend(db_grids);
                cfg.layers = db_layers;
                Some(pool)
            } else {
                None
            };
            let reg = map_api::state::build_pg_registry(&cfg).await?;
            // Service registry: PG (admin.dsn) or file sidecar fallback.
            let registry = map_api::admin::registry::Registry::open(&cfg).await;
            let state = map_api::AppState::new_managed(cfg, reg, registry, catalog_pool, config_path).await?;
            // Apply persisted management states (stop/pause/delete) + sync scan.
            state.refresh_catalog(false).await?;
            Ok::<map_api::AppState, anyhow::Error>(state)
        })?
    };
    let state = Arc::new(state);
    info!(
        cache_mib = state.config.server.cache_mib,
        cache_ttl_secs = state.config.server.cache_ttl_secs,
        cache_stats_secs = state.config.server.cache_stats_secs,
        disk_cache = state.config.server.disk_cache.is_some(),
        "cache configuration (foyer; refresh via TTL, POST /api/cache/purge, periodic stats)"
    );
    for l in state.layers.read().unwrap_or_else(|e| e.into_inner()).iter() {
        let kind = match &l.def.source {
            wms_common::layer::SourceKind::Cog { path } => format!("cog({path})"),
            wms_common::layer::SourceKind::Vector { path } => format!("vector({path})"),
            wms_common::layer::SourceKind::Pmtiles { path } => format!("pmtiles({path})"),
            wms_common::layer::SourceKind::Mbtiles { path } => format!("mbtiles({path})"),
            wms_common::layer::SourceKind::Arcgis { path } => format!("arcgis({path})"),
            wms_common::layer::SourceKind::Postgis(pg) => {
                format!("postgis({}/{})", pg.schema, pg.table)
            }
        };
        info!(layer = l.name(), source = %kind, grids = ?l.def.grids, "layer loaded");
    }

    let app = map_api::handlers::router(state.clone());
    runtime.block_on(async move {
        // Usage metering flush task (PG hour buckets; no-op in files mode).
        // Must be spawned inside the runtime (tokio timers need a reactor).
        map_api::admin::usage::spawn_flush(state.clone());
        // rust-gisserver 生成的 API Key 从共享库同步（15s），供管理面/数据面校验。
        map_api::apikeys::spawn_refresh(state.config.admin.dsn.clone(), state.api_keys.clone());
        // Periodic catalog re-scan (`server.catalog_scan_secs`, 0 = off).
        // Cluster convergence: re-read the (shared) XML catalog files so a
        // layer changed through ANOTHER node's admin API starts serving here
        // without a manual reload. Heavier than reread=false — it re-opens
        // data sources and purges caches — so keep it in tens of seconds.
        let scan_every = state.config.server.catalog_scan_secs;
        if scan_every > 0 {
            let scan_state = state.clone();
            tracing::info!(every_secs = scan_every, "periodic catalog rescan enabled");
            tokio::spawn(async move {
                let mut tick =
                    tokio::time::interval(std::time::Duration::from_secs(scan_every));
                tick.tick().await; // the first tick completes immediately
                loop {
                    tick.tick().await;
                    if let Err(e) = scan_state.refresh_catalog(true).await {
                        tracing::warn!("periodic catalog rescan failed: {e}");
                    }
                }
            });
        }
        // Periodic cache housekeeping: one stats log line per interval plus
        // forced moka maintenance, so TTL-expired entries are evicted even
        // when a cache receives no traffic. 0 disables the task.
        let stats_state = state.clone();
        let stats_every = stats_state.config.server.cache_stats_secs;
        if stats_every > 0 {
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(stats_every));
                tick.tick().await; // the first tick completes immediately
                loop {
                    tick.tick().await;
                    stats_state.cache_tick();
                }
            });
        }
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!("listening on http://{addr}");
        // with_connect_info feeds the audit middleware the peer address
        // (client IP column of the access log).
        // 优雅停机：收到 SIGINT/SIGTERM 后停止接收新连接、等待在途瓦片
        // 请求完成再退出，避免发布/重启掐断客户端已发出的请求。
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;
        Ok(())
    })
}

/// 停机信号：Ctrl-C 跨平台；SIGTERM 仅 Unix（Windows 下永不触发）。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            let _ = sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("收到 Ctrl-C，开始优雅停机"),
        _ = terminate => info!("收到 SIGTERM，开始优雅停机"),
    }
}

fn run_catalog_import(args: &CatalogImportArgs) -> anyhow::Result<()> {
    init_logging(&args.log_level);
    let text = std::fs::read_to_string(&args.config)
        .map_err(|e| anyhow::anyhow!("read config {}: {e}", args.config.display()))?;
    let config_value = map_api::xmlcfg::parse_to_value(&text)
        .map_err(|e| anyhow::anyhow!("parse config {}: {e}", args.config.display()))?;
    let config: map_api::config::ServerConfig = serde_json::from_value(config_value)
        .map_err(|e| anyhow::anyhow!("parse config {}: {e}", args.config.display()))?;

    // DSN 优先级：命令行 > catalog.dsn > admin.dsn。
    let dsn = args
        .dsn
        .clone()
        .or_else(|| config.catalog.dsn.clone())
        .or_else(|| config.admin.dsn.clone())
        .ok_or_else(|| anyhow::anyhow!("需要 --dsn，或在配置里设置 catalog.dsn / admin.dsn"))?;

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let (importer, warnings) =
            map_api::catalog_import::Importer::open(&dsn, &args.workspace, args.dry_run).await?;
        for w in &warnings {
            tracing::warn!("{w}");
        }
        let report = importer.import(&config).await?;
        println!(
            "scanned={} inserted={} updated={} skipped={} workspace_id={}{}",
            report.scanned,
            report.inserted,
            report.updated,
            report.skipped.len(),
            report.workspace_id,
            if report.dry_run { " (dry-run: 未写库)" } else { "" }
        );
        for s in &report.skipped {
            println!("  skipped: {s}");
        }
        Ok::<(), anyhow::Error>(())
    })
}

fn run_cog_create(args: &CogCreateArgs) -> anyhow::Result<()> {
    if args.width == 0 || args.height == 0 || args.tile == 0 {
        anyhow::bail!("width/height/tile must be > 0");
    }
    let bytes = cog::write::gradient_cog(
        args.width,
        args.height,
        args.tile,
        args.minx,
        args.maxy,
        args.maxx,
        args.miny,
    );
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args.output, &bytes)?;
    println!(
        "wrote {} ({}x{}, tile {}x{}, ext [{}, {}, {}, {}])",
        args.output.display(),
        args.width,
        args.height,
        args.tile,
        args.tile,
        args.minx,
        args.miny,
        args.maxx,
        args.maxy
    );
    Ok(())
}

fn run_pmtiles_pack(args: &PmtilesPackArgs) -> anyhow::Result<()> {
    if !args.input.is_dir() {
        anyhow::bail!("input {} is not a directory", args.input.display());
    }
    let tile_type = match args.tile_type.as_str() {
        "mvt" | "pbf" => pmtiles::tile_type::MVT,
        "png" => pmtiles::tile_type::PNG,
        other => anyhow::bail!("unknown tile type '{other}' (use mvt|png)"),
    };
    let compression = match args.compression.as_str() {
        "none" => pmtiles::compression::NONE,
        "gzip" | "gz" => pmtiles::compression::GZIP,
        other => anyhow::bail!("unknown compression '{other}' (use none|gzip)"),
    };

    // Walk z/x/y.ext
    let mut tiles: Vec<((u32, u32, u32), Vec<u8>)> = Vec::new();
    for zdir in std::fs::read_dir(&args.input)? {
        let zdir = zdir?;
        let z: u32 = zdir.file_name().to_string_lossy().parse()?;
        if !zdir.path().is_dir() {
            continue;
        }
        for xdir in std::fs::read_dir(zdir.path())? {
            let xdir = xdir?;
            let x: u32 = xdir.file_name().to_string_lossy().parse()?;
            if !xdir.path().is_dir() {
                continue;
            }
            for yfile in std::fs::read_dir(xdir.path())? {
                let yfile = yfile?;
                let fname = yfile.file_name().to_string_lossy().to_string();
                let stem = fname.split('.').next().unwrap_or("");
                let y: u32 = match stem.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let bytes = std::fs::read(yfile.path())?;
                tiles.push(((z, x, y), bytes));
            }
        }
    }
    if tiles.is_empty() {
        anyhow::bail!("no tiles found under {}", args.input.display());
    }
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    pmtiles::pack::pack_tiles(&args.output, &tiles, tile_type, compression, "map-api")
        .map_err(|e| anyhow::anyhow!(e))?;
    println!("packed {} tiles -> {}", tiles.len(), args.output.display());
    Ok(())
}
