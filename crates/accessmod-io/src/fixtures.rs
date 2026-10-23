//! Test fixtures shared by the test suites of this crate and of the CLI:
//! small GeoTIFF and GeoPackage inputs on a fixed UTM grid, raster reads,
//! and running an analysis. Built only with the `fixtures` feature.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use accessmod_core::cost::CellSize;
use gdal::raster::{Buffer, GdalType};
use gdal::spatial_ref::SpatialRef;
use gdal::vector::{Feature, FieldValue, Geometry, LayerAccess, LayerOptions, OGRwkbGeometryType};
use gdal::{Dataset, DriverManager};

use crate::analysis::{Analysis, Monitor};
use crate::error::Result;
use crate::grid::GridSpec;
use crate::raster::valid_value;
use crate::resources::ExecutionSettings;

/// CRS of the fixtures (WGS 84 / UTM 32N).
pub const EPSG: u32 = 32632;
/// West edge of the fixture grids, m.
pub const X0: f64 = 500_000.0;
/// North edge of the fixture grids, m.
pub const Y0: f64 = 5_000_000.0;
/// Cell size of the fixture grids, m.
pub const RES: f64 = 100.0;

/// The fixture grid: `cols × rows` cells of [`RES`] from `(X0, Y0)` in
/// [`EPSG`].
pub fn grid(cols: usize, rows: usize) -> GridSpec {
    GridSpec {
        cols,
        rows,
        geo_transform: geo_transform(X0, Y0),
        srs: SpatialRef::from_epsg(EPSG).expect("srs"),
        cell: CellSize::new(RES, RES).expect("positive cell size"),
    }
}

/// North-up geotransform at `(x0, y0)` with [`RES`] cells.
pub fn geo_transform(x0: f64, y0: f64) -> [f64; 6] {
    [x0, RES, 0.0, y0, 0.0, -RES]
}

/// Write a single-band GeoTIFF in [`EPSG`]; `data` is row-major.
pub fn write_raster<T: GdalType + Copy>(
    path: &Path,
    cols: usize,
    rows: usize,
    gt: [f64; 6],
    data: Vec<T>,
    nodata: Option<f64>,
) {
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver");
    let mut ds = driver
        .create_with_band_type::<T, _>(path, cols, rows, 1)
        .expect("create raster");
    ds.set_geo_transform(&gt).expect("geotransform");
    ds.set_spatial_ref(&SpatialRef::from_epsg(EPSG).expect("srs"))
        .expect("srs");
    let mut band = ds.rasterband(1).expect("band");
    band.set_no_data_value(nodata).expect("nodata");
    band.write((0, 0), (cols, rows), &mut Buffer::new((cols, rows), data))
        .expect("write");
}

/// Write a GeoPackage layer `layer` in EPSG `epsg` with one attribute
/// `field` and features `(value, WKT)` (`None`: attribute left null).
pub fn write_layer(
    path: &Path,
    layer: &str,
    epsg: u32,
    field: (&str, gdal::vector::OGRFieldType::Type),
    features: &[(Option<FieldValue>, &str)],
) {
    let features: Vec<(&str, Vec<Option<FieldValue>>)> = features
        .iter()
        .map(|(value, wkt)| (*wkt, vec![value.clone()]))
        .collect();
    write_features(path, layer, epsg, &[field], &features);
}

/// Write a GeoPackage layer `layer` in EPSG `epsg` with attributes `fields`
/// and features `(WKT, values)`, one value per field (`None`: null).
pub fn write_features(
    path: &Path,
    layer: &str,
    epsg: u32,
    fields: &[(&str, gdal::vector::OGRFieldType::Type)],
    features: &[(&str, Vec<Option<FieldValue>>)],
) {
    let driver = DriverManager::get_driver_by_name("GPKG").expect("GPKG driver");
    let mut ds = driver.create_vector_only(path).expect("create gpkg");
    let srs = SpatialRef::from_epsg(epsg).expect("srs");
    let created = ds
        .create_layer(LayerOptions {
            name: layer,
            srs: Some(&srs),
            ty: OGRwkbGeometryType::wkbUnknown,
            options: None,
        })
        .expect("layer");
    created.create_defn_fields(fields).expect("fields");
    for (wkt, values) in features {
        let mut feature = Feature::new(created.defn()).expect("feature");
        feature
            .set_geometry(Geometry::from_wkt(wkt).expect("wkt"))
            .expect("geometry");
        for ((name, _), value) in fields.iter().zip(values) {
            if let Some(value) = value {
                let index = created.defn().field_index(name).expect("field index");
                feature.set_field(index, value).expect("attribute");
            }
        }
        feature.create(&created).expect("create feature");
    }
}

/// Band 1 of a raster, row-major, with its NoData value.
pub fn read_with_nodata<T: GdalType + Copy>(path: &Path) -> (Vec<T>, Option<f64>) {
    let ds = Dataset::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let band = ds.rasterband(1).expect("band");
    let (cols, rows) = ds.raster_size();
    let values = band
        .read_as::<T>((0, 0), (cols, rows), (cols, rows), None)
        .expect("read")
        .data()
        .to_vec();
    (values, band.no_data_value())
}

/// Band 1 of a raster as `f64`, `None` for NoData and NaN.
pub fn read_valid(path: &Path) -> Vec<Option<f64>> {
    let (values, nodata) = read_with_nodata::<f64>(path);
    values.into_iter().map(|v| valid_value(v, nodata)).collect()
}

/// Band 1 of a raster, row-major.
pub fn read<T: GdalType + Copy>(path: &Path) -> Vec<T> {
    read_with_nodata(path).0
}

/// Execution settings with the system temporary directory.
pub fn settings(memory_bytes: u64, threads: usize) -> ExecutionSettings {
    ExecutionSettings {
        memory_bytes,
        threads,
        tmp_dir: std::env::temp_dir(),
    }
}

/// A cancellation flag that is never raised.
pub fn never() -> bool {
    false
}

/// Prepare and run `job`; the output directory.
pub fn run_job<A: Analysis>(job: A::Job, settings: &ExecutionSettings) -> Result<PathBuf> {
    let prepared = A::prepare(job, settings, Monitor::QUIET)?;
    A::run(prepared, settings, Monitor::QUIET).map(|(dir, _)| dir)
}

/// `_oracle/<relative>` of the workspace (AccessMod 5 exports).
pub fn oracle_dir(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_oracle")
        .join(relative)
}

/// Points in [`EPSG`] expressed in WGS 84 longitude, latitude.
pub fn to_wgs84(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform};
    let mut utm = SpatialRef::from_epsg(EPSG).expect("utm");
    let mut wgs84 = SpatialRef::from_epsg(4326).expect("wgs84");
    for srs in [&mut utm, &mut wgs84] {
        srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    }
    let transform = CoordTransform::new(&utm, &wgs84).expect("transform");
    let (mut xs, mut ys): (Vec<f64>, Vec<f64>) = points.iter().copied().unzip();
    transform
        .transform_coords(&mut xs, &mut ys, &mut [])
        .expect("to wgs84");
    xs.into_iter().zip(ys).collect()
}

/// Whether every oracle export in `paths` exists; prints why the test is
/// skipped otherwise (oracle tests never pass vacuously).
pub fn oracle_available(paths: &[&Path]) -> bool {
    match paths.iter().find(|p| !p.exists()) {
        Some(missing) => {
            eprintln!(
                "SKIPPED: {} not found; run scripts/oracle/export.sh",
                missing.display()
            );
            false
        }
        None => true,
    }
}

/// Copy `src` to a Float64 GeoTIFF `dst` on the same grid, CRS and NoData,
/// mapping every valid value through `f` (e.g. minutes to seconds).
pub fn map_raster(src: &Path, dst: &Path, f: impl Fn(f64) -> f64) {
    let source = Dataset::open(src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    let (values, nodata) = read_with_nodata::<f64>(src);
    let (cols, rows) = source.raster_size();
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver");
    let mut ds = driver
        .create_with_band_type::<f64, _>(dst, cols, rows, 1)
        .expect("create raster");
    ds.set_geo_transform(&source.geo_transform().expect("geotransform"))
        .expect("geotransform");
    ds.set_spatial_ref(&source.spatial_ref().expect("srs"))
        .expect("srs");
    let mapped: Vec<f64> = values
        .into_iter()
        .map(|v| valid_value(v, nodata).map_or(v, &f))
        .collect();
    let mut band = ds.rasterband(1).expect("band");
    band.set_no_data_value(nodata).expect("nodata");
    band.write((0, 0), (cols, rows), &mut Buffer::new((cols, rows), mapped))
        .expect("write");
}

/// WKT polygon over cells `[c0, c1) × [r0, r1)` of the fixture grid
/// (fractional cell units, see [`grid`]).
pub fn cell_polygon(c0: f64, r0: f64, c1: f64, r1: f64) -> String {
    let g = grid(1, 1);
    let points: Vec<String> = [(c0, r0), (c1, r0), (c1, r1), (c0, r1), (c0, r0)]
        .iter()
        .map(|&(c, r)| {
            let (x, y) = g.to_map(c, r);
            format!("{x} {y}")
        })
        .collect();
    format!("POLYGON (({}))", points.join(", "))
}

/// Rows of a CSV keyed by column `key`, every field by column name; quotes
/// around fields are dropped (fields hold no comma in the test tables).
pub fn csv_table(path: &Path, key: &str) -> BTreeMap<String, BTreeMap<String, String>> {
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
