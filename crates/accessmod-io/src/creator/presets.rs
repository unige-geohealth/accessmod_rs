//! The presets the Creator applies (`presets/*.json`, contract 6 of
//! `docs/project-creator.md`): scientific assumptions, versioned, recorded
//! in each project's manifest.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use accessmod_core::mobility::TransportMode;

/// A land cover class of the source product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandCoverClass {
    /// Value in the raster.
    pub value: i64,
    /// Label.
    pub label: String,
    /// Default travel mode.
    pub mode: TransportMode,
    /// Default speed, km/h (0: barrier).
    pub speed_kmh: f64,
    /// Open question for reviewers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
}

/// Land cover preset (`worldcover.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandCoverPreset {
    /// Preset name.
    pub preset: String,
    /// Version.
    pub version: u32,
    /// Review status.
    pub status: String,
    /// Product.
    pub product: String,
    /// Source NoData.
    pub nodata: i64,
    /// Classes.
    pub classes: Vec<LandCoverClass>,
}

/// A travel mode and speed on a road class.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Travel {
    /// Travel mode.
    pub mode: TransportMode,
    /// Speed, km/h.
    pub speed_kmh: f64,
}

/// A road class rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadRule {
    /// Class written in `road_class` (merged as class + 1000).
    pub class: i64,
    /// Label.
    pub label: String,
    /// OSM `highway` values.
    #[serde(default)]
    pub highway: Vec<String>,
    /// OSM `route` values (`ferry`): they take precedence over
    /// `highway`, a ferry being a boat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route: Vec<String>,
    /// Travel on it in the walking scenario.
    pub walking: Travel,
    /// Travel on it in the bicycle scenario.
    pub bicycle: Travel,
    /// Travel on it in the motorized scenario.
    pub motorized: Travel,
}

/// Roads preset (`osm_roads.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadPreset {
    /// Preset name.
    pub preset: String,
    /// Version.
    pub version: u32,
    /// Review status.
    pub status: String,
    /// Rules, in priority order.
    pub rules: Vec<RoadRule>,
    /// Field → values that exclude a feature (applied when the field
    /// exists).
    pub excluded: std::collections::BTreeMap<String, Vec<String>>,
}

impl RoadPreset {
    /// The rule of a feature's `route` (first) or `highway` (read from
    /// `fields`), unless a field excludes it.
    pub fn rule(&self, fields: &dyn Fn(&str) -> Option<String>) -> Option<&RoadRule> {
        let excluded = self
            .excluded
            .iter()
            .any(|(field, values)| fields(field).is_some_and(|v| values.contains(&v)));
        if excluded {
            return None;
        }
        let of = |field: &str, values: fn(&RoadRule) -> &Vec<String>| {
            let value = fields(field)?;
            self.rules.iter().find(|r| values(r).contains(&value))
        };
        of("route", |r| &r.route).or_else(|| of("highway", |r| &r.highway))
    }
}

/// A barrier rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarrierRule {
    /// Label.
    pub label: String,
    /// OSM `waterway` values.
    pub waterway: Vec<String>,
}

/// Barriers preset (`osm_barriers.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarrierPreset {
    /// Preset name.
    pub preset: String,
    /// Version.
    pub version: u32,
    /// Review status.
    pub status: String,
    /// Rules.
    pub rules: Vec<BarrierRule>,
}

/// Facilities preset (`osm_facilities.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FacilityPreset {
    /// Preset name.
    pub preset: String,
    /// Version.
    pub version: u32,
    /// Review status.
    pub status: String,
    /// Field → values that keep a feature.
    pub keep: std::collections::BTreeMap<String, Vec<String>>,
}

/// Every preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presets {
    /// Land cover classes and default scenario.
    pub land_cover: LandCoverPreset,
    /// Road classes.
    pub roads: RoadPreset,
    /// Barriers.
    pub barriers: BarrierPreset,
    /// Facilities.
    pub facilities: FacilityPreset,
}

fn parse<T: for<'de> Deserialize<'de>>(name: &str, text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::invalid(format!("preset {name}: {e}")))
}

impl Presets {
    /// The presets shipped with this engine.
    pub fn shipped() -> Result<Self> {
        Ok(Self {
            land_cover: parse("worldcover", include_str!("presets/worldcover.json"))?,
            roads: parse("osm_roads", include_str!("presets/osm_roads.json"))?,
            barriers: parse("osm_barriers", include_str!("presets/osm_barriers.json"))?,
            facilities: parse(
                "osm_facilities",
                include_str!("presets/osm_facilities.json"),
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_presets_parse_and_hold() {
        let p = Presets::shipped().expect("presets");
        assert_eq!(p.land_cover.classes.len(), 11);
        assert!(
            p.land_cover
                .classes
                .iter()
                .any(|c| c.value == 80 && c.speed_kmh == 0.0)
        );
        let class = |tags: &[(&str, &str)]| {
            let fields = |f: &str| {
                tags.iter()
                    .find(|(k, _)| *k == f)
                    .map(|(_, v)| (*v).to_owned())
            };
            p.roads.rule(&fields).map(|r| r.class)
        };
        assert_eq!(class(&[("highway", "primary")]), Some(2));
        assert_eq!(class(&[("highway", "construction")]), None);
        assert_eq!(class(&[("highway", "unheard_of")]), None);
        assert_eq!(class(&[]), None);
        assert_eq!(
            class(&[("highway", "primary"), ("access", "private")]),
            None
        );
        // Ferries: `route=ferry`, before any `highway` (a ferry is a boat).
        assert_eq!(class(&[("route", "ferry")]), Some(8));
        assert_eq!(
            class(&[("route", "ferry"), ("highway", "service")]),
            Some(8)
        );
        assert_eq!(class(&[("route", "bus"), ("highway", "primary")]), Some(2));
        let ferry = p.roads.rules.iter().find(|r| r.class == 8).expect("ferry");
        for t in [ferry.walking, ferry.bicycle, ferry.motorized] {
            assert_eq!((t.mode, t.speed_kmh), (TransportMode::Motorized, 20.0));
        }
        // Road classes stay below 1000 (merged as class + 1000) and unique.
        let mut classes: Vec<i64> = p.roads.rules.iter().map(|r| r.class).collect();
        classes.dedup();
        assert_eq!(classes.len(), p.roads.rules.len());
        assert!(classes.iter().all(|c| (1..1000).contains(c)));
        // Motorways and trunk roads are crossed on foot, not followed.
        let motorway = p
            .roads
            .rules
            .iter()
            .find(|r| r.highway.iter().any(|h| h == "trunk"))
            .expect("trunk");
        for t in [motorway.walking, motorway.bicycle] {
            assert_eq!((t.mode, t.speed_kmh), (TransportMode::Walking, 0.1));
        }
        // No road is a barrier in any scenario: a road at 0 km/h would be a
        // wall in the merged land cover.
        assert!(p.roads.rules.iter().all(|r| {
            [r.walking, r.bicycle, r.motorized]
                .iter()
                .all(|t| t.speed_kmh > 0.0)
        }));
    }
}
