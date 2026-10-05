//! `triangle-count`: the polygons of each element's mesh against a maximum.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext, TriangleCountError,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::TriangleMeasures;

use crate::support::Unavailable;

/// Requires each selected object's mesh to hold at most `maximum` triangles.
///
/// The count is of the mesh the host produced for the object, not of
/// anything the model states: a box extruded from a rectangle counts twelve
/// triangles, and a curved face as many as the host's chord budget made of
/// it. Another host, or another budget, may count differently. A finding on
/// such a tessellation carries approximate evidence and says that the count
/// depends on the tessellation. An object the host declared bodiless counts
/// none; one whose body could not be meshed is not evaluated.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `triangle_count`, judged by the generic range judge against `maximum`.
pub struct TriangleCountLimit;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

impl RuleCapability for TriangleCountLimit {
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

fn count_error(error: &TriangleCountError) -> Unavailable {
    let reason = match error {
        TriangleCountError::UnknownObject(_) | TriangleCountError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        TriangleCountError::InvalidMeasurement => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("triangle count: {error}"))
}
