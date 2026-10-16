//! Comparison with the AccessMod 5 zonal statistics on the demo project,
//! exported by `scripts/oracle/export.sh` into `_oracle/zonal-statistics/demo`
//! (`scripts/oracle/zonal-statistics-demo.sh`). Skips, with a message, when
//! the export is absent.

use std::collections::BTreeMap;
use std::path::Path;

use accessmod_io::fixtures::{map_raster, oracle_available, oracle_dir, run_job, settings};
use accessmod_io::job::SCHEMA_VERSION;
use accessmod_io::zonal::{ZONAL_FILE, ZonalStatistics, ZonalStatisticsJob, ZoneLayer};

/// `sum` per zone of an `r.univar -t` CSV.
fn sums(path: &Path) -> BTreeMap<String, f64> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let at = header.iter().position(|h| *h == "sum").expect("sum");
    lines
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f[0].to_owned(), f[at].parse().expect("number"))
        })
        .collect()
}

#[test]
fn demo_zonal_statistics_match_accessmod5() {
    let root = oracle_dir("zonal-statistics/demo");
    if !oracle_available(&[&root.join("oracle")]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let input = root.join("input");
    // AccessMod 5 stores whole minutes; the same times in seconds.
    let seconds = tmp.path().join("travel_time_seconds.tif");
    map_raster(&input.join("travel_time_minutes.tif"), &seconds, |m| {
        m * 60.0
    });
    // AM5 covers t < L minutes; on whole minutes that is t <= (L - 1) x 60 s.
    let limits_min = [30.0, 60.0];
    let job = ZonalStatisticsJob {
        schema_version: SCHEMA_VERSION,
        dem: input.join("dem.tif"),
        travel_time: seconds,
        population: input.join("population.tif"),
        zones: ZoneLayer {
            path: input.join("zones.fgb"),
            layer: None,
            id_field: "cat".into(),
            label_field: Some("admin_name".into()),
            filter: None,
        },
        time_limits_s: limits_min.iter().map(|l| (l - 1.0) * 60.0).collect(),
        output_dir: tmp.path().join("out"),
    };
    let out = run_job::<ZonalStatistics>(job, &settings(256 << 20, 2)).expect("run");
    let text = std::fs::read_to_string(out.join(ZONAL_FILE)).expect("table");
    // (zone, limit_s) -> (population, covered)
    let ours: BTreeMap<(String, String), (f64, f64)> = text
        .lines()
        .skip(1)
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            let n = |i: usize| f[i].parse::<f64>().expect("number");
            ((f[0].to_owned(), f[2].to_owned()), (n(3), n(4)))
        })
        .collect();
    let population = sums(&root.join("oracle/population.csv"));
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b.abs().max(1.0);
    for limit in limits_min {
        let covered = sums(&root.join(format!("oracle/covered-{limit}.csv")));
        let limit_s = format!("{}", (limit - 1.0) * 60.0);
        for (zone, total) in &population {
            let (our_total, our_covered) = ours[&(zone.clone(), limit_s.clone())];
            assert!(
                close(our_total, *total),
                "zone {zone}: {our_total} vs {total}"
            );
            let theirs = covered.get(zone).copied().unwrap_or(0.0);
            assert!(
                close(our_covered, theirs),
                "zone {zone} within {limit} min: {our_covered} vs {theirs}"
            );
        }
    }
}
