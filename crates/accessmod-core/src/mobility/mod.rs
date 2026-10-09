//! Mobility models: speed on a slope for each transport mode.
//!
//! The only model so far is [`MobilityModel::Accessmod5V1`], the AccessMod 5
//! kernel (`docker/alpine_base/modules/accessmod/mobility.h` at
//! `5.9.2-beta.6`, see `docs/reference-accessmod5.md`).

pub mod bicycle;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Transport mode of a land-cover class in a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    /// Tobler's hiking function.
    Walking,
    /// Physical cycling model with a downhill safety envelope (LUT).
    Bicycling,
    /// Constant speed, slope ignored.
    Motorized,
}

/// Named, frozen mobility model. Any change to an equation or constant
/// creates a new variant; existing variants never change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum MobilityModel {
    /// AccessMod 5 kernel as of `5.9.2-beta.6`.
    #[default]
    #[serde(rename = "accessmod5-v1")]
    Accessmod5V1,
}

impl MobilityModel {
    /// Speed in km/h for `mode` at scenario flat speed `flat_speed_kmh` on
    /// `slope` (ratio `Δz / L`, positive uphill in the direction of travel).
    ///
    /// Returns `0.0` (impassable) for a non-positive flat speed and, for
    /// bicycling, for a non-finite slope. Walking and motorized propagate a
    /// NaN slope or speed unchanged; callers treat non-finite costs as
    /// barriers.
    pub fn speed_kmh(self, mode: TransportMode, flat_speed_kmh: f64, slope: f64) -> f64 {
        match self {
            Self::Accessmod5V1 => match mode {
                TransportMode::Walking => walking_speed_kmh(flat_speed_kmh, slope),
                TransportMode::Bicycling => bicycle::speed_kmh(flat_speed_kmh, slope),
                TransportMode::Motorized => flat_speed_kmh,
            },
        }
    }
}

/// Tobler's hiking function scaled so that `flat_speed_kmh` is the speed on
/// flat ground: `v = flat / e^(−0.175) · e^(−3.5 · |slope + 0.05|)`.
///
/// Tobler, W. (1993). Three presentations on geographical analysis and
/// modeling. NCGIA Technical Report 93-1. Mirrors `am_walking_speed`
/// (`mobility.h` l.47); the operation order is kept for bit parity.
pub fn walking_speed_kmh(flat_speed_kmh: f64, slope: f64) -> f64 {
    let top_speed = flat_speed_kmh / (-0.175f64).exp();
    (-3.5 * (slope + 0.05).abs()).exp() * top_speed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_preserves_flat_speed() {
        let v = walking_speed_kmh(5.0, 0.0);
        assert!((v - 5.0).abs() < 1e-12, "{v}");
    }

    #[test]
    fn walking_peaks_at_five_percent_downhill() {
        let top = walking_speed_kmh(5.0, -0.05);
        assert!((top - 5.0 / (-0.175f64).exp()).abs() < 1e-12);
        assert!(top > walking_speed_kmh(5.0, -0.04));
        assert!(top > walking_speed_kmh(5.0, -0.06));
    }

    #[test]
    fn walking_is_symmetric_around_minus_five_percent() {
        let up = walking_speed_kmh(4.0, -0.05 + 0.2);
        let down = walking_speed_kmh(4.0, -0.05 - 0.2);
        assert!((up - down).abs() < 1e-12);
    }

    #[test]
    fn motorized_ignores_slope() {
        let m = MobilityModel::Accessmod5V1;
        assert_eq!(m.speed_kmh(TransportMode::Motorized, 60.0, 0.3), 60.0);
        assert_eq!(m.speed_kmh(TransportMode::Motorized, 60.0, -0.3), 60.0);
    }

    #[test]
    fn public_names_are_stable() {
        let name = |v: serde_json::Value| v.as_str().map(str::to_owned);
        let model = serde_json::to_value(MobilityModel::Accessmod5V1).ok();
        assert_eq!(model.and_then(name).as_deref(), Some("accessmod5-v1"));
        let mode = serde_json::to_value(TransportMode::Bicycling).ok();
        assert_eq!(mode.and_then(name).as_deref(), Some("bicycling"));
    }
}
