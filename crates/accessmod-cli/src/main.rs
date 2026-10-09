//! `accessmod` command-line interface.
//!
//! Human messages and progress go to stderr; with `--json`, stdout carries
//! only one JSON document. Exit codes: 0 success, 2 validation, 3 I/O,
//! 4 computation, 130 cancelled.

mod doctor;
mod inspect;
mod travel_time;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "accessmod",
    version,
    about = "Physical accessibility to health services: travel time, nearest facility, path distance"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)] // reason: parsed once per process; boxing buys nothing
enum Command {
    /// Check GDAL/PROJ, drivers, scratch directory, memory and disk.
    Doctor(doctor::Args),
    /// Describe raster and vector datasets.
    Dataset {
        #[command(subcommand)]
        command: inspect::Command,
    },
    /// Travel-time analysis.
    TravelTime {
        #[command(subcommand)]
        command: travel_time::Command,
    },
}

/// Failure of a command: exit code and the message already reported.
#[derive(Debug)]
pub struct Failure {
    /// Process exit code.
    pub code: u8,
}

/// Set by Ctrl-C; long operations poll it and stop cleanly.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Whether cancellation was requested.
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ACCESSMOD_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cancel = Cancel::default();
    let flag = cancel.0.clone();
    if let Err(e) = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed)) {
        tracing::warn!("cannot install the Ctrl-C handler: {e}");
    }

    let cli = Cli::parse();
    let result = match cli.command {
        Command::Doctor(args) => doctor::run(&args),
        Command::Dataset { command } => inspect::run(&command),
        Command::TravelTime { command } => travel_time::run(command, &cancel),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => ExitCode::from(failure.code),
    }
}

/// Print `value` as pretty JSON on stdout.
pub fn print_json(value: &serde_json::Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("error: cannot serialise output: {e}"),
    }
}
