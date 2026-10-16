//! Optimization: rules that pick, among existing catchments, the next one
//! covering the most population not covered yet (AccessMod 5
//! `tools/R/amAnalysisOptimization.R`).
//!
//! Selection is greedy: the candidate covering the most remaining people
//! wins; ties go to the largest catchment population, then to the first
//! candidate. With a zone quota, candidates are taken from the zones still
//! below their minimum number of facilities until every zone reaches it.

/// A catchment still in the selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// Population covered by the part of the catchment not covered yet.
    pub covered: f64,
    /// Population of the whole catchment.
    pub population: f64,
    /// Zone of its facility, if any.
    pub zone: Option<usize>,
}

/// Minimum number of facilities per zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneQuota {
    minimum: usize,
    /// Facilities selected per zone; `None` for zones without a quota (no
    /// catchment of theirs is a candidate).
    counts: Vec<Option<usize>>,
}

impl ZoneQuota {
    /// A quota of `minimum` facilities in each zone flagged in `required`.
    pub fn new(required: &[bool], minimum: usize) -> Self {
        Self {
            minimum,
            counts: required.iter().map(|&r| r.then_some(0)).collect(),
        }
    }

    /// Zones under a quota.
    pub fn zones(&self) -> usize {
        self.counts.iter().flatten().count()
    }

    /// Whether `zone` is still below its minimum.
    pub fn lacking(&self, zone: Option<usize>) -> bool {
        zone.and_then(|z| self.counts.get(z).copied().flatten())
            .is_some_and(|count| count < self.minimum)
    }

    /// Whether some zone is still below its minimum.
    pub fn open(&self) -> bool {
        self.counts.iter().flatten().any(|&c| c < self.minimum)
    }

    /// Count a facility selected in `zone`.
    pub fn record(&mut self, zone: Option<usize>) {
        if let Some(Some(count)) = zone.and_then(|z| self.counts.get_mut(z)) {
            *count += 1;
        }
    }
}

/// Index of the candidate to select next, or `None` when there is none.
///
/// While `quota` is open, only candidates of zones below their minimum
/// compete; when none is left, every candidate does.
pub fn select(candidates: &[Candidate], quota: Option<&ZoneQuota>) -> Option<usize> {
    let restricted = quota
        .filter(|q| q.open())
        .filter(|q| candidates.iter().any(|c| q.lacking(c.zone)));
    let eligible = |c: &Candidate| restricted.is_none_or(|q| q.lacking(c.zone));
    let mut best: Option<usize> = None;
    for (i, c) in candidates.iter().enumerate() {
        if !eligible(c) {
            continue;
        }
        best = match best {
            Some(b) => {
                let current = &candidates[b];
                let better = c.covered > current.covered
                    || (c.covered == current.covered && c.population > current.population);
                Some(if better { i } else { b })
            }
            None => Some(i),
        };
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(covered: f64, population: f64, zone: Option<usize>) -> Candidate {
        Candidate {
            covered,
            population,
            zone,
        }
    }

    #[test]
    fn most_covered_then_largest_then_first() {
        let c = [
            candidate(5.0, 9.0, None),
            candidate(7.0, 8.0, None),
            candidate(7.0, 10.0, None),
            candidate(7.0, 10.0, None),
        ];
        assert_eq!(select(&c, None), Some(2));
        assert_eq!(select(&[], None), None);
    }

    #[test]
    fn quota_restricts_to_lacking_zones() {
        let c = [
            candidate(9.0, 9.0, Some(0)),
            candidate(3.0, 3.0, Some(1)),
            candidate(8.0, 8.0, None),
        ];
        let mut quota = ZoneQuota::new(&[true, true, false], 1);
        assert_eq!(quota.zones(), 2);
        assert_eq!(select(&c, Some(&quota)), Some(0));
        quota.record(Some(0));
        // Zone 1 still lacks one: its candidate wins over larger ones.
        assert_eq!(select(&c, Some(&quota)), Some(1));
        quota.record(Some(1));
        assert!(!quota.open());
        assert_eq!(select(&c, Some(&quota)), Some(0));
        // A lacking zone without candidates leaves every candidate eligible.
        let quota = ZoneQuota::new(&[false, false, true], 1);
        assert_eq!(select(&c, Some(&quota)), Some(0));
    }
}
