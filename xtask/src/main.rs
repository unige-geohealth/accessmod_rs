//! Developer tasks: `cargo xtask <task>`.
//!
//! - `lut [--check] [--csv <path>]`: regenerate (or verify) the bicycle LUT
//!   of `accessmod5-v1` ([`lut`]).
//! - `layers`: check the dependency rules between crates ([`layers`]).

mod layers;
mod lut;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>\n\
    \n  lut [--check] [--csv <path>]   regenerate or verify the bicycle LUT\
    \n  layers                         check the dependency rules between crates";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("lut") => lut::run(&args[1..]),
        Some("layers") => layers::run(&args[1..]),
        _ => Err(format!("{USAGE}\n(got: {})", args.join(" "))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root (`xtask/` is one level below it).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
