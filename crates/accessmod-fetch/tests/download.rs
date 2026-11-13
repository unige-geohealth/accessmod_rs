//! Downloads against a local server that misbehaves on purpose.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use accessmod_fetch::{Cache, Client, Error, Format, Settings};

const TIFF: &[u8] = b"II*\0 a tiny tiff body, 40 bytes long...";

/// A server answering each path; `/flaky` fails twice with 503 first.
fn server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let flaky = Arc::new(AtomicUsize::new(0));
    let count = flaky.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
                    break;
                }
            }
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
