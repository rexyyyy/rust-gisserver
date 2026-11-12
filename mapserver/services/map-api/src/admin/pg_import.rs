//! `POST /admin/pg-import-shp` — 把 Shapefile 导入 PostGIS（manager「导入 SHP」调用）。
//!
//! 依赖宿主机 PostGIS 客户端工具 `shp2pgsql`（PostgreSQL / PostGIS 安装目录 bin 下），
//! 定位顺序：环境变量 `SHPPGSQL_BIN` → PATH → 常见安装目录。
//! 采用 `-D`（dump）输出，边读子进程 stdout 边用 COPY 写入 PostGIS，避免把整份 SQL
//! 拼进内存；多语句在同一隐式事务内执行，失败整体回滚。
//!
//! 坐标系：优先取 `.prj` 末尾的 `AUTHORITY["EPSG","xxxx"]`，其次按 WKT 名称猜
//! CGCS2000 / WGS84 地理坐标，识别不到时按 4326 导入并在响应里标记 `sridDetected=false`。

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::SinkExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::admin::http::{authorize, json as json_resp};
use crate::state::AppState;

/// `POST /admin/pg-import-shp`
/// body `{dsn, schema, shpPath, table, srid?}`。
pub async fn pg_import_shp(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    run_import(body, None).await
}

/// `POST /admin/pg-import-shp/stream` — 同一次导入，但以 SSE 推送进度
/// （`stage=start/copy/done/error`，`copy` 带已写入行数与耗时）。
/// 大 Shapefile 走这里，客户端能持续收到心跳而不是干等一个长请求。
pub async fn pg_import_shp_stream(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Response {
    if let Err(r) = authorize(&state, &headers) {
        return r;
    }
    let (tx, rx) = mpsc::channel::<Value>(64);
    tokio::spawn(async move {
        let resp = run_import(body, Some(tx.clone())).await;
        let ok = resp.status().is_success();
        let bytes = axum::body::to_bytes(resp.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap_or_default();
        let mut v: Value =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"error": "上游返回无法解析"}));
        if let Value::Object(m) = &mut v {
            m.insert("stage".into(), json!(if ok { "done" } else { "error" }));
        }
        let _ = tx.send(v).await;
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|v| (Ok::<_, Infallible>(Event::default().data(v.to_string())), rx))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// 导入主流程。`progress` 为 `Some` 时按阶段 / 行数上报（SSE 用），`None` 时
/// 保持原有的同步 JSON 行为，两者共用同一份 SQL 落库逻辑。
async fn run_import(body: Value, progress: Option<mpsc::Sender<Value>>) -> Response {
    let started = Instant::now();

    let dsn = match body.get("dsn").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return bad("缺少 dsn 字段（postgres://user:pass@host:port/db）"),
    };
    // 表名统一小写（与 PostGIS 非引号标识符落库行为一致），避免列名/表名大小写歧义。
    let schema = body
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or("public")
        .trim()
        .to_lowercase();
    let table = body
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let shp_path = body
        .get("shpPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if !is_ident(&schema) {
        return bad("schema 仅允许字母、数字与下划线，且不能以数字开头");
    }
    if !is_ident(&table) {
        return bad("数据图层名仅允许字母、数字与下划线，且不能以数字开头");
    }
    if shp_path.is_empty() {
        return bad("缺少 shpPath 字段");
    }
    let shp = PathBuf::from(&shp_path);
    let is_shp = shp
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("shp"))
        .unwrap_or(false);
    if !is_shp {
        return bad("仅支持 .shp 矢量文件");
    }
    if !shp.is_file() {
        return bad(&format!("文件不存在：{shp_path}"));
    }

    // DBF 属性表编码：shp2pgsql 默认按 UTF-8 解析 DBF，遇到 GBK/BIG5 数据会直接
    // 报 "Unable to convert field name to UTF-8" 而失败（实测），且它**不读 .cpg**。
    // 故此处检测后显式传 `-W <encoding>`。
    //
    // body 可选覆盖：
    //   encoding —— 直接指定编码名（GBK/BIG5/EUC-KR/SHIFT_JIS/UTF-8…），跳过自动判定；
    //   locale   —— 指定语言区域（zh_CN/zh_TW/ko_KR/ja_JP/en_US），改变自动判定的候选集。
    // 都不给时按进程环境判定（默认简体中文地区）。
    let explicit_encoding = body
        .get("encoding")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let locale = body
        .get("locale")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let region = locale.as_deref().map(gisserver::encoding::Region::from_locale);
    let dbf_encoding = detect_dbf_encoding(&shp, region, explicit_encoding.as_deref());

    let (detected_srid, srid_detected) = detect_srid(&shp);
    let srid = body
        .get("srid")
        .and_then(Value::as_i64)
        .map(|v| v as i32)
        .filter(|v| *v > 0)
        .unwrap_or(detected_srid);

    emit(
        &progress,
        json!({
            "stage": "start",
            "table": table,
            "srid": srid,
            "sridDetected": srid_detected,
            "dbfEncoding": dbf_encoding,
        }),
    )
    .await;

    let bin = match locate_shp2pgsql() {
        Some(b) => b,
        None => {
            return json_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "未找到 shp2pgsql（PostGIS 客户端工具）：请安装 PostGIS 客户端，或用环境变量 SHPPGSQL_BIN 指定可执行文件路径" }),
            )
        }
    };

    let pool = match gisserver::pg_source::PgPool::new(&dsn, 1).await {
        Ok(p) => p,
        Err(e) => {
            return json_resp(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("连接 PostGIS 失败：{e}") }),
            )
        }
    };
    let client = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return json_resp(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("获取 PostGIS 连接失败：{e}") }),
            )
        }
    };

    match client
        .query_opt(
            "SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2",
            &[&schema, &table],
        )
        .await
    {
        Ok(Some(_)) => {
            return json_resp(
                StatusCode::CONFLICT,
                json!({ "error": format!("表 {schema}.{table} 已存在，请换一个数据图层名") }),
            )
        }
        Ok(None) => {}
        Err(e) => {
            return json_resp(
                StatusCode::BAD_REQUEST,
                json!({ "error": format!("检查目标表失败：{e}") }),
            )
        }
    }

    let mut child_cmd = Command::new(&bin);
    child_cmd
        .arg("-s")
        .arg(srid.to_string())
        .arg("-I")
        .arg("-D");
    // DBF 编码显式指定（shp2pgsql 默认 UTF-8，遇 GBK/BIG5 直接报错失败）
    if let Some(enc) = dbf_encoding {
        child_cmd.arg("-W").arg(enc);
    }
    let mut child = match child_cmd
        .arg(&shp)
        .arg(format!("{schema}.{table}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return json_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("启动 shp2pgsql 失败（{}）：{e}", bin.display()) }),
            )
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return bad("无法读取 shp2pgsql 输出"),
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => return bad("无法读取 shp2pgsql 错误输出"),
    };
    let err_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut rd = BufReader::new(stderr);
        let _ = rd.read_to_string(&mut buf).await;
        buf
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut pending = String::new();
    let mut in_copy = false;
    let mut rows: u64 = 0;
    // CopyInSink 是 !Unpin，必须装箱后再用 SinkExt。
    let mut sink: Option<std::pin::Pin<Box<tokio_postgres::CopyInSink<Bytes>>>> = None;
    let mut failure: Option<String> = None;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if in_copy {
                    if line == "\\." {
                        if let Some(mut s) = sink.take() {
                            if let Err(e) = s.as_mut().finish().await {
                                failure = Some(format!("写入 COPY 数据失败：{e}"));
                                break;
                            }
                        }
                        in_copy = false;
                    } else if let Some(s) = sink.as_mut() {
                        if let Err(e) = s.send(Bytes::from(format!("{line}\n"))).await {
                            failure = Some(format!("写入 COPY 数据失败：{e}"));
                            break;
                        }
                        rows += 1;
                        // 每 1000 行上报一次：大 Shapefile 导入期间前端持续收到进度
                        if rows % 1000 == 0 {
                            emit(
                                &progress,
                                json!({
                                    "stage": "copy",
                                    "rows": rows,
                                    "elapsedMs": started.elapsed().as_millis() as u64,
                                }),
                            )
                            .await;
                        }
                    }
                } else if line.starts_with("COPY ") {
                    // COPY 之前的 DDL（CREATE TABLE 等）先执行
                    if !pending.trim().is_empty() {
                        if let Err(e) = client.batch_execute(&pending).await {
                            failure = Some(format!("执行建表语句失败：{e}"));
                            break;
                        }
                        pending.clear();
                    }
                    // shp2pgsql -D 输出的 COPY 头已含 "FROM stdin"，先去掉再交给 copy_in，
                    // 否则会拼出 "COPY ... FROM stdin FROM stdin"。
                    let head = line.trim().trim_end_matches(';').trim();
                    let base = if head.to_ascii_uppercase().ends_with("FROM STDIN") {
                        head[..head.len() - "FROM STDIN".len()].trim_end()
                    } else {
                        head
                    };
                    let stmt = format!("{base} FROM stdin");
                    match client.copy_in(&stmt).await {
                        Ok(s) => {
                            sink = Some(Box::pin(s));
                            in_copy = true;
                        }
                        Err(e) => {
                            failure = Some(format!("初始化 COPY 失败：{e}"));
                            break;
                        }
                    }
                } else {
                    pending.push_str(&line);
                    pending.push('\n');
                }
            }
            Ok(None) => break,
            Err(e) => {
                failure = Some(format!("读取 shp2pgsql 输出失败：{e}"));
                break;
            }
        }
    }

    // 未正常收到 \. 就中断时，丢弃 COPY 句柄（事务回滚）
    if sink.is_some() {
        drop(sink.take());
    }
    let status = child.wait().await;
    let stderr_text = err_task.await.unwrap_or_default();

    if let Some(msg) = failure {
        return import_failed(msg, &stderr_text);
    }
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => return import_failed(format!("shp2pgsql 退出码 {s}"), &stderr_text),
        Err(e) => return import_failed(format!("等待 shp2pgsql 结束失败：{e}"), &stderr_text),
    }

    if !pending.trim().is_empty() {
        if let Err(e) = client.batch_execute(&pending).await {
            return import_failed(format!("执行收尾语句失败：{e}"), &stderr_text);
        }
    }

    // 以 geometry_columns 的实际结果为准（shp2pgsql 已按 -s 写入 SRID）
    let geom_col = client
        .query_opt(
            "SELECT f_geometry_column, srid FROM geometry_columns \
             WHERE f_table_schema = $1 AND f_table_name = $2 LIMIT 1",
            &[&schema, &table],
        )
        .await
        .ok()
        .flatten()
        .map(|r| {
            let col: String = r.get(0);
            let srid: i32 = r.get(1);
            (col, srid)
        });
    let features: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM \"{schema}\".\"{table}\""),
            &[],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(0);

    json_resp(
        StatusCode::OK,
        json!({
            "ok": true,
            "schema": schema,
            "table": table,
            "srid": geom_col.as_ref().map(|(_, s)| *s).unwrap_or(srid),
            "sridDetected": srid_detected,
            "dbfEncoding": dbf_encoding,
            "geometryColumn": geom_col.map(|(c, _)| c),
            "features": features,
            "rows": rows,
            "elapsedMs": started.elapsed().as_millis() as u64,
        }),
    )
}

fn bad(msg: &str) -> Response {
    json_resp(StatusCode::BAD_REQUEST, json!({ "error": msg }))
}

/// 检测 shp 的 DBF 属性表编码，返回 `shp2pgsql -W` 认的编码名。
///
/// 为什么需要：`shp2pgsql` 默认按 UTF-8 解析 DBF，遇到 GBK/BIG5 数据会直接
/// 报 `Unable to convert field name to UTF-8` 并以非 0 退出（实测）；
/// 且它**不读 `.cpg` 旁文件**（实测：有 .cpg 仍失败），所以必须显式传 `-W`。
///
/// 判定链：显式指定 → `.cpg` 声明（生产方权威）→ DBF 字节采样打分（区域驱动）。
/// 都判不出时返回 None，走 shp2pgsql 默认行为（ASCII/UTF-8 数据无影响）。
///
/// `region`：本次导入使用的语言区域（None = 取进程环境，见 `Region::from_env`）。
/// `explicit`：调用方直接指定的编码名（跳过自动判定）。
fn detect_dbf_encoding(
    shp: &Path,
    region: Option<gisserver::encoding::Region>,
    explicit: Option<&str>,
) -> Option<&'static str> {
    // 调用方显式指定优先（转成 iconv 名）
    if let Some(name) = explicit {
        return gisserver::encoding::dbf::cpg_to_iconv_name(name);
    }
    let hint = match region {
        Some(r) => gisserver::encoding::dbf::detect_shp_for(shp, r),
        None => gisserver::encoding::dbf::detect_shp(shp),
    };
    hint.ok().and_then(|h| h.suggested_shp2pgsql)
}

/// 进度上报：非流式调用（`progress=None`）时是空操作，不影响同步接口。
async fn emit(tx: &Option<mpsc::Sender<Value>>, v: Value) {
    if let Some(t) = tx {
        let _ = t.send(v).await;
    }
}

/// 导入失败：把 shp2pgsql 的 stderr 尾部一起返回，便于定位编码/几何问题。
fn import_failed(msg: String, stderr_text: &str) -> Response {
    let tail: String = stderr_text
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    let detail = if tail.trim().is_empty() {
        msg
    } else {
        format!("{msg}：{tail}")
    };
    json_resp(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": detail }),
    )
}

/// PostGIS 标识符：字母/下划线开头，仅字母数字下划线，长度 ≤ 63。
fn is_ident(s: &str) -> bool {
    if s.is_empty() || s.len() > 63 {
        return false;
    }
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 从 `.prj` 推断 EPSG：优先 AUTHORITY 尾标，其次按 WKT 名称猜。
/// 返回 `(srid, 是否确定)`。
fn detect_srid(shp: &Path) -> (i32, bool) {
    let mut txt = std::fs::read_to_string(shp.with_extension("prj")).ok();
    if txt.is_none() {
        // 后缀可能是大写 .PRJ
        txt = std::fs::read_to_string(shp.with_extension("PRJ")).ok();
    }
    let txt = match txt {
        Some(t) => t,
        None => return (4326, false),
    };
    let needle = "AUTHORITY[\"EPSG\",\"";
    let mut last: Option<i32> = None;
    let mut rest = txt.as_str();
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = digits.parse::<i32>() {
            last = Some(v);
        }
        rest = &after[digits.len().min(after.len())..];
    }
    if let Some(v) = last {
        return (v, true);
    }
    let up = txt.to_ascii_uppercase();
    let projected = up.contains("PROJECTION[");
    let is_cgcs2000 = up.contains("CGCS2000") || up.contains("CHINA_2000") || up.contains("CHINA GEODETIC");
    if is_cgcs2000 {
        if !projected {
            return (4490, true);
        }
        // ESRI WKT 没有 AUTHORITY，只能按「3 度带 / 6 度带 + 带号或中央经线」换算 EPSG。
        if let Some(v) = cgcs2000_gk_srid(&up) {
            return (v, true);
        }
    }
    if !projected && (up.contains("WGS_1984") || up.contains("WGS 84")) {
        return (4326, true);
    }
    (4326, false)
}

/// CGCS2000 高斯克吕格投影 WKT → EPSG。
/// * 6 度带带号 13..=23 → 4478 + 带号；6 度带中央经线 75..=135 → 4502 + (CM-75)/6
/// * 3 度带带号 25..=45 → 4488 + 带号；3 度带中央经线 75..=135 → 4534 + (CM-75)/3
fn cgcs2000_gk_srid(up: &str) -> Option<i32> {
    let three = up.contains("3_DEGREE") || up.contains("3 DEGREE") || up.contains("3-DEGREE");
    if let Some(zone) = num_after(up, "ZONE_") {
        let epsg = if three { 4488 + zone } else { 4478 + zone };
        let ok = if three {
            (25..=45).contains(&zone)
        } else {
            (13..=23).contains(&zone)
        };
        if ok {
            return Some(epsg);
        }
    }
    if let Some(cm) = num_after(up, "CM_") {
        if (75..=135).contains(&cm) && (cm - 75) % if three { 3 } else { 6 } == 0 {
            let epsg = if three {
                4534 + (cm - 75) / 3
            } else {
                4502 + (cm - 75) / 6
            };
            return Some(epsg);
        }
    }
    None
}

/// 取 `key` 之后紧跟的整数字面量（如 `ZONE_34` → 34）。
fn num_after(up: &str, key: &str) -> Option<i32> {
    let i = up.find(key)?;
    let digits: String = up[i + key.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 定位 shp2pgsql：环境变量 → PATH → 常见安装目录（Windows/Linux）。
fn locate_shp2pgsql() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SHPPGSQL_BIN") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let exe = if cfg!(windows) {
        "shp2pgsql.exe"
    } else {
        "shp2pgsql"
    };
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    #[cfg(windows)]
    {
        for base in [
            "C:\\Program Files\\PostgreSQL",
            "C:\\Program Files (x86)\\PostgreSQL",
        ] {
            if let Ok(rd) = std::fs::read_dir(base) {
                let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                dirs.sort();
                dirs.reverse();
                for d in dirs {
                    let p = d.join("bin").join(exe);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    #[cfg(unix)]
    {
        for d in [
            "/usr/bin",
            "/usr/local/bin",
            "/usr/lib/postgresql",
        ] {
            let p = Path::new(d).join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}
