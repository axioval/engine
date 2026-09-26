//! Requirements on how many objects a rule selects.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, ObjectId, RuleFinding};

use crate::property_value::{finding, severity_of};
use crate::selection::select_objects;

/// Requires the number of selected objects to be at least `min` and at most
/// `max` (each optional, at least one given).
///
/// Too few is one finding about the rule, since there is no object to
/// report it against. Too many is one finding naming the selected objects,
/// except that with `max` zero every selected object is itself the
/// violation and gets its own finding. Objects whose membership could not be
/// decided are reported as not evaluated by selection; when they could
/// change the verdict, the rule is not evaluated rather than passed or
/// failed.
pub struct PopulationRequirement;
impl RuleCapability for PopulationRequirement {
    fn id(&self) -> &'static str {
        "axioval:capability.population"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("min", ParameterType::Integer),
            ParameterDescriptor::optional("max", ParameterType::Integer),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some((min, max)) = bounds(rule) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "population needs non-negative bounds, at least one, and min not above max",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let known = selected.len();
        let unknown = evaluation.not_evaluated_outcomes().len();
        let rule_finding = |message: String, related: Vec<ObjectId>| RuleFinding {
            rule_id: rule.id.clone(),
            severity: severity_of(rule),
            message,
            related,
            evidence: Vec::new(),
        };
        if let Some(min) = min {
            if known + unknown < min {
                evaluation.push_rule_finding(rule_finding(
                    format!("{known} applicable object(s); at least {min} required"),
                    Vec::new(),
                ));
            } else if known < min {
                evaluation.push_not_evaluated(
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{known} applicable object(s) known and {unknown} undecided; at least {min} required"
                    ),
                );
            }
        }
        match max {
            Some(0) => {
                for object in &selected {
                    evaluation.push_finding(finding(
                        rule,
                        object,
                        "no applicable object may exist".to_owned(),
                        vec![Evidence::exact(
                            object.id.source.clone(),
                            format!("selected:{}", object.id),
                        )],
                    ));
                }
            }
            Some(max) if known > max => evaluation.push_rule_finding(rule_finding(
                format!("{known} applicable objects; at most {max} allowed"),
                selected.iter().map(|object| object.id.clone()).collect(),
            )),
            Some(max) if known + unknown > max => evaluation.push_not_evaluated(
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{known} applicable object(s) known and {unknown} undecided; at most {max} allowed"
                ),
            ),
            _ => {}
        }
        evaluation
    }
}

/// `min` and `max`, when at least one is given, both are non-negative and
/// they are ordered.
fn bounds(rule: &CompiledRule) -> Option<(Option<usize>, Option<usize>)> {
    let bound = |name: &str| match rule.parameters.get(name) {
        Some(ParameterValue::Integer { value }) => usize::try_from(*value).ok().map(Some),
        None => Some(None),
        Some(_) => None,
    };
    let (min, max) = (bound("min")?, bound("max")?);
    let ordered = min.zip(max).is_none_or(|(min, max)| min <= max);
    ((min.is_some() || max.is_some()) && ordered).then_some((min, max))
}
