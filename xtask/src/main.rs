//! Developer tasks: `cargo xtask <task>`.
//!
//! - `lut [--check] [--csv <path>]`: regenerate (or verify) the bicycle LUT
//!   of `accessmod5-v1` ([`lut`]).
//! - `layers`: check the dependency rules between crates ([`layers`]).
//! - `schemas [--check]`: write (or verify) the JSON Schemas of `web/schemas/`
//!   ([`schemas`]).
//! - `i18n [--check]`: write (or verify) the interface catalogue ([`i18n`]).

mod i18n;
mod layers;
mod lut;
mod schemas;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>\n\
    \n  lut [--check] [--csv <path>]   regenerate or verify the bicycle LUT\
    \n  layers                         check the dependency rules between crates\
    \n  schemas [--check]              write or verify the JSON Schemas in web/schemas/\
    \n  i18n [--check]                 write or verify the interface catalogue (en, fr, es, pt)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("lut") => lut::run(&args[1..]),
        Some("layers") => layers::run(&args[1..]),
        Some("schemas") => schemas::run(&args[1..]),
        Some("i18n") => i18n::run(&args[1..]),
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
