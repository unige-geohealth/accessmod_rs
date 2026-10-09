//! `accessmod dataset inspect`: describe a raster or vector dataset.

use std::path::PathBuf;

use accessmod_io::grid::{RasterFrame, check_projected_metric};
use gdal::Dataset;
use gdal::spatial_ref::SpatialRef;
use gdal::vector::LayerAccess;
use serde_json::json;

use crate::{Failure, print_json};

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Size, grid, CRS, bands and layers of a dataset.
    Inspect {
        /// Raster or vector dataset.
        path: PathBuf,
        /// Print a JSON document on stdout.
        #[arg(long)]
        json: bool,
    },
}

fn crs(srs: Option<&SpatialRef>) -> serde_json::Value {
    match srs {
        None => serde_json::Value::Null,
        Some(srs) => json!({
            "name": srs.name(),
            "authority": srs.auth_name().map(|a| format!("{a}:{}", srs.auth_code().unwrap_or(0))),
            "projected": srs.is_projected(),
            "geographic": srs.is_geographic(),
            "linear_unit": srs.linear_units_name(),
            "metres_per_unit": srs.linear_units(),
            "wkt": srs.to_wkt().ok(),
        }),
    }
}

pub fn run(command: &Command) -> Result<(), Failure> {
    let Command::Inspect { path, json } = command;
    let dataset = Dataset::open(path).map_err(|e| {
        eprintln!("error: {}: {e}", path.display());
        Failure { code: 3 }
    })?;
    let srs = dataset.spatial_ref().ok();
    let mut report = json!({
        "path": path,
        "driver": dataset.driver().short_name(),
    });

    let bands = dataset.raster_count();
    if bands > 0 {
        let frame = RasterFrame::of(&dataset).ok();
        let mut issues = frame.map(|f| f.check_north_up()).unwrap_or_default();
        issues.extend(check_projected_metric(srs.as_ref()));
        let band_info: Vec<_> = (1..=bands)
            .filter_map(|i| dataset.rasterband(i).ok())
            .map(|b| {
                json!({
                    "type": b.band_type().name(),
                    "integer": b.band_type().is_integer(),
                    "nodata": b.no_data_value(),
                    "block_size": b.block_size(),
                })
            })
            .collect();
        report["raster"] = json!({
            "cols": dataset.raster_size().0,
            "rows": dataset.raster_size().1,
            "geo_transform": frame.map(|f| f.geo_transform),
            "crs": crs(srs.as_ref()),
            "bands": band_info,
            "usable_as_dem": issues.is_empty(),
            "issues": issues,
        });
    }

    let layers: Vec<_> = dataset
        .layers()
        .map(|layer| {
            let fields: Vec<_> = layer
                .defn()
                .fields()
                .map(|f| json!({"name": f.name(), "type": gdal::vector::field_type_to_name(f.field_type())}))
                .collect();
            let geometry_types: Vec<_> = layer
                .defn()
                .geom_fields()
                .map(|g| gdal::vector::geometry_type_to_name(g.field_type()))
                .collect();
            json!({
                "name": layer.name(),
                "features": layer.feature_count(),
                "geometry_types": geometry_types,
                "fields": fields,
                "crs": crs(layer.spatial_ref().as_ref()),
            })
        })
        .collect();
    if !layers.is_empty() {
        report["layers"] = json!(layers);
    }

    if *json {
        print_json(&report);
    } else {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    }
    Ok(())
}
