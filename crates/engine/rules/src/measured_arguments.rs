//! Binding a measured value's references to the rule reading it: each
//! `@name` to the rule's parameter of that name, each `@anchor` to the
//! object the rule checks ([`axioval_ir::measured`]).
//!
//! A selector parameter binds to the objects it picks, surely or not, read
//! once per rule through the run's one selection ([`select_objects`]) and
//! sorted by source-qualified identity; a length, path, text, property
//! reference or table binds to the value the parameter states, checked
//! against the measured parameter's kind. A reference that cannot be bound (a parameter the rule does not
//! state, of another kind or not realisable, or a selection whose objects
//! cannot all be listed) leaves the value not evaluated, never a default.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{NotEvaluatedReason, RuleContext};
use axioval_ir::ObjectId;
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::measured::{
    MeasuredArgument, MeasuredCall, MeasuredParameterKind, MeasuredSelection,
};

use crate::selection::select_objects;
use crate::support::{Unavailable, invalid, si_quantity};

/// What one rule's measured values bind, read once per rule: the objects
/// each selector parameter picks.
#[derive(Default)]
pub(crate) struct Arguments {
    selections: RefCell<BTreeMap<String, Result<MeasuredSelection, Unavailable>>>,
}

/// The objects `selector`, the rule's parameter `parameter`, picks: those
/// surely picked and those it cannot decide, each sorted. A source whose
/// objects cannot all be listed leaves the whole selection unknown.
fn selection(
    context: &RuleContext<'_>,
    parameter: &str,
    selector: &Selector,
) -> Result<MeasuredSelection, Unavailable> {
    let (matched, outcomes) = select_objects(context, selector);
    let mut undecided = BTreeSet::new();
    for outcome in outcomes.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(object) => {
                undecided.insert(object.clone());
            }
            None => {
                return Err((
                    outcome.reason().clone(),
                    format!(
                        "the objects `@{parameter}` selects cannot all be listed: {}",
                        outcome.message()
                    ),
                ));
            }
        }
    }
    Ok(MeasuredSelection {
        parameter: parameter.to_owned(),
        matched: matched
            .into_iter()
            .map(|object| object.id.clone())
            .collect(),
        undecided,
    })
}

impl Arguments {
    /// The objects the selector parameter `parameter` picks, read once.
    fn selection(
        &self,
        context: &RuleContext<'_>,
        parameter: &str,
        selector: &Selector,
    ) -> Result<MeasuredSelection, Unavailable> {
        if let Some(read) = self.selections.borrow().get(parameter) {
            return read.clone();
        }
        let read = selection(context, parameter, selector);
        self.selections
            .borrow_mut()
            .insert(parameter.to_owned(), read.clone());
        read
    }
}

/// The value the rule's parameter `parameter` binds for a measured
/// parameter of `kind`.
fn bound(
    context: &RuleContext<'_>,
    arguments: Option<&Arguments>,
    kind: MeasuredParameterKind,
    parameter: &str,
    value: &ParameterValue,
) -> Result<MeasuredArgument, Unavailable> {
    let reference = format!("`@{parameter}`");
    let not = |what: &str| invalid(format!("{reference} is not {what}"));
    Ok(match (kind, value) {
        (MeasuredParameterKind::Objects, ParameterValue::Selector { value: selector }) => {
            MeasuredArgument::Objects(match arguments {
                Some(arguments) => arguments.selection(context, parameter, selector)?,
                None => selection(context, parameter, selector)?,
            })
        }
        (MeasuredParameterKind::Objects, _) => return Err(not("a selector")),
        (MeasuredParameterKind::Length { minimum }, value) => {
            #[allow(clippy::cast_precision_loss)]
            let metres = match value {
                ParameterValue::Number { value } => *value,
                ParameterValue::Integer { value } => *value as f64,
                ParameterValue::Quantity { value, unit } => match si_quantity(*value, unit) {
                    Ok((metres, QuantityDimension::Length)) => metres,
                    _ => return Err(not("a length")),
                },
                _ => return Err(not("a number of metres or a length")),
            };
            if !(metres.is_finite() && metres >= minimum) {
                return Err(invalid(format!(
                    "{reference} is {metres} m, not a length of at least {minimum} m"
                )));
            }
            MeasuredArgument::Length(metres)
        }
        (MeasuredParameterKind::Path, ParameterValue::StringList { value: steps }) => {
            if steps.is_empty() || steps.iter().any(|step| step.trim().is_empty()) {
                return Err(invalid(format!("{reference} holds an empty step")));
            }
            MeasuredArgument::Path(steps.iter().map(|step| step.trim().to_owned()).collect())
        }
        (MeasuredParameterKind::Path, _) => return Err(not("a string list")),
        (
            MeasuredParameterKind::Choice { .. }
            | MeasuredParameterKind::Text
            | MeasuredParameterKind::SourceKind,
            ParameterValue::String { value: text },
        ) => match kind {
            MeasuredParameterKind::Choice { options } => MeasuredArgument::Choice(
                options
                    .iter()
                    .find(|option| option.eq_ignore_ascii_case(text.trim()))
                    .ok_or_else(|| {
                        invalid(format!(
                            "{reference} `{text}` is none of {}",
                            options.join(", ")
                        ))
                    })?,
            ),
            _ if text.trim().is_empty() => return Err(invalid(format!("{reference} is empty"))),
            MeasuredParameterKind::Text => MeasuredArgument::Text(text.trim().to_owned()),
            _ => MeasuredArgument::SourceKind(text.trim().to_owned()),
        },
        (
            MeasuredParameterKind::Choice { .. }
            | MeasuredParameterKind::Text
            | MeasuredParameterKind::SourceKind,
            _,
        ) => return Err(not("a string")),
        (
            MeasuredParameterKind::Property,
            ParameterValue::PropertyReference {
                property_set,
                property,
            },
        ) => MeasuredArgument::Property {
            set: property_set.clone(),
            name: property.clone(),
        },
        (MeasuredParameterKind::Property, _) => return Err(not("a property reference")),
        (MeasuredParameterKind::Table, ParameterValue::Table { value: rows }) => {
            MeasuredArgument::Table(rows.clone())
        }
        (MeasuredParameterKind::Table, _) => return Err(not("a table")),
        (MeasuredParameterKind::Vector | MeasuredParameterKind::Polygon, _) => {
            return Err(invalid(format!("{reference} names no value of a rule")));
        }
    })
}

/// Binds every reference of `call`: `@anchor` to `anchor`, `@name` to the
/// rule's parameter `name` in `parameters` (none outside a rule).
///
/// # Errors
///
/// A reference that cannot be bound, for its reason: an invalid
/// declaration for a parameter the rule does not state or of another kind,
/// incomplete evidence for a selection that cannot be listed.
pub(crate) fn bind(
    context: &RuleContext<'_>,
    parameters: Option<&BTreeMap<String, ParameterValue>>,
    arguments: Option<&Arguments>,
    anchor: &ObjectId,
    call: &mut MeasuredCall,
) -> Result<(), Unavailable> {
    let references: Vec<(&'static str, MeasuredArgument)> = call
        .references()
        .map(|(key, argument)| (key, argument.clone()))
        .collect();
    for (key, reference) in references {
        let argument = match &reference {
            MeasuredArgument::Anchor => {
                MeasuredArgument::Objects(MeasuredSelection::anchor(anchor.clone()))
            }
            MeasuredArgument::Parameter(parameter) => {
                let Some(parameters) = parameters else {
                    return Err(invalid(format!(
                        "`@{parameter}` names a rule parameter, which a selector never reads"
                    )));
                };
                let value = parameters.get(parameter).ok_or_else(|| {
                    invalid(format!("the rule states no parameter `{parameter}`"))
                })?;
                let kind = call
                    .parameter(key)
                    .map(|declared| declared.kind)
                    .ok_or_else(|| invalid(format!("`{key}` is no parameter")))?;
                bound(context, arguments, kind, parameter, value)?
            }
            _ => continue,
        };
        call.bind(key, argument)
            .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use axioval_engine::{RuleContext, ServiceRegistry};
    use axioval_ir::contract::{ParameterValue, TableRow};
    use axioval_ir::measured::{MeasuredArgument, MeasuredParameterKind};
    use axioval_ir::{NotEvaluatedReason, Project};

    use super::bound;

    /// A property reference and a table bind as the rule states them, and
    /// a parameter of another kind is the rule's invalid declaration.
    #[test]
    fn a_property_or_a_table_binds_as_stated() {
        let project = Project::new(Vec::new()).unwrap();
        let services = ServiceRegistry::new();
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let property = ParameterValue::PropertyReference {
            property: "Width".into(),
            property_set: Some("Attributes".into()),
        };
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Property,
                "overall",
                &property
            ),
            Ok(MeasuredArgument::Property {
                set: Some("Attributes".into()),
                name: "Width".into()
            })
        );
        let rows = vec![TableRow::from([(
            "width".to_owned(),
            ParameterValue::Number { value: 1.0 },
        )])];
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Table,
                "rows",
                &ParameterValue::Table {
                    value: rows.clone()
                }
            ),
            Ok(MeasuredArgument::Table(rows))
        );
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Table,
                "overall",
                &property
            ),
            Err((
                NotEvaluatedReason::InvalidDeclaration,
                "`@overall` is not a table".into()
            ))
        );
    }
}
