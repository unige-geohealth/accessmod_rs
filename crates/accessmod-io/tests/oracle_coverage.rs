//! Comparison with the AccessMod 5 geographic coverage (replay
//! `replay/examples/loop_capacity_stat` on the demo project), exported by
//! `scripts/oracle/export.sh` into `_oracle/capacity/demo`
//! (`scripts/oracle/capacity-demo.R`). Skips, with a message, when the
//! export is absent.

use std::collections::BTreeMap;
use std::path::Path;

use accessmod_core::cost::{Direction, Neighbourhood};
use accessmod_core::mobility::TransportMode;
use accessmod_io::coverage::{
    CAPACITY_FILE, CATCHMENTS_FILE, CoverageJob, CoverageOrder, GeographicCoverage, RESIDUAL_FILE,
};
use accessmod_io::fixtures::{oracle_available, oracle_dir, read_valid, run_job, settings};
use accessmod_io::job::{
    FacilityIssues, FacilitySource, MethodName, SCHEMA_VERSION, ScenarioEntry,
};
use gdal::vector::LayerAccess;

/// Rows of a CSV keyed by `key`, every field by column name.
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
            // Names hold no comma in the demo.
            let fields: BTreeMap<String, String> = header
                .iter()
                .cloned()
                .zip(line.split(',').map(|f| f.trim_matches('"').to_owned()))
                .collect();
            (fields[key].clone(), fields)
        })
        .collect()
}

/// Catchment area per facility identifier.
fn areas(path: &Path, field: &str) -> BTreeMap<String, f64> {
    let dataset = gdal::Dataset::open(path).expect("catchments");
    let mut layer = dataset.layer(0).expect("layer");
    let index = layer.defn().field_index(field).expect("field");
    layer
        .features()
        .map(|f| {
            let id = f.field_as_string(index).expect("field").expect("id");
            (id, f.geometry().expect("geometry").area())
        })
        .collect()
}

#[test]
fn demo_coverage_matches_accessmod5() {
    let root = oracle_dir("");
    let oracle = root.join("capacity/demo/oracle");
    if !oracle_available(&[&oracle, &root.join("travel-time/demo/input")]) {
        return;
    }
    let input = root.join("travel-time/demo/input");
    let entry = |class, mode, speed_kmh| ScenarioEntry {
        class,
        mode,
        speed_kmh,
        label: None,
    };
    let tmp = tempfile::tempdir().expect("tmp");
    // replay/examples/loop_capacity_stat/config.json
    let job = CoverageJob {
        schema_version: SCHEMA_VERSION,
        dem: input.join("dem.tif"),
        land_cover: input.join("land_cover.tif"),
        population: root.join("population-correction/demo/input/population.tif"),
        facilities: FacilitySource {
            path: input.join("facilities.fgb"),
            layer: None,
            id_field: "cat".into(),
            filter: None,
        },
        capacity_field: Some("capacity".into()),
        name_field: Some("name".into()),
        order: CoverageOrder::Field {
            field: "capacity".into(),
            descending: true,
        },
        scenario: vec![
            entry(1, TransportMode::Bicycling, 6.0),
            entry(2, TransportMode::Walking, 4.0),
            entry(3, TransportMode::Walking, 2.0),
            entry(1001, TransportMode::Motorized, 100.0),
            entry(1002, TransportMode::Motorized, 80.0),
            entry(1003, TransportMode::Motorized, 50.0),
        ],
        method: MethodName::Anisotropic,
        direction: Direction::TowardsFacilities,
        neighbourhood: Neighbourhood::Eight,
        limit_s: 7200.0,
        band_s: 60.0,
        mobility_model: Default::default(),
        ignore_capacity: false,
        remove_covered: true,
        catchments: true,
        facility_issues: FacilityIssues::Error,
        output_dir: tmp.path().join("out"),
    };
    let out = run_job::<GeographicCoverage>(job, &settings(256 << 20, 2)).expect("run");

    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0);
    let ours = table(&out.join(CAPACITY_FILE), "facility_id");
    let theirs = table(&oracle.join("capacity.csv"), "cat");
    assert_eq!(ours.len(), theirs.len());
    for (id, am5) in &theirs {
        let row = &ours[id];
        let n = |fields: &BTreeMap<String, String>, k: &str| {
            fields[k].parse::<f64>().unwrap_or(f64::NAN)
        };
        assert_eq!(row["order"], am5["amOrderComputed"], "facility {id}: order");
        for (our, their, scale) in [
            ("population_within_limit", "amPopTravelTimeMax", 1.0),
            ("catchment_travel_time_s", "amTravelTimeCatchment", 60.0),
            ("population_catchment", "amPopCatchmentTotal", 1.0),
            ("capacity_realised", "amCapacityRealised", 1.0),
            ("capacity_residual", "amCapacityResidual", 1.0),
            (
                "population_catchment_not_covered",
                "amPopCatchmentDiff",
                1.0,
            ),
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

    let (ours, theirs) = (
        areas(&out.join(CATCHMENTS_FILE), "facility_id"),
        areas(&oracle.join("catchments.fgb"), "cat_join"),
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
