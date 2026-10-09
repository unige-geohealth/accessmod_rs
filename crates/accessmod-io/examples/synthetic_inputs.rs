//! Deterministic synthetic landscape for the tutorial and benchmarks.
//!
//! `cargo run --release -p accessmod-io --example synthetic_inputs -- <dir> [cols] [rows] [facilities]`
//!
//! Writes into `<dir>` (EPSG:32632, 100 m cells):
//!
//! - `dem.tif`: rolling hills (Float32);
//! - `land_cover.tif` (UInt16): 1 built area, 2 grassland, 3 forest,
//!   1001 main road, 1002 secondary road, 9 river (left out of the scenario,
//!   so a barrier except where roads bridge it), 0 lake (NoData);
//! - `facilities.gpkg`: facilities on the main road, field `code`;
//! - `job.json`: an anisotropic job on these inputs.

use std::path::Path;

use gdal::DriverManager;
use gdal::raster::{Buffer, GdalType, RasterCreationOptions};
use gdal::spatial_ref::SpatialRef;
use gdal::vector::{
    Feature, FieldValue, Geometry, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType,
};

const EPSG: u32 = 32632;
const X0: f64 = 400_000.0;
const Y0: f64 = 5_100_000.0;
const RES: f64 = 100.0;

/// Rows written to GDAL at once, so large grids never need a full buffer.
const STRIP: usize = 256;

type Error = Box<dyn std::error::Error>;

/// Land-cover class and elevation of a cell.
fn cell(row: usize, col: usize, rows: usize, cols: usize) -> (u16, f32) {
    let (x, y) = (col as f64, row as f64);
    let (w, h) = (cols as f64, rows as f64);
    let dem = 450.0 + 220.0 * (x / (w / 7.0)).sin() * (y / (h / 5.0)).cos() + 0.08 * x
        - 60.0 * ((x - 0.7 * w).powi(2) + (y - 0.3 * h).powi(2)).sqrt() / w;

    let main_road = (y - 0.5 * h - 0.08 * h * (x / (w / 3.0)).sin()).abs() < 0.8;
    let secondary = (x - 0.3 * w - 0.05 * w * (y / (h / 2.0)).sin()).abs() < 0.8;
    let river = (x - 0.62 * w - 0.06 * w * (y / (h / 4.0)).sin()).abs() < 1.5;
    let lake = ((x - 0.15 * w) / (0.08 * w)).powi(2) + ((y - 0.2 * h) / (0.1 * h)).powi(2) < 1.0;
    let town = ((x - 0.45 * w).powi(2) + (y - 0.5 * h).powi(2)).sqrt() < 0.06 * w.min(h) * 1.5;
    let forest = (x / 23.0).sin() * (y / 17.0).cos() + 0.3 * (x / 7.0).sin() > 0.55;

    let class = if main_road {
        1001
    } else if secondary {
        1002
    } else if river {
        9
    } else if lake {
        0
    } else if town {
        1
    } else if forest {
        3
    } else {
        2
    };
    (class, dem as f32)
}

fn create<T: GdalType>(
    path: &Path,
    cols: usize,
    rows: usize,
    nodata: Option<f64>,
) -> Result<gdal::Dataset, Error> {
    let mut options = RasterCreationOptions::new();
    for (k, v) in [
        ("TILED", "YES"),
        ("COMPRESS", "DEFLATE"),
        ("BIGTIFF", "IF_SAFER"),
    ] {
        options.set_name_value(k, v)?;
    }
    let mut ds = DriverManager::get_driver_by_name("GTiff")?
        .create_with_band_type_with_options::<T, _>(path, cols, rows, 1, &options)?;
    ds.set_geo_transform(&[X0, RES, 0.0, Y0, 0.0, -RES])?;
    ds.set_spatial_ref(&SpatialRef::from_epsg(EPSG)?)?;
    ds.rasterband(1)?.set_no_data_value(nodata)?;
    Ok(ds)
}

fn write_rasters(dir: &Path, cols: usize, rows: usize) -> Result<(), Error> {
    let dem_ds = create::<f32>(&dir.join("dem.tif"), cols, rows, Some(-9999.0))?;
    let lc_ds = create::<u16>(&dir.join("land_cover.tif"), cols, rows, Some(0.0))?;
    for start in (0..rows).step_by(STRIP) {
        let height = STRIP.min(rows - start);
        let mut dem = Vec::with_capacity(height * cols);
        let mut lc = Vec::with_capacity(height * cols);
        for row in start..start + height {
            for col in 0..cols {
                let (class, z) = cell(row, col, rows, cols);
                lc.push(class);
                dem.push(z);
            }
        }
        dem_ds.rasterband(1)?.write(
            (0, start as isize),
            (cols, height),
            &mut Buffer::new((cols, height), dem),
        )?;
        lc_ds.rasterband(1)?.write(
            (0, start as isize),
            (cols, height),
            &mut Buffer::new((cols, height), lc),
        )?;
    }
    dem_ds.close()?;
    lc_ds.close()?;
    Ok(())
}

/// Facilities spread along the main road, avoiding the river.
fn facility_cells(cols: usize, rows: usize, count: usize) -> Vec<(usize, usize)> {
    (0..count)
        .map(|k| {
            let mut col = (cols as f64 * (k as f64 + 0.5) / count as f64) as usize;
            loop {
                let row = (0..rows).find(|&r| cell(r, col, rows, cols).0 == 1001);
                match row {
                    Some(row) if cell(row, col, rows, cols).0 == 1001 => return (row, col),
                    _ => col = (col + 1) % cols,
                }
            }
        })
        .collect()
}

fn write_facilities(dir: &Path, cols: usize, rows: usize, count: usize) -> Result<(), Error> {
    let path = dir.join("facilities.gpkg");
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let mut ds = DriverManager::get_driver_by_name("GPKG")?.create_vector_only(&path)?;
    let srs = SpatialRef::from_epsg(EPSG)?;
    let layer = ds.create_layer(LayerOptions {
        name: "facilities",
        srs: Some(&srs),
        ty: OGRwkbGeometryType::wkbPoint,
        options: None,
    })?;
    layer.create_defn_fields(&[
        ("code", OGRFieldType::OFTString),
        ("name", OGRFieldType::OFTString),
    ])?;
    let code = layer.defn().field_index("code")?;
    let name = layer.defn().field_index("name")?;
    for (k, (row, col)) in facility_cells(cols, rows, count).into_iter().enumerate() {
        let x = X0 + (col as f64 + 0.5) * RES;
        let y = Y0 - (row as f64 + 0.5) * RES;
        let mut feature = Feature::new(layer.defn())?;
        feature.set_geometry(Geometry::from_wkt(&format!("POINT ({x} {y})"))?)?;
        feature.set_field(code, &FieldValue::StringValue(format!("HF-{:02}", k + 1)))?;
        feature.set_field(
            name,
            &FieldValue::StringValue(format!("Health centre {}", k + 1)),
        )?;
        feature.create(&layer)?;
    }
    Ok(())
}

const JOB: &str = r#"{
  "schema_version": 1,
  "dem": "dem.tif",
  "land_cover": "land_cover.tif",
  "facilities": { "path": "facilities.gpkg", "id_field": "code" },
  "scenario": [
    { "class": 1, "mode": "walking", "speed_kmh": 5, "label": "built area" },
    { "class": 2, "mode": "walking", "speed_kmh": 4, "label": "grassland" },
    { "class": 3, "mode": "walking", "speed_kmh": 2, "label": "forest" },
    { "class": 1001, "mode": "motorized", "speed_kmh": 60, "label": "main road" },
    { "class": 1002, "mode": "bicycling", "speed_kmh": 15, "label": "secondary road" }
  ],
  "method": "anisotropic",
  "direction": "towards_facilities",
  "neighbourhood": 16,
  "outputs": { "direction": true },
  "output_dir": "out"
}
"#;

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = Path::new(
        args.first()
            .ok_or("usage: synthetic_inputs <dir> [cols] [rows] [facilities]")?,
    );
    let number =
        |i: usize, default: usize| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
    let (cols, rows, count) = (number(1, 300), number(2, 200), number(3, 3));
    std::fs::create_dir_all(dir)?;
    write_rasters(dir, cols, rows)?;
    write_facilities(dir, cols, rows, count)?;
    std::fs::write(dir.join("job.json"), JOB)?;
    eprintln!(
        "wrote {cols} x {rows} landscape with {count} facilities to {}",
        dir.display()
    );
    Ok(())
}
