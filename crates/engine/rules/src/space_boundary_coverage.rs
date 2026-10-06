//! `space-boundary-coverage`: how much of each space's body surface its
//! declared space boundaries cover, the gaps they leave and where they
//! overlap.
//!
//! ADR 0004: the coverage is measured by a [`BoundaryCoverageServiceHandle`],
//! as the measured values `boundary_coverage_off`, `boundary_coverage_share`,
//! `boundary_coverage_uncovered`, `boundary_coverage_overlap` and
//! `boundary_coverage_surface` ([`BoundaryMeasures`]); which share or area
//! is acceptable is the template's policy.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    BoundaryCoverageError, CapabilityEvaluation, CompiledRule, NotEvaluatedReason,
    ParameterDescriptor, RuleCapability, RuleContext,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::BoundaryMeasures;

use crate::support::Unavailable;

/// Requires each selected space's declared boundaries to cover its body's
/// surface: at least `minimum_covered_share` of it, leaving at most
/// `maximum_uncovered_area` uncovered and overlapping each other over at most
/// `maximum_overlap_area`. At least one of the three is required.
///
/// The boundaries are the ones the source declares for the space, with the
/// connection surfaces it states; the rule selects spaces, never boundaries.
/// A boundary surface counts on a face of the body when it lies within
/// `plane_tolerance` of the face's plane (default zero: on the plane, up to
/// a micrometre). A boundary lying on no face plane covers nothing and is
/// always its own finding, relating the element it bounds against: it is
/// misplaced whatever the thresholds are. A space whose boundary surfaces
/// cannot all be read (a surface form the host does not lower, or a
/// boundary stating none), whose body is curved or missing is not
/// evaluated.
///
/// Areas are intervals: a turned space measures within the rounding of its
/// projection, a tessellated boundary within its chord deviation. A check
/// is a finding only when the whole interval breaks its bound; one
/// straddling it is not evaluated, and everything one space leaves open is
/// one outcome. An overlap finding relates the elements of the boundaries
/// that surely overlap.
///
/// It runs as a template ([`axioval_engine::template`]) over the measured
/// coverage.
pub struct SpaceBoundaryCoverage;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SpaceBoundaryCoverage {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// Why the boundary-coverage service refused a space, as every reader of
/// it reports the refusal.
pub(crate) fn coverage_error(error: &BoundaryCoverageError) -> Unavailable {
    let reason = match error {
        BoundaryCoverageError::UnknownSpace(_)
        | BoundaryCoverageError::NoBody(_)
        | BoundaryCoverageError::Unavailable(_) => NotEvaluatedReason::BackendUnavailable,
        BoundaryCoverageError::Unsupported => NotEvaluatedReason::MissingService,
        BoundaryCoverageError::InvalidRequest(_) => NotEvaluatedReason::InvalidDeclaration,
        BoundaryCoverageError::InvalidMeasurement => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("space-boundary coverage: {error}"))
}
