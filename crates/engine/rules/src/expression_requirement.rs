//! A requirement stated as an expression over each selected object.

use axioval_engine::expression::{Evaluation, NotEvaluated, Reason, Value, evaluate};
use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{Expression, ParameterValue};

use crate::expression_leaves::ObjectLeaves;
use crate::selection::select_objects;
use crate::support::finding;

/// Requires the expression `requirement` to hold for each selected object.
///
/// True passes, false is a finding naming the subexpression that failed and
/// every value read, and not evaluated leaves the object not evaluated with
/// the reason, naming the subexpression. A rule definition may declare
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
        for object in selected {
            let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
            let result = evaluate(requirement, REQUIREMENT, &mut leaves);
            match &result.outcome {
                Ok(Value::Boolean(true)) => {}
                Ok(Value::Boolean(false) | Value::Null) => {
                    let failed = failing(requirement, REQUIREMENT, &mut leaves);
                    let mut labelled = result.labelled.clone();
                    let mut cited = evidence(&result);
                    let deviation = match rule.parameters.get(DEVIATION) {
                        Some(ParameterValue::Expression { value }) => {
                            let graded = evaluate(value, DEVIATION, &mut leaves);
                            cited.extend(evidence(&graded));
                            for (label, value) in &graded.labelled {
                                labelled
                                    .entry(label.clone())
                                    .or_insert_with(|| value.clone());
                            }
                            graded_deviation(&graded)
                        }
                        _ => None,
                    };
                    let message = match rule.parameters.get(MESSAGE) {
                        Some(ParameterValue::String { value: template }) => {
                            render(template, &labelled)
                        }
                        _ => format!(
                            "requirement does not hold: {failed} is false{}",
                            read_values(&result)
                        ),
                    };
                    evaluation.push_finding_deviating(
                        finding(rule, &object.id, message, cited, vec![]),
                        deviation,
                    );
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
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        format!("expression: {why}{}", read_values(&result)),
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
fn render(template: &str, labelled: &BTreeMap<String, Value>) -> String {
    let mut message = template.to_owned();
    for (label, value) in labelled {
        message = message.replace(&format!("{{{label}}}"), &value.to_string());
    }
    message
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
    }
}

/// The subexpression a false requirement fails on: its label, or its path
/// and kind. Descends through `and` (the first false operand), `not` of an
/// `or`, and the consequent of an `implies` that holds its antecedent.
fn failing(expression: &Expression, path: &str, leaves: &mut ObjectLeaves<'_>) -> String {
    let name = || match expression.label() {
        Some(label) => format!("`{label}`"),
        None => format!("`{path}` ({})", expression.kind()),
    };
    match expression {
        Expression::And { operands, .. } => {
            for (index, operand) in operands.iter().enumerate() {
                let operand_path = format!("{path}.and[{index}]");
                let outcome = evaluate(operand, &operand_path, leaves).outcome;
                if matches!(outcome, Ok(Value::Boolean(false) | Value::Null)) {
                    return failing(operand, &operand_path, leaves);
                }
            }
            name()
        }
        Expression::Implies { consequent, .. } => {
            let consequent_path = format!("{path}.implies.consequent");
            failing(consequent, &consequent_path, leaves)
        }
        _ => name(),
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
