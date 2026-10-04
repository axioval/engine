//! The leaves of an expression evaluated for one object of a rule: its
//! properties, derived values, and the rule's parameters and tables.

use std::cell::RefCell;
use std::collections::BTreeMap;

use axioval_engine::RuleContext;
use axioval_engine::expression::{ExpressionContext, Leaf, Value, derived_value};
use axioval_ir::contract::{ParameterValue, ScalarValue, TableRow};
use axioval_ir::{NotEvaluatedReason, Object};

use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{PropertyRef, Resolved, resolve};

/// Answers an expression's leaves for one selected object.
pub(crate) struct ObjectLeaves<'a> {
    context: &'a RuleContext<'a>,
    object: &'a Object,
    /// The rule's parameters; a selector reads none.
    parameters: Option<&'a BTreeMap<String, ParameterValue>>,
    /// Why each unreadable leaf was unreadable, in reading order.
    reasons: RefCell<Vec<NotEvaluatedReason>>,
}

impl<'a> ObjectLeaves<'a> {
    /// Leaves of `object`, reading the rule's `parameters` where given.
    pub(crate) fn new(
        context: &'a RuleContext<'a>,
        object: &'a Object,
        parameters: Option<&'a BTreeMap<String, ParameterValue>>,
    ) -> Self {
        Self {
            context,
            object,
            parameters,
            reasons: RefCell::new(Vec::new()),
        }
    }

    /// Why the first unreadable leaf was unreadable.
    pub(crate) fn first_reason(&self) -> Option<NotEvaluatedReason> {
        self.reasons.borrow().first().cloned()
    }
}

impl ExpressionContext for ObjectLeaves<'_> {
    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        if set == Some(axioval_ir::VALUE_SET) {
            return self.derived(name);
        }
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

    fn derived(&mut self, name: &str) -> Leaf {
        let leaf = derived_value(self.context.services, &self.object.id, name);
        if leaf.value.is_err() {
            self.reasons
                .borrow_mut()
                .push(NotEvaluatedReason::IncompleteEvidence);
        }
        leaf
    }

    fn parameter(&mut self, name: &str) -> Leaf {
        let Some(parameters) = self.parameters else {
            return Leaf::unreadable(format!("a selector reads no rule parameter, not `{name}`"));
        };
        match parameters.get(name).cloned().map(ScalarValue::try_from) {
            Some(Ok(scalar)) => match Value::from_literal(&scalar) {
                Ok(value) => Leaf::stated(value),
                Err(why) => Leaf::unreadable(why),
            },
            Some(Err(_)) => Leaf::unreadable(format!("parameter `{name}` is no single value")),
            None => Leaf::unreadable(format!("the rule has no parameter `{name}`")),
        }
    }

    fn lookup(&mut self, table: &str, keys: &BTreeMap<String, Value>, column: &str) -> Leaf {
        let Some(ParameterValue::Table { value: rows }) =
            self.parameters.and_then(|parameters| parameters.get(table))
        else {
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
