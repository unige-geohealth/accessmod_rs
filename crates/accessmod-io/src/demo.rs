//! The demo project: the AccessMod 5 sample data (southern Malawi, 1 km
//! grid), every dataset tagged `demo`.
//!
//! The files live in `crates/accessmod-io/demo/` and are embedded in the
//! binary, so any installation can build the demo again. Rasters are ERDAS
//! Imagine, vectors shapefiles and tables XLSX, as AccessMod 5 ships them:
//! the demo goes through the same imports as a user's files.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::analysis::Cancelled;
use crate::class::DataClass;
use crate::error::{Error, Result};
use crate::project::{ImportOptions, Project};
use crate::resources::ExecutionSettings;

/// Name of the demo project.
pub const NAME: &str = "Demo";
/// Tag of every demo dataset.
pub const TAG: &str = "demo";

const DESCRIPTION: &str = "AccessMod 5 sample data: southern Malawi, 1 km grid.";

/// The DEM, which defines the grid.
const DEM: &str = "rDEM.img";

macro_rules! files {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../demo/", $name)))),*]
    };
}

/// Embedded files.
const FILES: &[(&str, &[u8])] = files![
    "rDEM.img",
    "rLandCover__start.img",
    "rPopulation__start.img",
    "vBarrier__rivers.shp",
    "vBarrier__rivers.shx",
    "vBarrier__rivers.dbf",
    "vBarrier__rivers.prj",
    "vBarrier__wetlands.shp",
    "vBarrier__wetlands.shx",
    "vBarrier__wetlands.dbf",
    "vBarrier__wetlands.prj",
    "Exclusion_area.shp",
    "Exclusion_area.shx",
    "Exclusion_area.dbf",
    "Exclusion_area.prj",
    "vFacility__start.shp",
    "vFacility__start.shx",
    "vFacility__start.dbf",
    "vFacility__start.prj",
    "vRoad__start.shp",
    "vRoad__start.shx",
    "vRoad__start.dbf",
    "vRoad__start.prj",
    "vZone__start.shp",
    "vZone__start.shx",
    "vZone__start.dbf",
    "vZone__start.prj",
    "tLandCover__ldcv.xlsx",
    "tCapacity_scalingUp.xlsx",
];

/// Datasets imported after the DEM: file, class and tags. Class and tags
/// name a dataset uniquely, hence the second tag of the barriers.
pub const DATASETS: &[(&str, DataClass, &[&str])] = &[
    ("rLandCover__start.img", DataClass::LandCover, &[TAG]),
    ("rPopulation__start.img", DataClass::Population, &[TAG]),
    ("vBarrier__rivers.shp", DataClass::Barrier, &[TAG, "rivers"]),
    (
        "vBarrier__wetlands.shp",
        DataClass::Barrier,
        &[TAG, "wetlands"],
    ),
    ("Exclusion_area.shp", DataClass::ExclusionVector, &[TAG]),
    ("vFacility__start.shp", DataClass::Facility, &[TAG]),
    ("vRoad__start.shp", DataClass::Road, &[TAG]),
    ("vZone__start.shp", DataClass::Zone, &[TAG]),
    ("tLandCover__ldcv.xlsx", DataClass::LandCoverTable, &[TAG]),
    ("tCapacity_scalingUp.xlsx", DataClass::Capacity, &[TAG]),
];

/// Build the demo project in `root` (new or empty directory): it appears
/// whole or not at all.
pub fn create(
    root: &Path,
    settings: &ExecutionSettings,
    cancelled: Cancelled<'_>,
) -> Result<Project> {
    let files = SourceFiles::write(root)?;
    Project::create_with(
        root,
        NAME,
        DESCRIPTION,
        &files.0.join(DEM),
        settings,
        cancelled,
        |project| {
            for (file, class, tags) in DATASETS {
                let tags: Vec<String> = tags.iter().map(|t| (*t).to_owned()).collect();
                project.import(
                    &files.0.join(file),
                    *class,
                    &tags,
                    &ImportOptions::default(),
                    settings,
                    cancelled,
                )?;
            }
            project.set_meta("demo", "1")
        },
    )
}

/// The embedded files written to a directory next to the project, removed
/// when dropped.
struct SourceFiles(PathBuf);

impl SourceFiles {
    fn write(root: &Path) -> Result<Self> {
        let parent = match root.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let dir = parent.join(format!(
            ".accessmod-demo-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).map_err(Error::file(&dir))?;
        let files = Self(dir);
        for (name, bytes) in FILES {
            let path = files.0.join(name);
            std::fs::write(&path, bytes).map_err(Error::file(&path))?;
        }
        Ok(files)
    }
}

impl Drop for SourceFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
