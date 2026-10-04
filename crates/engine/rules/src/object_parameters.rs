//! Parameters computed per checked object: a rule may give a parameter its
//! capability declares `per_object` as an expression, or a table such
//! parameter holds with expression cells.
//!
//! The capability itself reads literals only. Each selected object's
//! computed parameters are evaluated to literals, objects whose literals
//! agree form one group, and the capability runs once per group over
//! exactly its objects. An object whose parameters cannot be computed is
//! not evaluated, with the reason and the subexpression's path.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::expression::{Reason, Value, evaluate};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::{ColumnKind, Expression, ParameterValue, Selector, TableRow};
use axioval_ir::{NotEvaluatedReason, Object, ObjectId};

use crate::expression_leaves::ObjectLeaves;
use crate::selection::select_objects;

/// Whether the rule gives any parameter, or any table cell, as an
/// expression.
pub(crate) fn has_object_parameters(rule: &CompiledRule) -> bool {
    rule.parameters.values().any(|value| match value {
        ParameterValue::Expression { .. } => true,
        ParameterValue::Table { value: rows } => rows
            .iter()
            .flat_map(TableRow::values)
            .any(|cell| matches!(cell, ParameterValue::Expression { .. })),
        _ => false,
    })
}

/// Evaluates `capability` for `rule` group by group of objects whose
/// computed parameters agree. Only called when
/// [`has_object_parameters`] holds.
pub(crate) fn per_object(
    capability: &dyn RuleCapability,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let descriptors = capability.parameters();
    let (selected, mut evaluation) = select_objects(context, &rule.selector);
    let mut groups: Vec<(BTreeMap<String, ParameterValue>, BTreeSet<ObjectId>)> = Vec::new();
    for object in selected {
        match literals(context, object, rule, &descriptors) {
            Ok(parameters) => match groups.iter_mut().find(|(group, _)| *group == parameters) {
                Some((_, objects)) => {
                    objects.insert(object.id.clone());
                }
                None => groups.push((parameters, BTreeSet::from([object.id.clone()]))),
            },
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
    for (parameters, objects) in groups {
        let narrowed = CompiledRule {
            parameters,
            selector: Selector::AllOf {
                operands: vec![rule.selector.clone(), Selector::Objects { objects }],
            },
            ..rule.clone()
        };
        evaluation.absorb(capability.evaluate(context, &narrowed));
    }
    evaluation
}

type Failure = (NotEvaluatedReason, String);

/// The rule's parameters with every expression evaluated for `object`.
fn literals(
    context: &RuleContext<'_>,
    object: &Object,
    rule: &CompiledRule,
    descriptors: &[ParameterDescriptor],
) -> Result<BTreeMap<String, ParameterValue>, Failure> {
    let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
    let mut computed = BTreeMap::new();
    for (name, value) in &rule.parameters {
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.name == *name);
        match value {
            ParameterValue::Expression { value: expression }
                if descriptor.is_some_and(|descriptor| descriptor.per_object) =>
            {
                let descriptor = descriptor.expect("checked above");
                match literal(&mut leaves, expression, name, descriptor.parameter_type)? {
                    Some(literal) => {
                        computed.insert(name.clone(), literal);
                    }
                    None if descriptor.required => {
                        return Err((
                            NotEvaluatedReason::IncompleteEvidence,
                            format!("required parameter `{name}` is computed as null"),
                        ));
                    }
                    // A computed null is a parameter not given.
                    None => {}
                }
            }
            ParameterValue::Table { value: rows } => {
                let columns = match descriptor.map(|descriptor| descriptor.parameter_type) {
                    Some(ParameterType::Table(columns)) => columns,
                    _ => &[],
                };
                let mut table = Vec::with_capacity(rows.len());
                for (index, row) in rows.iter().enumerate() {
                    let mut cells = TableRow::new();
                    for (column, cell) in row {
                        let ParameterValue::Expression { value: expression } = cell else {
                            cells.insert(column.clone(), cell.clone());
                            continue;
                        };
                        let kind = columns
                            .iter()
                            .find(|declared| declared.id == column)
                            .map_or(ParameterType::String, |declared| column_type(declared.kind));
                        let path = format!("{name}[{index}].{column}");
                        if let Some(literal) = literal(&mut leaves, expression, &path, kind)? {
                            cells.insert(column.clone(), literal);
                        }
                    }
                    table.push(cells);
                }
                computed.insert(name.clone(), ParameterValue::Table { value: table });
            }
            other => {
                computed.insert(name.clone(), other.clone());
            }
        }
    }
    Ok(computed)
}

/// The parameter type a cell of a column of `kind` reads as.
fn column_type(kind: ColumnKind) -> ParameterType {
    match kind {
        ColumnKind::Integer => ParameterType::Integer,
        ColumnKind::Number => ParameterType::Number,
        ColumnKind::Quantity => ParameterType::Quantity,
        ColumnKind::Boolean => ParameterType::Boolean,
        ColumnKind::Date => ParameterType::Date,
        ColumnKind::DateTime => ParameterType::DateTime,
        _ => ParameterType::String,
    }
}

/// `expression`'s value for the object as a literal of `kind`; `None` for
/// `null`.
fn literal(
    leaves: &mut ObjectLeaves<'_>,
    expression: &Expression,
    path: &str,
    kind: ParameterType,
) -> Result<Option<ParameterValue>, Failure> {
    let evaluation = evaluate(expression, path, leaves);
    let value = match evaluation.outcome {
        Ok(value) => value,
        Err(why) => {
            let reason = leaves
                .first_reason()
                .filter(|_| matches!(why.reason, Reason::Unreadable(_)))
                .unwrap_or_else(|| crate::expression_requirement::reason_of(&why));
            return Err((reason, format!("a computed parameter: {why}")));
        }
    };
    let mismatch = |value: &Value| {
        (
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "computed parameter `{path}` is {value}, not a {} value",
                kind.package_kind()
            ),
        )
    };
    let interval = |value: &Value| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("computed parameter `{path}` is {value}, an interval rather than one value"),
        )
    };
    Ok(Some(match (&value, kind) {
        (Value::Null, _) => return Ok(None),
        (Value::Boolean(value), ParameterType::Boolean) => {
            ParameterValue::Boolean { value: *value }
        }
        (
            Value::Number {
                value: number,
                unit,
            },
            ParameterType::Integer,
        ) if unit.is_plain() => {
            if !number.is_point() {
                return Err(interval(&value));
            }
            #[allow(clippy::cast_possible_truncation)]
            let integer = number.lower as i64;
            #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
            if integer as f64 != number.lower {
                return Err(mismatch(&value));
            }
            ParameterValue::Integer { value: integer }
        }
        (
            Value::Number {
                value: number,
                unit,
            },
            ParameterType::Number,
        ) if unit.is_plain() => {
            if !number.is_point() {
                return Err(interval(&value));
            }
            ParameterValue::Number {
                value: number.lower,
            }
        }
        (
            Value::Number {
                value: number,
                unit,
            },
            ParameterType::Quantity,
        ) if !unit.is_plain() => {
            if !number.is_point() {
                return Err(interval(&value));
            }
            ParameterValue::Quantity {
                value: number.lower,
                unit: unit.to_string(),
            }
        }
        (Value::Text(text) | Value::Enum(text), ParameterType::String) => ParameterValue::String {
            value: text.clone(),
        },
        (Value::Text(text) | Value::Enum(text), ParameterType::Enum) => ParameterValue::Enum {
            value: text.clone(),
        },
        (Value::Date(date), ParameterType::Date) => ParameterValue::Date { value: *date },
        (Value::DateTime(time), ParameterType::DateTime) => {
            ParameterValue::DateTime { value: *time }
        }
        _ => return Err(mismatch(&value)),
    }))
}
