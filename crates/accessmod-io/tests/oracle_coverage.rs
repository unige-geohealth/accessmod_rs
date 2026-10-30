//! Comparison with the AccessMod 5 geographic coverage (replay
//! `replay/examples/loop_capacity_stat` on the demo project, and an extended
//! variant), exported by `scripts/oracle/export.sh` into `_oracle/capacity`
//! (`scripts/oracle/capacity-demo.R`). Skips, with a message, when the
//! export is absent.

use std::collections::BTreeMap;
use std::path::Path;

use accessmod_core::cost::{Direction, Neighbourhood};
use accessmod_core::mobility::TransportMode;
use accessmod_io::coverage::{
    BARRIERS_FILE, CAPACITY_FILE, CATCHMENTS_FILE, CoverageJob, CoverageOrder, GeographicCoverage,
    RESIDUAL_FILE, ZONES_FILE,
};
use accessmod_io::fixtures::{
    csv_table, oracle_available, oracle_dir, read_valid, run_job, settings,
};
use accessmod_io::job::{
    FacilityIssues, FacilitySource, MethodName, SCHEMA_VERSION, ScenarioEntry,
};
use accessmod_io::zonal::ZoneLayer;
use gdal::vector::LayerAccess;

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

/// The job of replay/examples/loop_capacity_stat/config.json.
fn demo_job(root: &Path, output_dir: &Path) -> CoverageJob {
    let input = root.join("travel-time/demo/input");
    let entry = |class, mode, speed_kmh| ScenarioEntry {
        class,
        mode,
        speed_kmh,
        label: None,
    };
    CoverageJob {
        schema_version: SCHEMA_VERSION,
        dem: input.join("dem.tif"),
        land_cover: input.join("land_cover.tif"),
        population: root.join("population-correction/demo/input/population.tif"),
        facilities: FacilitySource {
            path: input.join("facilities.fgb"),
            layer: None,
            id_field: "cat".into(),
            filter: None,
            ids: None,
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
        population_on_barriers: false,
        zones: None,
        facility_issues: FacilityIssues::Error,
        output_dir: output_dir.to_path_buf(),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn number(fields: &BTreeMap<String, String>, key: &str) -> f64 {
    fields[key].parse::<f64>().unwrap_or(f64::NAN)
}

/// Our capacity table against AccessMod 5's, with `extra` columns
/// `(ours, theirs)` on top of the common ones.
fn compare_capacity(ours: &Path, theirs: &Path, extra: &[(&str, &str)]) {
    let ours = csv_table(ours, "facility_id");
    let theirs = csv_table(theirs, "cat");
    assert_eq!(ours.len(), theirs.len());
    let common = [
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
    ];
    let extra = extra.iter().map(|&(a, b)| (a, b, 1.0));
    for (id, am5) in &theirs {
        let row = &ours[id];
        assert_eq!(row["order"], am5["amOrderComputed"], "facility {id}: order");
        for (our, their, scale) in common.into_iter().chain(extra.clone()) {
            let (a, b) = (number(row, our), number(am5, their) * scale);
            // AM5 rounds the not-covered population to 6 decimals.
            assert!(
                close(a, b) || (a - b).abs() < 1e-6,
                "facility {id}: {our} {a} vs {their} {b}"
            );
        }
    }
}

/// Two rasters: same NoData mask, values within tolerance.
fn compare_raster(ours: &Path, theirs: &Path) {
    let (ours, theirs) = (read_valid(ours), read_valid(theirs));
    assert_eq!(ours.len(), theirs.len());
    for (a, b) in ours.iter().zip(&theirs) {
        match (a, b) {
            (Some(a), Some(b)) => assert!(close(*a, *b), "{a} vs {b}"),
            (None, None) => {}
            _ => panic!("NoData masks differ: {a:?} vs {b:?}"),
        }
    }
}

#[test]
fn demo_coverage_matches_accessmod5() {
    let root = oracle_dir("");
    let oracle = root.join("capacity/demo/oracle");
    if !oracle_available(&[&oracle, &root.join("travel-time/demo/input")]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let job = demo_job(&root, &tmp.path().join("out"));
    let out = run_job::<GeographicCoverage>(job, &settings(256 << 20, 2)).expect("run");
    compare_capacity(&out.join(CAPACITY_FILE), &oracle.join("capacity.csv"), &[]);
    compare_raster(
        &out.join(RESIDUAL_FILE),
        &oracle.join("population_residual.tif"),
    );
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

/// Population order, population on barriers, extended columns and zonal
/// coverage (`_oracle/capacity/demo-extended`).
#[test]
fn demo_extended_coverage_matches_accessmod5() {
    let root = oracle_dir("");
    let oracle = root.join("capacity/demo-extended/oracle");
    let zones = root.join("zonal-statistics/demo/input/zones.fgb");
    if !oracle_available(&[&oracle, &root.join("travel-time/demo/input"), &zones]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let mut job = demo_job(&root, &tmp.path().join("out"));
    job.order = CoverageOrder::Population {
        limit_s: 7200.0,
        descending: true,
    };
    job.catchments = false;
    job.population_on_barriers = true;
    job.zones = Some(ZoneLayer {
        path: zones,
        layer: None,
        id_field: "cat".into(),
        label_field: Some("admin_name".into()),
        filter: None,
    });
    let out = run_job::<GeographicCoverage>(job, &settings(256 << 20, 2)).expect("run");
    compare_capacity(
        &out.join(CAPACITY_FILE),
        &oracle.join("capacity.csv"),
        &[
            ("order_value", "amOrderValues_popTravelTime120min"),
            ("population_residual_before", "amPopResidualBefore"),
            ("population_residual_after", "amPopResidualAfter"),
        ],
    );
    compare_raster(
        &out.join(RESIDUAL_FILE),
        &oracle.join("population_residual.tif"),
    );
    compare_raster(
        &out.join(BARRIERS_FILE),
        &oracle.join("population_on_barriers.tif"),
    );

    // Totals, repeated on every AM5 row.
    let run: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("run.json")).expect("run.json"))
            .expect("json");
    let result = &run["run"]["result"];
    let am5 = csv_table(&oracle.join("capacity.csv"), "cat");
    let am5 = am5.values().next().expect("a row");
    for (our, their) in [
        ("population_total", "amPopTotal"),
        ("population", "amPopTotalNotOnBarrier"),
        ("population_on_barriers", "amPopTotalOnBarrier"),
    ] {
        let a = result[our].as_f64().expect("number");
        let b = number(am5, their);
        assert!(close(a, b), "{our} {a} vs {their} {b}");
    }

    // AM5 counts people on barriers as covered (deviation
    // coverage_zonal_barriers).
    let ours = csv_table(&out.join(ZONES_FILE), "zone_id");
    let theirs = csv_table(&oracle.join("zonal.csv"), "cat");
    assert_eq!(ours.len(), theirs.len());
    for (id, am5) in &theirs {
        let row = &ours[id];
        assert_eq!(row["label"], am5["admin_name"], "zone {id}");
        let population = number(row, "population");
        let covered = number(row, "population_covered") + number(row, "population_on_barriers");
        for (a, b) in [
            (population, number(am5, "amPopSum")),
            (covered, number(am5, "amPopCovered")),
            (
                covered / population * 100.0,
                number(am5, "amPopCoveredPercent"),
            ),
        ] {
            assert!(close(a, b) || (a - b).abs() < 1e-6, "zone {id}: {a} vs {b}");
        }
    }
}
