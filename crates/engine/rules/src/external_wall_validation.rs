//! Exact source-neutral external-wall validation capability.
//!
//! ADR 0004: envelope membership is measured by an
//! [`EnvelopeMembershipServiceHandle`]; whether the model's declaration agrees
//! with the derivation is decided here.
//!
//! Applicability is deliberately absent. The source provider inspected a
//! model's industry domain and returned an "irrelevant" flag the rule had to
//! interpret; if a rule should not apply to a model, that belongs to the
//! selector, not behind the evidence seam.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, EnvelopeDerivation, EnvelopeMembershipError,
    EnvelopeMembershipRequest, EnvelopeMembershipServiceHandle, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Finding, ObjectId, Severity};
use std::collections::BTreeSet;

use crate::selection::select_objects;

/// Requires a model's declared external walls to match the derived envelope.
pub struct ExternalWallValidation;

impl RuleCapability for ExternalWallValidation {
    fn id(&self) -> &'static str {
        "axioval:capability.external-wall-validation"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "envelope_derivation",
            ParameterType::String,
        )]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        // Selection establishes that the rule applies at all; the finding is
        // reported against the model scope rather than per selected wall.
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        if selected.is_empty() {
            return evaluation;
        }

        let Some(derivation) = derivation(rule) else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "envelope derivation must be 'all-spaces' or 'gross-area-groups'",
            );
            return evaluation;
        };

        let Some(service) = context.services.get::<EnvelopeMembershipServiceHandle>() else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::MissingService,
                "envelope-membership service is not registered",
            );
            return evaluation;
        };

        let request = EnvelopeMembershipRequest::new(derivation);
        match service.measure_envelope_membership(&request) {
            Ok(measured) => {
                // The derivation spans the whole model; the selector says
                // which objects this rule is about.
                let selected: BTreeSet<&ObjectId> = selected.iter().map(|o| &o.id).collect();
                for object_id in measured.undeclared() {
                    if selected.contains(object_id) {
                        evaluation.push_object_not_evaluated(
                            object_id.clone(),
                            NotEvaluatedReason::IncompleteEvidence,
                            "the model states neither external nor internal",
                        );
                    }
                }
                if measured.agrees() {
                    return evaluation;
                }
                // Report each disagreeing wall against itself, so a reviewer
                // opens the element rather than a whole-model message.
                for object_id in measured
                    .declared_only()
                    .into_iter()
                    .filter(|id| selected.contains(id))
                {
                    evaluation.push_finding(Finding {
                        rule_id: rule.id.clone(),
                        object_id,
                        severity: Severity::Warning,
                        related: Vec::new(),
                        message: format!(
                            "declared external but not on the {} envelope",
                            derivation.as_str()
                        ),
                        evidence: vec![measured.evidence().clone()],
                    });
                }
                for object_id in measured
                    .derived_only()
                    .into_iter()
                    .filter(|id| selected.contains(id))
                {
                    evaluation.push_finding(Finding {
                        rule_id: rule.id.clone(),
                        object_id,
                        severity: Severity::Warning,
                        related: Vec::new(),
                        message: format!(
                            "on the {} envelope but not declared external",
                            derivation.as_str()
                        ),
                        evidence: vec![measured.evidence().clone()],
                    });
                }
            }
            Err(error) => evaluation.push_not_evaluated(
                match error {
                    EnvelopeMembershipError::Unavailable
                    | EnvelopeMembershipError::UnsupportedDerivation => {
                        NotEvaluatedReason::IncompleteEvidence
                    }
                    EnvelopeMembershipError::InexactEvidence => NotEvaluatedReason::InvalidEvidence,
                },
                error.to_string(),
            ),
        }
        evaluation
    }
}

fn derivation(rule: &CompiledRule) -> Option<EnvelopeDerivation> {
    match rule.parameters.get("envelope_derivation")? {
        ParameterValue::String { value } if value == "all-spaces" => {
            Some(EnvelopeDerivation::AllSpaces)
        }
        ParameterValue::String { value } if value == "gross-area-groups" => {
            Some(EnvelopeDerivation::GrossAreaGroups)
        }
        _ => None,
    }
}
