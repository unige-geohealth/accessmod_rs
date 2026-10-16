//! Scaling up: rules that pick where to add a facility and how large it is
//! (AccessMod 5 `tools/R/amAnalysisScalingUp.R`).
//!
//! Each suitability factor is rescaled over the candidate cells to
//! `0 ..= 10 000` (`amRasterRescale`), weighted, and the suitability of a
//! cell is the truncated weighted mean. The new facility gets the capacity
//! of the class whose population range holds the population it reaches.
//!
//! AccessMod 5 evaluates these rules with `r.mapcalc` expressions in which R
//! prints the minimum, maximum and weights with 15 significant digits, and
//! integer rasters (travel times in minutes) use integer arithmetic; both
//! are reproduced here, so that ties between candidate cells fall the same
//! way.

/// Upper bound of the rescaled values (`config$scalingUpRescaleRange`).
pub const RESCALE_MAX: f64 = 10_000.0;

/// A number as R prints it into an expression (`sprintf("%s", x)`): 15
/// significant digits.
pub fn r_number(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// R `round` (half to even).
pub fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// Rescaling of one factor over the candidate cells (`amRasterRescale`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rescale {
    min: f64,
    max: f64,
    /// Values and bounds are integers: integer arithmetic.
    integer: bool,
    /// High values are least suitable.
    reverse: bool,
    weight: f64,
}

impl Rescale {
    /// Rescaling for values in `[min, max]` (over the candidate cells, no
    /// value missing), weighted by `weight`. `integer` when the factor is an
    /// integer raster (AccessMod 5 travel times in minutes, integer
    /// priority rasters).
    pub fn new(min: f64, max: f64, integer: bool, reverse: bool, weight: f64) -> Self {
        let (min, max) = if integer {
            (min, max)
        } else {
            (r_number(min), r_number(max))
        };
        Self {
            min,
            max,
            integer,
            reverse,
            weight: r_number(weight),
        }
    }

    /// Largest value (as printed by R for a floating-point factor): the value
    /// missing cells take when missing values count as the maximum.
    pub fn max_value(&self) -> f64 {
        self.max
    }

    /// Rescaled and weighted value of `v`.
    pub fn apply(&self, v: f64) -> f64 {
        if self.min == self.max {
            return 5_000.0 * self.weight;
        }
        let scaled = if self.integer {
            let (v, min, max) = (v as i64, self.min as i64, self.max as i64);
            let part = (v - min) * RESCALE_MAX as i64 / (max - min);
            (if self.reverse {
                RESCALE_MAX as i64 - part
            } else {
                part
            }) as f64
        } else {
            let part = (v - self.min) * RESCALE_MAX / (self.max - self.min);
            if self.reverse {
                RESCALE_MAX - part + 0.0
            } else {
                part + 0.0
            }
        };
        scaled * self.weight
    }
}

/// Suitability of a cell from its weighted rescaled factors (in table
/// order) and the sum of the weights of the factors used.
pub fn suitability(weighted: impl IntoIterator<Item = f64>, weight_sum: f64) -> f64 {
    let sum = weighted.into_iter().fold(0.0, |acc, v| acc + v);
    (sum / r_number(weight_sum)).trunc()
}

/// A class of facility: the population range it serves and its capacity.
#[derive(Debug, Clone, PartialEq)]
pub struct CapacityClass {
    /// Smallest population of the range.
    pub min: f64,
    /// Largest population of the range.
    pub max: f64,
    /// Capacity of a facility of this class.
    pub capacity: f64,
    /// Name of the class.
    pub label: String,
}

/// Index of the class for a facility reaching `population`: the one whose
/// rounded range holds it; when none or several do, the class of the
/// smallest or of the largest capacity, whichever capacity is nearer to
/// `population` (the smallest on a tie), first in table order.
pub fn capacity_class(population: f64, classes: &[CapacityClass]) -> Option<usize> {
    let matching: Vec<usize> = classes
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            population >= round_half_even(c.min) && population <= round_half_even(c.max)
        })
        .map(|(i, _)| i)
        .collect();
    if let [single] = matching.as_slice() {
        return Some(*single);
    }
    let lowest = classes.iter().map(|c| c.capacity).reduce(f64::min)?;
    let highest = classes.iter().map(|c| c.capacity).reduce(f64::max)?;
    let target = if (population - highest).abs() >= (population - lowest).abs() {
        lowest
    } else {
        highest
    };
    classes.iter().position(|c| c.capacity == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r_prints_fifteen_digits() {
        assert_eq!(r_number(1.0 / 3.0), 0.333333333333333);
        assert_eq!(r_number(78313.4765625), 78313.4765625);
        assert_eq!(r_number(0.1 + 0.2), 0.3);
    }

    #[test]
    fn half_to_even() {
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(-0.5), 0.0);
        assert_eq!(round_half_even(2.4), 2.0);
    }

    #[test]
    fn rescale_integer_and_float() {
        // Travel time in minutes from 0 to 3, low values most suitable.
        let tt = Rescale::new(0.0, 3.0, true, true, 1.0);
        assert_eq!(
            [0.0, 1.0, 2.0, 3.0].map(|v| tt.apply(v)),
            [10_000.0, 6_667.0, 3_334.0, 0.0]
        );
        let d = Rescale::new(0.0, 3.0, false, false, 1.0);
        assert_eq!(d.apply(1.0), 10_000.0 / 3.0);
        assert_eq!(
            Rescale::new(4.0, 4.0, false, false, 2.0).apply(4.0),
            10_000.0
        );
        assert_eq!(suitability([10_000.0, 3_333.4, 0.5], 3.0), 4_444.0);
    }

    fn class(min: f64, max: f64, capacity: f64) -> CapacityClass {
        CapacityClass {
            min,
            max,
            capacity,
            label: format!("{capacity}"),
        }
    }

    #[test]
    fn capacity_classes() {
        let classes = [
            class(0.0, 1000.0, 1000.0),
            class(1001.0, 2000.0, 1428.0),
            class(35701.0, 9_999_999.0, 51_000.0),
        ];
        assert_eq!(capacity_class(1500.0, &classes), Some(1));
        assert_eq!(capacity_class(1_777_205.5, &classes), Some(2));
        // Between ranges: the nearest of the smallest and largest capacity.
        assert_eq!(capacity_class(1000.5, &classes), Some(0));
        assert_eq!(capacity_class(30_000.0, &classes), Some(2));
        assert_eq!(capacity_class(1.0, &[]), None);
    }
}
