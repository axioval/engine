//! Which fall-protection defect an edge has.
//!
//! ADR 0004: the guard service measures edges and the elements near them; this
//! module decides what those measurements mean. A capability that reports only
//! "not guarded" is not actionable: a missing railing, a railing with a hole in
//! it, and a landing that is too far below are different defects with different
//! remedies. Each is named, and the worst one for a surface is what gets
//! reported.

use axioval_ir::ObjectId;

/// A fall-protection defect, ordered worst first.
///
/// The order is the reporting priority: when one edge has several defects, the
/// lowest discriminant wins. A missing barrier outranks a short one because a
/// reviewer must act on it first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuardDefect {
    /// Nothing guards the edge at all.
    MissingBarrier,
    /// A barrier exists but does not reach the required height.
    BarrierTooLow,
    /// The barrier is tall enough, but something climbable beside it defeats it.
    BarrierTooLowDueToClimbableObject,
    /// The barrier is tall enough only if measured from the floor, not the curb.
    BarrierTooLowDueToCurb,
    /// The barrier leaves a gap longer than the declaration allows.
    HoleInBarrier,
    /// Landings cover only part of the edge.
    InsufficientLandings,
    /// A landing is present but too narrow to stand on.
    LandingsTooSmall,
    /// The fall onto the landing is further than the declaration allows.
    LandingTooLow,
    /// The nearest landing is beyond the reach the declaration allows.
    LandingTooFarAway,
}

impl GuardDefect {
    /// Stable identifier, used in messages and by downstream consumers.
    pub fn code(self) -> &'static str {
        match self {
            Self::MissingBarrier => "missing_barrier",
            Self::BarrierTooLow => "barrier_too_low",
            Self::BarrierTooLowDueToClimbableObject => "barrier_too_low_due_to_climbable_object",
            Self::BarrierTooLowDueToCurb => "barrier_too_low_due_to_curb",
            Self::HoleInBarrier => "hole_in_barrier",
            Self::InsufficientLandings => "insufficient_landings",
            Self::LandingsTooSmall => "landings_too_small",
            Self::LandingTooLow => "landing_too_low",
            Self::LandingTooFarAway => "landing_too_far_away",
        }
    }
}

/// One defect together with the elements that explain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardDiagnosis {
    defect: GuardDefect,
    related: Vec<ObjectId>,
}

impl GuardDiagnosis {
    /// A diagnosis naming the elements a reviewer needs to look at.
    pub fn new(defect: GuardDefect, related: impl IntoIterator<Item = ObjectId>) -> Self {
        let mut related: Vec<ObjectId> = related.into_iter().collect();
        related.sort();
        related.dedup();
        Self { defect, related }
    }

    /// Which defect this is.
    pub fn defect(&self) -> GuardDefect {
        self.defect
    }

    /// Elements that explain the defect, sorted and deduplicated.
    pub fn related(&self) -> &[ObjectId] {
        &self.related
    }

    /// The worst diagnosis, or `None` when nothing was found.
    ///
    /// Reporting every defect on an edge would bury the one that matters, so a
    /// surface reports its worst. Ties keep the first, which preserves the
    /// order the edges were measured in and keeps output deterministic.
    pub fn worst(diagnoses: impl IntoIterator<Item = Self>) -> Option<Self> {
        diagnoses
            .into_iter()
            .min_by_key(|diagnosis| diagnosis.defect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(value: &str) -> ObjectId {
        ObjectId::new(axioval_ir::SourceId::new("cad", "model").unwrap(), value).unwrap()
    }

    #[test]
    fn worst_ignores_measurement_order() {
        let hole = GuardDiagnosis::new(GuardDefect::HoleInBarrier, [oid("rail")]);
        let missing = GuardDiagnosis::new(GuardDefect::MissingBarrier, []);
        assert_eq!(
            GuardDiagnosis::worst([hole.clone(), missing.clone()]),
            GuardDiagnosis::worst([missing.clone(), hole]),
            "the worst defect must win regardless of which edge was measured first"
        );
        assert_eq!(
            GuardDiagnosis::worst([missing.clone()]).map(|d| d.defect()),
            Some(GuardDefect::MissingBarrier)
        );
    }

    #[test]
    fn related_elements_are_sorted_and_deduplicated() {
        let diagnosis =
            GuardDiagnosis::new(GuardDefect::MissingBarrier, [oid("b"), oid("a"), oid("b")]);
        assert_eq!(diagnosis.related(), &[oid("a"), oid("b")]);
    }

    #[test]
    fn nothing_found_is_not_a_defect() {
        assert_eq!(GuardDiagnosis::worst([]), None);
    }
}
