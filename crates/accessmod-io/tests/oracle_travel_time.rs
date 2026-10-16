//! Comparison with AccessMod 5 travel times on the demo project.
//!
//! Inputs and raw AccessMod 5 outputs (seconds, before `amCleanTravelTime`)
//! are exported by `scripts/oracle/export.sh` into `_oracle/travel-time/demo`.
//! The test skips, with a message, when the export is absent.

use std::path::Path;

use accessmod_core::cost::Neighbourhood;
use accessmod_io::fixtures::{never, oracle_available, oracle_dir, read_with_nodata, settings};
use accessmod_io::job::{
    FacilityIssues, FacilitySource, MethodName, OutputOptions, SCHEMA_VERSION, ScenarioEntry,
    TravelTimeJob,
};
use accessmod_io::output::{NEAREST_FILE, TRAVEL_TIME_FILE};
use accessmod_io::travel_time::{prepare, run};

fn scenario(dir: &Path) -> Vec<ScenarioEntry> {
    std::fs::read_to_string(dir.join("demo-scenario.txt"))
        .expect("scenario")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            ScenarioEntry {
                class: f[0].parse().expect("class"),
                mode: serde_json::from_value(serde_json::Value::from(f[1])).expect("mode"),
                speed_kmh: f[2].parse().expect("speed"),
                label: None,
            }
        })
        .collect()
}

struct Case {
    name: String,
    method: MethodName,
    direction: accessmod_core::cost::Direction,
    neighbourhood: Neighbourhood,
    limit_s: Option<f64>,
}

fn cases(dir: &Path) -> Vec<Case> {
    std::fs::read_to_string(dir.join("travel-time-cases.txt"))
        .expect("cases")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let limit: f64 = f[4].parse().expect("limit");
            Case {
                name: f[0].to_owned(),
                method: serde_json::from_value(serde_json::Value::from(f[1])).expect("method"),
                direction: serde_json::from_value(serde_json::Value::from(f[2]))
                    .expect("direction"),
                neighbourhood: serde_json::from_value(serde_json::Value::from(
                    f[3].parse::<u8>().expect("n"),
                ))
                .expect("neighbourhood"),
                limit_s: (limit > 0.0).then_some(limit),
            }
        })
        .collect()
}

/// Comparison of one case.
#[derive(Debug, Default)]
struct Comparison {
    reached_both: usize,
    reached_only_oracle: usize,
    reached_only_ours: usize,
    max_relative_error: f64,
    max_absolute_error_s: f64,
    nearest_mismatches: usize,
}

fn compare(case: &Case, dir: &Path, tmp: &Path) -> Comparison {
    let input = dir.join("input");
    let job = TravelTimeJob {
        schema_version: SCHEMA_VERSION,
        dem: input.join("dem.tif"),
        land_cover: input.join("land_cover.tif"),
        facilities: FacilitySource {
            path: input.join("facilities.fgb"),
            layer: None,
            id_field: "cat".into(),
            filter: None,
        },
        scenario: scenario(dir),
        method: case.method,
        direction: case.direction,
        neighbourhood: case.neighbourhood,
        limit_s: case.limit_s,
        mobility_model: Default::default(),
        facility_issues: FacilityIssues::Error,
        outputs: OutputOptions::default(),
        output_dir: tmp.join(&case.name),
    };
    let settings = settings(256 << 20, 1);
    let prepared = prepare(job, &settings, &never).expect("prepare");
    let ids: Vec<i64> = prepared
        .facilities
        .kept
        .iter()
        .map(|f| f.id.as_i64().expect("integer cat"))
        .collect();
    let (out, _) = run(prepared, &settings, &never).expect("run");

    let (ours, ours_nodata) = read_with_nodata::<f64>(&out.join(TRAVEL_TIME_FILE));
    let (our_nearest, _) = read_with_nodata::<u32>(&out.join(NEAREST_FILE));
    let exported = dir.join("oracle");
    let (oracle, oracle_nodata) =
        read_with_nodata::<f64>(&exported.join(format!("{}-travel_time_seconds.tif", case.name)));
    let (oracle_nearest, _) =
        read_with_nodata::<i32>(&exported.join(format!("{}-nearest.tif", case.name)));

    let limit = case.limit_s.unwrap_or(f64::INFINITY);
    let mut c = Comparison::default();
    for i in 0..ours.len() {
        let our_reached = Some(ours[i]) != ours_nodata;
        // AccessMod 5 cuts the r.walk overshoot afterwards with t <= limit.
        let oracle_reached =
            !oracle[i].is_nan() && Some(oracle[i]) != oracle_nodata && oracle[i] <= limit;
        match (our_reached, oracle_reached) {
            (true, true) => {
                c.reached_both += 1;
                let abs = (ours[i] - oracle[i]).abs();
                let rel = if oracle[i] > 0.0 {
                    abs / oracle[i]
                } else {
                    abs
                };
                c.max_absolute_error_s = c.max_absolute_error_s.max(abs);
                c.max_relative_error = c.max_relative_error.max(rel);
                let our_id = ids[our_nearest[i] as usize - 1];
                if our_id != i64::from(oracle_nearest[i]) {
                    c.nearest_mismatches += 1;
                }
            }
            (false, true) => c.reached_only_oracle += 1,
            (true, false) => c.reached_only_ours += 1,
            (false, false) => {}
        }
    }
    c
}

#[test]
fn demo_matches_accessmod5() {
    let dir = oracle_dir("travel-time/demo");
    if !oracle_available(&[&dir.join("oracle")]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let mut failures = Vec::new();
    for case in cases(&dir) {
        let c = compare(&case, &dir, tmp.path());
        eprintln!("{:<22} {c:?}", case.name);
        // Anisotropic: same graph, f64 throughout (plan: 1e-9). Isotropic:
        // AccessMod 5 stores the friction in a Float32 map (r.recode gives
        // FCELL), so each friction carries a relative rounding error up to
        // 2^-24 and so does every sum of them; the f64 slack covers the rest.
        let tolerance = match case.method {
            MethodName::Anisotropic => 1e-9,
            MethodName::Isotropic => f64::powi(2.0, -24) + 1e-12,
        };
        if c.reached_only_oracle + c.reached_only_ours > 0 {
            failures.push(format!("{}: reachable masks differ", case.name));
        }
        if c.nearest_mismatches > 0 {
            failures.push(format!(
                "{}: {} nearest-facility mismatches",
                case.name, c.nearest_mismatches
            ));
        }
        if c.max_relative_error > tolerance {
            failures.push(format!(
                "{}: relative error {:.3e} > {tolerance:e}",
                case.name, c.max_relative_error
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
