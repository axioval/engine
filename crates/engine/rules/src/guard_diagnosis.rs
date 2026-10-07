//! Which fall-protection defect an edge has: the vocabulary
//! `horizontal-guard` words its findings in.
//!
//! ADR 0004: the guard service measures edges and the elements near them;
//! the template decides what those measurements mean. A capability that
//! reports only "not guarded" is not actionable: a missing railing, a
//! railing with a hole in it, and a landing that is too far below are
//! different defects with different remedies. Each is named, and each
//! distinct one on a surface is reported once.

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
    #[must_use]
    pub const fn code(self) -> &'static str {
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
