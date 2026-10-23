//! Data classes: what a dataset holds, with the identifiers of AccessMod 5
//! (`www/dictionary/classes.json`), so catalogues, job fields and `.am5p`
//! imports share one vocabulary.
//!
//! Job fields that designate a dataset carry the classes they accept in
//! their JSON Schema, as `x-accessmod-class` (see [`SCHEMA_KEY`]): on a path
//! string, or on a layer object whose `path` is the dataset; the lists follow
//! the selectors of the AccessMod 5 screens. Every class `classes.json`
//! marks non-internal is here; internal ones (stacks, speed, friction,
//! bridges) are left out.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Key of the JSON Schema extension listing the classes a field accepts.
pub const SCHEMA_KEY: &str = "x-accessmod-class";

/// Storage of a class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Grid on the project lattice.
    Raster,
    /// Points, lines or polygons.
    Vector,
    /// Rows without geometry.
    Table,
    /// Analysis parameters (a job document).
    Config,
}

macro_rules! classes {
    ($($variant:ident = $code:literal, $kind:ident, $label:literal, $importable:literal;)+) => {
        /// A data class, serialised with its AccessMod 5 identifier.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum DataClass {
            $(
                #[doc = $label]
                #[serde(rename = $code)]
                $variant,
            )+
        }

        impl DataClass {
            /// Every class, in AccessMod 5 order.
            pub const ALL: &[DataClass] = &[$(DataClass::$variant),+];

            /// AccessMod 5 identifier, e.g. `rLandCover`.
            pub fn code(self) -> &'static str {
                match self { $(DataClass::$variant => $code),+ }
            }

            /// Storage of the class.
            pub fn kind(self) -> Kind {
                match self { $(DataClass::$variant => Kind::$kind),+ }
            }

            /// English name (`classes.json`), for messages; the interface
            /// translates the identifier.
            pub fn label(self) -> &'static str {
                match self { $(DataClass::$variant => $label),+ }
            }

            /// Whether users import datasets of this class (the others are
            /// produced by analyses).
            pub fn importable(self) -> bool {
                match self { $(DataClass::$variant => $importable),+ }
            }

            /// The class with AccessMod 5 identifier `code`.
            pub fn from_code(code: &str) -> Option<DataClass> {
                match code { $($code => Some(DataClass::$variant),)+ _ => None }
            }
        }
    };
}

classes! {
    Dem = "rDem", Raster, "dem", true;
    LandCover = "rLandCover", Raster, "land cover", true;
    LandCoverMerged = "rLandCoverMerged", Raster, "land cover merged", true;
    Population = "rPopulation", Raster, "population", true;
    PopulationResidual = "rPopulationResidual", Raster, "population residual", true;
    TravelTime = "rTravelTime", Raster, "travel time", false;
    CostAllocation = "rNearest", Raster, "cost allocation", false;
    TravelTimeImported = "rTravelTimeImported", Raster, "imported travel time", true;
    PopulationOnBarrier = "rPopulationOnBarrier", Raster, "population on barrier", false;
    Priority = "rPriority", Raster, "priority", true;
    Exclusion = "rExclusion", Raster, "exclusion", true;
    Barrier = "vBarrier", Vector, "barrier", true;
    Road = "vRoad", Vector, "road", true;
    Facility = "vFacility", Vector, "facility", true;
    FacilityNew = "vFacilityNew", Vector, "facility scaling up", false;
    Zone = "vZone", Vector, "zone for stat", true;
    ExclusionVector = "vExclusion", Vector, "exclusion", true;
    Catchment = "vCatchment", Vector, "catchment", true;
    CatchmentNew = "vCatchmentNew", Vector, "catchment scaling up", true;
    ReferralNetwork = "vReferralNetwork", Vector, "referral network", false;
    LandCoverTable = "tLandCover", Table, "land cover", true;
    Scenario = "tScenario", Table, "scenario", true;
    ScenarioProcessed = "tScenarioOut", Table, "scenario processed", false;
    ExclusionTable = "tExclusion", Table, "exclusion", true;
    ExclusionTableProcessed = "tExclusionOut", Table, "exclusion processed", false;
    Capacity = "tCapacity", Table, "capacity", true;
    CapacityProcessed = "tCapacityOut", Table, "capacity processed", false;
    Suitability = "tSuitability", Table, "suitability", true;
    SuitabilityProcessed = "tSuitabilityOut", Table, "suitability processed", false;
    Referral = "tReferral", Table, "referral", false;
    ReferralNearestDistance = "tReferralDist", Table, "referral nearest by dist", false;
    ReferralNearestTime = "tReferralTime", Table, "referral nearest by time", false;
    CoverageResult = "tCapacityStat", Table, "result geographic coverage analysis", false;
    ScalingUpResult = "tCapacityStatNew", Table, "result scaling up analysis", false;
    OptimizationResult = "tOptimization", Table, "optimization", false;
    ZonalCoverage = "tZonalStat", Table, "zonal coverage", false;
    PopulationAdjustmentSummary = "tPopDistribAdjustSummary", Table, "population adjustment summary", false;
    AnalysisParameters = "lAnalysisParameters", Config, "analysis parameters", false;
}

/// Every `x-accessmod-class` list in `schema`, with the JSON pointer of the
/// schema node holding it.
pub fn schema_classes(schema: &serde_json::Value) -> Vec<(String, Vec<String>)> {
    fn walk(value: &serde_json::Value, pointer: String, out: &mut Vec<(String, Vec<String>)>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(list) = map.get(SCHEMA_KEY) {
                    let codes = list
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| c.as_str().unwrap_or_default().to_owned())
                        .collect();
                    out.push((pointer.clone(), codes));
                }
                for (key, child) in map {
                    walk(child, format!("{pointer}/{key}"), out);
                }
            }
            serde_json::Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    walk(child, format!("{pointer}/{i}"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(schema, String::new(), &mut out);
    out
}

/// Dataset references of a job `document`, with the classes their field
/// accepts: every `(text, classes)` where a field of `schema` marked with
/// [`SCHEMA_KEY`] holds a string, or an object whose `path` (directly or
/// in its single variant, e.g. `{"raster": "…"}`) is a string. `$ref`s are
/// followed; of `oneOf` / `anyOf` alternatives, only those whose constant
/// properties match the value are walked (tagged enums). Callers keep the
/// texts that are paths of the job.
pub fn dataset_fields(
    schema: &serde_json::Value,
    document: &serde_json::Value,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    walk_fields(schema, schema, document, 0, &mut out);
    out
}

/// Deepest schema nesting followed (schemas here are shallow; guards
/// against cycles).
const MAX_DEPTH: usize = 64;

fn walk_fields(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
    depth: usize,
    out: &mut Vec<(String, Vec<String>)>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Some(classes) = schema.get(SCHEMA_KEY).and_then(|c| c.as_array())
        && let Some(path) = referenced_path(value)
    {
        let classes = classes
            .iter()
            .filter_map(|c| c.as_str().map(str::to_owned))
            .collect();
        out.push((path, classes));
    }
    if let Some(target) = schema.get("$ref").and_then(|r| r.as_str())
        && let Some(resolved) = target
            .strip_prefix("#/")
            .and_then(|pointer| root.pointer(&format!("/{pointer}")))
    {
        walk_fields(root, resolved, value, depth + 1, out);
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        for alternative in schema
            .get(key)
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            if key == "allOf" || constants_match(root, alternative, value) {
                walk_fields(root, alternative, value, depth + 1, out);
            }
        }
    }
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(|p| p.as_object()),
        value.as_object(),
    ) {
        for (name, property) in properties {
            if let Some(child) = object.get(name) {
                walk_fields(root, property, child, depth + 1, out);
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for child in array {
            walk_fields(root, items, child, depth + 1, out);
        }
    }
}

/// Whether the `const` properties of `schema` (after a `$ref`) hold in
/// `value`: the discriminant of a tagged enum variant.
fn constants_match(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    value: &serde_json::Value,
) -> bool {
    let schema = schema
        .get("$ref")
        .and_then(|r| r.as_str())
        .and_then(|r| r.strip_prefix("#/"))
        .and_then(|p| root.pointer(&format!("/{p}")))
        .unwrap_or(schema);
    let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) else {
        return true;
    };
    properties
        .iter()
        .all(|(name, property)| match property.get("const") {
            Some(constant) => value.get(name) == Some(constant),
            None => true,
        })
}

/// The path a field value names: a string, `{"path": …}`, or the single
/// variant of an externally tagged enum holding one of these.
fn referenced_path(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(map) => match map.get("path") {
            Some(serde_json::Value::String(path)) => Some(path.clone()),
            Some(_) => None,
            None if map.len() == 1 => map.values().next().and_then(referenced_path),
            None => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_through_serde() {
        for &class in DataClass::ALL {
            let json = serde_json::to_value(class).unwrap();
            assert_eq!(json, serde_json::json!(class.code()));
            assert_eq!(DataClass::from_code(class.code()), Some(class));
            assert_eq!(serde_json::from_value::<DataClass>(json).unwrap(), class);
        }
    }

    #[test]
    fn prefixes_match_kinds() {
        for &class in DataClass::ALL {
            let prefix = match class.kind() {
                Kind::Raster => 'r',
                Kind::Vector => 'v',
                Kind::Table => 't',
                Kind::Config => 'l',
            };
            assert!(class.code().starts_with(prefix), "{}", class.code());
        }
    }

    #[test]
    fn dataset_fields_follow_refs_and_enum_variants() {
        let schema = crate::analysis::job_schemas()
            .into_iter()
            .find(|(n, _)| *n == "scaling-up")
            .map(|(_, s)| s)
            .unwrap();
        let job = serde_json::json!({
            "dem": "datasets/1.tif",
            "facilities": {"path": "datasets/3.fgb", "id_field": "id"},
            "exclusions": [{"layer": {"raster": "datasets/9.tif"}, "keep": "outside"}],
            "suitability": [
                {"factor": "priority", "layer": {"raster": "datasets/5.tif"}, "weight": 1, "prefer": "high"},
                {"factor": "distance", "layer": "facilities", "weight": 1, "prefer": "low"}
            ]
        });
        let mut found = dataset_fields(&schema, &job);
        found.sort();
        assert_eq!(
            found,
            vec![
                ("datasets/1.tif".to_owned(), vec!["rDem".to_owned()]),
                (
                    "datasets/3.fgb".to_owned(),
                    vec!["vFacility".to_owned(), "vFacilityNew".to_owned()]
                ),
                ("datasets/5.tif".to_owned(), vec!["rPriority".to_owned()]),
                (
                    "datasets/9.tif".to_owned(),
                    vec!["rExclusion".to_owned(), "vExclusion".to_owned()]
                ),
                // Not a path: callers keep only paths of the job.
                (
                    "facilities".to_owned(),
                    vec![
                        "vRoad".to_owned(),
                        "vBarrier".to_owned(),
                        "vFacility".to_owned()
                    ]
                ),
            ]
        );
    }

    #[test]
    fn schema_classes_finds_nested_lists() {
        let schema = serde_json::json!({
            "properties": {"dem": {"type": "string", "x-accessmod-class": ["rDem"]}},
            "$defs": {"A": {"properties": {"p": {"x-accessmod-class": ["vZone", "vFacility"]}}}}
        });
        let mut found = schema_classes(&schema);
        found.sort();
        assert_eq!(
            found,
            vec![
                (
                    "/$defs/A/properties/p".to_owned(),
                    vec!["vZone".to_owned(), "vFacility".to_owned()]
                ),
                ("/properties/dem".to_owned(), vec!["rDem".to_owned()]),
            ]
        );
    }
}
