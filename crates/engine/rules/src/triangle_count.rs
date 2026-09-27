//! `triangle-count`: the polygons of each element's mesh against a maximum.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, TriangleCountError, TriangleCountServiceHandle,
};

use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Requires each selected object's mesh to hold at most `maximum` triangles.
///
/// The count is of the mesh the host produced for the object, not of
/// anything the model states: a box extruded from a rectangle counts twelve
/// triangles, and a curved face as many as the host's chord budget made of
/// it. Another host, or another budget, may count differently. A finding on
/// such a tessellation carries approximate evidence and says that the count
/// depends on the tessellation. An object the host declared bodiless counts
/// none; one whose body could not be meshed is not evaluated.
pub struct TriangleCountLimit;

impl RuleCapability for TriangleCountLimit {
    fn id(&self) -> &'static str {
        "axioval:capability.triangle-count"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "maximum",
            ParameterType::Integer,
        )]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let maximum = match parameters.integer("maximum").and_then(|maximum| {
            let maximum = maximum.ok_or_else(|| invalid("parameter `maximum` is required"))?;
            u64::try_from(maximum).map_err(|_| invalid("`maximum` is negative"))
        }) {
            Ok(maximum) => maximum,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("triangle-count: {message}"),
                );
            }
        };
        let Some(counts) = context.services.get::<TriangleCountServiceHandle>() else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "triangle-count service is not registered",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let count = match counts
                .count_triangles(&object.id)
                .map_err(|error| count_error(&error))
            {
                Ok(count) => count,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            if count.triangles() > maximum {
                let tessellated = if count.is_exact() {
                    ""
                } else {
                    "; the mesh tessellates curved faces, so the count depends on the host's \
                     tessellation"
                };
                evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    format!(
                        "mesh has {} triangles; at most {maximum} allowed{tessellated}",
                        count.triangles()
                    ),
                    vec![count.evidence().clone()],
                    vec![],
                ));
            }
        }
        evaluation
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
