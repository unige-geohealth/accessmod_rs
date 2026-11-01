//! Map previews of project datasets: raster tiles in Web Mercator as PNG,
//! vector layers as GeoJSON, values at a point, bounds in WGS 84.
//!
//! Raster tiles go through `GDALWarp`, which reads the overview level that
//! fits the zoom ([`crate::preview`] builds them): a tile of a national
//! grid never reads the whole grid.

use std::ffi::{CString, c_char, c_int};
use std::path::{Path, PathBuf};

use gdal::raster::{Buffer, GdalType};
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use gdal::vector::LayerAccess;
use gdal::{Dataset, DriverManager};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::facilities::single_point;
use crate::raster::valid_value;
use crate::vector::json_attribute;

/// Side of a tile, pixels.
pub const TILE: usize = 256;

/// Half the side of the Web Mercator square, metres.
const ORIGIN_SHIFT: f64 = 20_037_508.342_789_244;

/// Deepest zoom served (cells of about 0.6 m at the equator).
pub const MAX_ZOOM: u32 = 24;

/// How cell values become colours.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Style {
    /// Integer classes, one stable colour each (land cover, cost allocation).
    Classes,
    /// Continuous values on a ramp from `min` to `max`.
    Ramp {
        /// Value of the first colour.
        min: f64,
        /// Value of the last colour.
        max: f64,
    },
}

/// Bounds of tile `z/x/y` in Web Mercator: (min x, min y, max x, max y).
pub fn tile_bounds(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let size = 2.0 * ORIGIN_SHIFT / f64::from(1u32 << z.min(MAX_ZOOM));
    let min_x = f64::from(x) * size - ORIGIN_SHIFT;
    let max_y = ORIGIN_SHIFT - f64::from(y) * size;
    (min_x, max_y - size, min_x + size, max_y)
}

/// Tile `(x, y)` of zoom `z` holding longitude `lon`, latitude `lat`.
pub fn tile_at(lon: f64, lat: f64, z: u32) -> (u32, u32) {
    let n = f64::from(1u32 << z.min(MAX_ZOOM));
    let lat = lat.clamp(-85.051_128, 85.051_128).to_radians();
    let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0);
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
        .floor()
        .clamp(0.0, n - 1.0);
    (x as u32, y as u32)
}

fn check_tile(z: u32, x: u32, y: u32) -> Result<()> {
    if z > MAX_ZOOM || u64::from(x) >= 1u64 << z || u64::from(y) >= 1u64 << z {
        return Err(Error::invalid(format!("no tile {z}/{x}/{y}")));
    }
    Ok(())
}

/// A rendered tile.
#[derive(Debug)]
pub struct Tile {
    /// PNG image.
    pub png: Vec<u8>,
    /// Whether no cell has a value (fully transparent).
    pub empty: bool,
}

/// Tile `z/x/y` of band 1 of `path` as a PNG (transparent where NoData).
pub fn raster_tile(path: &Path, z: u32, x: u32, y: u32, style: Style) -> Result<Tile> {
    check_tile(z, x, y)?;
    let values = warp_tile(path, z, x, y)?;
    let empty = values.iter().all(Option::is_none);
    let mut rgba = Vec::with_capacity(TILE * TILE * 4);
    for value in values {
        rgba.extend_from_slice(&match value {
            Some(v) => colour(v, style),
            None => [0, 0, 0, 0],
        });
    }
    Ok(Tile {
        png: encode_png(&rgba)?,
        empty,
    })
}

/// Values of tile `z/x/y`, row-major; `None` where NoData or outside.
fn warp_tile(path: &Path, z: u32, x: u32, y: u32) -> Result<Vec<Option<f64>>> {
    let source = Dataset::open(path).map_err(Error::gdal(path))?;
    let source_nodata = source
        .rasterband(1)
        .map_err(Error::gdal(path))?
        .no_data_value();
    let driver = DriverManager::get_driver_by_name("MEM").map_err(Error::gdal(path))?;
    let mut target = driver
        .create_with_band_type::<f64, _>("", TILE, TILE, 1)
        .map_err(Error::gdal(path))?;
    let (min_x, _, max_x, max_y) = tile_bounds(z, x, y);
    let pixel = (max_x - min_x) / TILE as f64;
    target
        .set_geo_transform(&[min_x, pixel, 0.0, max_y, 0.0, -pixel])
        .map_err(Error::gdal(path))?;
    target
        .set_spatial_ref(&SpatialRef::from_epsg(3857).map_err(Error::gdal(path))?)
        .map_err(Error::gdal(path))?;
    {
        let mut band = target.rasterband(1).map_err(Error::gdal(path))?;
        band.set_no_data_value(Some(f64::NAN))
            .map_err(Error::gdal(path))?;
        band.fill(f64::NAN, None).map_err(Error::gdal(path))?;
    }
    let arguments = ["-r", "near", "-wo", "INIT_DEST=NO_DATA", "-q"];
    let arguments: Vec<CString> = arguments
        .iter()
        .map(|a| CString::new(*a).map_err(|_| Error::invalid("warp argument with NUL")))
        .collect::<Result<_>>()?;
    let mut argv: Vec<*mut c_char> = arguments
        .iter()
        .map(|a| a.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    // SAFETY: `argv` is a NULL-terminated list of C strings alive for the
    // call; GDAL copies them into the options, freed below.
    let options =
        unsafe { gdal_sys::GDALWarpAppOptionsNew(argv.as_mut_ptr(), std::ptr::null_mut()) };
    if options.is_null() {
        return Err(Error::invalid("invalid warp options"));
    }
    let mut sources = [source.c_dataset()];
    let mut usage_error: c_int = 0;
    // SAFETY: both dataset handles are valid while `source` and `target`
    // live; with a destination dataset GDALWarp returns that same handle,
    // which stays owned by `target`.
    let result = unsafe {
        gdal_sys::GDALWarp(
            std::ptr::null(),
            target.c_dataset(),
            1,
            sources.as_mut_ptr(),
            options,
            &mut usage_error,
        )
    };
    // SAFETY: `options` came from GDALWarpAppOptionsNew and is freed once.
    unsafe { gdal_sys::GDALWarpAppOptionsFree(options) };
    if result.is_null() || usage_error != 0 {
        return Err(Error::Gdal {
            path: path.to_path_buf(),
            source: gdal::errors::GdalError::CplError {
                class: gdal_sys::CPLErr::CE_Failure,
                number: 0,
                msg: "GDALWarp failed".into(),
            },
        });
    }
    let values = read_all::<f64>(&target, path)?;
    Ok(values
        .into_iter()
        .map(|v| valid_value(v, source_nodata).filter(|v| v.is_finite()))
        .collect())
}

fn read_all<T: GdalType + Copy>(dataset: &Dataset, path: &Path) -> Result<Vec<T>> {
    let band = dataset.rasterband(1).map_err(Error::gdal(path))?;
    let buffer: Buffer<T> = band
        .read_as::<T>((0, 0), (TILE, TILE), (TILE, TILE), None)
        .map_err(Error::gdal(path))?;
    Ok(buffer.into_shape_and_vec().1)
}

/// RGBA colour of `value` in `style`.
pub fn colour(value: f64, style: Style) -> [u8; 4] {
    match style {
        Style::Classes => class_colour(value as i64),
        Style::Ramp { min, max } => {
            let t = if max > min {
                ((value - min) / (max - min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ramp(t)
        }
    }
}

/// A stable, well-spread colour per class value (golden-ratio hues).
fn class_colour(value: i64) -> [u8; 4] {
    let hue = (value as f64 * 0.618_033_988_749_895).rem_euclid(1.0);
    let lightness = if value.rem_euclid(2) == 0 { 0.5 } else { 0.62 };
    let (r, g, b) = hsl_to_rgb(hue, 0.6, lightness);
    [r, g, b, 255]
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

/// Viridis-like ramp, `t` in `[0, 1]`.
fn ramp(t: f64) -> [u8; 4] {
    const STOPS: [[f64; 3]; 9] = [
        [68.0, 1.0, 84.0],
        [72.0, 40.0, 120.0],
        [62.0, 74.0, 137.0],
        [49.0, 104.0, 142.0],
        [38.0, 130.0, 142.0],
        [31.0, 158.0, 137.0],
        [53.0, 183.0, 121.0],
        [109.0, 205.0, 89.0],
        [253.0, 231.0, 37.0],
    ];
    let scaled = t * (STOPS.len() - 1) as f64;
    let i = (scaled.floor() as usize).min(STOPS.len() - 2);
    let f = scaled - i as f64;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    let mix = |k: usize| (a[k] + (b[k] - a[k]) * f).round().clamp(0.0, 255.0) as u8;
    [mix(0), mix(1), mix(2), 255]
}

fn encode_png(rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, TILE as u32, TILE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| Error::invalid(format!("PNG: {e}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| Error::invalid(format!("PNG: {e}")))?;
    }
    Ok(out)
}

/// WGS 84 in longitude, latitude order.
pub(crate) fn wgs84() -> Result<SpatialRef> {
    let mut srs = SpatialRef::from_epsg(4326).map_err(Error::gdal("EPSG:4326"))?;
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    Ok(srs)
}

fn traditional(mut srs: SpatialRef) -> SpatialRef {
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    srs
}

/// Value of band 1 of `path` at longitude `lon`, latitude `lat`; `None`
/// outside the raster or on NoData.
pub fn value_at(path: &Path, lon: f64, lat: f64) -> Result<Option<f64>> {
    PointSampler::open(path, Some(wgs84()?))?.sample(lon, lat)
}

/// Values of band 1 of a raster at points, one cell read each.
#[derive(Debug)]
pub struct PointSampler {
    dataset: Dataset,
    path: PathBuf,
    geo_transform: [f64; 6],
    nodata: Option<f64>,
    transform: Option<CoordTransform>,
}

impl PointSampler {
    /// Open the raster at `path` for points in CRS `from` (the raster CRS
    /// when `None`).
    pub fn open(path: &Path, from: Option<SpatialRef>) -> Result<Self> {
        let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
        let transform = match from {
            Some(from) => {
                let to = traditional(dataset.spatial_ref().map_err(Error::gdal(path))?);
                let from = traditional(from);
                (from != to)
                    .then(|| CoordTransform::new(&from, &to))
                    .transpose()
                    .map_err(Error::gdal(path))?
            }
            None => None,
        };
        let geo_transform = dataset.geo_transform().map_err(Error::gdal(path))?;
        let nodata = dataset
            .rasterband(1)
            .map_err(Error::gdal(path))?
            .no_data_value();
        Ok(Self {
            dataset,
            path: path.to_path_buf(),
            geo_transform,
            nodata,
            transform,
        })
    }

    /// Value at `(x, y)`; `None` outside the raster, on NoData or when the
    /// point cannot be reprojected.
    pub fn sample(&self, x: f64, y: f64) -> Result<Option<f64>> {
        let (mut xs, mut ys) = ([x], [y]);
        if let Some(transform) = &self.transform
            && transform
                .transform_coords(&mut xs, &mut ys, &mut [])
                .is_err()
        {
            return Ok(None);
        }
        let gt = &self.geo_transform;
        let col = ((xs[0] - gt[0]) / gt[1]).floor();
        let row = ((ys[0] - gt[3]) / gt[5]).floor();
        let (cols, rows) = self.dataset.raster_size();
        if !(col >= 0.0 && row >= 0.0 && col < cols as f64 && row < rows as f64) {
            return Ok(None);
        }
        let path = &self.path;
        let band = self.dataset.rasterband(1).map_err(Error::gdal(path))?;
        let value = band
            .read_as::<f64>((col as isize, row as isize), (1, 1), (1, 1), None)
            .map_err(Error::gdal(path))?
            .into_shape_and_vec()
            .1
            .first()
            .copied();
        Ok(value.and_then(|v| valid_value(v, self.nodata)))
    }
}

/// Bounds of the raster at `path` in WGS 84: `[west, south, east, north]`
/// from its corners and edge midpoints.
pub fn bounds_wgs84(path: &Path) -> Result<[f64; 4]> {
    let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
    let srs = traditional(dataset.spatial_ref().map_err(Error::gdal(path))?);
    let transform = CoordTransform::new(&srs, &wgs84()?).map_err(Error::gdal(path))?;
    let gt = dataset.geo_transform().map_err(Error::gdal(path))?;
    let (cols, rows) = dataset.raster_size();
    let (x0, y0) = (gt[0], gt[3]);
    let (x1, y1) = (gt[0] + gt[1] * cols as f64, gt[3] + gt[5] * rows as f64);
    let xm = (x0 + x1) / 2.0;
    let ym = (y0 + y1) / 2.0;
    let mut xs = [x0, xm, x1, x1, x1, xm, x0, x0];
    let mut ys = [y0, y0, y0, ym, y1, y1, y1, ym];
    transform
        .transform_coords(&mut xs, &mut ys, &mut [])
        .map_err(Error::gdal(path))?;
    let fold =
        |values: &[f64], f: fn(f64, f64) -> f64, start: f64| values.iter().copied().fold(start, f);
    Ok([
        fold(&xs, f64::min, f64::INFINITY),
        fold(&ys, f64::min, f64::INFINITY),
        fold(&xs, f64::max, f64::NEG_INFINITY),
        fold(&ys, f64::max, f64::NEG_INFINITY),
    ])
}

/// The first layer of the vector dataset at `path` as a GeoJSON
/// FeatureCollection in WGS 84, at most `limit` features; `truncated` tells
/// whether some were left out. With `sample`, each feature also has a
/// `"sample"` member: the value of that raster at its point (a point or a
/// one-point multipoint), `null` outside, on NoData or for other geometries.
pub fn geojson(path: &Path, limit: usize, sample: Option<&Path>) -> Result<Value> {
    let dataset = Dataset::open(path).map_err(Error::gdal(path))?;
    let mut layer = dataset.layer(0).map_err(Error::gdal(path))?;
    let srs = layer
        .spatial_ref()
        .ok_or_else(|| Error::invalid(format!("{}: no CRS", path.display())))?;
    let srs = traditional(srs);
    let transform = CoordTransform::new(&srs, &wgs84()?).map_err(Error::gdal(path))?;
    let sampler = sample
        .map(|raster| PointSampler::open(raster, Some(srs.clone())))
        .transpose()?;
    let names: Vec<String> = layer.defn().fields().map(|f| f.name()).collect();
    let mut features = Vec::new();
    let mut truncated = false;
    for feature in layer.features() {
        if features.len() >= limit {
            truncated = true;
            break;
        }
        let Some(geometry) = feature.geometry() else {
            continue;
        };
        // The value of `sample` at the point (a point or one-point multipoint).
        let value = match &sampler {
            Some(sampler) => Some(match single_point(geometry) {
                Ok((x, y)) => sampler.sample(x, y)?,
                Err(_) => None,
            }),
            None => None,
        };
        let geometry = geometry.transform(&transform).map_err(Error::gdal(path))?;
        let geometry: Value = geometry
            .json()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let mut properties = serde_json::Map::new();
        for (i, name) in names.iter().enumerate() {
            let value = feature.field(i).ok().flatten();
            properties.insert(name.clone(), json_attribute(value).unwrap_or(Value::Null));
        }
        let mut out = json!({
            "type": "Feature",
            "id": feature.fid(),
            "geometry": geometry,
            "properties": properties,
        });
        if let (Some(value), Some(object)) = (value, out.as_object_mut()) {
            object.insert("sample".into(), json!(value));
        }
        features.push(out);
    }
    Ok(json!({
        "type": "FeatureCollection",
        "features": features,
        "truncated": truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_zero_covers_the_world() {
        let (x0, y0, x1, y1) = tile_bounds(0, 0, 0);
        assert!((x0 + ORIGIN_SHIFT).abs() < 1e-6 && (x1 - ORIGIN_SHIFT).abs() < 1e-6);
        assert!((y0 + ORIGIN_SHIFT).abs() < 1e-6 && (y1 - ORIGIN_SHIFT).abs() < 1e-6);
        let (_, _, x1, _) = tile_bounds(1, 0, 0);
        assert!(x1.abs() < 1e-6);
        assert!(check_tile(1, 2, 0).is_err());
        assert!(check_tile(30, 0, 0).is_err());
    }

    #[test]
    fn colours_are_stable_and_ramps_clamped() {
        assert_eq!(colour(3.0, Style::Classes), colour(3.0, Style::Classes));
        assert_ne!(colour(1.0, Style::Classes), colour(2.0, Style::Classes));
        let ramp = Style::Ramp {
            min: 0.0,
            max: 10.0,
        };
        assert_eq!(colour(-5.0, ramp), colour(0.0, ramp));
        assert_eq!(colour(50.0, ramp), [253, 231, 37, 255]);
        assert_eq!(
            colour(0.0, Style::Ramp { min: 1.0, max: 1.0 }),
            [68, 1, 84, 255]
        );
    }
}
