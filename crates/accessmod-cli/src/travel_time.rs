//! `accessmod travel-time …`

use std::path::PathBuf;

use accessmod_io::error::Error;
use accessmod_io::job::{
    FacilityIssues, FacilitySource, JobFile, OutputOptions, SCHEMA_VERSION, ScenarioEntry,
    TravelTimeJob,
};
use accessmod_io::travel_time::TravelTime;
use clap::Subcommand;
use serde::de::DeserializeOwned;

use crate::exec::{self, ExecArgs, fail};
use crate::{Cancel, Failure, print_json};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the JSON schema of the job document.
    Schema,
    /// Validate a job against its inputs, without computing.
    Validate(RunArgs),
    /// Validate and run a job.
    Run(RunArgs),
}

#[derive(Debug, clap::Args)]
pub struct RunArgs {
    /// Job document (JSON). Mutually exclusive with the direct parameters.
    #[arg(long, value_name = "FILE", conflicts_with = "direct")]
    job: Option<PathBuf>,
    #[command(flatten)]
    direct: DirectArgs,
    #[command(flatten)]
    exec: ExecArgs,
}

/// Job given as parameters instead of a file.
#[derive(Debug, clap::Args)]
#[group(id = "direct", multiple = true)]
pub struct DirectArgs {
    /// DEM raster; defines the grid.
    #[arg(long, value_name = "FILE")]
    dem: Option<PathBuf>,
    /// Land-cover raster with integer classes.
    #[arg(long, value_name = "FILE")]
    land_cover: Option<PathBuf>,
    /// Facility point layer.
    #[arg(long, value_name = "FILE")]
    facilities: Option<PathBuf>,
    /// Facility layer name (default: first layer).
    #[arg(long, value_name = "NAME")]
    facilities_layer: Option<String>,
    /// Facility identifier field.
    #[arg(long, value_name = "FIELD")]
    id_field: Option<String>,
    /// OGR SQL attribute filter selecting facilities, e.g. "type = 'HC'".
    #[arg(long = "facilities-where", value_name = "SQL")]
    facilities_where: Option<String>,
    /// Scenario entry CLASS=MODE:SPEED_KMH, e.g. 1=walking:5 (repeat).
    #[arg(long = "class", value_name = "CLASS=MODE:KMH")]
    classes: Vec<String>,
    /// isotropic or anisotropic.
    #[arg(long, value_name = "METHOD")]
    method: Option<String>,
    /// from_facilities or towards_facilities.
    #[arg(long, value_name = "DIRECTION")]
    direction: Option<String>,
    /// 8 or 16.
    #[arg(long, value_name = "N")]
    neighbourhood: Option<u8>,
    /// Inclusive travel-time limit, seconds.
    #[arg(long, value_name = "SECONDS")]
    limit_s: Option<f64>,
    /// Skip facilities outside the grid or on a barrier (with a warning)
    /// instead of failing.
    #[arg(long)]
    skip_facility_issues: bool,
    /// Also write direction.tif.
    #[arg(long)]
    direction_output: bool,
    /// Output directory (must not exist or be empty).
    #[arg(long, value_name = "DIR")]
    output_dir: Option<PathBuf>,
}

pub fn run(command: Command, cancel: &Cancel) -> Result<(), Failure> {
    match command {
        Command::Schema => {
            print_json(&TravelTimeJob::schema());
            Ok(())
        }
        Command::Validate(args) => {
            let job = job(&args).map_err(|e| fail(&e, args.exec.json))?;
            exec::validate::<TravelTime>(job, &args.exec, cancel)
        }
        Command::Run(args) => {
            let job = job(&args).map_err(|e| fail(&e, args.exec.json))?;
            exec::execute::<TravelTime>(job, &args.exec, cancel)
        }
    }
}

/// Value of a serde enum from its JSON name.
fn named<T: DeserializeOwned>(flag: &str, text: &str, issues: &mut Vec<String>) -> Option<T> {
    match serde_json::from_value(serde_json::Value::String(text.to_owned())) {
        Ok(value) => Some(value),
        Err(_) => {
            issues.push(format!("--{flag} {text:?} is not a valid value"));
            None
        }
    }
}

fn parse_class(text: &str) -> Result<ScenarioEntry, String> {
    let bad = || format!("--class {text:?}: expected CLASS=MODE:SPEED_KMH, e.g. 1=walking:5");
    let (class, rest) = text.split_once('=').ok_or_else(bad)?;
    let (mode, speed) = rest.split_once(':').ok_or_else(bad)?;
    let class = class.trim().parse().map_err(|_| bad())?;
    let speed_kmh = speed.trim().parse().map_err(|_| bad())?;
    let mode = serde_json::from_value(serde_json::Value::String(mode.trim().to_owned()))
        .map_err(|_| format!("--class {text:?}: mode must be walking, bicycling or motorized"))?;
    Ok(ScenarioEntry {
        class,
        mode,
        speed_kmh,
        label: None,
    })
}

/// Job from `--job` or from the direct parameters.
fn job(args: &RunArgs) -> Result<TravelTimeJob, Error> {
    if let Some(path) = &args.job {
        return TravelTimeJob::from_file(path);
    }
    let d = &args.direct;
    let mut issues = Vec::new();
    let mut required = |name: &str, present: bool| {
        if !present {
            issues.push(format!("--{name} is required (or give --job)"));
        }
    };
    required("dem", d.dem.is_some());
    required("land-cover", d.land_cover.is_some());
    required("facilities", d.facilities.is_some());
    required("id-field", d.id_field.is_some());
    required("class", !d.classes.is_empty());
    required("method", d.method.is_some());
    required("direction", d.direction.is_some());
    required("neighbourhood", d.neighbourhood.is_some());
    required("output-dir", d.output_dir.is_some());

    let scenario: Vec<ScenarioEntry> = d
        .classes
        .iter()
        .filter_map(|c| parse_class(c).map_err(|e| issues.push(e)).ok())
        .collect();
    let method = d
        .method
        .as_deref()
        .and_then(|m| named("method", m, &mut issues));
    let direction = d
        .direction
        .as_deref()
        .and_then(|m| named("direction", m, &mut issues));
    let neighbourhood = d.neighbourhood.and_then(|n| {
        serde_json::from_value(serde_json::Value::from(n))
            .map_err(|_| issues.push(format!("--neighbourhood must be 8 or 16, got {n}")))
            .ok()
    });
    let (Some(dem), Some(land_cover), Some(facilities), Some(id_field)) =
        (&d.dem, &d.land_cover, &d.facilities, &d.id_field)
    else {
        return Err(Error::Validation { issues });
    };
    let (Some(method), Some(direction), Some(neighbourhood), Some(output_dir), true) = (
        method,
        direction,
        neighbourhood,
        &d.output_dir,
        issues.is_empty(),
    ) else {
        return Err(Error::Validation { issues });
    };
    let cwd = std::env::current_dir().map_err(|e| Error::File {
        path: PathBuf::from("."),
        source: e,
    })?;
    Ok(TravelTimeJob {
        schema_version: SCHEMA_VERSION,
        dem: dem.clone(),
        land_cover: land_cover.clone(),
        facilities: FacilitySource {
            path: facilities.clone(),
            layer: d.facilities_layer.clone(),
            id_field: id_field.clone(),
            filter: d.facilities_where.clone(),
            ids: None,
        },
        scenario,
        method,
        direction,
        neighbourhood,
        limit_s: d.limit_s,
        mobility_model: Default::default(),
        facility_issues: if d.skip_facility_issues {
            FacilityIssues::Skip
        } else {
            FacilityIssues::Error
        },
        outputs: OutputOptions {
            direction: d.direction_output,
        },
        output_dir: output_dir.clone(),
    }
    .resolve_paths(&cwd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use accessmod_core::mobility::TransportMode;

    #[test]
    fn class_entries() {
        let entry = parse_class("12=bicycling:18.5").expect("valid");
        assert_eq!(entry.class, 12);
        assert_eq!(entry.mode, TransportMode::Bicycling);
        assert_eq!(entry.speed_kmh, 18.5);
        assert!(parse_class("12").is_err());
        assert!(parse_class("x=walking:5").is_err());
        assert!(parse_class("1=flying:5").is_err());
    }
}
