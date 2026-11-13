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
//!   countries, releases), as URLs.
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
        let mut wait = self.settings.backoff;
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.try_download(url, target, format, cancelled, progress) {
                Err(Error::Network { message, .. }) if attempt < self.settings.attempts => {
                    let _ = message;
                    // Wait, still cancellable.
                    let until = std::time::Instant::now() + wait;
                    while std::time::Instant::now() < until {
                        if cancelled() {
                            return Err(Error::Cancelled);
                        }
                        std::thread::sleep(Duration::from_millis(100).min(wait));
                    }
                    wait *= 2;
                }
                other => return other,
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
