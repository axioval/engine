//! A requirement stated as an expression over each selected object.

use axioval_engine::expression::{Evaluation, NotEvaluated, Reason, Value, evaluate};
use std::collections::BTreeMap;

use axioval_engine::draft::{ExpressionTraces, ObjectTrace};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::{Expression, ParameterValue};
use axioval_ir::{Explanation, NotEvaluatedReason};

use crate::expression_leaves::ObjectLeaves;
use crate::selection::select_objects;
use crate::support::finding;

/// Requires the expression `requirement` to hold for each selected object.
///
/// True passes, false is a finding naming the subexpression that failed and
/// every value read, `null` (a value the requirement needs is stated
/// absent) is a missing-information finding, never a pass, and not
/// evaluated leaves the object not evaluated with the reason, naming the
/// subexpression. A rule definition may declare
/// parameters of its own (scalar values, string lists and tables) for the
/// expression to read, as the book's Expressions chapter describes.
pub struct ExpressionRequirement;

/// The parameter holding the requirement.
const REQUIREMENT: &str = "requirement";
/// The parameter computing a failing object's graded deviation.
const DEVIATION: &str = "deviation";
/// The parameter holding a finding's message template.
const MESSAGE: &str = "message";

impl RuleCapability for ExpressionRequirement {
    fn id(&self) -> &'static str {
        "axioval:capability.expression"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required(REQUIREMENT, ParameterType::Expression),
            ParameterDescriptor::optional(DEVIATION, ParameterType::NumberExpression),
            ParameterDescriptor::optional(MESSAGE, ParameterType::String),
        ]
    }

    fn takes_authored_parameters(&self) -> bool {
        true
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some(ParameterValue::Expression { value: requirement }) =
            rule.parameters.get(REQUIREMENT)
        else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "expression: `requirement` is not an expression",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let arguments = crate::measured_arguments::Arguments::default();
        for object in selected {
            let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters))
                .with_arguments(&arguments);
            let result = evaluate(requirement, REQUIREMENT, &mut leaves);
            if let Some(traces) = context.services.get::<ExpressionTraces>() {
                traces.record(ObjectTrace {
                    rule: rule.id.to_string(),
                    object: object.id.clone(),
                    verdict: match &result.outcome {
                        Ok(Value::Boolean(true)) => "passed",
                        Ok(Value::Boolean(false) | Value::Null) => "failed",
                        _ => "notEvaluated",
                    },
                    trace: Explanation {
                        entries: result.trace.clone(),
                        truncated: false,
                    },
                });
            }
            match &result.outcome {
                Ok(Value::Boolean(true)) => {}
                Ok(Value::Boolean(false) | Value::Null) => {
                    let (deciding, failed) = failing(requirement, REQUIREMENT, &result);
                    let mut labelled = shown(&result);
                    let mut cited = evidence(&result);
                    let deviation = match rule.parameters.get(DEVIATION) {
                        Some(ParameterValue::Expression { value }) => {
                            let graded = evaluate(value, DEVIATION, &mut leaves);
                            cited.extend(evidence(&graded));
                            for (label, value) in shown(&graded) {
                                labelled.entry(label).or_insert(value);
                            }
                            graded_deviation(&graded)
                        }
                        _ => None,
                    };
                    let message = match rule.parameters.get(MESSAGE) {
                        Some(ParameterValue::String { value: template }) => {
                            render(template, &labelled)
                        }
                        _ if result.outcome == Ok(Value::Null) => format!(
                            "requirement cannot be confirmed: {failed} is null, as a value \
                             it needs is stated absent{}",
                            read_values(&result)
                        ),
                        _ => format!(
                            "requirement does not hold: {failed} is false{}",
                            read_values(&result)
                        ),
                    };
                    // What the measured values bound from the rule were
                    // measured against, as their providers cite it.
                    let mut related = leaves.take_related();
                    related.sort();
                    related.dedup();
                    let mut found = finding(rule, &object.id, message, cited, related);
                    found.explanation = Some(result.explain(&deciding));
                    evaluation.push_finding_deviating(found, deviation);
                }
                Ok(other) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidDeclaration,
                    format!(
                        "expression: the requirement is {}, not a truth",
                        other.kind()
                    ),
                ),
                Err(why) => {
                    let reason = leaves
                        .first_reason()
                        .filter(|_| matches!(why.reason, Reason::Unreadable(_)))
                        .unwrap_or_else(|| reason_of(why));
                    evaluation.push_object_not_evaluated_explained(
                        object.id.clone(),
                        reason,
                        format!("expression: {why}{}", read_values(&result)),
                        result.explain(&why.path),
                    );
                }
            }
        }
        evaluation
    }
}

/// The deviation a `deviation` expression grades a finding by: its plain
/// number interval, below zero read as zero. A value that is not a plain
/// number, or not evaluated, grades nothing: the rule's severity stands.
fn graded_deviation(evaluation: &Evaluation) -> Option<Deviation> {
    match &evaluation.outcome {
        Ok(Value::Number { value, unit }) if unit.is_plain() => {
            Deviation::try_new(value.lower.max(0.0), value.upper.max(0.0))
        }
        _ => None,
    }
}

/// `template` with each `{label}` replaced by the value of the labelled
/// subexpression, with its unit and interval; an unknown label stays.
fn render(template: &str, labelled: &BTreeMap<String, String>) -> String {
    let mut message = template.to_owned();
    for (label, value) in labelled {
        message = message.replace(&format!("{{{label}}}"), value);
    }
    message
}

/// Every labelled subexpression's value as the evaluation shows it (an
/// `inUnit` value in its unit), by label: the first, where a label repeats.
fn shown(evaluation: &Evaluation) -> BTreeMap<String, String> {
    let mut shown = BTreeMap::new();
    for step in &evaluation.trace {
        if let (Some(label), Some(value)) = (&step.label, &step.value) {
            shown.entry(label.clone()).or_insert_with(|| value.clone());
        }
    }
    shown
}

/// Why an outcome is not evaluated, in the report's terms.
pub(crate) fn reason_of(why: &NotEvaluated) -> NotEvaluatedReason {
    match &why.reason {
        Reason::Straddles { .. }
        | Reason::UndecidedCondition(_)
        | Reason::Unreadable(_)
        | Reason::UndecidedMembers(_) => NotEvaluatedReason::IncompleteEvidence,
        Reason::Mismatch(_) | Reason::Domain | Reason::ZeroDivisor | Reason::Overflow => {
            NotEvaluatedReason::InvalidEvidence
        }
        Reason::InvalidPattern(_) => NotEvaluatedReason::InvalidDeclaration,
        Reason::BudgetExhausted => NotEvaluatedReason::ResourceLimit,
    }
}

/// The subexpression a false or `null` requirement fails on, by its path,
/// and how a finding names it: its label, or its path and kind. Descends
/// through `and` (the first operand the evaluation found as untrue as the
/// requirement: false, else `null`) and through `implies` (its consequent,
/// or a `null` antecedent), reading the values the evaluation traced.
fn failing(expression: &Expression, path: &str, evaluation: &Evaluation) -> (String, String) {
    let value = |path: &str| {
        evaluation
            .trace
            .iter()
            .find(|step| step.path == path)
            .and_then(|step| step.value.clone())
    };
    let wanted = value(path).unwrap_or_else(|| "false".into());
    descend(expression, path, &wanted, &value)
}

fn descend(
    expression: &Expression,
    path: &str,
    wanted: &str,
    value: &dyn Fn(&str) -> Option<String>,
) -> (String, String) {
    let name = match expression.label() {
        Some(label) => format!("`{label}`"),
        None => format!("`{path}` ({})", expression.kind()),
    };
    let is = |path: &str, wanted: &str| value(path).as_deref() == Some(wanted);
    let here = || (path.to_owned(), name.clone());
    match expression {
        Expression::And { operands, .. } => operands
            .iter()
            .enumerate()
            .map(|(index, operand)| (operand, format!("{path}.and[{index}]")))
            .find(|(_, operand_path)| is(operand_path, wanted))
            .map_or_else(here, |(operand, operand_path)| {
                descend(operand, &operand_path, wanted, value)
            }),
        Expression::Implies {
            antecedent,
            consequent,
            ..
        } => {
            let consequent_path = format!("{path}.implies.consequent");
            let antecedent_path = format!("{path}.implies.antecedent");
            if is(&consequent_path, wanted) {
                descend(consequent, &consequent_path, wanted, value)
            } else if wanted == "null" && is(&antecedent_path, "null") {
                descend(antecedent, &antecedent_path, wanted, value)
            } else {
                here()
            }
        }
        _ => here(),
    }
}

/// `; read …` listing every value the evaluation read.
fn read_values(evaluation: &Evaluation) -> String {
    let mut values: Vec<String> = Vec::new();
    for read in &evaluation.reads {
        let source = match &read.source {
            axioval_engine::expression::Source::Property { set, name } => match set {
                Some(set) => format!("{set}.{name}"),
                None => name.clone(),
            },
            axioval_engine::expression::Source::Parameter(name)
            | axioval_engine::expression::Source::Derived(name) => name.clone(),
            axioval_engine::expression::Source::Lookup { table, column } => {
                format!("{table}.{column}")
            }
            // Its members' values are cited, not listed.
            axioval_engine::expression::Source::Aggregate(_) => continue,
            axioval_engine::expression::Source::Rule { rule, read } => format!(
                "{rule}.{}",
                match read {
                    axioval_engine::expression::RuleRead::Outcome => "outcome",
                    axioval_engine::expression::RuleRead::Selected => "selected",
                    axioval_engine::expression::RuleRead::FindingCount => "findingCount",
                    axioval_engine::expression::RuleRead::Deviation => "deviation",
                }
            ),
        };
        let value = match &read.leaf.value {
            Ok(value) => value.to_string(),
            Err(why) => format!("unreadable ({why})"),
        };
        let entry = format!("{source} = {value}");
        if !values.contains(&entry) {
            values.push(entry);
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!("; read {}", values.join(", "))
    }
}

fn evidence(evaluation: &Evaluation) -> Vec<axioval_ir::Evidence> {
    evaluation
        .reads
        .iter()
        .flat_map(|read| read.leaf.evidence.iter().cloned())
        .collect()
}
