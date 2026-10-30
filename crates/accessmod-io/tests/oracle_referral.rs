//! Comparison with the AccessMod 5 demo referral (replay
//! `replay/examples/demo_referral`, units set to seconds and metres),
//! exported by `scripts/oracle/export.sh` into `_oracle/referral/demo`.
//! Skips, with a message, when the export is absent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use accessmod_core::cost::Neighbourhood;
use accessmod_core::mobility::TransportMode;
use accessmod_io::fixtures::{oracle_available, oracle_dir, run_job, settings};
use accessmod_io::job::{FacilityIssues, FacilitySource, MethodName, ScenarioEntry};
use accessmod_io::referral::{Referral, ReferralJob};

fn root() -> PathBuf {
    oracle_dir("")
}

/// Fields of one CSV line, honouring double quotes.
fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                out.last_mut().expect("one field").push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => out.push(String::new()),
            (c, _) => out.last_mut().expect("one field").push(c),
        }
    }
    out
}

/// (origin, destination) -> (time_s, distance_m) from a CSV with the given
/// column names.
fn pairs(path: &Path, cols: [&str; 4]) -> BTreeMap<(i64, i64), (f64, f64)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    let header = fields(lines.next().expect("header"));
    let at = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .unwrap_or_else(|| panic!("no column {name}"))
    };
    let idx = cols.map(at);
    lines
        .map(fields)
        .filter(|f| !f[idx[2]].is_empty())
        .map(|f| {
            let n = |i: usize| f[idx[i]].parse::<f64>().expect("number");
            ((n(0) as i64, n(1) as i64), (n(2), n(3)))
        })
        .collect()
}

fn job(neighbourhood: Neighbourhood, out: &Path) -> ReferralJob {
    let input = root().join("travel-time/demo/input");
    let facilities = FacilitySource {
        path: input.join("facilities.fgb"),
        layer: None,
        id_field: "cat".into(),
        filter: None,
        ids: None,
    };
    let entry = |class, mode, speed_kmh| ScenarioEntry {
        class,
        mode,
        speed_kmh,
        label: None,
    };
    ReferralJob {
        schema_version: 1,
        dem: input.join("dem.tif"),
        land_cover: input.join("land_cover.tif"),
        origins: facilities.clone(),
        destinations: facilities,
        // replay/examples/demo_referral/config.json
        scenario: vec![
            entry(1, TransportMode::Bicycling, 6.0),
            entry(2, TransportMode::Walking, 4.0),
            entry(3, TransportMode::Walking, 2.0),
            entry(1001, TransportMode::Motorized, 100.0),
            entry(1002, TransportMode::Motorized, 80.0),
            entry(1003, TransportMode::Motorized, 50.0),
        ],
        method: MethodName::Anisotropic,
        neighbourhood,
        limit_s: None,
        mobility_model: Default::default(),
        nearest_only: false,
        self_pairs: false,
        search_from: Default::default(),
        facility_issues: FacilityIssues::Error,
        output_dir: out.to_path_buf(),
    }
}

#[test]
fn demo_referral_matches_accessmod5() {
    let oracle = root().join("referral/demo/oracle");
    if !oracle_available(&[&oracle, &root().join("travel-time/demo/input")]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let settings = settings(256 << 20, 4);
    let mut failures = Vec::new();
    for (name, neighbourhood) in [("8", Neighbourhood::Eight), ("16", Neighbourhood::Sixteen)] {
        let out = tmp.path().join(name);
        run_job::<Referral>(job(neighbourhood, &out), &settings).expect("run");
        let ours = pairs(
            &out.join("referral.csv"),
            ["origin_id", "destination_id", "time_s", "distance_m"],
        );
        let theirs = pairs(
            &oracle.join(format!("tReferral-{name}.csv")),
            ["from__cat", "to__cat", "time_s", "distance_m"],
        );
        let mut time_mismatch = 0;
        let mut max_distance_rel = 0.0f64;
        let mut max_distance_abs = 0.0f64;
        for (pair, (time, distance)) in &theirs {
            let Some((our_time, our_distance)) = ours.get(pair) else {
                failures.push(format!("{name}: pair {pair:?} missing"));
                continue;
            };
            // AccessMod 5 rounds times up to whole seconds.
            if our_time.ceil() != *time {
                time_mismatch += 1;
                eprintln!("{name} {pair:?}: time {our_time} vs {time}");
            }
            let abs = (our_distance - distance).abs();
            max_distance_abs = max_distance_abs.max(abs);
            max_distance_rel = max_distance_rel.max(abs / distance.max(1.0));
        }
        eprintln!(
            "{name} neighbours: {} pairs (AM5 {}), time mismatches {time_mismatch}, \
             distance max abs {max_distance_abs:.1} m, max rel {max_distance_rel:.2e}",
            ours.len(),
            theirs.len()
        );
        if ours.len() != theirs.len() {
            failures.push(format!("{name}: {} pairs vs {}", ours.len(), theirs.len()));
        }
        if time_mismatch > 0 {
            failures.push(format!("{name}: {time_mismatch} time mismatches"));
        }
        // Distances: ours run between cell centres, AccessMod 5 measures r.path
        // lines from the facility points (half a cell or so at each end, both
        // signs). Bound: one cell diagonal (demo cells are 1000 m).
        if max_distance_abs > 2f64.sqrt() * 1000.0 {
            failures.push(format!(
                "{name}: distance differs by {max_distance_abs:.1} m"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
