//! AccessMod I/O boundary: GDAL/PROJ rasters and vectors, job validation,
//! canonical outputs and run reports.
//!
//! Everything that touches files or GDAL lives here; the numerical engine is
//! `accessmod-core`.

pub mod am5;
pub mod analysis;
pub mod archive;
pub(crate) mod catchment;
pub mod class;
pub mod coverage;
pub mod demo;
pub mod error;
pub mod facilities;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod grid;
pub mod inputs;
pub mod install;
pub mod job;
pub mod merge;
pub mod optimization;
pub mod output;
pub mod population;
pub mod preview;
pub mod project;
pub mod projection;
pub mod raster;
pub mod referral;
pub mod report;
pub mod resources;
pub mod rows;
pub mod scaling_up;
pub mod tiles;
pub mod travel_time;
pub mod vector;
pub mod warp;
pub mod zonal;
