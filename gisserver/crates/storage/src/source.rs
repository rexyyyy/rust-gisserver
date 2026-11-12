//! Positioned byte-range sources (ported from TerraServe's `RangeSource` seam).
//!
//! Every COG / PMTiles byte read flows through [`RangeSource`], so a source can
//! be a local file or a remote `http(s)://` object (including S3 presigned
//! URLs) without the readers knowing which. Implementations are positioned
//! (no shared cursor) and `Sync`, so multiple threads read distinct ranges
//! concurrently.

use std::fs::File;
use std::io::Read;
use std::sync::OnceLock;
use wms_common::MapServerError;

/// Dedicated rayon pool for **remote** (HTTP) range reads.
///
/// A synchronous `ureq` GET (30 s timeout) must never run on the global rayon
/// pool: local COG tile decode shares that pool, and one slow remote source
/// (or a hung peer) would occupy every worker and stall all local rendering.
/// `install` on this pool keeps remote reads confined to `REMOTE_THREADS`
/// workers; when already inside a rayon worker it runs inline instead of
/// nesting (no deadlock).
static REMOTE_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

const REMOTE_THREADS: usize = 8;

fn remote_pool() -> &'static rayon::ThreadPool {
    REMOTE_POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(REMOTE_THREADS)
            .thread_name(|i| format!("rms-remote-{i}"))
            .build()
            .expect("remote range-read pool")
    })
}

/// Positioned read that fills `buf` completely (or errors), **without moving
/// the shared file cursor**. Windows uses `seek_read`, Unix `read_at`; both
/// are atomic w.r.t. the fd, so any number of threads may share one open
/// `File` and read disjoint ranges concurrently.
///
/// This replaces the old `(&File).seek()` + `read_exact()` pair, which was a
/// data race on a shared fd: concurrent readers could interleave
/// seek-then-read and decode bytes from the wrong offset (intermittent
/// garbage tiles -- the hardest class of bug to reproduce).
#[cfg(windows)]
pub fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        match file.seek_read(&mut buf[done..], offset + done as u64)? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("short read at offset {}", offset + done as u64),
                ))
            }
            n => done += n,
        }
    }
    Ok(())
}

#[cfg(unix)]
pub fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        match file.read_at(&mut buf[done..], offset + done as u64)? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("short read at offset {}", offset + done as u64),
                ))
            }
            n => done += n,
        }
    }
    Ok(())
}

/// A positioned byte-range reader.
pub trait RangeSource: Send + Sync {
    fn read_range(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>>;
    /// Total length in bytes, when knowable. `None` for e.g. chunked HTTP.
    fn len(&self) -> Option<u64> {
        None
    }
    fn is_empty(&self) -> bool {
        self.len().map(|l| l == 0).unwrap_or(false)
    }
}

/// Reads byte ranges from a local file via positioned reads (seek per read),
/// so it is `Sync` and thread-safe.
pub struct LocalFileRangeSource {
    file: File,
    file_len: u64,
}

impl LocalFileRangeSource {
    pub fn open(path: &str) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        Ok(Self { file, file_len })
    }
}

impl RangeSource for LocalFileRangeSource {
    fn read_range(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        // Reads never fail at EOF: they return the available tail, so walkers
        // (e.g. a full-file stream) can advance until the next read errors.
        if offset >= self.file_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("offset {offset} beyond file length {}", self.file_len),
            ));
        }
        let end = (offset + len as u64).min(self.file_len);
        let n = (end - offset) as usize;
        let mut buf = vec![0u8; n];
        read_exact_at(&self.file, &mut buf, offset)?;
        Ok(buf)
    }

    fn len(&self) -> Option<u64> {
        Some(self.file_len)
    }
}

/// Reads byte ranges from an `http(s)://` URL using `Range: bytes=...` headers.
/// Works with COGs on plain HTTPS and on S3-compatible endpoints when the URL
/// is presigned. One TCP connection is reused for all ranges (ureq agent).
pub struct HttpRangeSource {
    url: String,
    agent: ureq::Agent,
    content_len: Option<u64>,
}

impl HttpRangeSource {
    pub fn new(url: &str) -> Result<HttpRangeSource, MapServerError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        // HEAD to learn the total length (best-effort; some servers reject HEAD).
        let len = match agent.head(url).call() {
            Ok(resp) => resp
                .header("Content-Length")
                .and_then(|v| v.parse::<u64>().ok()),
            Err(_) => None,
        };
        Ok(HttpRangeSource {
            url: url.to_string(),
            agent,
            content_len: len,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl RangeSource for HttpRangeSource {
    fn read_range(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        if let Some(total) = self.content_len {
            if offset >= total {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("offset {offset} beyond content length {total}"),
                ));
            }
        }
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "range overflow"))?;
        let end = self
            .content_len
            .map(|t| end.min(t))
            .unwrap_or(end);
        let range = if end <= offset {
            format!("bytes={offset}-")
        } else {
            format!("bytes={offset}-{}", end - 1)
        };
        // The blocking HTTP read runs on the dedicated remote pool so a slow
        // source cannot starve local COG decode on the global rayon pool.
        let url = self.url.clone();
        let agent = &self.agent;
        remote_pool().install(|| {
            let resp = agent
                .get(&url)
                .set("Range", &range)
                .call()
                .map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("GET range: {e}"),
                    )
                })?;
            if resp.status() != 206 && resp.status() != 200 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("range GET status {}", resp.status()),
                ));
            }
            let mut buf = Vec::with_capacity(len);
            resp.into_reader()
                .take(len as u64 + 1)
                .read_to_end(&mut buf)
                .map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::Other, format!("read body: {e}"))
                })?;
            Ok(buf)
        })
    }

    fn len(&self) -> Option<u64> {
        self.content_len
    }
}

/// Open a source from a URI: `http(s)://` -> HTTP range reads, else a local path.
pub fn source_from_uri(uri: &str) -> Result<Box<dyn RangeSource>, MapServerError> {
    source_from_uri_with_s3(uri, None)
}

/// [`source_from_uri`] with optional global S3 credentials: when `s3` is
/// `Some`, `s3://bucket/key` URIs open an authenticated [`S3RangeSource`]
/// (path-style, works with MinIO / RustFS / AWS / S3-compatible clouds).
/// Without credentials an `s3://` URI is a configuration error, not a
/// silent plain-HTTP downgrade.
pub fn source_from_uri_with_s3(
    uri: &str,
    s3: Option<&S3Config>,
) -> Result<Box<dyn RangeSource>, MapServerError> {
    let u = uri.trim();
    if let Some(rest) = u.strip_prefix("s3://") {
        let cfg = s3.ok_or_else(|| {
            MapServerError::from(format!(
                "s3 source '{u}' requires server.s3 credentials in the config"
            ))
        })?;
        let (bucket, key) = rest
            .split_once('/')
            .ok_or_else(|| MapServerError::from(format!("s3 source '{u}' must be s3://bucket/key")))?;
        Ok(Box::new(S3RangeSource::new(cfg, bucket, key)?))
    } else if u.starts_with("http://") || u.starts_with("https://") {
        Ok(Box::new(HttpRangeSource::new(u)?))
    } else {
        Ok(Box::new(LocalFileRangeSource::open(u)?))
    }
}

// ---------------------------------------------------------------- S3 (SigV4)

/// SHA-256 of the empty string (SigV4 payload hash for GET / HEAD).
const EMPTY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Global S3 credentials for `s3://bucket/key` layer sources.
///
/// Pure data — parsing/validation happens in the map-api config layer, the
/// signing math in [`S3RangeSource`]. Works with AWS S3, MinIO, RustFS and
/// S3-compatible clouds via **path-style** addressing
/// (`{endpoint}/{bucket}/{key}`): MinIO / RustFS default to path-style, so
/// this needs no virtual-host DNS setup on either side.
#[derive(Clone, Debug)]
pub struct S3Config {
    /// Full endpoint including scheme, e.g. `http://172.17.11.6:9000`.
    pub endpoint: String,
    /// SigV4 signing region. MinIO / RustFS: any value works (`auto` is
    /// conventional); AWS: the bucket's region (e.g. `cn-north-1`).
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

/// Authenticated byte-range reads from an S3-compatible object store.
///
/// Same seam as [`HttpRangeSource`] but with per-request AWS SigV4 signing,
/// so no presigned-URL generator is needed. One ureq agent (connection
/// reuse) per source; blocking calls run on the dedicated remote pool, exactly
/// like the HTTP implementation.
pub struct S3RangeSource {
    endpoint: String,
    host: String,
    bucket: String,
    /// Object key, raw (unencoded; encoded per request).
    key: String,
    region: String,
    access_key: String,
    secret_key: String,
    agent: ureq::Agent,
    content_len: Option<u64>,
}

impl S3RangeSource {
    pub fn new(cfg: &S3Config, bucket: &str, key: &str) -> Result<S3RangeSource, MapServerError> {
        let endpoint = cfg.endpoint.trim_end_matches('/').to_string();
        let host = endpoint
            .split_once("://")
            .map(|(_, rest)| rest.to_string())
            .ok_or_else(|| MapServerError::from(format!("s3 endpoint '{}' has no scheme", cfg.endpoint)))?;
        if host.is_empty() {
            return Err(MapServerError::from(format!("s3 endpoint '{}' has no host", cfg.endpoint)));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        // HEAD to learn the total length (best-effort, same as HttpRangeSource).
        let content_len = Self::request(
            &agent, &endpoint, &host, bucket, key, cfg, "HEAD", None,
        )
        .ok()
        .and_then(|resp| {
            resp.header("Content-Length")
                .and_then(|v| v.parse::<u64>().ok())
        });
        Ok(S3RangeSource {
            endpoint,
            host,
            bucket: bucket.to_string(),
            key: key.to_string(),
            region: cfg.region.clone(),
            access_key: cfg.access_key.clone(),
            secret_key: cfg.secret_key.clone(),
            agent,
            content_len,
        })
    }

    /// Signed GET (or HEAD when `method == "HEAD"`) against
    /// `{endpoint}/{bucket}/{key}`, path-style.
    fn request(
        agent: &ureq::Agent,
        endpoint: &str,
        host: &str,
        bucket: &str,
        key: &str,
        cfg: &S3Config,
        method: &str,
        range: Option<&str>,
    ) -> std::io::Result<ureq::Response> {
        let canon_uri = format!("/{}{}", bucket, encode_key_path(key));
        let url = format!("{endpoint}{canon_uri}");
        let (amzdate, datestamp) = amz_dates_now();
        let scope = format!("{datestamp}/{}/s3/aws4_request", cfg.region);
        let canonical = canonical_request(
            method,
            &canon_uri,
            host,
            &amzdate,
            &scope,
            &cfg.access_key,
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amzdate}\n{scope}\n{}",
            sha256_hex(canonical.as_bytes())
        );
        let sig = sigv4_signature(&cfg.secret_key, &datestamp, &cfg.region, &string_to_sign);
        let auth = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={sig}",
            cfg.access_key, scope
        );
        let req = match method {
            "HEAD" => agent.head(&url),
            _ => agent.get(&url),
        }
        .set("x-amz-date", &amzdate)
        .set("x-amz-content-sha256", EMPTY_SHA256)
        .set("Authorization", &auth);
        let req = match range {
            Some(r) => req.set("Range", r),
            None => req,
        };
        req.call().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("s3 {method} {url}: {e}"),
            )
        })
    }
}

impl RangeSource for S3RangeSource {
    fn read_range(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        if let Some(total) = self.content_len {
            if offset >= total {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("offset {offset} beyond object length {total}"),
                ));
            }
        }
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "range overflow"))?;
        let end = self.content_len.map(|t| end.min(t)).unwrap_or(end);
        let range = if end <= offset {
            format!("bytes={offset}-")
        } else {
            format!("bytes={offset}-{}", end - 1)
        };
        // Same remote-pool confinement as HttpRangeSource: never occupy the
        // global rayon workers with a blocking network call.
        let agent = &self.agent;
        let (endpoint, host, bucket, key) = (
            self.endpoint.clone(),
            self.host.clone(),
            self.bucket.clone(),
            self.key.clone(),
        );
        let cfg = S3Config {
            endpoint: endpoint.clone(),
            region: self.region.clone(),
            access_key: self.access_key.clone(),
            secret_key: self.secret_key.clone(),
        };
        remote_pool().install(move || {
            let resp = Self::request(
                agent, &endpoint, &host, &bucket, &key, &cfg, "GET", Some(&range),
            )?;
            let status = resp.status();
            if status != 206 && status != 200 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("s3 range GET status {status}"),
                ));
            }
            // 206 = range honored. 200 = server ignored Range (rare): the
            // body starts at 0, so reads past EOF simply come back short —
            // same tail semantics as the local file source.
            let mut buf = Vec::with_capacity(len);
            resp.into_reader()
                .take(len as u64 + 1)
                .read_to_end(&mut buf)
                .map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::Other, format!("read body: {e}"))
                })?;
            if status == 200 && offset > 0 {
                // Range ignored: slice our window out of the body (bounded by
                // `len`, which we already capped via `take`).
                let from = (offset as usize).min(buf.len());
                return Ok(buf.split_off(from.min(buf.len())));
            }
            Ok(buf)
        })
    }

    fn len(&self) -> Option<u64> {
        self.content_len
    }
}

// --- SigV4 primitives -------------------------------------------------------

/// SigV4 canonical request for our fixed header set
/// (`host;x-amz-content-sha256;x-amz-date`, empty query, empty payload).
fn canonical_request(
    method: &str,
    canon_uri: &str,
    host: &str,
    amzdate: &str,
    _scope: &str,
    _access_key: &str,
) -> String {
    format!(
        "{method}\n{canon_uri}\n\nhost:{host}\nx-amz-content-sha256:{EMPTY_SHA256}\nx-amz-date:{amzdate}\n\nhost;x-amz-content-sha256;x-amz-date\n{EMPTY_SHA256}"
    )
}

/// AWS SigV4 signing key derivation: `HMAC(HMAC(HMAC(HMAC("AWS4"+secret, date), region), "s3"), "aws4_request")`.
fn sigv4_signing_key(secret: &str, datestamp: &str, region: &str) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(format!("AWS4{secret}").as_bytes())
        .expect("hmac accepts any key length");
    mac.update(datestamp.as_bytes());
    let k_date = mac.finalize().into_bytes();
    let mut mac = HmacSha256::new_from_slice(&k_date).expect("hmac key");
    mac.update(region.as_bytes());
    let k_region = mac.finalize().into_bytes();
    let mut mac = HmacSha256::new_from_slice(&k_region).expect("hmac key");
    mac.update(b"s3");
    let k_service = mac.finalize().into_bytes();
    let mut mac = HmacSha256::new_from_slice(&k_service).expect("hmac key");
    mac.update(b"aws4_request");
    mac.finalize().into_bytes().to_vec()
}

/// SigV4 最终签名：`hex(HMAC_SHA256(signing_key, string_to_sign))`。
fn sigv4_signature(secret: &str, datestamp: &str, region: &str, string_to_sign: &str) -> String {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let key = sigv4_signing_key(secret, datestamp, region);
    let mut mac = HmacSha256::new_from_slice(&key).expect("hmac accepts any key length");
    mac.update(string_to_sign.as_bytes());
    hex_lower(&mac.finalize().into_bytes())
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex_lower(&h.finalize())
}

fn hex_lower(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// URI-encode one key path segment per SigV4 rules: unreserved characters
/// (`A-Za-z0-9-._~`) pass through, everything else becomes `%XX` (uppercase).
fn encode_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for &b in seg.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Encode an object key for the canonical URI, preserving `/` separators.
fn encode_key_path(key: &str) -> String {
    key.split('/').map(encode_segment).collect::<Vec<_>>().join("/")
}

/// Civil (Y-M-D) date from days since the Unix epoch (Howard Hinnant's
/// `civil_from_days`) — avoids a chrono/time dependency for one timestamp.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Current instant as `(amzdate, datestamp)`, e.g.
/// `("20231114T221320Z", "20231114")` — UTC, per SigV4.
fn amz_dates_now() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    amz_dates_from(secs)
}

/// [`amz_dates_now`] from a fixed Unix timestamp (unit-testable).
fn amz_dates_from(secs: i64) -> (String, String) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        format!(
            "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        ),
        format!("{y:04}{m:02}{d:02}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sha256_matches_aws_constant() {
        // The SigV4 payload hash for GET/HEAD is the SHA-256 of the empty
        // string; this constant is the AWS-published value.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(EMPTY_SHA256, sha256_hex(b""));
    }

    #[test]
    fn key_path_encoding_preserves_slashes() {
        assert_eq!(encode_key_path("tiles/z10/x1/y2.tif"), "tiles/z10/x1/y2.tif");
        assert_eq!(encode_key_path("a b/c+d.bin"), "a%20b/c%2Bd.bin");
        assert_eq!(encode_key_path("中文/tile.tif"), "%E4%B8%AD%E6%96%87/tile.tif");
        // Unreserved characters stay literal.
        assert_eq!(encode_segment("A-z_9.~"), "A-z_9.~");
    }

    #[test]
    fn amz_date_conversion_known_instants() {
        let (amz, date) = amz_dates_from(0);
        assert_eq!(amz, "19700101T000000Z");
        assert_eq!(date, "19700101");
        // 1700000000 == 2023-11-14T22:13:20Z (known instant).
        let (amz, date) = amz_dates_from(1_700_000_000);
        assert_eq!(amz, "20231114T221320Z");
        assert_eq!(date, "20231114");
    }

    #[test]
    fn canonical_request_shape() {
        let cr = canonical_request(
            "GET",
            "/bucket/path/to/tile.tif",
            "127.0.0.1:9000",
            "20231114T221320Z",
            "20231114/auto/s3/aws4_request",
            "AK",
        );
        let lines: Vec<&str> = cr.split('\n').collect();
        assert_eq!(lines[0], "GET");
        assert_eq!(lines[1], "/bucket/path/to/tile.tif");
        assert_eq!(lines[2], "", "canonical query is empty");
        assert_eq!(lines[3], "host:127.0.0.1:9000");
        assert_eq!(lines[4], format!("x-amz-content-sha256:{EMPTY_SHA256}"));
        assert_eq!(lines[5], "x-amz-date:20231114T221320Z");
        assert_eq!(lines[6], "", "header block ends with a blank line");
        assert_eq!(lines[7], "host;x-amz-content-sha256;x-amz-date");
        assert_eq!(lines[8], EMPTY_SHA256);
    }

    #[test]
    fn signing_key_is_deterministic() {
        // Determinism + 32-byte width; signature correctness is pinned by
        // `sigv4_signature_known_answer` and validated end-to-end against a
        // live MinIO/RustFS endpoint.
        let k1 = sigv4_signing_key("secret", "20231114", "auto");
        let k2 = sigv4_signing_key("secret", "20231114", "auto");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
        assert_ne!(k1, sigv4_signing_key("secret", "20231115", "auto"));
    }

    #[test]
    fn sigv4_signature_known_answer() {
        // Reference produced by an independent HMAC-SHA256 implementation
        // (OpenSSL), same inputs as `signing_key_is_deterministic`.
        let s2s = "AWS4-HMAC-SHA256\n20231114T221320Z\n20231114/auto/s3/aws4_request\n".to_string()
            + &"0".repeat(64);
        assert_eq!(
            sigv4_signature("secret", "20231114", "auto", &s2s),
            "ee052df65acbe2ae850892a319d6f8ca7b23925a37d7f66b4113f9054a143e29"
        );
        // 签名必须随 string-to-sign 变（否则 canonical 部分形同虚设）。
        assert_ne!(sigv4_signature("secret", "20231114", "auto", &s2s), sigv4_signature("secret", "20231114", "auto", &format!("{s2s}1")));
    }

    #[test]
    fn s3_uri_requires_credentials() {
        assert!(source_from_uri_with_s3("s3://bucket/key", None).is_err());
    }

    #[test]
    fn local_source_reads_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        std::fs::write(&p, b"hello world").unwrap();
        let s = LocalFileRangeSource::open(p.to_str().unwrap()).unwrap();
        assert_eq!(s.len(), Some(11));
        assert_eq!(s.read_range(6, 5).unwrap(), b"world");
        assert_eq!(s.read_range(0, 5).unwrap(), b"hello");
        // Reads clamp at EOF (tail reads are not errors)...
        assert_eq!(s.read_range(8, 5).unwrap(), b"rld");
        // ...but reading *past* EOF is.
        assert!(s.read_range(11, 5).is_err());
    }
}
