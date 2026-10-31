//! AccessMod 6 desktop window.
//!
//! The engine is the `accessmod` binary shipped beside this one, run as
//! `accessmod serve --session-stdin --port 0`: a separate process, so an
//! engine crash leaves the window up and offers a restart. The window
//! shows the interface the engine serves on 127.0.0.1, behind a session
//! token handed over on stdin (`docs/desktop.md`). Closing stdin, on quit
//! or if this process dies, stops the engine.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

const WINDOW: &str = "main";
/// The window's own page (`ui/index.html`): "Starting AccessMod…".
const HOME: &str = if cfg!(windows) {
    "http://tauri.localhost/index.html"
} else {
    "tauri://localhost/index.html"
};
/// First launch builds the demo project before listening.
const START_TIMEOUT: Duration = Duration::from_secs(300);

/// The running engine.
#[derive(Default)]
struct Engine {
    /// Its stdin: dropping it stops the engine.
    stdin: Mutex<Option<ChildStdin>>,
    /// Origin of its interface, e.g. `http://127.0.0.1:53785`.
    origin: Mutex<Option<String>>,
    /// Set on quit: the engine stopping is then expected.
    quitting: Mutex<bool>,
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Engine::default())
        .setup(|app| {
            if let Some(url) = dev_url() {
                // Development: the window over `npm run dev`, no engine.
                if let Ok(mut slot) = app.state::<Engine>().origin.lock() {
                    *slot = Some(url.origin().ascii_serialization());
                }
                open_window(app.handle(), WebviewUrl::External(url))?;
                return Ok(());
            }
            open_window(app.handle(), WebviewUrl::App("index.html".into()))?;
            let handle = app.handle().clone();
            std::thread::spawn(move || start(&handle));
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
    app.run(|app, event| {
        if let RunEvent::Exit = event {
            stop(app);
        }
    });
}

/// `ACCESSMOD_DEV_URL`, e.g. `http://localhost:5173` (Vite, hot reload):
/// debug builds only, so an installed application always runs its engine.
fn dev_url() -> Option<Url> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let url = std::env::var("ACCESSMOD_DEV_URL").ok()?;
    Url::parse(&url)
        .inspect_err(|e| eprintln!("ACCESSMOD_DEV_URL {url:?}: {e}"))
        .ok()
}

fn open_window(app: &AppHandle, url: WebviewUrl) -> tauri::Result<()> {
    let navigation = app.clone();
    let downloads = app.clone();
    WebviewWindowBuilder::new(app, WINDOW, url)
        .title("AccessMod 6")
        .inner_size(1400.0, 900.0)
        .min_inner_size(900.0, 600.0)
        // Files dropped on the window go to the page (import drop zones).
        .disable_drag_drop_handler()
        .on_navigation(move |url| allow_navigation(&navigation, url))
        .on_download(move |_, event| download(&downloads, event))
        .build()?;
    Ok(())
}

/// The window shows only its own pages and the engine; other links open
/// in the default browser.
fn allow_navigation(app: &AppHandle, url: &Url) -> bool {
    let local =
        matches!(url.scheme(), "tauri" | "asset") || url.host_str() == Some("tauri.localhost");
    let origin = url.origin().ascii_serialization();
    let engine = app.state::<Engine>();
    let ours = engine
        .origin
        .lock()
        .map(|o| o.as_deref() == Some(origin.as_str()))
        .unwrap_or(false);
    if local || ours {
        return true;
    }
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
    false
}

/// Downloads go to the Downloads folder, under a name not yet taken.
fn download(app: &AppHandle, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { destination, .. } => {
            let Ok(dir) = app.path().download_dir() else {
                return false;
            };
            let name = destination
                .file_name()
                .map_or_else(|| "accessmod-download".into(), |n| n.to_owned());
            *destination = free_path(&dir, Path::new(&name));
            true
        }
        DownloadEvent::Finished { success, .. } => {
            if !success {
                app.dialog()
                    .message("The download failed.")
                    .kind(MessageDialogKind::Error)
                    .title("AccessMod")
                    .show(|_| {});
            }
            true
        }
        _ => true,
    }
}

fn free_path(dir: &Path, name: &Path) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Start the engine, show its interface, and watch it. Runs on its own
/// thread; on failure, asks whether to try again.
fn start(app: &AppHandle) {
    loop {
        let outcome = run_engine(app);
        if app
            .state::<Engine>()
            .quitting
            .lock()
            .map(|q| *q)
            .unwrap_or(true)
        {
            return;
        }
        let message = match outcome {
            Ok(code) => format!(
                "The AccessMod engine stopped unexpectedly ({code}).\n\nIts log: {}",
                log_path(app).display()
            ),
            Err(e) => format!("The AccessMod engine could not start: {e}"),
        };
        let retry = app
            .dialog()
            .message(format!("{message}\n\nRestart it?"))
            .kind(MessageDialogKind::Error)
            .title("AccessMod")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Restart".into(),
                "Quit".into(),
            ))
            .blocking_show();
        if !retry {
            app.exit(1);
            return;
        }
        if let Some(window) = app.get_webview_window(WINDOW)
            && let Ok(url) = Url::parse(HOME)
        {
            let _ = window.navigate(url);
        }
    }
}

/// Run one engine until it exits: its exit status, or why it never served.
fn run_engine(app: &AppHandle) -> Result<String, String> {
    let token = token()?;
    let mut command = Command::new(engine_binary()?);
    command
        .args(["serve", "--session-stdin", "--port", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log_file(app));
    // Installed: data files are resources of the bundle. A development
    // run (`ACCESSMOD_BIN`) uses those beside the binary.
    if let Ok(share) = app.path().resource_dir().map(|d| d.join("share"))
        && share.join("proj").is_dir()
    {
        command.env("PROJ_DATA", share.join("proj"));
        command
            .arg("--web-dir")
            .arg(share.join("accessmod").join("web"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("{e} ({})", engine_binary().unwrap_or_default().display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    writeln!(stdin, "{token}").map_err(|e| e.to_string())?;
    let engine = app.state::<Engine>();
    if let Ok(mut slot) = engine.stdin.lock() {
        *slot = Some(stdin);
    }
    // The first stdout line is `{"url": …}`, once listening.
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(line);
    });
    let origin = receiver
        .recv_timeout(START_TIMEOUT)
        .ok()
        .and_then(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
        .and_then(|v| v["url"].as_str().map(str::to_string));
    match origin {
        Some(origin) => {
            if let Ok(mut slot) = engine.origin.lock() {
                *slot = Some(origin.clone());
            }
            let open = format!("{origin}/api/session/open?token={token}");
            if let (Some(window), Ok(url)) = (app.get_webview_window(WINDOW), Url::parse(&open)) {
                let _ = window.navigate(url);
            }
        }
        None => {
            let _ = child.kill();
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if let Ok(mut slot) = engine.origin.lock() {
        *slot = None;
    }
    Ok(status.to_string())
}

/// Stop the engine: closing its stdin ends it.
fn stop(app: &AppHandle) {
    let engine = app.state::<Engine>();
    if let Ok(mut quitting) = engine.quitting.lock() {
        *quitting = true;
    }
    if let Ok(mut stdin) = engine.stdin.lock() {
        stdin.take();
    }
}

/// `ACCESSMOD_BIN` (development), else `accessmod` beside this executable.
fn engine_binary() -> Result<PathBuf, String> {
    if let Some(bin) = std::env::var_os("ACCESSMOD_BIN") {
        return Ok(PathBuf::from(bin));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no executable directory")?;
    Ok(dir.join(format!("accessmod{}", std::env::consts::EXE_SUFFIX)))
}

fn token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn log_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_log_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("engine.log")
}

/// The engine's stderr, appended to its log; nowhere if it cannot open.
fn log_file(app: &AppHandle) -> Stdio {
    let path = log_path(app);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map(Stdio::from)
        .unwrap_or_else(|_: std::io::Error| Stdio::null())
}
