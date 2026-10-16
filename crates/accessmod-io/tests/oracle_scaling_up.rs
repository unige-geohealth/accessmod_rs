//! Comparison with the AccessMod 5 scaling up (replay
//! `replay/examples/demo_scaling_up` on the demo project), exported by
//! `scripts/oracle/export.sh` into `_oracle/scaling-up/demo`
//! (`scripts/oracle/scaling-up-demo.R`). The job is the benchmark's
//! (`benchmark/scaling-up/demo-job.json`). Skips, with a message, when the
//! export is absent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use accessmod_io::fixtures::{oracle_available, oracle_dir, read_valid, run_job, settings};
use accessmod_io::job::JobFile;
use accessmod_io::scaling_up::{
    CAPACITY_FILE, CATCHMENTS_FILE, FACILITIES_FILE, RESIDUAL_FILE, ScalingUp, ScalingUpJob,
};
use gdal::vector::LayerAccess;

/// Rows of a CSV keyed by `key`, every field by column name (no quoted
/// commas in these tables).
fn table(path: &Path, key: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .expect("header")
        .split(',')
        .map(|h| h.trim_matches('"').to_owned())
        .collect();
    lines
        .map(|line| {
            let fields: BTreeMap<String, String> = header
                .iter()
                .cloned()
                .zip(line.split(',').map(|f| f.trim_matches('"').to_owned()))
                .collect();
            (fields[key].clone(), fields)
        })
        .collect()
}

/// Feature attribute `key` of a layer, to a value of each feature.
fn features<T>(
    path: &Path,
    key: &str,
    value: impl Fn(&gdal::vector::Feature) -> T,
) -> BTreeMap<String, T> {
    let dataset = gdal::Dataset::open(path).expect("dataset");
    let mut layer = dataset.layer(0).expect("layer");
    let index = layer.defn().field_index(key).expect("field");
    layer
        .features()
        .filter_map(|f| {
            let id = f
                .field_as_string(index)
                .expect("field")
                .filter(|s| !s.is_empty())?;
            Some((id, value(&f)))
        })
        .collect()
}

#[test]
fn demo_scaling_up_matches_accessmod5() {
    let root = oracle_dir("");
    let oracle = root.join("scaling-up/demo/oracle");
    let job_file =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark/scaling-up/demo-job.json");
    if !oracle_available(&[&oracle, &root.join("travel-time/demo/input")]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let mut job = ScalingUpJob::from_file(&job_file).expect("job");
    job.output_dir = tmp.path().join("out");
    let out = run_job::<ScalingUp>(job, &settings(256 << 20, 2)).expect("run");

    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0);
    let n =
        |fields: &BTreeMap<String, String>, k: &str| fields[k].parse::<f64>().unwrap_or(f64::NAN);
    let ours = table(&out.join(CAPACITY_FILE), "facility_id");
    let theirs = table(&oracle.join("capacity.csv"), "amId");
    assert_eq!(ours.len(), theirs.len(), "facilities added");
    for (id, am5) in &theirs {
        let row = &ours[id];
        assert_eq!(row["label"], am5["amLabel"], "facility {id}: class");
        for (our, their, scale) in [
            ("capacity", "amCapacity", 1.0),
            ("population_within_limit", "amPopTravelTimeMax", 1.0),
            ("population_band_correlation", "amCorrPopTime", 1.0),
            ("catchment_travel_time_s", "amTravelTimeCatchment", 60.0),
            ("population_catchment", "amPopCatchmentTotal", 1.0),
            ("capacity_realised", "amCapacityRealised", 1.0),
            ("capacity_residual", "amCapacityResidual", 1.0),
            (
                "population_catchment_not_covered",
                "amPopCatchmentDiff",
                1.0,
            ),
            ("population_residual_before", "amPopResidualBefore", 1.0),
            ("population_residual_after", "amPopResidualAfter", 1.0),
            ("population_covered_percent", "amPopCoveredPercent", 1.0),
        ] {
            let (a, b) = (n(row, our), n(am5, their) * scale);
            // AM5 rounds the not-covered population to 6 decimals.
            assert!(
                close(a, b) || (a - b).abs() < 1e-6,
                "facility {id}: {our} {a} vs {their} {b}"
            );
        }
    }

    // Same cells: AM5 writes the existing facilities too, without amId.
    let point = |f: &gdal::vector::Feature| {
        let (x, y, _) = f.geometry().expect("geometry").get_point(0);
        (x, y)
    };
    let (ours, theirs) = (
        features(&out.join(FACILITIES_FILE), "facility_id", point),
        features(&oracle.join("facilities.fgb"), "amId", point),
    );
    assert_eq!(ours.len(), theirs.len());
    for (id, (x, y)) in &theirs {
        let (a, b) = ours[id];
        assert!(
            (a - x).abs() < 1e-6 && (b - y).abs() < 1e-6,
            "facility {id}: ({a}, {b}) vs ({x}, {y})"
        );
    }

    let (ours, theirs) = (
        read_valid(&out.join(RESIDUAL_FILE)),
        read_valid(&oracle.join("population_residual.tif")),
    );
    for (a, b) in ours.iter().zip(&theirs) {
        match (a, b) {
            (Some(a), Some(b)) => assert!(close(*a, *b), "residual {a} vs {b}"),
            (None, None) => {}
            _ => panic!("residual NoData masks differ"),
        }
    }

    let area = |f: &gdal::vector::Feature| f.geometry().expect("geometry").area();
    let (ours, theirs) = (
        features(&out.join(CATCHMENTS_FILE), "facility_id", area),
        features(&oracle.join("catchments.fgb"), "amId", area),
    );
    assert_eq!(ours.len(), theirs.len());
    for (id, area) in &theirs {
        assert!(
            close(ours[id], *area),
            "catchment {id}: {} vs {area}",
            ours[id]
        );
    }
}
