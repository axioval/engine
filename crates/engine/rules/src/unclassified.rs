//! Objects a ruleset's classification leaves unclassified.

use axioval_engine::{
    CapabilityEvaluation, ClassOutcome, Classifications, CompiledRule, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::{CLASSIFICATION_SET, Evidence};

use crate::selection::select_objects;
use crate::support::{Parameters, finding};

/// Reports every selected object the ruleset's classification
/// `classification` assigns no class: every one of its rows surely does
/// not match.
///
/// An object whose class a row cannot decide is not evaluated, never
/// unclassified. A classification the ruleset does not declare leaves the
/// rule not evaluated.
pub struct UnclassifiedObject;

impl RuleCapability for UnclassifiedObject {
    fn id(&self) -> &'static str {
        "axioval:capability.unclassified-object"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "classification",
            ParameterType::String,
        )]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Ok(Some(id)) = Parameters(rule).string("classification") else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "unclassified-object: `classification` names no classification",
            );
        };
        let Some(classifications) = context.services.get::<std::sync::Arc<Classifications>>()
        else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "unclassified-object: no classification is derived outside a run",
            );
        };
        if !classifications.contains(id) {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                format!("unclassified-object: the ruleset declares no classification `{id}`"),
            );
        }
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            match classifications.outcome(id, &object.id) {
                Some(ClassOutcome::Classified { .. }) => {}
                Some(ClassOutcome::Unclassified) => evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    format!("unclassified: no row of classification `{id}` matches"),
                    vec![Evidence::exact(
                        object.id.source.clone(),
                        format!("{CLASSIFICATION_SET}/{id}#no-row"),
                    )],
                    Vec::new(),
                )),
                Some(ClassOutcome::Undecided(reason, message)) => {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason.clone(),
                        message.clone(),
                    );
                }
                None => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    format!("classification `{id}` did not classify {}", object.id),
                ),
            }
        }
        evaluation
    }
}
