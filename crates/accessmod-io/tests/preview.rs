//! Preview data: external overviews that leave the raster untouched, and
//! statistics by hand.

use accessmod_io::fixtures::{X0, Y0, geo_transform, write_raster};
use accessmod_io::preview::{
    HISTOGRAM_BINS, Resampling, ValueCount, build, overview_path, statistics,
};
use gdal::Dataset;

#[test]
fn class_raster_gets_overviews_and_class_counts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("land_cover.tif");
    let (cols, rows) = (600, 400);
    // Classes 1, 2, 3 by column thirds; the first row is NoData.
    let data: Vec<i32> = (0..rows * cols)
        .map(|i| {
            if i < cols {
                -1
            } else {
                1 + (i % cols / 200) as i32
            }
        })
        .collect();
    write_raster(&path, cols, rows, geo_transform(X0, Y0), data, Some(-1.0));
    let before = std::fs::read(&path).expect("read");

    let preview = build(&path, Resampling::Nearest, 2, &|| false).expect("preview");

    assert_eq!(preview.overviews, vec![2, 4]);
    assert_eq!(
        std::fs::read(&path).expect("read"),
        before,
        "raster unchanged"
    );
    assert!(overview_path(&path).exists());
    let dataset = Dataset::open(&path).expect("open");
    let band = dataset.rasterband(1).expect("band");
    assert_eq!(band.overview_count().expect("count"), 2);
    let overview = band.overview(0).expect("overview");
    assert_eq!(overview.size(), (300, 200));

    let stats = preview.stats;
    let per_class = (rows as u64 - 1) * 200;
    assert_eq!(stats.cells, (cols * rows) as u64);
    assert_eq!(stats.valid_cells, 3 * per_class);
    assert_eq!((stats.min, stats.max), (Some(1.0), Some(3.0)));
    assert_eq!(stats.mean, Some(2.0));
    assert_eq!(stats.histogram.len(), HISTOGRAM_BINS);
    assert_eq!(stats.histogram[0], per_class);
    assert_eq!(stats.histogram[HISTOGRAM_BINS / 2], per_class);
    assert_eq!(stats.histogram[HISTOGRAM_BINS - 1], per_class);
    assert_eq!(
        stats.values,
        Some(
            (1..=3)
                .map(|value| ValueCount {
                    value,
                    count: per_class
                })
                .collect()
        )
    );
}

#[test]
fn continuous_raster_has_no_class_counts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("travel_time.tif");
    let data: Vec<f64> = (0..20).map(f64::from).chain([f64::NAN]).collect();
    write_raster(&path, 7, 3, geo_transform(X0, Y0), data, None);
    let stats = statistics(&path, &|| false).expect("stats");
    assert_eq!(stats.valid_cells, 20);
    assert_eq!((stats.min, stats.max), (Some(0.0), Some(19.0)));
    assert_eq!(stats.mean, Some(9.5));
    assert_eq!(stats.histogram.iter().sum::<u64>(), 20);
    assert_eq!(stats.values, None);
    // Small enough to draw directly: no overviews.
    let preview = build(&path, Resampling::Average, 1, &|| false).expect("preview");
    assert!(preview.overviews.is_empty());
    assert!(!overview_path(&path).exists());
}

#[test]
fn empty_raster_has_no_range() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("empty.tif");
    write_raster(&path, 4, 4, geo_transform(X0, Y0), vec![0u8; 16], Some(0.0));
    let stats = statistics(&path, &|| false).expect("stats");
    assert_eq!(stats.valid_cells, 0);
    assert_eq!(stats.min, None);
    assert!(stats.histogram.is_empty());
}

#[test]
fn cancelled_overviews_leave_nothing_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("dem.tif");
    write_raster(
        &path,
        1024,
        1024,
        geo_transform(X0, Y0),
        vec![1.5f32; 1024 * 1024],
        None,
    );
    let err = accessmod_io::preview::build_overviews(&path, Resampling::Average, 1, &|| true)
        .expect_err("cancelled");
    assert!(
        matches!(err, accessmod_io::error::Error::Cancelled),
        "{err}"
    );
    assert!(!overview_path(&path).exists());
}
