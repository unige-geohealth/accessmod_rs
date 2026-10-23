//! `accessmod` command-line interface.
//!
//! Human messages and progress go to stderr; with `--json`, stdout carries
//! only one JSON document. Exit codes: 0 success, 2 validation, 3 I/O,
//! 4 computation, 130 cancelled.

mod doctor;
mod exec;
mod inspect;
mod project;
mod travel_time;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use accessmod_io::coverage::GeographicCoverage;
use accessmod_io::merge::MergeLandCover;
use accessmod_io::optimization::Optimization;
use accessmod_io::population::AdjustPopulation;
use accessmod_io::referral::Referral;
use accessmod_io::scaling_up::ScalingUp;
use accessmod_io::zonal::ZonalStatistics;
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
    /// Projects: datasets on one grid, saved jobs and runs.
    Project {
        #[command(subcommand)]
        command: project::Command,
    },
    /// Describe datasets; import and manage them in a project.
    Dataset {
        #[command(subcommand)]
        command: inspect::Command,
    },
    /// Travel-time analysis.
    TravelTime {
        #[command(subcommand)]
        command: travel_time::Command,
    },
    /// Travel time and distance between facilities (many-to-many).
    Referral {
        #[command(subcommand)]
        command: exec::JobCommand,
    },
    /// Merge land cover, roads and barriers into one land-cover raster.
    MergeLandCover {
        #[command(subcommand)]
        command: exec::JobCommand,
    },
    /// Adjust population distribution: move population on barriers within its
    /// zone, or scale zones to known populations.
    AdjustPopulationDistribution {
        #[command(subcommand)]
        command: exec::JobCommand,
    },
    /// Zonal statistics: population covered within travel-time limits, per
    /// zone.
    ZonalStatistics {
        #[command(subcommand)]
        command: exec::JobCommand,
    },
    /// Geographic coverage: facilities take, in turn, the population they reach
    /// up to their capacity.
    GeographicCoverage {
        #[command(subcommand)]
        command: exec::JobCommand,
    },
    /// Scaling up: add facilities where they are most suitable and cover the
    /// most people.
    ScalingUp {
        #[command(subcommand)]
        command: exec::JobCommand,
    },
    /// Optimization: rank existing facilities by the population their
    /// catchments add to the coverage.
    Optimization {
        #[command(subcommand)]
        command: exec::JobCommand,
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
        Command::Project { command } => project::run(command, &cancel),
        Command::Dataset { command } => match command {
            inspect::Command::Inspect { path, json } => inspect::run(&path, json),
            inspect::Command::Catalogue(command) => project::dataset(command, &cancel),
        },
        Command::TravelTime { command } => travel_time::run(command, &cancel),
        Command::Referral { command } => exec::job_command::<Referral>(command, &cancel),
        Command::MergeLandCover { command } => {
            exec::job_command::<MergeLandCover>(command, &cancel)
        }
        Command::AdjustPopulationDistribution { command } => {
            exec::job_command::<AdjustPopulation>(command, &cancel)
        }
        Command::ZonalStatistics { command } => {
            exec::job_command::<ZonalStatistics>(command, &cancel)
        }
        Command::GeographicCoverage { command } => {
            exec::job_command::<GeographicCoverage>(command, &cancel)
        }
        Command::ScalingUp { command } => exec::job_command::<ScalingUp>(command, &cancel),
        Command::Optimization { command } => exec::job_command::<Optimization>(command, &cancel),
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
