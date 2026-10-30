//! Importing files into a project.
//!
//! - Rasters must lie on the project lattice, in its CRS (no resampling or
//!   reprojection here). They are copied on the whole project grid, with
//!   their cell type and NoData value, as tiled GeoTIFF; cells they do not
//!   cover are NoData. Overviews and statistics follow
//!   ([`crate::preview`]).
//! - Vector layers are copied to FlatGeobuf in the project CRS with every
//!   attribute; their geometries must suit the class (points for
//!   facilities, polygons for zones, …).
//! - Tables are CSV files with a header, copied as they are.

use std::path::Path;

use gdal::raster::{GdalDataType, GdalType};
use gdal::vector::{Geometry, LayerAccess, OGRwkbGeometryType};

use super::{DatasetRecord, Origin, Project, Summary, TableSummary, VectorSummary};
use crate::analysis::Cancelled;
use crate::class::{DataClass, Kind};
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::output::GeoTiff;
use crate::preview::{self, Resampling};
use crate::raster::{AlignedBand, for_each_chunk, open_aligned_raster, open_class_raster};
use crate::resources::ExecutionSettings;
use crate::vector::{FeatureWriter, VectorLayer};

/// Options of an import.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Layer of a vector dataset; the first when `None`.
    pub layer: Option<String>,
}

impl Project {
    /// Import `source` as a dataset of `class` tagged `tags`.
    pub fn import(
        &self,
        source: &Path,
        class: DataClass,
        tags: &[String],
        options: &ImportOptions,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<DatasetRecord> {
        if class == DataClass::Dem {
            return Err(Error::invalid(
                "a project has one DEM, given when the project is created",
            ));
        }
        if !class.importable() {
            return Err(Error::invalid(format!(
                "{} datasets ({}) are written by analyses, not imported",
                class.label(),
                class.code()
            )));
        }
        let grid = self.grid()?;
        self.import_with(
            source,
            class,
            tags,
            options,
            |target| match class.kind() {
                Kind::Raster => copy_raster(source, target, &grid, class, settings, cancelled),
                Kind::Vector => copy_vector(
                    source,
                    target,
                    &grid,
                    class,
                    options.layer.as_deref(),
                    cancelled,
                ),
                Kind::Table => copy_table(source, target),
                Kind::Config => Err(config_is_a_job()),
            },
            cancelled,
        )
    }

    /// Import the DEM that defines the grid (project creation).
    pub(super) fn import_dem(
        &self,
        source: &Path,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<DatasetRecord> {
        let (_, grid) = GridSpec::open_dem(source)?;
        self.import_with(
            source,
            DataClass::Dem,
            &["dem".to_owned()],
            &ImportOptions::default(),
            |target| copy_raster(source, target, &grid, DataClass::Dem, settings, cancelled),
            cancelled,
        )
    }

    /// Reserve a row, write the file with `write`, complete the row; on
    /// failure nothing is left.
    fn import_with(
        &self,
        source: &Path,
        class: DataClass,
        tags: &[String],
        options: &ImportOptions,
        write: impl FnOnce(&Path) -> Result<Summary>,
        cancelled: Cancelled<'_>,
    ) -> Result<DatasetRecord> {
        if !source.exists() {
            return Err(Error::invalid(format!(
                "{} does not exist",
                source.display()
            )));
        }
        let _hold = self.activity()?;
        let origin = Origin::Import {
            source: source.display().to_string(),
            layer: options.layer.clone(),
        };
        let (id, file) = self.reserve(class, tags, &origin)?;
        let result = self
            .resolve(Path::new(&file))
            .and_then(|target| write(&target))
            .and_then(|summary| self.complete(id, &file, &summary, cancelled));
        if result.is_err() {
            self.abandon(id, &file);
        }
        result
    }
}

/// Summary of a file already written.
pub(super) fn summarise(
    path: &Path,
    class: DataClass,
    settings: &ExecutionSettings,
    cancelled: Cancelled<'_>,
) -> Result<Summary> {
    match class.kind() {
        Kind::Raster => preview::build(path, resampling(class), settings.threads, cancelled)
            .map(Summary::Raster),
        Kind::Vector => vector_summary(path).map(Summary::Vector),
        Kind::Table => table_summary(path).map(Summary::Table),
        Kind::Config => Err(config_is_a_job()),
    }
}

fn config_is_a_job() -> Error {
    Error::invalid("analysis parameters are saved jobs of the project, not datasets")
}

/// Overview resampling of a raster class.
pub fn resampling(class: DataClass) -> Resampling {
    match class {
        DataClass::LandCover
        | DataClass::LandCoverMerged
        | DataClass::CostAllocation
        | DataClass::Exclusion => Resampling::Nearest,
        _ => Resampling::Average,
    }
}

/// Classes stored as integer classes.
fn integer_class(class: DataClass) -> bool {
    matches!(
        class,
        DataClass::LandCover | DataClass::LandCoverMerged | DataClass::CostAllocation
    )
}

// ---- rasters --------------------------------------------------------------

/// A raster cell type that can be copied.
trait Cell: GdalType + Copy {
    fn from_f64(value: f64) -> Self;
    fn to_f64(self) -> f64;
    /// Candidate NoData values for cells the source does not cover, when it
    /// has none, in order of preference.
    fn fills() -> &'static [f64];
    /// TIFF predictor: 2 for integers, 3 for floats.
    const PREDICTOR: u8;
}

macro_rules! integer_cell {
    ($($t:ty),+) => {$(
        impl Cell for $t {
            fn from_f64(value: f64) -> Self {
                value as $t
            }
            fn to_f64(self) -> f64 {
                self as f64
            }
            fn fills() -> &'static [f64] {
                &[<$t>::MAX as f64, <$t>::MIN as f64, (<$t>::MAX - 1) as f64]
            }
            const PREDICTOR: u8 = 2;
        }
    )+};
}

integer_cell!(u8, i8, u16, i16, u32, i32, u64, i64);

impl Cell for f32 {
    fn from_f64(value: f64) -> Self {
        value as f32
    }
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
    fn fills() -> &'static [f64] {
        &[f64::NAN]
    }
    const PREDICTOR: u8 = 3;
}

impl Cell for f64 {
    fn from_f64(value: f64) -> Self {
        value
    }
    fn to_f64(self) -> f64 {
        self
    }
    fn fills() -> &'static [f64] {
        &[f64::NAN]
    }
    const PREDICTOR: u8 = 3;
}

fn copy_raster(
    source: &Path,
    target: &Path,
    grid: &GridSpec,
    class: DataClass,
    settings: &ExecutionSettings,
    cancelled: Cancelled<'_>,
) -> Result<Summary> {
    let role = class.label();
    let (dataset, alignment) = if integer_class(class) {
        open_class_raster(source, grid, role)?
    } else {
        open_aligned_raster(source, grid, role)?
    };
    let band = AlignedBand::new(&dataset, source, alignment, grid)?;
    let band_type = dataset
        .rasterband(1)
        .map_err(Error::gdal(source))?
        .band_type();
    let threads = settings.threads;
    match band_type {
        GdalDataType::UInt8 => copy_typed::<u8>(&band, target, grid, threads, cancelled),
        GdalDataType::Int8 => copy_typed::<i8>(&band, target, grid, threads, cancelled),
        GdalDataType::UInt16 => copy_typed::<u16>(&band, target, grid, threads, cancelled),
        GdalDataType::Int16 => copy_typed::<i16>(&band, target, grid, threads, cancelled),
        GdalDataType::UInt32 => copy_typed::<u32>(&band, target, grid, threads, cancelled),
        GdalDataType::Int32 => copy_typed::<i32>(&band, target, grid, threads, cancelled),
        GdalDataType::UInt64 => copy_typed::<u64>(&band, target, grid, threads, cancelled),
        GdalDataType::Int64 => copy_typed::<i64>(&band, target, grid, threads, cancelled),
        GdalDataType::Float32 => copy_typed::<f32>(&band, target, grid, threads, cancelled),
        GdalDataType::Float64 => copy_typed::<f64>(&band, target, grid, threads, cancelled),
        other => Err(Error::invalid(format!(
            "{role} {}: unsupported band type {}",
            source.display(),
            other.name()
        ))),
    }?;
    preview::build(target, resampling(class), threads, cancelled).map(Summary::Raster)
}

/// A NoData value for cells the source does not cover, when it has none:
/// one the source never holds, so no value turns into NoData.
fn free_fill<T: Cell>(band: &AlignedBand<'_>, cancelled: Cancelled<'_>) -> Result<f64> {
    let candidates = T::fills();
    let mut present = vec![false; candidates.len()];
    band.for_each_covered_row::<T>(cancelled, |_, _, values| {
        for value in values {
            let value = value.to_f64();
            for (seen, candidate) in present.iter_mut().zip(candidates) {
                // NaN equals nothing: it is always free.
                *seen |= value == *candidate;
            }
        }
        Ok(())
    })?;
    candidates
        .iter()
        .zip(&present)
        .find(|(_, seen)| !**seen)
        .map(|(value, _)| *value)
        .ok_or_else(|| {
            Error::invalid(
                "the raster covers part of the project grid, has no NoData value, and holds \
                 every candidate NoData value of its type: set a NoData value before importing",
            )
        })
}

fn copy_typed<T: Cell>(
    band: &AlignedBand<'_>,
    target: &Path,
    grid: &GridSpec,
    threads: usize,
    cancelled: Cancelled<'_>,
) -> Result<()> {
    let (covered_rows, covered_cols) = band.coverage();
    let whole = covered_rows == (0..grid.rows) && covered_cols == (0..grid.cols);
    let nodata = match band.nodata() {
        Some(value) => Some(value),
        None if whole => None,
        None => Some(free_fill::<T>(band, cancelled)?),
    };
    let out = GeoTiff::create_with::<T>(target, grid, T::PREDICTOR, threads, nodata)?;
    let fill = T::from_f64(nodata.unwrap_or(0.0));
    let mut buffer: Vec<T> = Vec::new();
    for_each_chunk(0..grid.rows, 0..grid.cols, cancelled, |rows, cols| {
        let window = band.read::<T>(rows.clone(), cols.clone())?;
        buffer.clear();
        for row in rows.clone() {
            for col in cols.clone() {
                let value = window.as_ref().and_then(|w| w.get(row, col));
                buffer.push(value.unwrap_or(fill));
            }
        }
        out.write(rows, cols, &mut buffer)
    })?;
    out.close()
}

// ---- vectors --------------------------------------------------------------

/// Geometry families a vector class accepts.
fn accepts(class: DataClass, family: Family) -> bool {
    use Family::*;
    match class {
        DataClass::Facility | DataClass::FacilityNew => family == Point,
        DataClass::Road => family == Line,
        DataClass::Barrier => matches!(family, Line | Polygon),
        DataClass::Zone | DataClass::Catchment | DataClass::CatchmentNew => family == Polygon,
        _ => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Point,
    Line,
    Polygon,
    Other,
}

fn family(kind: OGRwkbGeometryType::Type) -> Family {
    // SAFETY: a pure function of its argument.
    let flat = unsafe { gdal_sys::OGR_GT_Flatten(kind) };
    match flat {
        OGRwkbGeometryType::wkbPoint | OGRwkbGeometryType::wkbMultiPoint => Family::Point,
        OGRwkbGeometryType::wkbLineString | OGRwkbGeometryType::wkbMultiLineString => Family::Line,
        OGRwkbGeometryType::wkbPolygon | OGRwkbGeometryType::wkbMultiPolygon => Family::Polygon,
        _ => Family::Other,
    }
}

fn copy_vector(
    source: &Path,
    target: &Path,
    grid: &GridSpec,
    class: DataClass,
    layer: Option<&str>,
    cancelled: Cancelled<'_>,
) -> Result<Summary> {
    let role = class.label();
    let input = VectorLayer::open(source, layer, None, grid, role)?;
    let dataset = input.dataset()?;
    let mut layer = input.layer(&dataset)?;
    let fields: Vec<(String, gdal::vector::OGRFieldType::Type)> = layer
        .defn()
        .fields()
        .map(|f| (f.name(), f.field_type()))
        .collect();
    if let Some((name, _)) = fields.iter().find(|(_, t)| is_list(*t)) {
        return Err(input.invalid(format!(
            "field {name:?} holds a list, which a project layer (FlatGeobuf) cannot store"
        )));
    }
    let kind = layer
        .defn()
        .geom_fields()
        .next()
        .map_or(OGRwkbGeometryType::wkbUnknown, |g| g.field_type());
    let field_refs: Vec<(&str, gdal::vector::OGRFieldType::Type)> =
        fields.iter().map(|(n, t)| (n.as_str(), *t)).collect();
    let mut out = FeatureWriter::create(target, &input.name, &grid.srs, kind, &field_refs)?;
    let mut count = 0u64;
    for (index, feature) in layer.features().enumerate() {
        if index % 1024 == 0 && cancelled() {
            return Err(Error::Cancelled);
        }
        let problem = |issue: &str| {
            input.invalid(format!(
                "feature {} {issue}",
                feature.fid().map_or(index as u64, |f| f)
            ))
        };
        let Some(geometry) = feature.geometry() else {
            return Err(problem("has no geometry"));
        };
        let geometry_family = family(geometry.geometry_type());
        if !accepts(class, geometry_family) {
            return Err(problem(&format!(
                "is a {}, which a {role} layer cannot hold",
                geometry.geometry_name().to_lowercase()
            )));
        }
        let geometry: Geometry = input.to_grid_geometry(geometry).map_err(problem)?;
        let values = fields
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                feature
                    .field(i)
                    .map_err(|e| problem(&format!("field {name:?} cannot be read ({e})")))
            })
            .collect::<Result<Vec<_>>>()?;
        out.write(geometry, &values)?;
        count += 1;
    }
    out.close()?;
    Ok(Summary::Vector(VectorSummary {
        features: count,
        geometry: geometry_name(kind),
        fields: fields.into_iter().map(|(n, _)| n).collect(),
    }))
}

/// Whether an OGR field type is a list.
fn is_list(kind: gdal::vector::OGRFieldType::Type) -> bool {
    use gdal::vector::OGRFieldType as T;
    [
        T::OFTIntegerList,
        T::OFTInteger64List,
        T::OFTRealList,
        T::OFTStringList,
    ]
    .contains(&kind)
}

fn geometry_name(kind: OGRwkbGeometryType::Type) -> String {
    // SAFETY: OGRGeometryTypeToName returns a static string for any type.
    unsafe {
        std::ffi::CStr::from_ptr(gdal_sys::OGRGeometryTypeToName(kind))
            .to_string_lossy()
            .into_owned()
    }
}

/// Summary of a vector file already written (run outputs).
fn vector_summary(path: &Path) -> Result<VectorSummary> {
    let dataset = gdal::Dataset::open(path).map_err(Error::gdal(path))?;
    let layer = dataset.layer(0).map_err(Error::gdal(path))?;
    let kind = layer
        .defn()
        .geom_fields()
        .next()
        .map_or(OGRwkbGeometryType::wkbUnknown, |g| g.field_type());
    Ok(VectorSummary {
        features: layer.feature_count(),
        geometry: geometry_name(kind),
        fields: layer.defn().fields().map(|f| f.name()).collect(),
    })
}

// ---- tables ---------------------------------------------------------------

fn copy_table(source: &Path, target: &Path) -> Result<Summary> {
    let summary = table_summary(source)?;
    std::fs::copy(source, target).map_err(Error::file(target))?;
    Ok(Summary::Table(summary))
}

/// Largest CSV record read, bytes.
const MAX_RECORD_BYTES: usize = 1 << 20;

/// Header and record count of a CSV file (RFC 4180).
fn table_summary(path: &Path) -> Result<TableSummary> {
    let file = std::fs::File::open(path).map_err(Error::file(path))?;
    let mut columns: Option<Vec<String>> = None;
    let mut rows = 0u64;
    crate::output::read_csv(file, MAX_RECORD_BYTES, |record| {
        match &columns {
            None => {
                columns = Some(
                    record
                        .iter()
                        .map(|c| c.trim_start_matches('\u{feff}').trim().to_owned())
                        .collect(),
                );
            }
            Some(_) => rows += 1,
        }
        Ok(())
    })
    .map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    let columns =
        columns.ok_or_else(|| Error::invalid(format!("{}: empty table", path.display())))?;
    Ok(TableSummary { rows, columns })
}

/// GDAL drivers a file sent by a user may be read with: formats that never
/// read other files they name. A VRT, for one, would open any file of the
/// server; GDAL identifies formats by content, so the extension is no guard.
const SELF_CONTAINED: &[&str] = &["GTiff", "ESRI Shapefile", "GPKG", "FlatGeobuf", "GeoJSON"];

/// Check that `path`, a file sent by a user (an upload), reads nothing but
/// itself and its companion files: a GeoTIFF, shapefile, GeoPackage,
/// FlatGeobuf or GeoJSON layer, or a CSV table (copied as it is).
pub fn check_self_contained(path: &Path) -> Result<()> {
    let csv = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
    if csv {
        return Ok(());
    }
    let options = gdal::DatasetOptions {
        open_flags: gdal::GdalOpenFlags::GDAL_OF_RASTER
            | gdal::GdalOpenFlags::GDAL_OF_VECTOR
            | gdal::GdalOpenFlags::GDAL_OF_READONLY,
        allowed_drivers: Some(SELF_CONTAINED),
        ..gdal::DatasetOptions::default()
    };
    gdal::Dataset::open_ex(path, options).map(drop).map_err(|_| {
        Error::invalid(format!(
            "{}: not a supported file (GeoTIFF, shapefile, GeoPackage, FlatGeobuf, GeoJSON or CSV)",
            path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_accept_their_geometries() {
        assert!(accepts(DataClass::Facility, Family::Point));
        assert!(!accepts(DataClass::Facility, Family::Polygon));
        assert!(accepts(DataClass::Barrier, Family::Line));
        assert!(accepts(DataClass::Barrier, Family::Polygon));
        assert!(!accepts(DataClass::Zone, Family::Line));
        assert!(accepts(DataClass::ExclusionVector, Family::Point));
    }

    #[test]
    fn multi_geometries_share_their_family() {
        assert_eq!(family(OGRwkbGeometryType::wkbMultiPolygon), Family::Polygon);
        assert_eq!(family(OGRwkbGeometryType::wkbPoint25D), Family::Point);
        assert_eq!(family(OGRwkbGeometryType::wkbLineStringZM), Family::Line);
    }
}
