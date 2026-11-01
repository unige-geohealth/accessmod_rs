//! Map previews: a real tile through GDALWarp, values at a point, GeoJSON.

use accessmod_io::fixtures::{EPSG, X0, Y0, geo_transform, grid, write_features, write_raster};
use accessmod_io::preview::{Resampling, build};
use accessmod_io::tiles::{
    Style, TILE, bounds_wgs84, colour, geojson, raster_tile, tile_at, value_at,
};
use gdal::vector::{FieldValue, OGRFieldType};

/// Decode a PNG tile into RGBA pixels.
fn pixels(png: &[u8]) -> Vec<[u8; 4]> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().expect("png header");
    let mut data = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut data).expect("png frame");
    assert_eq!((info.width, info.height), (TILE as u32, TILE as u32));
    data.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

#[test]
fn a_tile_shows_the_raster_and_nothing_around() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("lc.tif");
    let (cols, rows) = (300, 200);
    write_raster(
        &path,
        cols,
        rows,
        geo_transform(X0, Y0),
        vec![7u8; cols * rows],
        Some(0.0),
    );
    build(&path, Resampling::Nearest, 1, &|| false).expect("overviews");

    let [w, s, e, n] = bounds_wgs84(&path).expect("bounds");
    assert!(w < e && s < n);
    let (lon, lat) = ((w + e) / 2.0, (s + n) / 2.0);
    // A zoom where the 30 km raster spans a few tiles.
    let z = 10;
    let (x, y) = tile_at(lon, lat, z);
    let rendered = raster_tile(&path, z, x, y, Style::Classes).expect("tile");
    assert!(!rendered.empty);
    let tile = pixels(&rendered.png);
    let class7 = colour(7.0, Style::Classes);
    assert!(tile.contains(&class7), "the raster is drawn");
    assert!(
        tile.iter().all(|p| *p == class7 || p[3] == 0),
        "elsewhere transparent"
    );

    // Far away: fully transparent.
    let far = raster_tile(&path, z, (x + 50) % 1024, y, Style::Classes).expect("tile");
    assert!(far.empty);
    let far = pixels(&far.png);
    assert!(far.iter().all(|p| p[3] == 0));

    assert_eq!(value_at(&path, lon, lat).expect("value"), Some(7.0));
    assert_eq!(value_at(&path, w - 1.0, lat).expect("outside"), None);
}

#[test]
fn vector_layers_become_geojson_in_wgs84() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hf.gpkg");
    let points: Vec<(String, Vec<Option<FieldValue>>)> = (0..5)
        .map(|i| {
            let (x, y) = grid(10, 10).cell_centre(i, i);
            (
                format!("POINT ({x} {y})"),
                vec![Some(FieldValue::IntegerValue(i as i32))],
            )
        })
        .collect();
    let features: Vec<(&str, Vec<Option<FieldValue>>)> = points
        .iter()
        .map(|(w, v)| (w.as_str(), v.clone()))
        .collect();
    write_features(
        &path,
        "hf",
        EPSG,
        &[("code", OGRFieldType::OFTInteger)],
        &features,
    );
    let collection = geojson(&path, 3, None).expect("geojson");
    assert_eq!(collection["type"], "FeatureCollection");
    assert_eq!(collection["features"].as_array().map(Vec::len), Some(3));
    assert_eq!(collection["truncated"], true);
    let first = &collection["features"][0];
    assert_eq!(first["properties"]["code"], 0);
    let lon = first["geometry"]["coordinates"][0].as_f64().expect("lon");
    assert!((-180.0..=180.0).contains(&lon));
}

#[test]
fn geojson_samples_a_raster_at_each_point() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Class 1 everywhere, NoData on the diagonal cell (1, 1).
    let mut classes = vec![1u8; 100];
    classes[11] = 0;
    write_raster(
        &dir.path().join("lc.tif"),
        10,
        10,
        geo_transform(X0, Y0),
        classes,
        Some(0.0),
    );
    let path = dir.path().join("hf.gpkg");
    let points: Vec<String> = [(0, 0), (1, 1)]
        .iter()
        .map(|&(r, c)| {
            let (x, y) = grid(10, 10).cell_centre(r, c);
            format!("POINT ({x} {y})")
        })
        .collect();
    let features: Vec<(&str, Vec<Option<FieldValue>>)> =
        points.iter().map(|w| (w.as_str(), vec![])).collect();
    write_features(&path, "hf", EPSG, &[], &features);
    let collection = geojson(&path, 10, Some(&dir.path().join("lc.tif"))).expect("geojson");
    let samples: Vec<_> = (0..2)
        .map(|i| collection["features"][i]["sample"].clone())
        .collect();
    assert_eq!(samples, [serde_json::json!(1.0), serde_json::Value::Null]);
}
