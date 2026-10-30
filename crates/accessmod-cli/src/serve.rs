//! `accessmod serve`: the web interface and its API (`accessmod-server`).

use std::io::{BufRead, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use accessmod_server::{AppState, Config, Guard};

use crate::Failure;
use crate::exec::{ExecArgs, settings};

/// Options of `accessmod serve`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Data directory holding the projects (default: `ACCESSMOD_DATA`, or
    /// `AccessMod` in the home directory).
    #[arg(long, value_name = "DIR")]
    pub data: Option<PathBuf>,
    /// Address to listen on. Anything but a loopback address exposes the
    /// server to the network.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: IpAddr,
    /// Port.
    #[arg(long, default_value_t = 5099)]
    pub port: u16,
    /// Built web front end to serve (`web/dist`); a release archive serves
    /// the client it ships (`share/accessmod/web`) by default.
    #[arg(long, value_name = "DIR")]
    pub web_dir: Option<PathBuf>,
    /// Refuse paths on this machine's disk in requests (default on a
    /// non-loopback address).
    #[arg(long)]
    pub no_local_paths: bool,
    /// Listen on a network address although the server has no accounts
    /// yet: anyone who can reach it can use every project. For tests on a
    /// trusted network only.
    #[arg(long)]
    pub insecure_network: bool,
    /// Run for a parent application (the desktop window): read a session
    /// token from the first line of stdin and require it, print
    /// `{"url": …}` on stdout once listening, and stop when stdin closes.
    #[arg(long)]
    pub session_stdin: bool,
    #[command(flatten)]
    pub exec: ExecArgs,
}

fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ACCESSMOD_DATA") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("AccessMod")
}

/// The first line of stdin; a thread then waits for stdin to close and
/// ends the process, so the server never outlives its parent.
fn session_token() -> Result<String, Failure> {
    let mut line = String::new();
    let mut stdin = std::io::stdin().lock();
    let read = stdin.read_line(&mut line);
    let token = line.trim().to_string();
    if read.is_err() || token.len() < 16 {
        eprintln!("error: --session-stdin expects a token of 16 characters or more on stdin");
        return Err(Failure { code: 2 });
    }
    drop(stdin);
    std::thread::spawn(|| {
        let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
        tracing::info!("stdin closed: the parent application is gone, stopping");
        std::process::exit(0);
    });
    Ok(token)
}

/// Run `accessmod serve` until Ctrl-C.
pub fn run(args: Args) -> Result<(), Failure> {
    let settings = settings(&args.exec)?;
    let data_dir = args.data.unwrap_or_else(default_data_dir);
    let local_paths = !args.no_local_paths && args.host.is_loopback();
    if !args.host.is_loopback() {
        if !args.insecure_network {
            eprintln!(
                "error: {} is a network address and this server has no accounts yet; listen on \
                 127.0.0.1, or pass --insecure-network on a trusted network",
                args.host
            );
            return Err(Failure { code: 2 });
        }
        tracing::warn!(
            "listening on {} without accounts: anyone on the network can use this server",
            args.host
        );
    }
    let token = if args.session_stdin {
        Some(session_token()?)
    } else {
        None
    };
    let guard = Guard {
        loopback_host: args.host.is_loopback(),
        token,
    };
    let state = AppState::new(Config {
        data_dir: data_dir.clone(),
        settings,
        local_paths,
        upload_limit: accessmod_server::DEFAULT_UPLOAD_LIMIT,
        demo: true,
    })
    .map_err(|e| {
        eprintln!("error: {e}");
        Failure { code: 3 }
    })?;
    tracing::info!("data directory {}", data_dir.display());
    accessmod_server::serve_blocking(
        state,
        SocketAddr::new(args.host, args.port),
        args.web_dir.or_else(accessmod_io::install::shipped_web_dir),
        guard,
        |bound| {
            if args.session_stdin {
                // The parent reads this line to open its window.
                let mut out = std::io::stdout().lock();
                let _ = writeln!(
                    out,
                    "{}",
                    serde_json::json!({ "url": format!("http://{bound}") })
                );
                let _ = out.flush();
            }
        },
    )
    .map_err(|e| {
        eprintln!("error: {e}");
        Failure { code: 3 }
    })
}
