//! Zonal statistics: population covered within travel-time limits, per
//! zone (AccessMod 5: `tools/R/amAnalysisZonal.R`).
//!
//! A cell is covered within limit `L` when its travel time `t` satisfies
//! `0 <= t <= L` (seconds, inclusive like the travel-time limit). AccessMod 5
//! compares its stored travel time, rounded up to whole minutes, with
//! `t < L`.

/// Whether a cell reached in `time_s` is covered within `limit_s`.
pub fn covered(time_s: f64, limit_s: f64) -> bool {
    time_s >= 0.0 && time_s <= limit_s
}

/// Population of one zone, and the part covered within each limit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZoneCoverage {
    /// Population of the zone.
    pub population: f64,
    /// Population covered within each limit, in the order of the limits.
    pub covered: Vec<f64>,
}

impl ZoneCoverage {
    /// Empty coverage for `limits` limits.
    pub fn new(limits: usize) -> Self {
        Self {
            population: 0.0,
            covered: vec![0.0; limits],
        }
    }

    /// Count a cell holding `population`, reached in `time_s` (`None`: not
    /// reached).
    pub fn add(&mut self, population: f64, time_s: Option<f64>, limits_s: &[f64]) {
        self.population += population;
        if let Some(time_s) = time_s {
            for (covered, &limit) in self.covered.iter_mut().zip(limits_s) {
                if self::covered(time_s, limit) {
                    *covered += population;
                }
            }
        }
    }

    /// Percentage of the population covered within limit `k`; 0 for a zone
    /// without population (AccessMod 5 replaces NA by 0).
    pub fn percent(&self, k: usize) -> f64 {
        if self.population > 0.0 {
            100.0 * self.covered[k] / self.population
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_inclusive_and_negative_times_are_not_covered() {
        assert!(covered(0.0, 60.0));
        assert!(covered(60.0, 60.0));
        assert!(!covered(60.5, 60.0));
        assert!(!covered(-1.0, 60.0));
    }

    #[test]
    fn coverage_per_limit() {
        let limits = [600.0, 1800.0];
        let mut zone = ZoneCoverage::new(2);
        zone.add(10.0, Some(300.0), &limits);
        zone.add(30.0, Some(1200.0), &limits);
        zone.add(60.0, None, &limits);
        assert_eq!(zone.population, 100.0);
        assert_eq!(zone.covered, vec![10.0, 40.0]);
        assert_eq!(zone.percent(1), 40.0);
        assert_eq!(ZoneCoverage::new(1).percent(0), 0.0);
    }
}
