//! Comparison with the AccessMod 5 toolbox "Adjust population distribution"
//! on the demo project, exported by `scripts/oracle/export.sh` into
//! `_oracle/population-correction/demo`
//! (`scripts/oracle/population-correction-demo.sh`). Skips, with a message,
//! when the export is absent.

use std::collections::BTreeMap;
use std::path::Path;

use accessmod_io::fixtures::{oracle_available, oracle_dir, read_valid, run_job, settings};
use accessmod_io::job::SCHEMA_VERSION;
use accessmod_io::population::{
    AdjustPopulation, AdjustPopulationJob, POPULATION_FILE, ZONES_FILE, ZoneSource,
};

/// `column` of a CSV keyed by its first column (no quoting in these files).
fn column(path: &Path, column: &str) -> BTreeMap<String, Option<f64>> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let at = header.iter().position(|h| *h == column).expect("column");
    lines
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            (fields[0].to_owned(), fields[at].parse().ok())
        })
        .collect()
}

#[test]
fn demo_adjustment_matches_accessmod5() {
    let root = oracle_dir("population-correction/demo");
    if !oracle_available(&[&root.join("oracle")]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let input = root.join("input");
    for (case, field) in [("estimated", None), ("known", Some("pop_known"))] {
        let job = AdjustPopulationJob {
            schema_version: SCHEMA_VERSION,
            dem: input.join("dem.tif"),
            population: input.join("population.tif"),
            land_cover: input.join("land_cover.tif"),
            zones: ZoneSource {
                path: input.join("zones.fgb"),
                layer: None,
                id_field: "cat".into(),
                filter: None,
                population_field: field.map(str::to_owned),
            },
            output_dir: tmp.path().join(case),
        };
        let out = run_job::<AdjustPopulation>(job, &settings(256 << 20, 2)).expect("run");
        let ours = read_valid(&out.join(POPULATION_FILE));
        let theirs = read_valid(&root.join(format!("oracle/population-{case}.tif")));
        let mut max_rel = 0.0f64;
        for (a, b) in ours.iter().zip(&theirs) {
            match (a, b) {
                (Some(a), Some(b)) => {
                    max_rel = max_rel.max((a - b).abs() / a.abs().max(b.abs()).max(1e-300));
                }
                (None, None) => {}
                _ => panic!("{case}: NoData masks differ"),
            }
        }
        // Sums are added in a different order: relative 1e-15, not bits.
        assert!(max_rel < 1e-12, "{case}: relative difference {max_rel:e}");
        let our_ratios = column(&out.join(ZONES_FILE), "ratio");
        let their_ratios = column(
            &root.join(format!("oracle/zones-{case}.csv")),
            "am_pop_ratio",
        );
        assert_eq!(our_ratios.len(), their_ratios.len());
        for (zone, theirs) in &their_ratios {
            let (ours, theirs) = (our_ratios[zone].expect("ratio"), theirs.expect("ratio"));
            assert!(
                (ours - theirs).abs() <= 1e-12 * theirs,
                "{case} zone {zone}: {ours} vs {theirs}"
            );
        }
        eprintln!("{case}: max relative difference {max_rel:.1e}");
    }
}
