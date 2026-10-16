//! AccessMod I/O boundary: GDAL/PROJ rasters and vectors, job validation,
//! canonical outputs and run reports.
//!
//! Everything that touches files or GDAL lives here; the numerical engine is
//! `accessmod-core`.

pub mod analysis;
pub mod error;
pub mod facilities;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod grid;
pub mod inputs;
pub mod job;
pub mod merge;
pub mod output;
pub mod raster;
pub mod referral;
pub mod report;
pub mod resources;
pub mod travel_time;
pub mod vector;
