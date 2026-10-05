//! `triangle-count` as it was implemented before it became a template
//! (#282), kept only as the parity reference the template is held to in
//! the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, TriangleCountServiceHandle,
};

use super::count_error;
use crate::selection::select_objects;
use crate::support::{Parameters, finding, invalid};

/// Requires each selected object's mesh to hold at most `maximum` triangles.
pub struct TriangleCountLimit;

impl RuleCapability for TriangleCountLimit {
    fn id(&self) -> &'static str {
        super::template::ID
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
