//! Exact source-neutral space-validation capability.
//!
//! ADR 0004: geometry is measured by a [`SpaceServiceHandle`], as the
//! measured values and lists of [`SpaceMeasures`]; every threshold and
//! severity decision is the template's.
//!
//! Each aspect is requested and judged independently, so an adapter that
//! cannot measure one of them costs only that aspect. The source bundled all
//! seven behind one call and failed the whole space when any single branch was
//! missing.
//!
//! Every finding message starts with its sub-check's category code (see
//! [`SpaceCategory`]), so results can be grouped by problem as well as by space.
//!
//! [`SpaceServiceHandle`]: axioval_engine::SpaceServiceHandle

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext, SpaceError,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::SpaceMeasures;

/// The sub-check a finding comes from, written as the message's leading code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceCategory {
    /// Another space has the same body.
    DuplicateSpace,
    /// Clear height below the requirement.
    InsufficientHeight,
    /// A run of space boundary no element covers.
    UncoveredBoundary,
    /// The space contains, or is contained by, another body.
    ContainedBody,
    /// The space intersects another space.
    IntersectingSpace,
    /// The space intersects a component.
    IntersectingComponent,
    /// The top cap is not fully covered.
    UncoveredTopCap,
    /// The bottom cap is not fully covered.
    UncoveredBottomCap,
    /// A connected region of storey floor belongs to no space.
    UnallocatedArea,
}

impl SpaceCategory {
    /// The stable code a finding message starts with.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DuplicateSpace => "duplicate_space",
            Self::InsufficientHeight => "insufficient_height",
            Self::UncoveredBoundary => "uncovered_boundary",
            Self::ContainedBody => "contained_body",
            Self::IntersectingSpace => "intersecting_space",
            Self::IntersectingComponent => "intersecting_component",
            Self::UncoveredTopCap => "uncovered_top_cap",
            Self::UncoveredBottomCap => "uncovered_bottom_cap",
            Self::UnallocatedArea => "unallocated_area",
        }
    }
}

/// Validates spaces against height, duplication, coverage and overlap rules.
///
/// It runs as a template ([`axioval_engine::template`]): each aspect of each
/// selected space a check of its own, and the storeys' unallocated floor
/// checks of the project.
pub struct SpaceValidation;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SpaceValidation {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn grades_deviation(&self) -> bool {
        TEMPLATE.grades
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

/// Why the space service's refusal leaves an aspect not evaluated: an
/// unavailable, refused or unmeasured aspect is incomplete evidence, an inexact or
/// incoherent answer invalid evidence.
pub(crate) fn reason(error: &SpaceError) -> NotEvaluatedReason {
    match error {
        SpaceError::Unavailable | SpaceError::Refused(_) | SpaceError::Unmeasured(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
        SpaceError::InexactEvidence | SpaceError::InvalidQuantity => {
            NotEvaluatedReason::InvalidEvidence
        }
    }
}
