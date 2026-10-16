//! Zonal statistics on small fixtures built with GDAL.

use std::path::Path;

use accessmod_io::fixtures::{
    EPSG, X0, Y0, cell_polygon, geo_transform, never, run_job, settings, write_features,
    write_raster,
};
use accessmod_io::job::SCHEMA_VERSION;
use accessmod_io::output::FLOAT_NODATA;
use accessmod_io::zonal::{
    ZONAL_FILE, ZonalStatistics, ZonalStatisticsJob, ZoneLayer, prepare, run,
};
use gdal::vector::{FieldValue, OGRFieldType};

/// Zones `(id, label, polygon)` in a layer `zones`.
fn write_zones(path: &Path, zones: &[(i64, &str, String)]) {
    let features: Vec<(&str, Vec<Option<FieldValue>>)> = zones
        .iter()
        .map(|(id, label, wkt)| {
            (
                wkt.as_str(),
                vec![
                    Some(FieldValue::Integer64Value(*id)),
                    Some(FieldValue::StringValue((*label).into())),
                ],
            )
        })
        .collect();
    write_features(
        path,
        "zones",
        EPSG,
        &[
            ("id", OGRFieldType::OFTInteger64),
            ("name", OGRFieldType::OFTString),
        ],
        &features,
    );
}

fn job(dir: &Path, limits: Vec<f64>) -> ZonalStatisticsJob {
    ZonalStatisticsJob {
        schema_version: SCHEMA_VERSION,
        dem: dir.join("dem.tif"),
        travel_time: dir.join("tt.tif"),
        population: dir.join("pop.tif"),
        zones: ZoneLayer {
            path: dir.join("zones.gpkg"),
            layer: None,
            id_field: "id".into(),
            label_field: Some("name".into()),
            filter: None,
        },
        time_limits_s: limits,
        output_dir: dir.join("out"),
    }
}

#[test]
fn coverage_by_hand() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path();
    let gt = geo_transform(X0, Y0);
    write_raster::<f32>(&path.join("dem.tif"), 4, 1, gt, vec![0.0; 4], None);
    // Times: 0 s, 600 s, 1200 s, not reached.
    let tt = vec![0.0, 600.0, 1200.0, FLOAT_NODATA];
    write_raster::<f64>(&path.join("tt.tif"), 4, 1, gt, tt, Some(FLOAT_NODATA));
    write_raster::<f32>(
        &path.join("pop.tif"),
        4,
        1,
        gt,
        vec![10.0, 20.0, 30.0, 40.0],
        None,
    );
    // Zone 7 is two features (cells 0 and 1); zone 9 holds cells 2 and 3.
    write_zones(
        &path.join("zones.gpkg"),
        &[
            (7, "west", cell_polygon(0.0, 0.0, 1.0, 1.0)),
            (9, "east", cell_polygon(2.0, 0.0, 4.0, 1.0)),
            (7, "", cell_polygon(1.0, 0.0, 2.0, 1.0)),
        ],
    );
    let out = run_job::<ZonalStatistics>(
        job(path, vec![1200.0, 600.0, 600.0]),
        &settings(256 << 20, 1),
    )
    .expect("run");
    let table = std::fs::read_to_string(out.join(ZONAL_FILE)).expect("table");
    assert_eq!(
        table,
        "zone_id,label,time_limit_s,population,population_covered,coverage_percent\n\
         9,east,600,70,0,0\n\
         7,west,600,30,30,100\n\
         9,east,1200,70,30,42.857142857142854\n\
         7,west,1200,30,30,100\n"
    );
}

#[test]
fn outputs_do_not_depend_on_memory() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path();
    let (cols, rows) = (900, 700);
    let gt = geo_transform(X0, Y0);
    write_raster::<f32>(
        &path.join("dem.tif"),
        cols,
        rows,
        gt,
        vec![0.0; cols * rows],
        None,
    );
    let tt: Vec<f64> = (0..cols * rows)
        .map(|i| {
            if i % 11 == 0 {
                FLOAT_NODATA
            } else {
                (i % 5000) as f64
            }
        })
        .collect();
    write_raster::<f64>(&path.join("tt.tif"), cols, rows, gt, tt, Some(FLOAT_NODATA));
    let pop: Vec<f32> = (0..cols * rows).map(|i| (i % 97) as f32 * 0.37).collect();
    write_raster::<f32>(&path.join("pop.tif"), cols, rows, gt, pop, None);
    let zones: Vec<(i64, &str, String)> = (0..6)
        .map(|k| {
            let c0 = (k % 3) as f64 * 300.0 + 0.3;
            let r0 = (k / 3) as f64 * 350.0 + 0.6;
            (
                k as i64 % 4,
                "z",
                cell_polygon(c0, r0, c0 + 310.0, r0 + 340.0),
            )
        })
        .collect();
    write_zones(&path.join("zones.gpkg"), &zones);
    let job = |out: &str| {
        let mut job = job(path, vec![1000.0, 3000.0]);
        job.output_dir = path.join(out);
        job
    };
    // 21.5 MiB: part of the 165 zone tiles resident.
    let tight_budget = 43 << 19;
    let tight = prepare(job("a"), &settings(tight_budget, 1), &never).expect("prepare");
    assert!(tight.plan.resident_tiles < tight.plan.total_tiles);
    let (out_a, report_a) = run(tight, &settings(tight_budget, 1), &never).expect("run a");
    assert!(
        report_a.execution.details.store.tile_writes > 0,
        "scratch not exercised"
    );
    let out_b = run_job::<ZonalStatistics>(job("b"), &settings(4 << 30, 4)).expect("run b");
    let a = std::fs::read(out_a.join(ZONAL_FILE)).expect("read a");
    let b = std::fs::read(out_b.join(ZONAL_FILE)).expect("read b");
    assert!(a == b, "table differs between budgets");
}
