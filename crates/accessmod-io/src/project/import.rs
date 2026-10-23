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

    /// Register `file` (relative to the project, already in place) as a
    /// dataset of `class`, e.g. a run output.
    pub(super) fn register(
        &self,
        file: &str,
        class: DataClass,
        tags: &[String],
        origin: &Origin,
        settings: &ExecutionSettings,
        cancelled: Cancelled<'_>,
    ) -> Result<DatasetRecord> {
        let (id, _) = self.reserve(class, tags, origin)?;
        let result = self
            .resolve(Path::new(file))
            .and_then(|path| summarise(&path, class, settings, cancelled))
            .and_then(|summary| self.complete(id, file, &summary, cancelled));
        if result.is_err() {
            // The file belongs to the run: keep it, drop the row only.
            let _ = self.db.execute("DELETE FROM datasets WHERE id = ?1", [id]);
        }
        result
    }
}

/// Summary of a file already written.
fn summarise(
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
    /// NoData for cells the source does not cover, when it has none.
    fn fill() -> f64;
    /// TIFF predictor: 2 for integers, 3 for floats.
    const PREDICTOR: u8;
}

macro_rules! integer_cell {
    ($($t:ty => $fill:expr),+) => {$(
        impl Cell for $t {
            fn from_f64(value: f64) -> Self { value as $t }
            fn fill() -> f64 { $fill as f64 }
            const PREDICTOR: u8 = 2;
        }
    )+};
}

integer_cell!(
    u8 => u8::MAX, i8 => i8::MIN, u16 => u16::MAX, i16 => i16::MIN,
    u32 => u32::MAX, i32 => i32::MIN, u64 => u64::MAX, i64 => i64::MIN
);

impl Cell for f32 {
    fn from_f64(value: f64) -> Self {
        value as f32
    }
    fn fill() -> f64 {
        f64::NAN
    }
    const PREDICTOR: u8 = 3;
}

impl Cell for f64 {
    fn from_f64(value: f64) -> Self {
        value
    }
    fn fill() -> f64 {
        f64::NAN
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
        None => Some(T::fill()),
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
        let values: Vec<_> = (0..fields.len())
            .map(|i| feature.field(i).ok().flatten())
            .collect();
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

/// Header and row count of a CSV file. Quoted fields with commas or line
/// breaks in the header are not split correctly; data rows are counted as
/// lines.
fn table_summary(path: &Path) -> Result<TableSummary> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(Error::file(path))?;
    let mut lines = std::io::BufReader::new(file).lines();
    let header = lines
        .next()
        .transpose()
        .map_err(Error::file(path))?
        .ok_or_else(|| Error::invalid(format!("{}: empty table", path.display())))?;
    let columns = header
        .trim_start_matches('\u{feff}')
        .split(',')
        .map(|c| c.trim().trim_matches('"').to_owned())
        .collect();
    let mut rows = 0u64;
    for line in lines {
        if !line.map_err(Error::file(path))?.trim().is_empty() {
            rows += 1;
        }
    }
    Ok(TableSummary { rows, columns })
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
