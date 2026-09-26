//! Requirements on the classifications an object carries.

use axioval_engine::{
    CapabilityEvaluation, ClassificationAssignment, ClassificationError,
    ClassificationServiceHandle, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::Evidence;
use axioval_ir::contract::ParameterValue;
use regex::Regex;

use crate::property_value::finding;
use crate::selection::select_objects;
use crate::xsd_pattern;

/// Requires an object to carry a classification, optionally with a code and
/// a system meeting the given literals or XML Schema patterns.
///
/// An object's classifications are every assignment its source states,
/// each with its whole code chain, so a code matches when it is the assigned
/// item or any ancestor of it. The code and the system requirements are each
/// met by any assignment, not necessarily the same one. Without any
/// classification the object fails, unless `optional`. With `prohibited` the
/// verdict is inverted: meeting the requirement is the violation.
///
/// An assignment whose system the source does not state can neither meet
/// nor rule out a system requirement; when it could decide the verdict the
/// object is not evaluated.
pub struct ClassificationRequirement;
impl RuleCapability for ClassificationRequirement {
    fn selectable(&self) -> bool {
        true
    }

    fn id(&self) -> &'static str {
        "axioval:capability.classification"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("codes", ParameterType::StringList),
            ParameterDescriptor::optional("code_patterns", ParameterType::StringList),
            ParameterDescriptor::optional("systems", ParameterType::StringList),
            ParameterDescriptor::optional("system_patterns", ParameterType::StringList),
            ParameterDescriptor::optional("optional", ParameterType::Boolean),
            ParameterDescriptor::optional("prohibited", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let flag = |name: &str| {
            matches!(
                rule.parameters.get(name),
                Some(ParameterValue::Boolean { value: true })
            )
        };
        let (optional, prohibited) = (flag("optional"), flag("prohibited"));
        let (code, system) = match (
            Matcher::read(rule, "codes", "code_patterns"),
            Matcher::read(rule, "systems", "system_patterns"),
        ) {
            (Ok(code), Ok(system)) if !(optional && prohibited) => (code, system),
            (Err(message), _) | (_, Err(message)) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("classification parameters are invalid: {message}"),
                );
            }
            _ => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    "classification cannot be both optional and prohibited",
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<ClassificationServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "classification service is not registered",
                );
            }
            return evaluation;
        };
        for object in selected {
            let assignments = match service.classifications(&object.id) {
                Ok(assignments) => assignments,
                Err(error) => {
                    let reason = match error {
                        ClassificationError::UncoveredSource(_) => {
                            NotEvaluatedReason::MissingService
                        }
                        _ => NotEvaluatedReason::InvalidEvidence,
                    };
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        error.to_string(),
                    );
                    continue;
                }
            };
            let violation = match judge(&assignments, &code, &system, optional, prohibited) {
                Judgement::Undecided => {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        "a classification of this object is not linked to a system",
                    );
                    continue;
                }
                Judgement::Violation(message) => Some(message),
                Judgement::Holds => None,
            };
            if let Some(message) = violation {
                let evidence = Evidence::exact(
                    object.id.source.clone(),
                    format!("classifications:{}", object.id),
                );
                evaluation.push_finding(finding(rule, object, message, vec![evidence]));
            }
        }
        evaluation
    }
}

enum Judgement {
    Holds,
    Violation(String),
    /// An assignment without a system could decide the verdict.
    Undecided,
}

fn judge(
    assignments: &[ClassificationAssignment],
    code: &Matcher,
    system: &Matcher,
    optional: bool,
    prohibited: bool,
) -> Judgement {
    let classified = !assignments.is_empty();
    let code_met = code.is_none()
        || assignments
            .iter()
            .flat_map(|assignment| assignment.codes.iter().flatten())
            .any(|value| code.matches(value));
    let known_systems: Vec<&str> = assignments
        .iter()
        .filter_map(|assignment| assignment.system.as_deref())
        .collect();
    let system_met = system.is_none() || known_systems.iter().any(|value| system.matches(value));
    // An unstated system could flip an unmet requirement to met.
    if classified
        && code_met
        && !system_met
        && assignments
            .iter()
            .any(|assignment| assignment.system.is_none())
    {
        return Judgement::Undecided;
    }
    let met = classified && code_met && system_met;
    let violation = if prohibited {
        met.then(|| "the object carries a prohibited classification".to_owned())
    } else if !classified {
        (!optional).then(|| "the object has no classification".to_owned())
    } else {
        (!met).then(|| {
            let systems = if known_systems.is_empty() {
                "none stated".to_owned()
            } else {
                known_systems.join(", ")
            };
            format!("no classification meets the requirement (systems: {systems})")
        })
    };
    violation.map_or(Judgement::Holds, Judgement::Violation)
}

/// Literals or patterns one text must meet; neither means no requirement.
pub(crate) struct Matcher {
    literals: Vec<String>,
    patterns: Vec<Regex>,
}

impl Matcher {
    pub(crate) fn read(
        rule: &CompiledRule,
        literals: &str,
        patterns: &str,
    ) -> Result<Self, String> {
        let list = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::StringList { value }) => value.clone(),
            _ => Vec::new(),
        };
        let patterns = list(patterns)
            .iter()
            .map(|pattern| {
                xsd_pattern::compile(pattern).map_err(|error| format!("{pattern:?}: {error}"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            literals: list(literals),
            patterns,
        })
    }

    pub(crate) fn is_none(&self) -> bool {
        self.literals.is_empty() && self.patterns.is_empty()
    }

    /// Literals and patterns together are one requirement: all must hold.
    pub(crate) fn matches(&self, value: &str) -> bool {
        (self.literals.is_empty() || self.literals.iter().any(|literal| literal == value))
            && (self.patterns.is_empty() || self.patterns.iter().any(|regex| regex.is_match(value)))
    }
}
