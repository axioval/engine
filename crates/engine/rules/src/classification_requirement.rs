//! Requirements on the classifications an object carries.

use axioval_engine::{
    CapabilityEvaluation, ClassificationAssignment, ClassificationError,
    ClassificationServiceHandle, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::Evidence;
use axioval_ir::contract::ParameterValue;
use regex::Regex;

use crate::selection::select_objects;
use crate::support::finding;
use crate::xsd_pattern;

/// Requires an object to carry a classification, optionally in a system and
/// with a code meeting the given literals or XML Schema patterns.
///
/// This is the requirement a `classification` selector cannot state: a
/// system without a code, a system or code given as a pattern, and an
/// optional classification. It reads the same classification service and
/// decides the same way: one assignment must meet the system and the code
/// together, and a code matches when it is the assigned item or any ancestor
/// of it. An unclassified object fails, unless `optional`: an optional
/// classification holds for an object that carries none at all, and must be
/// met by one that carries any, as IDS reads an optional facet. With
/// `prohibited` the verdict is inverted: meeting the requirement is the
/// violation.
///
/// An assignment whose system the source does not state can neither meet
/// nor rule out a system requirement; when it could decide the verdict the
/// object is not evaluated.
pub struct ClassificationRequirement;

impl RuleCapability for ClassificationRequirement {
    /// Reads the model's semantics only; a selector or parameter naming a
    /// measured value or a derived relationship makes the plan's demand
    /// everything regardless (#410).
    fn geometry_reach(&self, _rule: &CompiledRule) -> axioval_engine::GeometryReach {
        axioval_engine::GeometryReach::None
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
                    format!("classification: parameters are invalid: {message}"),
                );
            }
            _ => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    "classification: a requirement cannot be both optional and prohibited",
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<ClassificationServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "classification service is not registered; classifications are unknown",
                );
            }
            return evaluation;
        };
        for object in selected {
            let assignments = match service.classifications(&object.id) {
                Ok(assignments) => assignments,
                Err(error) => {
                    let reason = match error {
                        ClassificationError::Unreadable(_) => NotEvaluatedReason::InvalidEvidence,
                        _ => NotEvaluatedReason::BackendUnavailable,
                    };
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        error.to_string(),
                    );
                    continue;
                }
            };
            match judge(&assignments, &code, &system, optional, prohibited) {
                Judgement::Holds => {}
                Judgement::Undecided => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "a classification of this object is not linked to a system",
                ),
                Judgement::Violation(message) => {
                    let evidence = Evidence::exact(
                        object.id.source.clone(),
                        format!("classifications:{}", object.id),
                    );
                    evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        message,
                        vec![evidence],
                        Vec::new(),
                    ));
                }
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

/// Three-valued: `None` when the assignment's system is unknown and the
/// requirement names one.
fn in_system(assignment: &ClassificationAssignment, system: &Matcher) -> Option<bool> {
    if system.is_none() {
        return Some(true);
    }
    assignment
        .system
        .as_deref()
        .map(|name| system.matches(name))
}

fn has_code(assignment: &ClassificationAssignment, code: &Matcher) -> bool {
    code.is_none()
        || assignment
            .codes
            .iter()
            .flatten()
            .any(|value| code.matches(value))
}

fn judge(
    assignments: &[ClassificationAssignment],
    code: &Matcher,
    system: &Matcher,
    optional: bool,
    prohibited: bool,
) -> Judgement {
    // Whether one assignment meets the system and the code together; `None`
    // when an assignment of unknown system could make it so.
    let mut met = Some(false);
    for assignment in assignments {
        let code_met = has_code(assignment, code);
        // An unknown system cannot make a code that does not match meet.
        let meets = if code_met {
            in_system(assignment, system)
        } else {
            Some(false)
        };
        met = or(met, meets);
    }
    let holds = if prohibited {
        met.map(|met| !met)
    } else if optional && assignments.is_empty() {
        Some(true)
    } else {
        met
    };
    match holds {
        Some(true) => Judgement::Holds,
        None => Judgement::Undecided,
        Some(false) if prohibited => Judgement::Violation(format!(
            "the object carries a prohibited classification ({}): {}",
            required(code, system),
            classified(assignments)
        )),
        Some(false) if assignments.is_empty() => Judgement::Violation(format!(
            "the object has no classification; required: {}",
            required(code, system)
        )),
        Some(false) => Judgement::Violation(format!(
            "no classification meets the requirement ({}); classified as {}",
            required(code, system),
            classified(assignments)
        )),
    }
}

/// What the rule requires, as its author stated it: `system Uniclass, code
/// EF_25`, a pattern between slashes.
fn required(code: &Matcher, system: &Matcher) -> String {
    let mut parts = Vec::new();
    if !system.is_none() {
        parts.push(format!("system {}", system.shown()));
    }
    if !code.is_none() {
        parts.push(format!("code {}", code.shown()));
    }
    if parts.is_empty() {
        "any classification".to_owned()
    } else {
        parts.join(", ")
    }
}

/// The object's assignments, each as its system and assigned code, distinct
/// and sorted: `Uniclass EF_25_10`.
fn classified(assignments: &[ClassificationAssignment]) -> String {
    let mut shown: Vec<String> = assignments
        .iter()
        .map(|assignment| {
            let system = assignment.system.as_deref().unwrap_or("no system");
            match assignment.codes.first().cloned().flatten() {
                Some(code) => format!("{system} {code}"),
                None => format!("{system} without a code"),
            }
        })
        .collect();
    shown.sort_unstable();
    shown.dedup();
    shown.join(", ")
}

/// Three-valued disjunction.
fn or(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// Literals or patterns one text must meet; neither means no requirement.
struct Matcher {
    literals: Vec<String>,
    patterns: Vec<Regex>,
    /// The patterns as written.
    written: Vec<String>,
}

impl Matcher {
    fn read(rule: &CompiledRule, literals: &str, patterns: &str) -> Result<Self, String> {
        let list = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::StringList { value }) => value.clone(),
            _ => Vec::new(),
        };
        let written = list(patterns);
        let patterns = written
            .iter()
            .map(|pattern| {
                xsd_pattern::compile(pattern).map_err(|error| format!("{pattern:?}: {error}"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            literals: list(literals),
            patterns,
            written,
        })
    }

    /// The literals one of which, and the patterns one of which, a text
    /// must meet: `EF_25 or EF_30`, `/EF_.*/`.
    fn shown(&self) -> String {
        let literals = self.literals.join(" or ");
        let patterns = self
            .written
            .iter()
            .map(|pattern| format!("/{pattern}/"))
            .collect::<Vec<_>>()
            .join(" or ");
        match (literals.is_empty(), patterns.is_empty()) {
            (false, false) => format!("{literals} matching {patterns}"),
            (true, _) => patterns,
            (false, true) => literals,
        }
    }

    fn is_none(&self) -> bool {
        self.literals.is_empty() && self.patterns.is_empty()
    }

    /// Literals and patterns together are one requirement: both must hold.
    fn matches(&self, value: &str) -> bool {
        (self.literals.is_empty() || self.literals.iter().any(|literal| literal == value))
            && (self.patterns.is_empty() || self.patterns.iter().any(|regex| regex.is_match(value)))
    }
}
