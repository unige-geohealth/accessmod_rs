//! Downloads for AccessMod (`docs/project-creator.md`, contracts 9–11).
//!
//! - [`Client`]: HTTPS with rustls and the platform's certificate verifier
//!   (corporate CAs work, verification is never off), proxies from
//!   `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`, timeouts, retries with
//!   backoff on 429, 5xx and broken connections.
//! - [`Cache`]: files fetched once into a directory shared by projects,
//!   written to a temporary name and renamed only once complete and
//!   checked (length, expected format: an HTML error page served with 200
//!   is refused).
//! - [`catalogue`]: where each source of the Project Creator lives (tiles,
//!   countries, releases), as URLs, and the on-demand extracts of the HOT
//!   raw-data API ([`Client::snapshot`]: posted, polled, downloaded).
//!
//! This crate knows nothing of GDAL or projects; the I/O layer never does
//! HTTP (`cargo xtask layers`).

pub mod catalogue;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Errors of a download.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file does not exist at the source (404, 403 on public buckets):
    /// e.g. a tile at sea.
    #[error("{0}: not found at the source")]
    Absent(String),
    /// The network or the server failed, after retries.
    #[error("{url}: {message}")]
    Network {
        /// URL asked.
        url: String,
        /// What went wrong.
        message: String,
    },
    /// The content is not what was expected.
    #[error("{url}: {message}")]
    Content {
        /// URL asked.
        url: String,
        /// What is wrong.
        message: String,
    },
    /// A local file could not be written.
    #[error("{path}: {source}")]
    File {
        /// File.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
    /// Cancelled by the user.
    #[error("cancelled")]
    Cancelled,
}

/// Result of this crate.
pub type Result<T> = std::result::Result<T, Error>;

fn file_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::File {
        path: path.to_path_buf(),
        source,
    }
}

/// What a file must look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// A TIFF (GeoTIFF, COG).
    Tiff,
    /// A zip archive.
    Zip,
    /// JSON (an object or an array).
    Json,
}

impl Format {
    fn accepts(self, head: &[u8]) -> bool {
        match self {
            Format::Tiff => {
                head.starts_with(b"II*\0")
                    || head.starts_with(b"MM\0*")
                    || head.starts_with(b"II+\0")
                    || head.starts_with(b"MM\0+")
            }
            Format::Zip => head.starts_with(b"PK\x03\x04"),
            Format::Json => head
                .iter()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| *b == b'{' || *b == b'['),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Format::Tiff => "a TIFF",
            Format::Zip => "a zip archive",
            Format::Json => "JSON",
        }
    }
}

/// A file fetched.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fetched {
    /// Local path.
    pub path: PathBuf,
    /// URL it came from.
    pub url: String,
    /// Size, bytes.
    pub bytes: u64,
    /// SHA-256.
    pub sha256: String,
}

/// Connection settings.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Time to connect.
    pub connect: Duration,
    /// Longest silence while receiving.
    pub read: Duration,
    /// Attempts per file (1 = no retry).
    pub attempts: u32,
    /// First wait before a retry, doubled each time.
    pub backoff: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(30),
            read: Duration::from_secs(120),
            attempts: 4,
            backoff: Duration::from_secs(2),
        }
    }
}

/// An HTTPS client.
#[derive(Debug, Clone)]
pub struct Client {
    agent: ureq::Agent,
    settings: Settings,
}

/// Progress of one file: bytes received, total when known.
pub type Progress<'a> = &'a mut dyn FnMut(u64, Option<u64>);

impl Client {
    /// A client with `settings`; proxies from the environment.
    pub fn new(settings: Settings) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .proxy(ureq::Proxy::try_from_env())
            .timeout_connect(Some(settings.connect))
            .timeout_recv_response(Some(settings.read))
            .timeout_recv_body(None)
            .http_status_as_error(false)
            .user_agent(concat!(
                "AccessMod/",
                env!("CARGO_PKG_VERSION"),
                " (Project Creator)"
            ))
            .build()
            .into();
        Self { agent, settings }
    }

    /// Download `url` into `target` (replaced when complete): checked
    /// against `format` and the announced length; retried on transient
    /// failures.
    pub fn download(
        &self,
        url: &str,
        target: &Path,
        format: Format,
        cancelled: &dyn Fn() -> bool,
        progress: Progress<'_>,
    ) -> Result<Fetched> {
        let dir = target.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(file_error(dir))?;
        let progress = std::cell::RefCell::new(progress);
        self.retried(cancelled, || {
            self.try_download(url, target, format, cancelled, *progress.borrow_mut())
        })
    }

    fn try_download(
        &self,
        url: &str,
        target: &Path,
        format: Format,
        cancelled: &dyn Fn() -> bool,
        progress: Progress<'_>,
    ) -> Result<Fetched> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let network = |message: String| Error::Network {
            url: url.to_owned(),
            message,
        };
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| network(e.to_string()))?;
        let status = response.status().as_u16();
        match status {
            200 => {}
            403 | 404 | 410 => return Err(Error::Absent(url.to_owned())),
            408 | 425 | 429 | 500..=599 => return Err(network(format!("HTTP {status}"))),
            _ => {
                return Err(Error::Content {
                    url: url.to_owned(),
                    message: format!("HTTP {status}"),
                });
            }
        }
        let announced: Option<u64> = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        let partial = target.with_extension(format!(
            "part-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let result = (|| {
            let mut file = std::fs::File::create(&partial).map_err(file_error(&partial))?;
            let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
            let mut buffer = vec![0u8; 1 << 16];
            let mut hasher = Sha256::new();
            let mut received = 0u64;
            let mut head = Vec::new();
            loop {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                let n = reader
                    .read(&mut buffer)
                    .map_err(|e| network(format!("transfer broken: {e}")))?;
                if n == 0 {
                    break;
                }
                if head.len() < 16 {
                    head.extend_from_slice(&buffer[..n.min(16 - head.len())]);
                    if head.len() >= 8 && !format.accepts(&head) {
                        return Err(Error::Content {
                            url: url.to_owned(),
                            message: format!("not {} (an error page?)", format.name()),
                        });
                    }
                }
                file.write_all(&buffer[..n]).map_err(file_error(&partial))?;
                hasher.update(&buffer[..n]);
                received += n as u64;
                progress(received, announced);
            }
            if !format.accepts(&head) {
                return Err(Error::Content {
                    url: url.to_owned(),
                    message: format!("not {}", format.name()),
                });
            }
            if let Some(expected) = announced
                && received != expected
            {
                return Err(network(format!(
                    "{received} bytes received of {expected}: transfer cut"
                )));
            }
            file.sync_all().map_err(file_error(&partial))?;
            Ok(Fetched {
                path: target.to_path_buf(),
                url: url.to_owned(),
                bytes: received,
                sha256: hex(&hasher.finalize()),
            })
        })();
        match result {
            Ok(fetched) => {
                std::fs::rename(&partial, target).map_err(file_error(target))?;
                Ok(fetched)
            }
            Err(error) => {
                let _ = std::fs::remove_file(&partial);
                Err(error)
            }
        }
    }
}

/// How long [`Client::snapshot`] waits between two looks at a task.
const POLL: Duration = Duration::from_secs(3);

/// Largest JSON answer of the raw-data API read, bytes.
const JSON_LIMIT: u64 = 1 << 20;

impl Client {
    /// An on-demand extract of the HOT raw-data API at `api`: `body` posted
    /// to `{api}/snapshot/`, its task polled until done (at most `wait`),
    /// then its zip downloaded into `target`. A request the API refuses
    /// (an area too large…) is [`Error::Content`] with its reason.
    pub fn snapshot(
        &self,
        api: &str,
        body: &serde_json::Value,
        target: &Path,
        wait: Duration,
        cancelled: &dyn Fn() -> bool,
        progress: Progress<'_>,
    ) -> Result<Fetched> {
        let url = format!("{api}/snapshot/");
        let posted = self.retried(cancelled, || self.call_json(&url, Some(body)))?;
        let task = posted
            .get("task_id")
            .and_then(serde_json::Value::as_str)
            .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
            .ok_or_else(|| Error::Content {
                url: url.clone(),
                message: "no task id in the answer".into(),
            })?
            .to_owned();
        let status_url = format!("{api}/tasks/status/{task}/");
        let deadline = std::time::Instant::now() + wait;
        loop {
            // A broken look is retried at the next one, until the deadline.
            if let Ok(answer) = self.call_json(&status_url, None) {
                let status = answer.get("status").and_then(serde_json::Value::as_str);
                match status {
                    Some("SUCCESS") => {
                        let download = answer
                            .pointer("/result/download_url")
                            .and_then(serde_json::Value::as_str)
                            .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
                            .ok_or_else(|| Error::Content {
                                url: status_url.clone(),
                                message: "a finished task without a download URL".into(),
                            })?;
                        return self.download(download, target, Format::Zip, cancelled, progress);
                    }
                    Some("FAILURE" | "REVOKED") => {
                        return Err(Error::Content {
                            url: status_url,
                            message: format!(
                                "the extract failed ({})",
                                answer
                                    .get("result")
                                    .map(ToString::to_string)
                                    .unwrap_or_default()
                            ),
                        });
                    }
                    _ => {}
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Network {
                    url: status_url,
                    message: format!("the extract was not ready within {} s", wait.as_secs()),
                });
            }
            self.pause(POLL, cancelled)?;
        }
    }

    /// `call` again on network errors, as [`Client::download`].
    fn retried<T>(&self, cancelled: &dyn Fn() -> bool, call: impl Fn() -> Result<T>) -> Result<T> {
        let mut wait = self.settings.backoff;
        let mut attempt = 0;
        loop {
            attempt += 1;
            if cancelled() {
                return Err(Error::Cancelled);
            }
            match call() {
                Err(Error::Network { .. }) if attempt < self.settings.attempts => {
                    self.pause(wait, cancelled)?;
                    wait *= 2;
                }
                other => return other,
            }
        }
    }

    /// Sleep `wait`, still cancellable.
    fn pause(&self, wait: Duration, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let until = std::time::Instant::now() + wait;
        while std::time::Instant::now() < until {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100).min(wait));
        }
        Ok(())
    }

    /// A JSON answer: `body` posted, or a GET without one.
    fn call_json(&self, url: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value> {
        let network = |message: String| Error::Network {
            url: url.to_owned(),
            message,
        };
        let response = match body {
            Some(body) => self
                .agent
                .post(url)
                .header("content-type", "application/json")
                .send(body.to_string().as_bytes()),
            None => self.agent.get(url).call(),
        };
        let mut response = response.map_err(|e| network(e.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(JSON_LIMIT)
            .read_to_string()
            .map_err(|e| network(format!("answer broken: {e}")))?;
        let json: Option<serde_json::Value> = serde_json::from_str(&text).ok();
        match status {
            200..=299 => json.ok_or_else(|| Error::Content {
                url: url.to_owned(),
                message: "the answer is not JSON".into(),
            }),
            408 | 425 | 429 | 500..=599 => Err(network(format!("HTTP {status}"))),
            _ => Err(Error::Content {
                url: url.to_owned(),
                message: json
                    .as_ref()
                    .and_then(refusal)
                    .unwrap_or_else(|| format!("HTTP {status}")),
            }),
        }
    }
}

/// The reason of a refusal by the raw-data API: `{"detail": "…"}` or
/// `{"detail": [{"msg": "…"}, …]}`.
fn refusal(answer: &serde_json::Value) -> Option<String> {
    match answer.get("detail")? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let messages: Vec<&str> = items
                .iter()
                .filter_map(|i| i.get("msg").and_then(serde_json::Value::as_str))
                .collect();
            (!messages.is_empty()).then(|| messages.join("; "))
        }
        _ => None,
    }
}

/// The current month in UTC, `YYYY-MM`.
pub fn current_month() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    let (year, month) = year_month(days);
    format!("{year:04}-{month:02}")
}

/// Year and month of a day counted from 1970-01-01 (proleptic Gregorian).
fn year_month(days: i64) -> (i64, u32) {
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a local file.
pub fn sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(file_error(path))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buffer).map_err(file_error(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Size of a cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Usage {
    /// Bytes, summed over the files.
    pub bytes: u64,
    /// Files.
    pub files: u64,
}

/// A directory of fetched files, shared by projects: a file is there
/// complete or not at all.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// The cache in `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Where `key` (a relative path, `/`-separated) lives.
    pub fn path(&self, key: &str) -> PathBuf {
        let mut path = self.root.clone();
        for part in key
            .split('/')
            .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        {
            path.push(part);
        }
        path
    }

    /// Bytes and files in the cache (metadata only; links are not
    /// followed); an absent cache is empty.
    pub fn usage(&self) -> Result<Usage> {
        let mut usage = Usage::default();
        let mut dirs = vec![self.root.clone()];
        while let Some(dir) = dirs.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(file_error(&dir)(e)),
            };
            for entry in entries {
                let entry = entry.map_err(file_error(&dir))?;
                let path = entry.path();
                let meta = std::fs::symlink_metadata(&path).map_err(file_error(&path))?;
                if meta.is_dir() {
                    dirs.push(path);
                } else {
                    usage.files += 1;
                    usage.bytes += meta.len();
                }
            }
        }
        Ok(usage)
    }

    /// Remove everything in the cache (the directory itself is kept);
    /// files are fetched again when next asked. Returns what was removed.
    pub fn clear(&self) -> Result<Usage> {
        let usage = self.usage()?;
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Usage::default()),
            Err(e) => return Err(file_error(&self.root)(e)),
        };
        for entry in entries {
            let path = entry.map_err(file_error(&self.root))?.path();
            let meta = std::fs::symlink_metadata(&path).map_err(file_error(&path))?;
            let removed = if meta.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match removed {
                Ok(()) => {}
                // Gone meanwhile: nothing left to remove.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(file_error(&path)(e)),
            }
        }
        Ok(usage)
    }

    /// The file of `key`, fetched from `url` unless already cached;
    /// `offline`: never contact the source.
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &self,
        client: &Client,
        key: &str,
        url: &str,
        format: Format,
        offline: bool,
        cancelled: &dyn Fn() -> bool,
        progress: Progress<'_>,
    ) -> Result<Fetched> {
        let path = self.path(key);
        if path.is_file() {
            let bytes = std::fs::metadata(&path).map_err(file_error(&path))?.len();
            return Ok(Fetched {
                sha256: sha256(&path)?,
                path,
                url: url.to_owned(),
                bytes,
            });
        }
        if offline {
            return Err(Error::Network {
                url: url.to_owned(),
                message: format!("not in the cache ({key}) and offline"),
            });
        }
        client.download(url, &path, format, cancelled, progress)
    }
}

impl Cache {
    /// The extract `snapshot` from the raw-data API at `api`, unless
    /// already cached; `offline`: never contact the API.
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot(
        &self,
        client: &Client,
        api: &str,
        snapshot: &catalogue::Snapshot,
        wait: Duration,
        offline: bool,
        cancelled: &dyn Fn() -> bool,
        progress: Progress<'_>,
    ) -> Result<Fetched> {
        let path = self.path(&snapshot.key);
        let url = format!("{api}/snapshot/");
        if path.is_file() {
            let bytes = std::fs::metadata(&path).map_err(file_error(&path))?.len();
            return Ok(Fetched {
                sha256: sha256(&path)?,
                path,
                url,
                bytes,
            });
        }
        if offline {
            return Err(Error::Network {
                url,
                message: format!("not in the cache ({}) and offline", snapshot.key),
            });
        }
        client.snapshot(api, &snapshot.body, &path, wait, cancelled, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_follow_the_calendar() {
        assert_eq!(year_month(0), (1970, 1));
        assert_eq!(year_month(31), (1970, 2));
        assert_eq!(year_month(-1), (1969, 12));
        // 2000-02-29 and 2026-11-19.
        assert_eq!(year_month(11_016), (2000, 2));
        assert_eq!(year_month(20_776), (2026, 11));
        assert_eq!(current_month().len(), 7);
    }

    #[test]
    fn refusals_are_read_from_the_answer() {
        let area = serde_json::json!({"detail": [{"msg": "Polygon Area 1501271 Sq.KM is higher than Threshold : 80000 Sq.KM"}]});
        assert_eq!(
            refusal(&area).as_deref(),
            Some("Polygon Area 1501271 Sq.KM is higher than Threshold : 80000 Sq.KM")
        );
        assert_eq!(
            refusal(&serde_json::json!({"detail": "no"})).as_deref(),
            Some("no")
        );
        assert_eq!(refusal(&serde_json::json!({"other": 1})), None);
    }

    #[test]
    fn formats_are_told_by_their_first_bytes() {
        assert!(Format::Tiff.accepts(b"II*\0rest"));
        assert!(Format::Tiff.accepts(b"MM\0+rest"));
        assert!(!Format::Tiff.accepts(b"<!DOCTYPE html>"));
        assert!(Format::Zip.accepts(b"PK\x03\x04...."));
        assert!(Format::Json.accepts(b"  [{\"a\":1}]"));
        assert!(!Format::Json.accepts(b"<html>"));
    }

    #[test]
    fn cache_keys_stay_inside() {
        let cache = Cache::new("/data/cache");
        assert_eq!(
            cache.path("dem/../../etc/passwd"),
            PathBuf::from("/data/cache/dem/etc/passwd")
        );
        assert_eq!(cache.path("/a//b/./c"), PathBuf::from("/data/cache/a/b/c"));
    }

    #[test]
    fn cache_usage_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sources");
        let cache = Cache::new(&root);
        // Absent: empty, and clearing it is no error.
        assert_eq!(cache.usage().unwrap(), Usage::default());
        assert_eq!(cache.clear().unwrap(), Usage::default());
        std::fs::create_dir_all(root.join("dem/tiles")).unwrap();
        std::fs::write(root.join("countries.json"), b"[]").unwrap();
        std::fs::write(root.join("dem/tiles/a.tif"), vec![0u8; 1000]).unwrap();
        let usage = Usage {
            bytes: 1002,
            files: 2,
        };
        assert_eq!(cache.usage().unwrap(), usage);
        assert_eq!(cache.clear().unwrap(), usage);
        assert!(root.is_dir(), "the directory is kept");
        assert_eq!(cache.usage().unwrap(), Usage::default());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }
}
