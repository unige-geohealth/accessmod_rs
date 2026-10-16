//! Adjust population distribution (AccessMod 5:
//! `modules/amPopulationBarrierCorrection/helper.R`).
//!
//! Population on barriers cannot be reached. Within each zone, the
//! population off barriers is scaled so that the zone keeps its total, or
//! reaches a known total:
//!
//! - `ratio = target / population off barriers` of the zone, where `target`
//!   is the zone's whole population or its known population;
//! - adjusted cell = population × ratio, for cells off barriers inside a
//!   zone; other cells have no population.

/// Population of one zone, accumulated cell by cell.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ZoneTotals {
    /// Cells of the zone holding population.
    pub cells: u64,
    /// Population of the zone.
    pub population: f64,
    /// Population of the zone off barriers.
    pub off_barrier: f64,
}

impl ZoneTotals {
    /// Count a cell holding `population`, on a barrier or not.
    pub fn add(&mut self, population: f64, on_barrier: bool) {
        self.cells += 1;
        self.population += population;
        if !on_barrier {
            self.off_barrier += population;
        }
    }

    /// Population on barriers.
    pub fn on_barrier(&self) -> f64 {
        self.population - self.off_barrier
    }

    /// Scale factor reaching `target` (the zone's population when `None`),
    /// or `None` when no population lies off barriers to carry it.
    pub fn ratio(&self, target: Option<f64>) -> Option<f64> {
        let target = target.unwrap_or(self.population);
        (self.off_barrier > 0.0 && target.is_finite()).then(|| target / self.off_barrier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_moves_barrier_population_to_the_rest_of_the_zone() {
        let mut zone = ZoneTotals::default();
        zone.add(30.0, false);
        zone.add(10.0, true);
        zone.add(10.0, false);
        assert_eq!(zone.cells, 3);
        assert_eq!(zone.on_barrier(), 10.0);
        // 50 people, 40 off barriers: each off-barrier cell is scaled by 1.25.
        assert_eq!(zone.ratio(None), Some(1.25));
        assert_eq!(zone.ratio(Some(80.0)), Some(2.0));
    }

    #[test]
    fn zones_without_population_off_barriers_have_no_ratio() {
        let mut zone = ZoneTotals::default();
        assert_eq!(zone.ratio(None), None);
        zone.add(5.0, true);
        assert_eq!(zone.ratio(Some(10.0)), None);
    }
}
