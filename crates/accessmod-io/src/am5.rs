//! AccessMod 5 data archives (Data manager › Archive) imported as a new
//! project: the way AccessMod 5 projects come here, with no change to
//! AccessMod 5 (a `.am5p` is a GRASS database, never read here).
//!
//! An archive holds one directory per dataset, named by AccessMod 5's
//! `amGetNameConvertExport`: type, class label in the user's language and
//! tags, with accents removed and punctuation turned into `_`, e.g.
//! `raster_land_cover_merged_demo/raster_land_cover_merged_demo.img`. The
//! class is found by matching that name against the labels of every
//! language ([`LABELS`], from AccessMod 5's `classes.json`); labels prefix
//! one another (`raster_land_cover` < `raster_land_cover_merged`), so the
//! longest wins and the rest are the tags. Rasters are ERDAS Imagine,
//! vectors shapefiles (or GeoPackages), tables XLSX; analysis parameters
//! (JSON, for AccessMod 5's replay) are not imported.
//!
//! The import is whole or nothing: the DEM defines the grid, every raster
//! must lie on it, every file must be self-contained; one misfit refuses
//! the archive, with a report per file.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::analysis::Monitor;
use crate::archive::{self, Limits};
use crate::class::{DataClass, Kind};
use crate::error::{Error, Result};
use crate::grid::GridSpec;
use crate::project::{Project, check_self_contained, normalize_tags};
use crate::resources::ExecutionSettings;

/// Progress stage: checking the datasets against the DEM.
pub const STAGE_CHECK: &str = "check";
/// Progress stage: importing the datasets.
pub const STAGE_IMPORT: &str = "import";

/// AccessMod 5 classes with their type and labels per language
/// (`cargo xtask i18n`).
const LABELS: &str = include_str!("am5_classes.json");

#[derive(Deserialize)]
struct Label {
    class: String,
    #[serde(rename = "type")]
    kind: String,
    en: String,
    fr: String,
    es: String,
    pt: String,
}

/// Every `type_label` name prefix with its AccessMod 5 class and type,
/// longest first.
fn prefixes() -> Vec<(String, String, String)> {
    let labels: Vec<Label> = serde_json::from_str(LABELS).unwrap_or_default();
    let mut out: Vec<(String, String, String)> = Vec::new();
    for label in &labels {
        for text in [&label.en, &label.fr, &label.es, &label.pt] {
            if text.trim().is_empty() {
                continue;
            }
            let prefix = sub_punct(&format!("{} {}", label.kind, text)).to_lowercase();
            if !out
                .iter()
                .any(|(p, c, _)| *p == prefix && *c == label.class)
            {
                out.push((prefix, label.class.clone(), label.kind.clone()));
            }
        }
    }
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    out
}

/// AccessMod 5's `amSubPunct`: Latin letters to ASCII, runs of punctuation
/// and blanks to one `_`, none leading or trailing.
fn sub_punct(text: &str) -> String {
    let ascii = crate::project::latin_to_ascii(text);
    let mut out = String::with_capacity(ascii.len());
    for c in ascii.chars() {
        if c.is_ascii_punctuation() || c.is_whitespace() {
            if !out.ends_with('_') {
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out.trim_matches('_').to_owned()
}

/// A directory name of an archive, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// AccessMod 5 class identifier, e.g. `rLandCoverMerged`.
    pub code: String,
    /// AccessMod 5 type: `raster`, `vector`, `shape`, `table` or `config`.
    pub kind: String,
    /// Tags, normalized.
    pub tags: Vec<String>,
}

/// Read the directory name `name` of an archive; `None` when it names no
/// AccessMod 5 class. The longest label wins: see [`parse_names`].
pub fn parse_name(name: &str) -> Option<Name> {
    parse_names(name).into_iter().next()
}

/// Every reading of the directory name `name`, longest label first, one per
/// class. AccessMod 5 joins label and tags with `_` like the words of a
/// label: `raster_land_cover_merged_demo` is a merged land cover tagged
/// `demo`, or a land cover tagged `merged demo`.
pub fn parse_names(name: &str) -> Vec<Name> {
    let lower = name.to_lowercase();
    let mut out: Vec<Name> = Vec::new();
    for found in prefixes().into_iter().filter_map(|(prefix, code, kind)| {
        let rest = if lower == prefix {
            ""
        } else {
            lower.strip_prefix(&prefix)?.strip_prefix('_')?
        };
        // Tags keep their case: the same end of the original name, when
        // lower case kept its length (else the lower-case tags).
        let rest = name
            .len()
            .checked_sub(rest.len())
            .and_then(|start| name.get(start..))
            .filter(|original| original.to_lowercase() == rest)
            .unwrap_or(rest);
        Some(Name {
            code,
            kind,
            tags: normalize_tags(&rest.split('_').collect::<Vec<_>>()),
        })
    }) {
        if !out.iter().any(|n| n.code == found.code) {
            out.push(found);
        }
    }
    out
}

/// A dataset of an archive to import.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Directory in the archive.
    pub name: String,
    /// File chosen in it.
    pub file: PathBuf,
    /// Class.
    pub class: DataClass,
    /// Tags.
    pub tags: Vec<String>,
}

/// Something of an archive that is not imported, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Skipped {
    /// Directory or file in the archive.
    pub name: String,
    /// Why it is left out.
    pub reason: String,
}

/// What an archive holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Contents {
    /// The DEM, which defines the grid.
    pub dem: Entry,
    /// Other datasets, in name order.
    pub datasets: Vec<Entry>,
    /// What is left out.
    pub skipped: Vec<Skipped>,
    /// Datasets whose name also reads as another class.
    pub ambiguous: Vec<String>,
    /// Datasets that cannot be imported, with why: the archive is refused.
    pub misfits: Vec<String>,
}

/// Extensions read for a kind, preferred first: GeoPackage keeps column
/// names a shapefile cuts to 10 characters.
fn extensions(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Raster => &["img", "tif", "tiff", "geotiff"],
        Kind::Vector => &["gpkg", "fgb", "shp", "geojson"],
        Kind::Table => &["xlsx", "csv"],
        Kind::Config => &[],
    }
}

/// The file of `kind` in `dir`, by preferred extension, then name.
fn pick(dir: &Path, kind: Kind) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .collect();
    files.sort();
    extensions(kind).iter().find_map(|ext| {
        files
            .iter()
            .find(|f| {
                f.extension()
                    .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
            })
            .cloned()
    })
}

/// The directory holding the datasets: `root`, or the single directory
/// it holds when the archive was zipped from a folder.
fn datasets_root(root: &Path) -> PathBuf {
    let mut dir = root.to_path_buf();
    for _ in 0..4 {
        let entries: Vec<_> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries.flatten().collect(),
            Err(_) => break,
        };
        match entries.as_slice() {
            [only]
                if only.file_type().is_ok_and(|t| t.is_dir())
                    && parse_name(&only.file_name().to_string_lossy()).is_none() =>
            {
                dir = only.path();
            }
            _ => break,
        }
    }
    dir
}

/// What the extracted archive `root` holds; an error without exactly one
/// DEM.
pub fn scan(root: &Path) -> Result<Contents> {
    let root = datasets_root(root);
    let mut names: Vec<(String, PathBuf, bool)> = std::fs::read_dir(&root)
        .map_err(Error::file(&root))?
        .flatten()
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            (
                e.file_name().to_string_lossy().into_owned(),
                e.path(),
                is_dir,
            )
        })
        .collect();
    names.sort();
    let mut dems = Vec::new();
    let mut datasets: Vec<Entry> = Vec::new();
    let mut skipped = Vec::new();
    let mut ambiguous = Vec::new();
    let mut misfits = Vec::new();
    let mut skip = |name: &str, reason: String| {
        skipped.push(Skipped {
            name: name.to_owned(),
            reason,
        });
    };
    for (name, path, is_dir) in names {
        if !is_dir {
            skip(&name, "not a dataset directory".to_owned());
            continue;
        }
        let readings = parse_names(&name);
        let Some(first) = readings.first() else {
            skip(&name, "no AccessMod 5 data class in its name".to_owned());
            continue;
        };
        if first.kind == "config" {
            skip(
                &name,
                "AccessMod 5 analysis parameters (replay) are not imported".to_owned(),
            );
            continue;
        }
        // The longest label of a class used here; a shorter one is the
        // same name read with more tags.
        let mut usable = readings
            .iter()
            .filter(|r| r.kind != "config")
            .filter_map(|r| DataClass::from_code(&r.code).map(|c| (c, r)));
        let Some((class, parsed)) = usable.next() else {
            skip(
                &name,
                format!("AccessMod 5 class {} is not used here", first.code),
            );
            continue;
        };
        if usable.next().is_some() {
            ambiguous.push(name.clone());
        }
        let parsed = parsed.clone();
        let Some(file) = pick(&path, class.kind()) else {
            misfits.push(format!(
                "- {name}: no {} file in it (damaged archive?)",
                kind_name(class.kind())
            ));
            continue;
        };
        let entry = Entry {
            name: name.clone(),
            file,
            class,
            tags: parsed.tags,
        };
        if class == DataClass::Dem {
            dems.push(entry);
        } else if let Some(other) = datasets
            .iter()
            .find(|d| d.class == class && d.tags == entry.tags)
        {
            skip(&name, format!("same class and tags as {}", other.name));
        } else {
            datasets.push(entry);
        }
    }
    let dem = match dems.len() {
        1 => dems.remove(0),
        0 => {
            return Err(Error::invalid(
                "the archive holds no DEM: in AccessMod 5, select the DEM with the other \
                 data before creating the archive",
            ));
        }
        _ => {
            let list: Vec<&str> = dems.iter().map(|d| d.name.as_str()).collect();
            return Err(Error::invalid(format!(
                "the archive holds several DEMs ({}): a project has one",
                list.join(", ")
            )));
        }
    };
    Ok(Contents {
        dem,
        datasets,
        skipped,
        ambiguous,
        misfits,
    })
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Raster => "raster",
        Kind::Vector => "vector",
        Kind::Table => "table",
        Kind::Config => "parameters",
    }
}

/// Whether the zip file names `names` look like an AccessMod 5 data
/// archive: no manifest, and a DEM directory.
pub fn is_archive(names: &[String]) -> bool {
    !names.iter().any(|n| n == archive::MANIFEST)
        && names.iter().any(|n| {
            n.split('/')
                .filter(|part| !part.is_empty())
                .rev()
                .nth(1)
                .and_then(parse_name)
                .is_some_and(|p| p.code == DataClass::Dem.code())
        })
}

/// Project name from an archive file name: its stem without AccessMod 5's
/// `_YYYY_MM_DD@HH_MM` stamp.
pub fn project_name(file: &Path) -> String {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // `_YYYY_MM_DD@HH_MM`: 11 bytes, `@`, 5 bytes.
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit() || c == '_');
    let unstamped = stem.rsplit_once('@').and_then(|(head, time)| {
        let start = head.len().checked_sub(11)?;
        let date = head.get(start..)?;
        (time.len() == 5 && digits(time) && date.starts_with('_') && digits(date))
            .then(|| head.get(..start))
            .flatten()
    });
    let name = unstamped.unwrap_or(&stem);
    let name = name.replace('_', " ").trim().to_owned();
    if name.is_empty() {
        "AccessMod 5 project".to_owned()
    } else {
        name
    }
}

/// Outcome of an import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// Datasets imported, the DEM included.
    pub imported: usize,
    /// What was left out.
    pub skipped: Vec<Skipped>,
    /// What changed on the way.
    pub warnings: Vec<String>,
}

impl Report {
    /// One line: datasets imported, what was left out grouped by reason
    /// (names listed up to a few), warnings.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("imported {} datasets", self.imported)];
        let mut reasons: Vec<(&str, Vec<&str>)> = Vec::new();
        for skipped in &self.skipped {
            match reasons.iter_mut().find(|(r, _)| *r == skipped.reason) {
                Some((_, names)) => names.push(&skipped.name),
                None => reasons.push((&skipped.reason, vec![&skipped.name])),
            }
        }
        for (reason, names) in reasons {
            parts.push(format!("left out ({reason}): {}", list(&names)));
        }
        parts.extend(self.warnings.iter().cloned());
        parts.join("; ")
    }
}

/// `names`, or the first two and how many more.
fn list(names: &[&str]) -> String {
    if names.len() <= 3 {
        names.join(", ")
    } else {
        format!("{} and {} more", names[..2].join(", "), names.len() - 2)
    }
}

/// Import the AccessMod 5 data archive `archive` as the new project
/// directory `target` named `name`: extracted next to it, checked whole,
/// then built; on any error nothing is left.
pub fn import_archive(
    archive: &Path,
    target: &Path,
    name: &str,
    limits: Limits,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(Project, Report)> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::invalid("a project directory needs a parent"))?;
    let staging = parent.join(format!(
        ".am5-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let result = archive::extract_plain(archive, &staging, limits, monitor)
        .and_then(|()| build(archive, &staging, target, name, settings, monitor));
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn build(
    archive: &Path,
    staging: &Path,
    target: &Path,
    name: &str,
    settings: &ExecutionSettings,
    monitor: Monitor<'_>,
) -> Result<(Project, Report)> {
    let contents = scan(staging)?;
    monitor.stage(STAGE_CHECK);
    check(&contents, monitor)?;
    let file_name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let origin = |entry: &Entry| {
        let file = entry
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{file_name}/{}/{file}", entry.name)
    };
    let cancelled = monitor.cancelled();
    let total = contents.datasets.len() as u64 + 1;
    monitor.advance(STAGE_IMPORT, 0, Some(total));
    let project = Project::create_with(
        target,
        name,
        "",
        &contents.dem.file,
        settings,
        cancelled,
        |project| {
            let dem = project.info()?.dem;
            if !contents.dem.tags.is_empty() {
                project.retag(dem, &contents.dem.tags)?;
            }
            monitor.advance(STAGE_IMPORT, 1, Some(total));
            for (done, entry) in contents.datasets.iter().enumerate() {
                project
                    .import_am5(
                        &entry.file,
                        &origin(entry),
                        entry.class,
                        &entry.tags,
                        settings,
                        cancelled,
                    )
                    .map_err(|e| named(&entry.name, e))?;
                monitor.advance(STAGE_IMPORT, done as u64 + 2, Some(total));
            }
            Ok(())
        },
    )?;
    let mut warnings = Vec::new();
    let shapefiles: Vec<&str> = contents
        .datasets
        .iter()
        .filter(|d| {
            d.file
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
        })
        .map(|d| d.name.as_str())
        .collect();
    if !shapefiles.is_empty() {
        warnings.push(format!(
            "AccessMod 5 wrote shapefiles, which cut column names to 10 characters: {}",
            list(&shapefiles)
        ));
    }
    let minutes: Vec<&str> = contents
        .datasets
        .iter()
        .filter(|d| {
            matches!(
                d.class,
                DataClass::TravelTime | DataClass::TravelTimeImported
            )
        })
        .map(|d| d.name.as_str())
        .collect();
    if !minutes.is_empty() {
        warnings.push(format!(
            "AccessMod 5 travel times (minutes) are stored in seconds: {}",
            list(&minutes)
        ));
    }
    if !contents.ambiguous.is_empty() {
        let names: Vec<&str> = contents.ambiguous.iter().map(String::as_str).collect();
        warnings.push(format!(
            "names that also read as another class with more tags (check class and tags): {}",
            list(&names)
        ));
    }
    let report = Report {
        imported: contents.datasets.len() + 1,
        skipped: contents.skipped,
        warnings,
    };
    Ok((project, report))
}

/// Every file self-contained, every raster on the DEM grid: all misfits
/// in one error, before anything is written.
fn check(contents: &Contents, monitor: Monitor<'_>) -> Result<()> {
    let grid = check_self_contained(&contents.dem.file)
        .and_then(|()| GridSpec::open_dem(&contents.dem.file))
        .map(|(_, grid)| grid)
        .map_err(|e| named(&contents.dem.name, e))?;
    let mut misfits = contents.misfits.clone();
    for entry in &contents.datasets {
        if monitor.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let checked = check_self_contained(&entry.file).and_then(|()| match entry.class.kind() {
            Kind::Raster => {
                crate::raster::open_aligned_raster(&entry.file, &grid, entry.class.label())
                    .map(|_| ())
            }
            _ => Ok(()),
        });
        if let Err(error) = checked {
            misfits.push(format!("- {}: {}", entry.name, message(&error)));
        }
    }
    if misfits.is_empty() {
        return Ok(());
    }
    Err(Error::invalid(format!(
        "the archive does not fit the grid of its DEM ({}), nothing was imported:\n{}\n\
         Create the project from the DEM, then import the files one by one.",
        contents.dem.name,
        misfits.join("\n")
    )))
}

/// The text of an error, validation issues joined.
fn message(error: &Error) -> String {
    match error {
        Error::Validation { issues } => issues.join("; "),
        other => other.to_string(),
    }
}

/// `error` about the dataset `name` of the archive; cancellation stays one.
fn named(name: &str, error: Error) -> Error {
    match error {
        Error::Cancelled => Error::Cancelled,
        other => Error::invalid(format!("{name}: {}", message(&other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_never_collide() {
        let prefixes = prefixes();
        assert!(prefixes.len() > 100, "{} prefixes", prefixes.len());
        for (i, (a, ca, _)) in prefixes.iter().enumerate() {
            for (b, cb, _) in &prefixes[i + 1..] {
                assert!(a != b || ca == cb, "{a} names {ca} and {cb}");
            }
        }
    }

    #[test]
    fn names_give_class_and_tags_longest_label_first() {
        let cases = [
            ("raster_dem_dem", "rDem", vec!["dem"]),
            ("raster_land_cover_demo", "rLandCover", vec!["demo"]),
            (
                "raster_land_cover_merged_demo",
                "rLandCoverMerged",
                vec!["demo"],
            ),
            (
                "table_scenario_processed_scenario_velo",
                "tScenarioOut",
                vec!["scenario", "velo"],
            ),
            ("table_scenario_demo", "tScenario", vec!["demo"]),
            (
                "vector_facility_scaling_up_to_remove",
                "vFacilityNew",
                vec!["to", "remove"],
            ),
            (
                "table_referral_nearest_by_time_test",
                "tReferralTime",
                vec!["test"],
            ),
            ("vector_road_64", "vRoad", vec!["64"]),
            (
                "raster_occupation_du_sol_fusionnee_Base",
                "rLandCoverMerged",
                vec!["Base"],
            ),
            (
                "config_analysis_parameters_ca",
                "lAnalysisParameters",
                vec!["ca"],
            ),
        ];
        for (name, code, tags) in cases {
            let parsed = parse_name(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(parsed.code, code, "{name}");
            assert_eq!(parsed.tags, tags, "{name}");
        }
        assert_eq!(parse_name("readme"), None);
        assert_eq!(parse_name("raster_land_coverage"), None);
    }

    #[test]
    fn names_read_as_every_class_whose_label_prefixes_them() {
        let codes = |name: &str| {
            parse_names(name)
                .into_iter()
                .map(|n| (n.code, n.tags.join(" ")))
                .collect::<Vec<_>>()
        };
        let pair = |c: &str, t: &str| (c.to_owned(), t.to_owned());
        assert_eq!(
            codes("raster_land_cover_merged_demo"),
            [
                pair("rLandCoverMerged", "demo"),
                pair("rLandCover", "merged demo")
            ]
        );
        assert_eq!(
            codes("raster_land_cover_merged_bridge_demo"),
            [
                pair("rLandCoverBridge", "demo"),
                pair("rLandCoverMerged", "bridge demo"),
                pair("rLandCover", "merged bridge demo")
            ]
        );
        assert_eq!(codes("vector_road_demo"), [pair("vRoad", "demo")]);
    }

    #[test]
    fn project_names_drop_the_archive_stamp() {
        let name = |f: &str| project_name(Path::new(f));
        assert_eq!(
            name("accessmod_6_demo_transfert_2026_11_13@16_35.zip"),
            "accessmod 6 demo transfert"
        );
        assert_eq!(name("am5_export_2024_01_02@03_04.zip"), "am5 export");
        assert_eq!(name("my_project.zip"), "my project");
        assert_eq!(name("_2024_01_02@03_04.zip"), "AccessMod 5 project");
        assert_eq!(name("@.zip"), "@");
    }

    #[test]
    fn archives_are_told_by_a_dem_directory() {
        let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(is_archive(&names(&[
            "raster_dem_dem/raster_dem_dem.img",
            "vector_road_demo/vector_road_demo.shp"
        ])));
        assert!(is_archive(&names(&["x/raster_dem_dem/raster_dem_dem.img"])));
        assert!(!is_archive(&names(&[
            "manifest.json",
            "raster_dem_dem/raster_dem_dem.img"
        ])));
        assert!(!is_archive(&names(&[
            "vector_road_demo/vector_road_demo.shp"
        ])));
    }
}
