//! `accessmod serve`: the web interface and its API (`accessmod-server`).

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use accessmod_server::{AppState, Config};

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
    /// Built web front end to serve (`web/dist`).
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
    let state = AppState::new(Config {
        data_dir: data_dir.clone(),
        settings,
        local_paths,
    })
    .map_err(|e| {
        eprintln!("error: {e}");
        Failure { code: 3 }
    })?;
    tracing::info!("data directory {}", data_dir.display());
    accessmod_server::serve_blocking(state, SocketAddr::new(args.host, args.port), args.web_dir)
        .map_err(|e| {
            eprintln!("error: {e}");
            Failure { code: 3 }
        })
}
