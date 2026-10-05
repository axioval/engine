//! `body-extent`: the depth of an object's body along one of its own
//! placement axes, against a stated length or a range.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, MetricDirection, NotEvaluatedReason, ObjectFrame,
    ObjectFrameError, ParameterDescriptor, RuleCapability, RuleContext, VerticalExtentError,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::ExtentMeasures;

use crate::support::{Unavailable, invalid};

/// Requires each selected object's body, measured along one of its own
/// placement axes, to be as long as a property states or within a range.
///
/// A straight wall's layer set states its thickness; the body measures it
/// across the wall. With `axis` `forward` (the placement's second axis,
/// across a wall whose layers run along its first) and `target_property`
/// the material set's `TotalThickness`, a wall whose body is thicker or
/// thinner than its layers is found. `minimum` and `maximum` bound the
/// extent instead.
///
/// The extent is the whole body's depth along the axis: the highest less
/// the lowest point projected onto it. It is the thickness only where the
/// body is a slab of constant thickness across that axis; a curved wall, or
/// one with a projecting part, measures deeper. Select the objects the
/// measure fits.
///
/// Extents are intervals: a tessellated body measures within its chord
/// deviation, and an axis off the coordinate axes within the rounding of
/// the projection. A verdict needs the whole interval on one side of a
/// bound; one straddling it is not evaluated. The target and range are
/// widened by the binary rounding of decimal coordinates, a few units in
/// the last place, so an exactly modelled body is not found by rounding.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `body_extent` along the axis, judged by the generic range judge, its
/// bounds widened by the rounding of the body's coordinates
/// (`body_position`), the stated length and the tolerance.
pub struct BodyExtent;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

impl RuleCapability for BodyExtent {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run(&TEMPLATE, context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Right,
    Forward,
    Up,
}

impl Axis {
    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "right" => Ok(Self::Right),
            "forward" => Ok(Self::Forward),
            "up" => Ok(Self::Up),
            other => Err(invalid(format!(
                "axis `{other}` is unsupported; use `right`, `forward` or `up`"
            ))),
        }
    }

    fn of(self, frame: &ObjectFrame) -> MetricDirection {
        let frame = frame.frame();
        match self {
            Self::Right => frame.right(),
            Self::Forward => frame.forward(),
            Self::Up => frame.up(),
        }
    }

    #[cfg(feature = "parity-reference")]
    fn name(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Forward => "forward",
            Self::Up => "up",
        }
    }
}

pub(crate) fn frame_error(error: &ObjectFrameError) -> Unavailable {
    let reason = match error {
        ObjectFrameError::UncoveredSource(_) => NotEvaluatedReason::MissingService,
        ObjectFrameError::NotPlaced(_)
        | ObjectFrameError::Unsupported(_)
        | ObjectFrameError::Unreadable(_) => NotEvaluatedReason::IncompleteEvidence,
        ObjectFrameError::UnknownObject(_)
        | ObjectFrameError::InvalidFrame
        | ObjectFrameError::InexactEvidence
        | ObjectFrameError::ResponseRequestMismatch => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("object frame: {error}"))
}

pub(crate) fn extent_error(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, format!("body extent: {error}"))
}

/// A few units in the last place of the largest magnitude involved: decimal
/// coordinates and lengths read in binary differ from what was meant by
/// that much, and no more.
pub(crate) fn rounding_slack(magnitudes: &[f64]) -> f64 {
    axioval_engine::template::ROUNDING_ULPS
        * magnitudes
            .iter()
            .fold(0.0_f64, |most, value| most.max(value.abs()))
}
