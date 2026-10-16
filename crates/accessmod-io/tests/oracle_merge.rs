//! Comparison with the AccessMod 5 "Merge land cover" toolbox on the demo
//! project, exported by `scripts/oracle/export.sh` into
//! `_oracle/merge-land-cover/demo` (`scripts/oracle/merge-land-cover-demo.sh`).
//! Skips, with a message, when the export is absent.

use std::path::Path;

use accessmod_io::fixtures::{oracle_available, oracle_dir, read_valid, run_job, settings};
use accessmod_io::job::SCHEMA_VERSION;
use accessmod_io::merge::{
    BRIDGES_FILE, BarrierItem, LandCoverItem, MERGED_FILE, MergeLandCover, MergeLandCoverJob,
    RoadItem, StackItem,
};

fn root() -> std::path::PathBuf {
    oracle_dir("merge-land-cover/demo")
}

fn differing_cells(ours: &Path, theirs: &Path) -> usize {
    let (a, b) = (read_valid(ours), read_valid(theirs));
    assert_eq!(a.len(), b.len(), "grid sizes differ");
    a.iter().zip(&b).filter(|(x, y)| x != y).count()
}

fn job(classes: Vec<i64>, clean_bridges: bool, out: &Path) -> MergeLandCoverJob {
    let input = root().join("input");
    let barrier = |name: &str| {
        StackItem::Barrier(BarrierItem {
            path: input.join(name),
            layer: None,
            filter: None,
        })
    };
    MergeLandCoverJob {
        schema_version: SCHEMA_VERSION,
        dem: input.join("dem.tif"),
        stack: vec![
            StackItem::Road(RoadItem {
                path: input.join("roads.fgb"),
                layer: None,
                class_field: "class_n".into(),
                label_field: Some("label".into()),
                filter: None,
                classes: Some(classes),
                add_1000: true,
            }),
            StackItem::LandCover(LandCoverItem {
                path: input.join("land_cover.tif"),
                labels: Vec::new(),
                reclass: Vec::new(),
            }),
            barrier("river.fgb"),
            barrier("wetland.fgb"),
        ],
        clean_bridges,
        output_dir: out.to_path_buf(),
    }
}

#[test]
fn demo_merge_matches_accessmod5() {
    let oracle = root().join("oracle");
    if !oracle_available(&[&oracle]) {
        return;
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let settings = settings(256 << 20, 2);
    // Line rasterisation differs from v.to.rast -d on a few cells (lines
    // passing near a cell corner, docs/merge-land-cover.md): on the demo,
    // 1 of about 1 320 road cells and 1 of about 1 680 river cells. The
    // merged rasters differ on 2 cells (2026-10-16).
    const MAX_DIFFERING_CELLS: usize = 4;
    let mut failures = Vec::new();
    for (name, classes, clean) in [
        ("roads_first", vec![1, 2, 3], false),
        ("roads_first", vec![1, 2, 3], true),
        ("roads_reversed", vec![3, 2, 1], true),
    ] {
        let label = format!("{name}{}", if clean { " cleaned" } else { "" });
        let out = tmp.path().join(label.replace(' ', "-"));
        run_job::<MergeLandCover>(job(classes, clean, &out), &settings).expect("run");
        let reference = if clean {
            oracle.join(format!("merged-cleaned-{name}.tif"))
        } else {
            oracle.join(format!("merged-{name}.tif"))
        };
        let merged = differing_cells(&out.join(MERGED_FILE), &reference);
        let bridges = if clean {
            differing_cells(
                &out.join(BRIDGES_FILE),
                &oracle.join(format!("bridges-{name}.tif")),
            )
        } else {
            0
        };
        eprintln!("{label}: merged differs on {merged} cells, bridges on {bridges}");
        if merged > MAX_DIFFERING_CELLS || bridges > MAX_DIFFERING_CELLS {
            failures.push(format!(
                "{label}: merged differs on {merged} cells, bridges on {bridges}"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
