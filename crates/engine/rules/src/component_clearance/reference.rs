//! `component-clearance` as it judged before its decision became a template
//! over its checks (#286), kept to hold the template to (see
//! `templates.md`). Compiled only with the `parity-reference` feature; never
//! registered. It shares the checks themselves with the template's list.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, FreeSpaceServiceHandle, NotEvaluatedReason,
    ObjectFrameServiceHandle, ParameterDescriptor, PlanSpanServiceHandle, RuleCapability,
    RuleContext, VerticalExtentServiceHandle,
};

use super::{Config, ID, Obstacles, Services, Split, judged};
use crate::selection::select_objects;
use crate::support::finding;
use crate::wall_sides::Walls;

/// The capability as it judged before its template.
pub struct ComponentClearance;

impl RuleCapability for ComponentClearance {
    fn id(&self) -> &'static str {
        ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("component-clearance: {message}"),
                );
            }
        };
        let (Some(frames), Some(extents), Some(free_space)) = (
            context.services.get::<ObjectFrameServiceHandle>(),
            context.services.get::<VerticalExtentServiceHandle>(),
            context.services.get::<FreeSpaceServiceHandle>(),
        ) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "component-clearance needs the object-frame, vertical-extent and free-space \
                 services",
            );
        };
        let spans = context.services.get::<PlanSpanServiceHandle>();
        if config.walls.is_some() && spans.is_none() {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "component-clearance with `front_axis` `against-wall` needs the plan-span service",
            );
        }
        let walls = config
            .walls
            .map(|(selector, _, _)| Walls::select(context, selector));
        let supports = config
            .support
            .map(|(selector, _)| Split::select(context, selector, "support"));
        let services = Services {
            frames,
            extents,
            free_space,
            spans,
            walls: walls.as_ref(),
            supports: supports.as_ref(),
        };
        let obstacles = Obstacles::select(context, &config);
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            for (label, result) in judged(context, &config, &services, &obstacles, object) {
                match result {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!("{label} {message}"),
                        evidence,
                        related,
                    )),
                    Err((reason, message)) => evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        format!("{label}: {message}"),
                    ),
                }
            }
        }
        evaluation
    }
}
