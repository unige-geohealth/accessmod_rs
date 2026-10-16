//! Geographic coverage: the catchment of one facility from the population
//! it can reach, per travel-time band (AccessMod 5:
//! `tools/R/amAnalysisCatchment.R`, `amCatchmentAnalyst`).
//!
//! Travel times are grouped in bands (`band = ceil(t / band_s)`, AccessMod 5
//! uses whole minutes). With the residual population of each band and its
//! running total, a facility of capacity `C` covers every band whose running
//! total stays within `C` (inner ring) and, when the next band (outer ring)
//! exceeds it, the fraction of that band's population that fills `C`.
//!
//! The five cases of AccessMod 5 (capacity 10, bands 0..4):
//!
//! | case | when | catchment ends | removed |
//! | --- | --- | --- | --- |
//! | E | capacity ignored | the limit | every band |
//! | D | a running total equals `C` | that band | inner ring |
//! | C | `C` exceeds the population within the limit | last band | inner ring |
//! | B | the first populated band already exceeds `C` | that band | part of it |
//! | A | otherwise | the outer band | inner ring and part of the outer band |

/// Residual population of one travel-time band (bands without any
/// population cell are left out, as AccessMod 5 does).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// Band number: `ceil(t / band_s)`.
    pub band: u32,
    /// Residual population of the band.
    pub population: f64,
}

/// Case of a catchment (see the module documentation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CatchmentCase {
    /// Partial outer band.
    A,
    /// Partial first band.
    B,
    /// Capacity larger than the population within the limit.
    C,
    /// Capacity met exactly by whole bands.
    D,
    /// Capacity ignored.
    E,
}

/// Catchment of one facility.
#[derive(Debug, Clone, PartialEq)]
pub struct Catchment {
    /// Case; `None` when no band holds population or no case applies.
    pub case: Option<CatchmentCase>,
    /// Capacity used (the population within the limit when ignored).
    pub capacity: f64,
    /// Capacity used by the population covered.
    pub capacity_realised: f64,
    /// Capacity left.
    pub capacity_residual: f64,
    /// Residual population within the limit.
    pub population_within_limit: Option<f64>,
    /// Population of the bands of the catchment.
    pub population_catchment: Option<f64>,
    /// Last band of the catchment.
    pub band_limit: Option<u32>,
    /// Bands up to this one lose all their population.
    pub inner: Option<u32>,
    /// This band loses this fraction of its population.
    pub outer: Option<(u32, f64)>,
    /// Pearson correlation between band and band population.
    pub correlation: Option<f64>,
}

impl Catchment {
    /// Population of the catchment bands not covered (the part of the outer
    /// band left). Never negative: the capacity realised is a sum of the
    /// same bands in another order, so rounding can put it a few ulps above
    /// (AccessMod 5 rounds to 6 decimals instead).
    pub fn population_not_covered(&self) -> Option<f64> {
        self.population_catchment
            .map(|population| (population - self.capacity_realised).max(0.0))
    }
}

/// Pearson correlation of `(x, y)` pairs; `None` with fewer than two pairs
/// or a constant series (R `cor` gives NA).
fn correlation(pairs: impl Iterator<Item = (f64, f64)> + Clone) -> Option<f64> {
    let n = pairs.clone().count() as f64;
    if n < 2.0 {
        return None;
    }
    let (mx, my) = pairs
        .clone()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (mx, my) = (mx / n, my / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
        syy += (y - my) * (y - my);
    }
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

/// Catchment of a facility of `capacity` (`None`: capacity ignored) over
/// `bands` (ascending), reached within `limit_band`.
pub fn catchment(bands: &[Band], capacity: Option<f64>, limit_band: u32) -> Catchment {
    let mut running = Vec::with_capacity(bands.len());
    let mut total = 0.0;
    for band in bands {
        total += band.population;
        running.push((band.band, band.population, total));
    }
    let mut result = Catchment {
        case: None,
        capacity: capacity.unwrap_or(0.0),
        capacity_realised: 0.0,
        capacity_residual: capacity.unwrap_or(0.0),
        population_within_limit: None,
        population_catchment: None,
        band_limit: None,
        inner: None,
        outer: None,
        correlation: None,
    };
    let Some(&(first_band, _, _)) = running.first() else {
        return result;
    };
    let within = total;
    result.population_within_limit = Some(within);
    result.correlation = correlation(bands.iter().map(|b| (f64::from(b.band), b.population)));
    let capacity_value = capacity.unwrap_or(within);
    result.capacity = capacity_value;
    // Smallest running total above zero.
    let first_populated = running.iter().find(|r| r.2 > 0.0).map(|r| r.2);
    // Inner ring: last band whose running total stays within the capacity.
    let (inner_population, inner_band) = running
        .iter()
        .rev()
        .find(|r| r.2 <= capacity_value)
        .map_or((0.0, first_band), |r| (r.2, r.0));
    // Outer ring: first band whose running total exceeds the capacity.
    let outer = running.iter().find(|r| r.2 > capacity_value).copied();
    let partial = |(band, population, _): (u32, f64, f64)| {
        (band, (capacity_value - inner_population) / population)
    };

    let (case, residual, limit, inner, outer) = if capacity.is_none() {
        (CatchmentCase::E, 0.0, limit_band, Some(inner_band), None)
    } else if capacity_value == inner_population {
        (CatchmentCase::D, 0.0, inner_band, Some(inner_band), None)
    } else if capacity_value > within {
        (
            CatchmentCase::C,
            capacity_value - within,
            inner_band,
            Some(inner_band),
            None,
        )
    } else if first_populated.is_some_and(|p| capacity_value < p) {
        let outer = outer.expect("a band exceeds a capacity below the first total");
        (CatchmentCase::B, 0.0, outer.0, None, Some(partial(outer)))
    } else if let Some(outer) = outer.filter(|_| inner_population > 0.0) {
        (
            CatchmentCase::A,
            0.0,
            outer.0,
            Some(inner_band),
            Some(partial(outer)),
        )
    } else {
        return result;
    };
    result.case = Some(case);
    result.capacity_residual = residual;
    result.capacity_realised = capacity_value - residual;
    result.band_limit = Some(limit);
    result.inner = inner;
    result.outer = outer;
    result.population_catchment = running
        .iter()
        .filter(|r| r.0 <= limit)
        .map(|r| r.2)
        .fold(None, |max: Option<f64>, v| {
            Some(max.map_or(v, |m| m.max(v)))
        });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bands 0..4 with the given populations.
    fn bands(populations: &[f64]) -> Vec<Band> {
        populations
            .iter()
            .enumerate()
            .map(|(band, &population)| Band {
                band: band as u32,
                population,
            })
            .collect()
    }

    // The examples of amCatchmentAnalyst (capacity 10, limit 4), written
    // there as running totals.
    #[test]
    fn case_a_partial_outer_band() {
        let c = catchment(&bands(&[1.0, 2.0, 2.0, 4.0, 6.0]), Some(10.0), 4);
        assert_eq!(c.case, Some(CatchmentCase::A));
        assert_eq!((c.inner, c.band_limit), (Some(3), Some(4)));
        assert_eq!(c.outer, Some((4, 1.0 / 6.0)));
        assert_eq!((c.capacity_realised, c.capacity_residual), (10.0, 0.0));
        assert_eq!(c.population_catchment, Some(15.0));
    }

    #[test]
    fn case_b_partial_first_band() {
        let c = catchment(&bands(&[11.0, 2.0, 7.0, 5.0, 25.0]), Some(10.0), 4);
        assert_eq!(c.case, Some(CatchmentCase::B));
        assert_eq!((c.inner, c.band_limit), (None, Some(0)));
        assert_eq!(c.outer, Some((0, 10.0 / 11.0)));
        assert_eq!(c.population_catchment, Some(11.0));
    }

    #[test]
    fn case_c_capacity_left() {
        let c = catchment(&bands(&[1.0, 2.0, 1.0, 1.0, 1.0]), Some(10.0), 4);
        assert_eq!(c.case, Some(CatchmentCase::C));
        assert_eq!((c.inner, c.band_limit), (Some(4), Some(4)));
        assert_eq!((c.capacity_realised, c.capacity_residual), (6.0, 4.0));
    }

    #[test]
    fn case_d_whole_bands() {
        let c = catchment(&bands(&[2.0, 2.0, 5.0, 1.0, 1.0]), Some(10.0), 4);
        assert_eq!(c.case, Some(CatchmentCase::D));
        assert_eq!((c.inner, c.band_limit, c.outer), (Some(3), Some(3), None));
        assert_eq!(c.population_not_covered(), Some(0.0));
    }

    #[test]
    fn case_e_capacity_ignored() {
        let c = catchment(&bands(&[2.0, 2.0, 5.0]), None, 4);
        assert_eq!(c.case, Some(CatchmentCase::E));
        assert_eq!((c.capacity, c.capacity_realised), (9.0, 9.0));
        assert_eq!((c.inner, c.band_limit), (Some(2), Some(4)));
    }

    #[test]
    fn no_population_reached() {
        let c = catchment(&[], Some(10.0), 4);
        assert_eq!(c.case, None);
        assert_eq!(c.capacity_residual, 10.0);
        // Reached cells holding no one: case C, everything left.
        let c = catchment(&bands(&[0.0, 0.0]), Some(10.0), 4);
        assert_eq!(c.case, Some(CatchmentCase::C));
        assert_eq!((c.capacity_residual, c.correlation), (10.0, None));
    }
}
