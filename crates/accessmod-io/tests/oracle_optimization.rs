//! Comparison with the AccessMod 5 optimization (`tests/optimization` of
//! AccessMod 5 on the demo project, with and without the zone minimum, and
//! 15 km discs around the facilities), exported by
//! `scripts/oracle/export.sh` into `_oracle/optimization/demo`
//! (`scripts/oracle/optimization-demo.R`). The job is the benchmark's
//! (`benchmark/optimization/demo-job.json`). Skips, with a message, when the
//! export is absent.

use std::path::{Path, PathBuf};

use accessmod_io::fixtures::{csv_table, oracle_available, oracle_dir, run_job, settings};
use accessmod_io::job::JobFile;
use accessmod_io::optimization::{OPTIMIZATION_FILE, Optimization, OptimizationJob};

/// AccessMod 5 rounds each extracted population to single precision
/// (`exactextractr` on a Float32 raster); ours stays in double precision.
fn single_precision_equal(ours: f64, am5: f64) -> bool {
    (ours - am5).abs() <= 2f64.powi(-23) * am5.abs().max(1.0)
}

/// Run `job` and compare its table with AccessMod 5's `oracle` table.
fn compare(job: OptimizationJob, oracle: &Path, zones: bool) {
    let out = run_job::<Optimization>(job, &settings(256 << 20, 1)).expect("run");
    let ours = csv_table(&out.join(OPTIMIZATION_FILE), "rank");
    let theirs = csv_table(oracle, "amRank");
    assert_eq!(ours.len(), theirs.len(), "{}: rows", oracle.display());
    let n = |s: &str| s.parse::<f64>().expect("number");
    for (rank, am5) in &theirs {
        let row = &ours[rank];
        assert_eq!(row["facility_id"], am5["amFacilityName"], "rank {rank}");
        for (our, their) in [
            ("population_catchment", "amPopCatchmentTotal"),
            ("population_covered", "amPopCovered"),
            ("population_covered_cumulative", "amPopCoveredCumul"),
        ] {
            let (a, b) = (n(&row[our]), n(&am5[their]));
            assert!(
                single_precision_equal(a, b),
                "rank {rank}: {our} {a} vs {their} {b}"
            );
        }
        if zones {
            assert_eq!(row["zone_label"], am5["amAdminRegion"], "rank {rank}");
            assert_eq!(row["zone_id"], am5["amAdminId"], "rank {rank}");
        }
    }
}

fn demo_job(out: &Path) -> Option<(OptimizationJob, PathBuf)> {
    let root = oracle_dir("");
    let oracle = root.join("optimization/demo/oracle");
    let job_file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmark/optimization/demo-job.json");
    if !oracle_available(&[
        &oracle,
        &root.join("population-correction/demo/input"),
        &root.join("zonal-statistics/demo/input"),
        &root.join("travel-time/demo/input"),
    ]) {
        return None;
    }
    let mut job = OptimizationJob::from_file(&job_file).expect("job");
    job.output_dir = out.to_path_buf();
    Some((job, oracle))
}

#[test]
fn demo_optimization_with_zone_minimum_matches_accessmod5() {
    let tmp = tempfile::tempdir().expect("tmp");
    let Some((job, oracle)) = demo_job(&tmp.path().join("out")) else {
        return;
    };
    compare(job, &oracle.join("optimization_admin.csv"), true);
}

#[test]
fn demo_optimization_without_zones_matches_accessmod5() {
    let tmp = tempfile::tempdir().expect("tmp");
    let Some((mut job, oracle)) = demo_job(&tmp.path().join("out")) else {
        return;
    };
    job.zone_quota = None;
    compare(job, &oracle.join("optimization_no_admin.csv"), false);
}

#[test]
fn overlapping_discs_match_accessmod5() {
    let tmp = tempfile::tempdir().expect("tmp");
    let Some((mut job, oracle)) = demo_job(&tmp.path().join("out")) else {
        return;
    };
    // Discs do not follow the cells: exact coverage fractions and polygon
    // differences decide every value.
    job.zone_quota = None;
    job.catchments.path = oracle.join("discs.fgb");
    compare(job, &oracle.join("optimization_discs.csv"), false);
}
