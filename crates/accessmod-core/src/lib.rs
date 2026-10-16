//! AccessMod engine core.
//!
//! Domain types, mobility models and the travel-time cost model. This crate
//! never depends on GDAL, PROJ, a project layout or an external process.
//!
//! Units: times in seconds, distances in metres, speeds in km/h (scenario
//! input only), slopes as ratios (`Δz / L`, not percent).

#![forbid(unsafe_code)]

pub mod cost;
pub mod merge;
pub mod mobility;
pub mod raster;
pub mod referral;
pub mod solver;
pub mod store;
