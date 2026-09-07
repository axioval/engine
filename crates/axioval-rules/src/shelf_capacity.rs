//! Exact source-neutral shelf-capacity capability.
//!
//! ADR 0004: the measurement (running metres of shelving) comes from a
//! [`LinearQuantityServiceHandle`]; the decision -- whether that clears the
//! declared minimum -- is made here, in source-neutral policy.
//!
//! In the the provider application this comparison lived inside
//! `production/accessibility`-style adapter code, so the rule could only
//! restate a verdict it had already been handed. Splitting it this way is what
//! lets the rule port without dragging an IFC-shaped verdict producer along.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, LinearQuantityError, LinearQuantityKind,
    LinearQuantityRequest, LinearQuantityServiceHandle, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Finding, Severity};

use crate::selection::select_objects;

/// Minimum running metres of shelving a space must provide.
pub struct ShelfCapacity;

impl RuleCapability for ShelfCapacity {
    fn id(&self) -> &'static str {
        "axioval:capability.shelf-capacity"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "minimum_running_metres",
            ParameterType::Number,
        )]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, mut evaluation) = select_objects(context, &rule.selector);

        let Some(minimum) = minimum_running_metres(rule) else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidDeclaration,
                    "shelf capacity minimum must be a finite, non-negative number",
                );
            }
            return evaluation;
        };

        let Some(service) = context.services.get::<LinearQuantityServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "linear-quantity service is not registered",
                );
            }
            return evaluation;
        };

        for object in selected {
            let request = LinearQuantityRequest::new(
                object.id.clone(),
                LinearQuantityKind::ShelfRunningLength,
            );
            match service.measure_linear_quantity(&request) {
                Ok(measured) => {
                    let interval = measured.measured();
                    if interval.definitely_at_least(minimum) {
                        continue;
                    }
                    // Fail closed on ambiguity: an interval that straddles the
                    // minimum has not been shown to fail, so reporting a
                    // violation would assert more than was measured.
                    if !interval.definitely_below(minimum) {
                        evaluation.push_object_not_evaluated(
                            object.id.clone(),
                            NotEvaluatedReason::IncompleteEvidence,
                            "measured shelf length spans the required minimum",
                        );
                        continue;
                    }
                    evaluation.push_finding(Finding {
                        rule_id: rule.id.clone(),
                        object_id: object.id.clone(),
                        severity: Severity::Warning,
                        message: format!(
                            "shelf running metres {:.3} below required {minimum:.3}",
                            interval.upper_metres()
                        ),
                        evidence: vec![measured.evidence().clone()],
                    });
                }
                Err(error) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    match error {
                        LinearQuantityError::Unavailable => NotEvaluatedReason::IncompleteEvidence,
                        // Both mean the adapter produced something it cannot
                        // stand behind, which is an evidence defect, not a
                        // missing measurement.
                        LinearQuantityError::InexactEvidence
                        | LinearQuantityError::InvalidInterval => {
                            NotEvaluatedReason::InvalidEvidence
                        }
                    },
                    error.to_string(),
                ),
            }
        }
        evaluation
    }
}

fn minimum_running_metres(rule: &CompiledRule) -> Option<f64> {
    match rule.parameters.get("minimum_running_metres")? {
        ParameterValue::Number { value } if value.is_finite() && *value >= 0.0 => Some(*value),
        _ => None,
    }
}
