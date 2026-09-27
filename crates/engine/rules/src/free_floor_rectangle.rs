//! Exact source-neutral free-floor-rectangle capability.

use axioval_engine::{
    BoxClearance, CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PlacementOrientation, PlacementShape, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;

use crate::free_floor::{self, Options, unavailable};
use crate::selection::select_objects;

/// Exact free-floor rectangle placement using a trusted free-space service.
pub struct FreeFloorRectangle;

impl RuleCapability for FreeFloorRectangle {
    fn id(&self) -> &'static str {
        "axioval:capability.free-floor-rectangle"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("width_metres", ParameterType::Number),
            ParameterDescriptor::required("length_metres", ParameterType::Number),
            ParameterDescriptor::required("height_metres", ParameterType::Number),
            // Optional in the signature so that a rule without it is reported
            // per object as an invalid declaration, not rejected with its whole
            // package.
            ParameterDescriptor::optional("orientation", ParameterType::String),
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
        let Some((width, length, height)) = dimensions(rule) else {
            return invalid("free-floor rectangle dimensions are invalid", evaluation);
        };
        let Ok(shape) = BoxClearance::try_new(width, length, height) else {
            return invalid(
                "free-floor rectangle dimensions must be positive and finite",
                evaluation,
            );
        };
        let orientation = match rule.parameters.get("orientation") {
            Some(ParameterValue::String { value }) if value == "any" => PlacementOrientation::Any,
            Some(ParameterValue::String { value }) => {
                let message = format!(
                    "free-floor rectangle orientation `{value}` is not supported; \
                     only `any` has a frame source"
                );
                return invalid(&message, evaluation);
            }
            _ => {
                return invalid(
                    "free-floor rectangle needs an explicit `orientation`",
                    evaluation,
                );
            }
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
            &PlacementShape::Box { shape, orientation },
            &options,
            "NO_FREE_FLOOR_SPACE_FOR_RECTANGLE",
        )
    }
}

fn dimensions(rule: &CompiledRule) -> Option<(f64, f64, f64)> {
    let number = |name: &str| match rule.parameters.get(name)? {
        ParameterValue::Number { value } => Some(*value),
        _ => None,
    };
    Some((
        number("width_metres")?,
        number("length_metres")?,
        number("height_metres")?,
    ))
}
