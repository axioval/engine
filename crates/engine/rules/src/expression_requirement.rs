//! A requirement stated as an expression over each selected object.

use std::cell::RefCell;
use std::collections::BTreeMap;

use axioval_engine::expression::{
    Evaluation, ExpressionContext, Leaf, NotEvaluated, Reason, Value, evaluate,
};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::{Expression, ParameterValue, ScalarValue, TableRow};
use axioval_ir::{NotEvaluatedReason, Object};

use crate::selection::select_objects;
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{PropertyRef, Resolved, finding, resolve};

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

impl RuleCapability for ExpressionRequirement {
    fn id(&self) -> &'static str {
        "axioval:capability.expression"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            REQUIREMENT,
            ParameterType::Expression,
        )]
    }

    fn takes_authored_parameters(&self) -> bool {
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
            let mut leaves = ObjectLeaves {
                context,
                object,
                rule,
                reasons: RefCell::new(Vec::new()),
            };
            let result = evaluate(requirement, REQUIREMENT, &mut leaves);
            match &result.outcome {
                Ok(Value::Boolean(true)) => {}
                Ok(Value::Boolean(false) | Value::Null) => {
                    let failed = failing(requirement, REQUIREMENT, &mut leaves);
                    evaluation.push_finding(finding(
                        rule,
                        &object.id,
                        format!(
                            "requirement does not hold: {failed} is false{}",
                            read_values(&result)
                        ),
                        evidence(&result),
                        vec![],
                    ));
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
                        .reasons
                        .borrow()
                        .first()
                        .cloned()
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

/// Why an outcome is not evaluated, in the report's terms.
fn reason_of(why: &NotEvaluated) -> NotEvaluatedReason {
    match &why.reason {
        Reason::Straddles { .. } | Reason::UndecidedCondition(_) | Reason::Unreadable(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
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

/// Answers an expression's leaves for one selected object.
struct ObjectLeaves<'a> {
    context: &'a RuleContext<'a>,
    object: &'a Object,
    rule: &'a CompiledRule,
    /// Why each unreadable leaf was unreadable, in reading order.
    reasons: RefCell<Vec<NotEvaluatedReason>>,
}

impl ExpressionContext for ObjectLeaves<'_> {
    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        match resolve(self.context, self.object, PropertyRef { set, name }) {
            Ok(resolved) => {
                let evidence = resolved.evidence();
                let value = match &resolved {
                    // A stated absence is `null`, never a value not read.
                    Resolved::Absent(_) => Ok(Value::Null),
                    Resolved::Present(property) => Value::from_property(&property.value),
                };
                if value.is_err() {
                    self.reasons
                        .borrow_mut()
                        .push(NotEvaluatedReason::InvalidEvidence);
                }
                Leaf { value, evidence }
            }
            Err((reason, message)) => {
                self.reasons.borrow_mut().push(reason);
                Leaf::unreadable(message)
            }
        }
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        match self
            .rule
            .parameters
            .get(name)
            .cloned()
            .map(ScalarValue::try_from)
        {
            Some(Ok(scalar)) => match Value::from_literal(&scalar) {
                Ok(value) => Leaf::stated(value),
                Err(why) => Leaf::unreadable(why),
            },
            Some(Err(_)) => Leaf::unreadable(format!("parameter `{name}` is no single value")),
            None => Leaf::unreadable(format!("the rule has no parameter `{name}`")),
        }
    }

    fn lookup(&mut self, table: &str, keys: &BTreeMap<String, Value>, column: &str) -> Leaf {
        let Some(ParameterValue::Table { value: rows }) = self.rule.parameters.get(table) else {
            return Leaf::unreadable(format!("the rule has no table `{table}`"));
        };
        lookup(rows, keys, column).map_or_else(Leaf::unreadable, Leaf::stated)
    }
}

/// The `column` cell of the most specific row whose key cells match `keys`:
/// a text cell is a wildcard pattern matched against the key's text, any
/// other cell must equal the key, and a blank cell accepts any key. No
/// matching row, or one leaving `column` blank, is `null`; tied or undecided
/// rows have no value.
fn lookup(
    rows: &[TableRow],
    keys: &BTreeMap<String, Value>,
    column: &str,
) -> Result<Value, String> {
    let mut problem = None;
    let matched = match_rows(rows, RowSelection::MostSpecific, |row| {
        let mut verdict = RowTest::Match(0);
        for (key, value) in keys {
            let Some(cell) = row.get(key) else {
                continue;
            };
            let outcome = match (cell, value) {
                (_, Value::Null) => RowTest::NoMatch,
                (ParameterValue::String { value: pattern }, value) => match key_text(value) {
                    Some(text) => match TextPattern::new(pattern, true) {
                        Ok(pattern) => pattern.test(&text),
                        Err(why) => {
                            problem.get_or_insert(why);
                            RowTest::Undecided
                        }
                    },
                    None => RowTest::NoMatch,
                },
                (cell, value) => match ScalarValue::try_from(cell.clone())
                    .ok()
                    .and_then(|cell| Value::from_literal(&cell).ok())
                {
                    Some(cell) => equal(&cell, value),
                    None => RowTest::NoMatch,
                },
            };
            verdict = verdict.and(outcome);
        }
        verdict
    });
    match matched {
        Matched::Rows(found) => Ok(match found.first() {
            None => Value::Null,
            Some((_, row)) => match row.get(column) {
                None => Value::Null,
                Some(cell) => {
                    let scalar = ScalarValue::try_from(cell.clone())
                        .map_err(|_| format!("column `{column}` holds no single value"))?;
                    Value::from_literal(&scalar)?
                }
            },
        }),
        Matched::Undecided => Err(problem.unwrap_or_else(|| "a row cannot be decided".into())),
        Matched::Ambiguous(tied) => Err(format!(
            "rows {} tie for the most specific",
            tied.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A key read as text, as `keyed-limit` reads keys.
fn key_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) | Value::Enum(text) => Some(text.clone()),
        Value::Boolean(value) => Some(value.to_string()),
        Value::Number { value, unit } if unit.is_plain() && value.is_point() => {
            Some(value.lower.to_string())
        }
        _ => None,
    }
}

fn equal(cell: &Value, value: &Value) -> RowTest {
    match (cell, value) {
        (
            Value::Number {
                value: cell,
                unit: cell_unit,
            },
            Value::Number { value, unit },
        ) if cell_unit == unit => {
            if cell.upper < value.lower || cell.lower > value.upper {
                RowTest::NoMatch
            } else if cell.is_point() && value.is_point() {
                RowTest::Match(1)
            } else {
                RowTest::Undecided
            }
        }
        (cell, value) if cell == value => RowTest::Match(1),
        _ => RowTest::NoMatch,
    }
}
