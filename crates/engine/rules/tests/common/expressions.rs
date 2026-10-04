//! Expression rules rewriting a capability, and the parity harness between
//! the capability's evaluation and theirs on the same fixtures.
#![allow(clippy::needless_pass_by_value)]

use axioval_engine::{CapabilityEvaluation, CompiledRule};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use serde_json::{Value, json};

/// The expression capability's id.
pub const EXPRESSION: &str = "axioval:capability.expression";

/// The measured value `name` of the object in scope.
pub fn measured(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:measured", "property": name})
}

/// The property `set`/`name` of the object in scope.
pub fn stated(set: &str, name: &str) -> Value {
    json!({"kind": "property", "propertySet": set, "property": name})
}

/// The field `name` of the measured member in scope.
pub fn field(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:member", "property": name})
}

pub fn quantity(value: f64, unit: &str) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}})
}

/// A length in metres.
pub fn m(value: f64) -> Value {
    quantity(value, "m")
}

/// An area in square metres.
pub fn m2(value: f64) -> Value {
    quantity(value, "m2")
}

/// A plain number.
pub fn plain(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "number", "value": value}})
}

pub fn integer(value: i64) -> Value {
    json!({"kind": "literal", "value": {"type": "integer", "value": value}})
}

/// `operand` rounded to the micrometre, as capabilities allow a few units
/// in the last place of slack.
pub fn mm(operand: Value) -> Value {
    json!({"kind": "round", "operand": operand, "step": m(1e-6)})
}

pub fn compare(operator: &str, left: Value, right: Value) -> Value {
    json!({"kind": "compare", "operator": operator, "left": left, "right": right})
}

pub fn at_most(left: Value, right: Value) -> Value {
    compare("lessThanOrEquals", left, right)
}

pub fn at_least(left: Value, right: Value) -> Value {
    compare("greaterThanOrEquals", left, right)
}

pub fn below(left: Value, right: Value) -> Value {
    compare("lessThan", left, right)
}

pub fn above(left: Value, right: Value) -> Value {
    compare("greaterThan", left, right)
}

pub fn between(operand: Value, low: Value, high: Value) -> Value {
    json!({"kind": "between", "operand": operand, "low": low, "high": high})
}

pub fn and(operands: Vec<Value>) -> Value {
    json!({"kind": "and", "operands": operands})
}

pub fn or(operands: Vec<Value>) -> Value {
    json!({"kind": "or", "operands": operands})
}

pub fn not(operand: Value) -> Value {
    json!({"kind": "not", "operand": operand})
}

pub fn defined(operand: &Value) -> Value {
    json!({"kind": "isDefined", "operand": operand})
}

pub fn implies(antecedent: Value, consequent: Value) -> Value {
    json!({"kind": "implies", "antecedent": antecedent, "consequent": consequent})
}

/// Holds where `test` does, and where `value` is `null`.
pub fn unless_null(value: &Value, test: Value) -> Value {
    implies(defined(value), test)
}

pub fn subtract(left: Value, right: Value) -> Value {
    json!({"kind": "subtract", "left": left, "right": right})
}

pub fn divide(left: Value, right: Value) -> Value {
    json!({"kind": "divide", "left": left, "right": right})
}

pub fn abs(operand: Value) -> Value {
    json!({"kind": "abs", "operand": operand})
}

/// `aggregate` `function` over the objects `path` reaches, those `filter`
/// selects, of `value`.
pub fn over_path(function: &str, path: &[&str], filter: &Selector, value: Option<Value>) -> Value {
    let mut aggregate = json!({"kind": "aggregate", "function": function,
        "over": {"kind": "path", "path": path},
        "where": serde_json::to_value(filter).unwrap()});
    if let Some(value) = value {
        aggregate["value"] = value;
    }
    aggregate
}

/// `aggregate` `function` over the measured member list `list`.
pub fn over_members(function: &str, list: &str, value: Option<Value>) -> Value {
    let mut aggregate = json!({"kind": "aggregate", "function": function,
        "over": {"kind": "measured", "name": list}});
    if let Some(value) = value {
        aggregate["value"] = value;
    }
    aggregate
}

/// The expression rule requiring `requirement` of every object `selector`
/// selects.
pub fn rule(selector: Selector, requirement: &Value) -> CompiledRule {
    super::rule(
        EXPRESSION,
        selector,
        vec![(
            "requirement",
            ParameterValue::Expression {
                value: serde_json::from_value(requirement.clone()).unwrap(),
            },
        )],
    )
}

/// [`rule`] of the severity `severity`.
pub fn graded(selector: Selector, requirement: &Value, severity: Severity) -> CompiledRule {
    let mut rule = rule(selector, requirement);
    rule.severity = severity;
    rule
}

/// One evaluation holding every finding and open object of `evaluations`:
/// a capability rewritten as several expression rules, one per check or
/// severity band.
pub fn merged(evaluations: Vec<CapabilityEvaluation>) -> CapabilityEvaluation {
    let mut merged = CapabilityEvaluation::default();
    for evaluation in evaluations {
        for finding in evaluation.findings() {
            merged.push_finding(finding.clone());
        }
        for outcome in evaluation.not_evaluated_outcomes() {
            if let Some(object) = outcome.object_id() {
                merged.push_object_not_evaluated(
                    object.clone(),
                    outcome.reason().clone(),
                    outcome.message(),
                );
            }
        }
    }
    merged
}

/// Asserts that the expression rewrite judged every object as the
/// capability `id` did: verdict, severity, exact evidence and reason.
#[track_caller]
pub fn assert_parity(id: &str, capability: &CapabilityEvaluation, rewrite: &CapabilityEvaluation) {
    let parity =
        axioval_rules::parity::compare_evaluations((id, capability), ("expression", rewrite));
    assert!(parity.holds(), "{}", parity.diff());
}

/// The differences between the capability `id`'s evaluation and the
/// rewrite's, one line each, for a documented difference.
pub fn differences(
    id: &str,
    capability: &CapabilityEvaluation,
    rewrite: &CapabilityEvaluation,
) -> Vec<String> {
    axioval_rules::parity::compare_evaluations((id, capability), ("expression", rewrite))
        .differences
        .iter()
        .map(ToString::to_string)
        .collect()
}
