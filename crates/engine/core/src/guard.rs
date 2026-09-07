//! Source-neutral fall-protection evidence.
//!
//! ADR 0004: a service returns what was *measured*; a capability decides what
//! it means. Here the measurement is the set of exposed horizontal edges in a
//! model, together with the candidate barriers, landings and climbable objects
//! near each one -- and how near.
//!
//! The search radii travel with the request. Broad-phase distances and
//! sampling density decide *which* candidates are worth measuring, so they are
//! measurement inputs; the heights, gaps and widths that decide whether an
//! edge is adequately guarded stay with the capability. Without this
//! separation an adapter has to read the rule declaration to size its search,
//! which is how policy leaked behind the seam in the first place.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::services::reviewable_exact_evidence;

/// Why fall-protection geometry could not be measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    /// A reported quantity is non-finite, or an interval is malformed.
    #[error("guard quantities must be finite and intervals ordered within [0, 1]")]
    InvalidQuantity,
    /// The evidence backing the measurement was not exact and reviewable.
    #[error("guard evidence must be exact and reviewable")]
    InexactEvidence,
    /// The adapter cannot measure fall protection for this model.
    #[error("guard measurement is unavailable")]
    Unavailable,
    /// The requested search radii are not usable.
    #[error("guard search radii must be finite and positive")]
    InvalidSearch,
}

/// How far to look for candidates, and how finely to sample an edge.
///
/// Measurement inputs, not thresholds: these bound the search, they do not
/// judge what is found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuardSearch {
    candidate_radius_metres: f64,
    sample_spacing_metres: f64,
}

impl GuardSearch {
    pub fn try_new(
        candidate_radius_metres: f64,
        sample_spacing_metres: f64,
    ) -> Result<Self, GuardError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !positive(candidate_radius_metres) || !positive(sample_spacing_metres) {
            return Err(GuardError::InvalidSearch);
        }
        Ok(Self {
            candidate_radius_metres,
            sample_spacing_metres,
        })
    }
    pub fn candidate_radius_metres(&self) -> f64 {
        self.candidate_radius_metres
    }
    pub fn sample_spacing_metres(&self) -> f64 {
        self.sample_spacing_metres
    }
}

/// An element near an exposed edge, with the geometry a policy needs.
#[derive(Clone, Debug, PartialEq)]
pub struct GuardCandidate {
    element: ObjectId,
    horizontal_gap_metres: f64,
    /// Height of the candidate's top above the walking surface. Negative when
    /// the candidate lies below it, which is how a landing is distinguished
    /// from a barrier.
    top_offset_metres: f64,
    /// Portion of the edge this candidate covers, normalised to `[0, 1]`.
    edge_interval: [f64; 2],
    landing_width_metres: f64,
    /// Top of a curb beneath the candidate, when one exists.
    curb_top_offset_metres: Option<f64>,
}

impl GuardCandidate {
    pub fn try_new(
        element: ObjectId,
        horizontal_gap_metres: f64,
        top_offset_metres: f64,
        edge_interval: [f64; 2],
        landing_width_metres: f64,
        curb_top_offset_metres: Option<f64>,
    ) -> Result<Self, GuardError> {
        let finite = |v: f64| v.is_finite();
        if !finite(horizontal_gap_metres)
            || !finite(top_offset_metres)
            || !finite(landing_width_metres)
            || horizontal_gap_metres < 0.0
            || landing_width_metres < 0.0
            || curb_top_offset_metres.is_some_and(|v| !v.is_finite())
        {
            return Err(GuardError::InvalidQuantity);
        }
        // A coverage interval outside [0, 1] or running backwards cannot be
        // unioned with its neighbours, and would silently distort coverage.
        let [start, end] = edge_interval;
        if !finite(start) || !finite(end) || start < 0.0 || end > 1.0 || start > end {
            return Err(GuardError::InvalidQuantity);
        }
        Ok(Self {
            element,
            horizontal_gap_metres,
            top_offset_metres,
            edge_interval,
            landing_width_metres,
            curb_top_offset_metres,
        })
    }

    pub fn element(&self) -> &ObjectId {
        &self.element
    }
    pub fn horizontal_gap_metres(&self) -> f64 {
        self.horizontal_gap_metres
    }
    pub fn top_offset_metres(&self) -> f64 {
        self.top_offset_metres
    }
    pub fn edge_interval(&self) -> [f64; 2] {
        self.edge_interval
    }
    pub fn landing_width_metres(&self) -> f64 {
        self.landing_width_metres
    }
    pub fn curb_top_offset_metres(&self) -> Option<f64> {
        self.curb_top_offset_metres
    }
}

/// An object next to a barrier that could be climbed to defeat it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClimbableCandidate {
    element: ObjectId,
    barrier: ObjectId,
    distance_to_barrier_metres: f64,
    top_offset_metres: f64,
    minimum_side_length_metres: f64,
}

impl ClimbableCandidate {
    pub fn try_new(
        element: ObjectId,
        barrier: ObjectId,
        distance_to_barrier_metres: f64,
        top_offset_metres: f64,
        minimum_side_length_metres: f64,
    ) -> Result<Self, GuardError> {
        if !distance_to_barrier_metres.is_finite()
            || !top_offset_metres.is_finite()
            || !minimum_side_length_metres.is_finite()
            || distance_to_barrier_metres < 0.0
            || minimum_side_length_metres < 0.0
        {
            return Err(GuardError::InvalidQuantity);
        }
        Ok(Self {
            element,
            barrier,
            distance_to_barrier_metres,
            top_offset_metres,
            minimum_side_length_metres,
        })
    }
    pub fn element(&self) -> &ObjectId {
        &self.element
    }
    pub fn barrier(&self) -> &ObjectId {
        &self.barrier
    }
    pub fn distance_to_barrier_metres(&self) -> f64 {
        self.distance_to_barrier_metres
    }
    pub fn top_offset_metres(&self) -> f64 {
        self.top_offset_metres
    }
    pub fn minimum_side_length_metres(&self) -> f64 {
        self.minimum_side_length_metres
    }
}

/// One exposed edge of a walking surface, and what sits near it.
#[derive(Clone, Debug, PartialEq)]
pub struct GuardEdge {
    surface: ObjectId,
    barriers: Vec<GuardCandidate>,
    landings: Vec<GuardCandidate>,
    climbables: Vec<ClimbableCandidate>,
}

impl GuardEdge {
    pub fn new(
        surface: ObjectId,
        barriers: Vec<GuardCandidate>,
        landings: Vec<GuardCandidate>,
        climbables: Vec<ClimbableCandidate>,
    ) -> Self {
        Self {
            surface,
            barriers,
            landings,
            climbables,
        }
    }
    pub fn surface(&self) -> &ObjectId {
        &self.surface
    }
    pub fn barriers(&self) -> &[GuardCandidate] {
        &self.barriers
    }
    pub fn landings(&self) -> &[GuardCandidate] {
        &self.landings
    }
    pub fn climbables(&self) -> &[ClimbableCandidate] {
        &self.climbables
    }

    /// Fraction of this edge covered by candidates whose gap is within
    /// `maximum_gap_metres`, unioning overlapping intervals.
    ///
    /// Union rather than sum: two barriers covering the same half of an edge
    /// guard half of it, not all of it. Summing would let overlapping rails
    /// hide an unguarded run.
    pub fn covered_fraction(candidates: &[GuardCandidate], maximum_gap_metres: f64) -> f64 {
        let mut intervals: Vec<[f64; 2]> = candidates
            .iter()
            .filter(|candidate| candidate.horizontal_gap_metres() <= maximum_gap_metres)
            .map(GuardCandidate::edge_interval)
            .collect();
        intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut covered = 0.0;
        let mut cursor = f64::NEG_INFINITY;
        for [start, end] in intervals {
            let from = start.max(cursor);
            if end > from {
                covered += end - from;
                cursor = end;
            }
        }
        covered
    }
}

/// Measured fall-protection geometry for a model.
#[derive(Clone, Debug, PartialEq)]
pub struct GuardEvidence {
    edges: Vec<GuardEdge>,
    evaluated_surfaces: usize,
    evidence: Evidence,
}

impl GuardEvidence {
    pub fn try_new(
        edges: Vec<GuardEdge>,
        evaluated_surfaces: usize,
        evidence: Evidence,
    ) -> Result<Self, GuardError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(GuardError::InexactEvidence);
        }
        Ok(Self {
            edges,
            evaluated_surfaces,
            evidence,
        })
    }
    pub fn edges(&self) -> &[GuardEdge] {
        &self.edges
    }
    /// How many walking surfaces the measurement considered.
    pub fn evaluated_surfaces(&self) -> usize {
        self.evaluated_surfaces
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures exposed edges and the elements that could guard them.
///
/// ADR 0004: every method returns a measurement. None returns a finding.
pub trait GuardService: Send + Sync + 'static {
    fn measure_guard_edges(&self, search: GuardSearch) -> Result<GuardEvidence, GuardError>;
}

/// Registry handle for a [`GuardService`].
#[derive(Clone)]
pub struct GuardServiceHandle(Arc<dyn GuardService>);

impl GuardServiceHandle {
    pub fn new(service: Arc<dyn GuardService>) -> Self {
        Self(service)
    }
    pub fn measure_guard_edges(&self, search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        self.0.measure_guard_edges(search)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn oid(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }
    fn candidate(gap: f64, interval: [f64; 2]) -> GuardCandidate {
        GuardCandidate::try_new(oid("rail"), gap, 1.1, interval, 0.0, None).unwrap()
    }

    /// Two rails guarding the same half of an edge guard half of it. Summing
    /// would report full coverage and hide the unguarded run.
    #[test]
    fn overlapping_candidates_are_unioned_not_summed() {
        let covered = GuardEdge::covered_fraction(
            &[candidate(0.0, [0.0, 0.5]), candidate(0.0, [0.25, 0.5])],
            0.1,
        );
        assert!((covered - 0.5).abs() < 1.0e-9, "{covered}");
    }

    #[test]
    fn candidates_beyond_the_gap_do_not_count_as_coverage() {
        let covered = GuardEdge::covered_fraction(&[candidate(0.9, [0.0, 1.0])], 0.1);
        assert!(covered.abs() < 1.0e-9, "{covered}");
    }

    #[test]
    fn disjoint_candidates_accumulate() {
        let covered = GuardEdge::covered_fraction(
            &[candidate(0.0, [0.0, 0.25]), candidate(0.0, [0.75, 1.0])],
            0.1,
        );
        assert!((covered - 0.5).abs() < 1.0e-9, "{covered}");
    }

    #[test]
    fn malformed_intervals_are_refused() {
        for interval in [[0.5, 0.25], [-0.1, 0.5], [0.0, 1.5], [f64::NAN, 1.0]] {
            assert_eq!(
                GuardCandidate::try_new(oid("r"), 0.0, 1.0, interval, 0.0, None),
                Err(GuardError::InvalidQuantity)
            );
        }
    }

    #[test]
    fn non_positive_search_radii_are_refused() {
        assert_eq!(
            GuardSearch::try_new(0.0, 0.1),
            Err(GuardError::InvalidSearch)
        );
        assert_eq!(
            GuardSearch::try_new(1.0, 0.0),
            Err(GuardError::InvalidSearch)
        );
        assert!(GuardSearch::try_new(1.0, 0.1).is_ok());
    }
}
