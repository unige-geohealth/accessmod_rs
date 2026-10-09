//! AccessMod I/O boundary: GDAL/PROJ rasters and vectors, job validation,
//! canonical outputs and run reports.
//!
//! Everything that touches files or GDAL lives here; the numerical engine is
//! `accessmod-core`.

pub mod error;
pub mod facilities;
pub mod grid;
pub mod inputs;
pub mod job;
pub mod output;
pub mod resources;
pub mod travel_time;
