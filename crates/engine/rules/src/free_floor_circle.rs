//! Exact source-neutral free-floor-circle capability.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CylinderClearance, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PlacementShape, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;

use crate::free_floor::{self, Options, unavailable};
use crate::selection::select_objects;

/// Exact free-floor circle placement using a trusted free-space service.
pub struct FreeFloorCircle;

impl RuleCapability for FreeFloorCircle {
    fn id(&self) -> &'static str {
        "axioval:capability.free-floor-circle"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("diameter_metres", ParameterType::Number),
            ParameterDescriptor::required("height_metres", ParameterType::Number),
        ];
        parameters.extend(free_floor::parameters());
        parameters
    }
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, evaluation) = select_objects(context, &rule.selector);
        let invalid = |message: &str, evaluation| {
            unavailable(
                &selected,
                &NotEvaluatedReason::InvalidDeclaration,
                message,
                evaluation,
            )
        };
        let Some((diameter, height)) = dimensions(rule) else {
            return invalid("free-floor circle dimensions are invalid", evaluation);
        };
        let Ok(shape) = CylinderClearance::try_new(diameter / 2.0, height) else {
            return invalid(
                "free-floor circle dimensions must be positive and finite",
                evaluation,
            );
        };
        let options = match Options::parse(rule, height) {
            Ok(options) => options,
            Err((_, message)) => return invalid(&message, evaluation),
        };
        free_floor::evaluate(
            context,
            rule,
            &selected,
            evaluation,
            &PlacementShape::Cylinder(shape),
            &options,
            "NO_FREE_FLOOR_SPACE_FOR_CIRCLE",
        )
    }
}

fn dimensions(rule: &CompiledRule) -> Option<(f64, f64)> {
    let number = |name: &str| match rule.parameters.get(name)? {
        ParameterValue::Number { value } => Some(*value),
        _ => None,
    };
    Some((number("diameter_metres")?, number("height_metres")?))
}
