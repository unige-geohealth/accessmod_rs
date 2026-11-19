//! Downloads against a local server that misbehaves on purpose.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use accessmod_fetch::catalogue::{self, Bounds};
use accessmod_fetch::{Cache, Client, Error, Format, Settings};

const TIFF: &[u8] = b"II*\0 a tiny tiff body, 40 bytes long...";
const ZIP: &[u8] = b"PK\x03\x04 a tiny zip body";

/// A server answering each path; `/flaky` fails twice with 503 first.
/// `/api` is a raw-data API whose task is ready at the second look;
/// `/big` refuses every request as too large.
fn server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let flaky = Arc::new(AtomicUsize::new(0));
    let count = flaky.clone();
    let looks = AtomicUsize::new(0);
    let own = base.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            let _ = reader.read_exact(&mut body);
            let reply = |status: &str, body: &[u8], length: usize| {
                let mut out = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n"
                )
                .into_bytes();
                out.extend_from_slice(body);
                out
            };
            let response = match path.as_str() {
                "/ok.tif" => reply("200 OK", TIFF, TIFF.len()),
                "/missing.tif" => reply("404 Not Found", b"", 0),
                "/page.tif" => reply("200 OK", b"<!DOCTYPE html><html>error</html>", 33),
                "/cut.tif" => reply("200 OK", &TIFF[..10], TIFF.len()),
                "/flaky.tif" => {
                    if count.fetch_add(1, Ordering::SeqCst) < 2 {
                        reply("503 Service Unavailable", b"", 0)
                    } else {
                        reply("200 OK", TIFF, TIFF.len())
                    }
                }
                "/ok.zip" => reply("200 OK", ZIP, ZIP.len()),
                "/api/snapshot/" if body.starts_with(b"{") => {
                    let json = br#"{"task_id":"t-1","track_link":"/tasks/status/t-1/"}"#;
                    reply("200 OK", json, json.len())
                }
                "/api/tasks/status/t-1/" => {
                    let json = if looks.fetch_add(1, Ordering::SeqCst) == 0 {
                        r#"{"id":"t-1","status":"STARTED","result":"N/A"}"#.to_owned()
                    } else {
                        format!(
                            r#"{{"id":"t-1","status":"SUCCESS","result":{{"download_url":"{own}/ok.zip"}}}}"#
                        )
                    };
                    reply("200 OK", json.as_bytes(), json.len())
                }
                "/big/snapshot/" => {
                    let json = br#"{"detail":[{"msg":"Polygon Area 1501271 Sq.KM is higher than Threshold : 80000 Sq.KM"}]}"#;
                    reply("400 Bad Request", json, json.len())
                }
                _ => reply("500 Internal Server Error", b"", 0),
            };
            let _ = stream.write_all(&response);
        }
    });
    (base, flaky)
}

fn client() -> Client {
    Client::new(Settings {
        connect: Duration::from_secs(5),
        read: Duration::from_secs(5),
        attempts: 3,
        backoff: Duration::from_millis(20),
    })
}

#[test]
fn downloads_are_checked_retried_and_atomic() {
    let (base, flaky) = server();
    let dir = tempfile::tempdir().unwrap();
    let c = client();
    let never = || false;
    let mut quiet = |_: u64, _: Option<u64>| {};

    let target = dir.path().join("ok.tif");
    let fetched = c
        .download(
            &format!("{base}/ok.tif"),
            &target,
            Format::Tiff,
            &never,
            &mut quiet,
        )
        .expect("ok");
    assert_eq!(std::fs::read(&target).unwrap(), TIFF);
    assert_eq!(fetched.bytes, TIFF.len() as u64);
    assert_eq!(fetched.sha256.len(), 64);

    let error = c
        .download(
            &format!("{base}/missing.tif"),
            &dir.path().join("m.tif"),
            Format::Tiff,
            &never,
            &mut quiet,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Absent(_)), "{error}");

    let error = c
        .download(
            &format!("{base}/page.tif"),
            &dir.path().join("p.tif"),
            Format::Tiff,
            &never,
            &mut quiet,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Content { .. }), "{error}");

    let error = c
        .download(
            &format!("{base}/cut.tif"),
            &dir.path().join("c.tif"),
            Format::Tiff,
            &never,
            &mut quiet,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Network { .. }), "{error}");

    c.download(
        &format!("{base}/flaky.tif"),
        &dir.path().join("f.tif"),
        Format::Tiff,
        &never,
        &mut quiet,
    )
    .expect("after retries");
    assert_eq!(flaky.load(Ordering::SeqCst), 3);

    let error = c
        .download(
            &format!("{base}/ok.tif"),
            &dir.path().join("x.tif"),
            Format::Tiff,
            &|| true,
            &mut quiet,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Cancelled));

    // Nothing but the complete files: no partial file left.
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["f.tif", "ok.tif"]);
}

#[test]
fn the_cache_fetches_once_and_works_offline() {
    let (base, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::new(dir.path());
    let c = client();
    let mut quiet = |_: u64, _: Option<u64>| {};
    let url = format!("{base}/ok.tif");
    let first = cache
        .get(
            &c,
            "dem/ok.tif",
            &url,
            Format::Tiff,
            false,
            &|| false,
            &mut quiet,
        )
        .expect("fetch");
    // Offline, the cached copy answers; an uncached key does not.
    let again = cache
        .get(
            &c,
            "dem/ok.tif",
            "http://127.0.0.1:1/never",
            Format::Tiff,
            true,
            &|| false,
            &mut quiet,
        )
        .expect("cached");
    assert_eq!(first.sha256, again.sha256);
    let error = cache
        .get(
            &c,
            "dem/other.tif",
            &url,
            Format::Tiff,
            true,
            &|| false,
            &mut quiet,
        )
        .unwrap_err();
    assert!(error.to_string().contains("offline"), "{error}");
}

#[test]
fn snapshots_are_posted_polled_downloaded_and_cached() {
    let (base, _) = server();
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::new(dir.path());
    let c = client();
    let mut quiet = |_: u64, _: Option<u64>| {};
    let tile = Bounds {
        west: 6.55,
        south: 46.38,
        east: 6.70,
        north: 46.52,
    };
    let request = catalogue::ferries(&tile, "2026-11");
    let wait = Duration::from_secs(30);
    let fetched = cache
        .snapshot(
            &c,
            &format!("{base}/api"),
            &request,
            wait,
            false,
            &|| false,
            &mut quiet,
        )
        .expect("snapshot");
    assert_eq!(std::fs::read(&fetched.path).unwrap(), ZIP);
    assert_eq!(fetched.path, cache.path(&request.key));
    // Cached: offline is enough.
    let again = cache
        .snapshot(
            &c,
            "http://127.0.0.1:1",
            &request,
            wait,
            true,
            &|| false,
            &mut quiet,
        )
        .expect("cached");
    assert_eq!(again.sha256, fetched.sha256);
    // A refusal carries the API's reason.
    let error = c
        .snapshot(
            &format!("{base}/big"),
            &request.body,
            &dir.path().join("big.zip"),
            wait,
            &|| false,
            &mut quiet,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Content { .. }), "{error}");
    assert!(
        error.to_string().contains("higher than Threshold"),
        "{error}"
    );
    let error = c
        .snapshot(
            &format!("{base}/api"),
            &request.body,
            &dir.path().join("x.zip"),
            wait,
            &|| true,
            &mut quiet,
        )
        .unwrap_err();
    assert!(matches!(error, Error::Cancelled), "{error}");
}
