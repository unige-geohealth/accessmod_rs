//! Data classes: what a dataset holds, with the identifiers of AccessMod 5
//! (`www/dictionary/classes.json`), so catalogues, job fields and `.am5p`
//! imports share one vocabulary.
//!
//! Job fields that designate a dataset carry the classes they accept in
//! their JSON Schema, as `x-accessmod-class` (see [`SCHEMA_KEY`]): on a path
//! string, or on a layer object whose `path` is the dataset. Internal
//! AccessMod 5 classes (stacks, speed, friction, bridges) are left out.

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
    Capacity = "tCapacity", Table, "capacity", true;
    Referral = "tReferral", Table, "referral", false;
    ReferralNearestDistance = "tReferralDist", Table, "referral nearest by dist", false;
    ReferralNearestTime = "tReferralTime", Table, "referral nearest by time", false;
    CoverageResult = "tCapacityStat", Table, "result geographic coverage analysis", false;
    ScalingUpResult = "tCapacityStatNew", Table, "result scaling up analysis", false;
    OptimizationResult = "tOptimization", Table, "optimization", false;
    ZonalCoverage = "tZonalStat", Table, "zonal coverage", false;
    PopulationAdjustmentSummary = "tPopDistribAdjustSummary", Table, "population adjustment summary", false;
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
            };
            assert!(class.code().starts_with(prefix), "{}", class.code());
        }
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
