//! Binding a measured value's references to the rule reading it: each
//! `@name` to the rule's parameter of that name, each `@anchor` to the
//! object the rule checks ([`axioval_ir::measured`]).
//!
//! A selector parameter binds to the objects it picks, surely or not, read
//! once per rule through the run's one selection ([`select_shared`], shared between its rules) and
//! sorted by source-qualified identity; a length, path, text, property
//! reference or table binds to the value the parameter states, checked
//! against the measured parameter's kind. A reference that cannot be bound (a parameter the rule does not
//! state, of another kind or not realisable, or a selection whose objects
//! cannot all be listed) leaves the value not evaluated, never a default.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{NotEvaluatedReason, RuleContext};
use axioval_ir::ObjectId;
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::measured::{
    MeasuredArgument, MeasuredCall, MeasuredParameterKind, MeasuredSelection,
};

use crate::selection::select_shared;
use crate::support::{Unavailable, invalid, si_quantity};

/// What one rule's measured values bind, read once per rule: the objects
/// each selector parameter picks, and each measured name naming no anchor,
/// parsed and bound (it binds alike for every object).
#[derive(Default)]
pub(crate) struct Arguments {
    /// The rule's own selector, which `@selection` names in a template.
    selector: Option<Selector>,
    selections: RefCell<BTreeMap<String, Result<Arc<MeasuredSelection>, Unavailable>>>,
    /// Each name read through [`Arguments::call`]: `None` where it is not
    /// prepared once for the rule (it names the anchor, binds nothing or
    /// does not parse), kept so it is parsed once.
    calls: RefCell<BTreeMap<String, Prepared>>,
    /// Each name naming a reference read through [`Arguments::anchored`],
    /// as a value and as a list: the rule's parameters bound, the anchor
    /// left for each object.
    anchored: RefCell<BTreeMap<(bool, String), Anchored>>,
    /// Member lists as a template writes them, without the arguments
    /// naming parameters the rule leaves unstated.
    written: RefCell<BTreeMap<&'static str, std::sync::Arc<str>>>,
}

/// A name prepared once for the rule; `None` where it is not.
type Prepared = Option<Result<axioval_engine::PreparedRead, Unavailable>>;

/// A name parsed with the rule's parameters bound, the anchor left; `None`
/// where it does not parse or names no reference.
type Anchored = Option<Result<MeasuredCall, Unavailable>>;

/// The objects `selector`, the rule's parameter `parameter`, picks: those
/// surely picked and those it cannot decide, each sorted. A source whose
/// objects cannot all be listed leaves the whole selection unknown.
fn selection(
    context: &RuleContext<'_>,
    parameter: &str,
    selector: &Selector,
) -> Result<MeasuredSelection, Unavailable> {
    let (matched, outcomes) = select_shared(context, selector);
    let mut undecided = BTreeSet::new();
    let mut first_undecided = None;
    let mut reasons = BTreeMap::new();
    for outcome in outcomes.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(object) => {
                undecided.insert(object.clone());
                first_undecided.get_or_insert_with(|| {
                    (outcome.reason().clone(), outcome.message().to_owned())
                });
                reasons.insert(object.clone(), outcome.message().to_owned());
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
        first_undecided,
        reasons,
    })
}

/// The objects `selector`, the rule's parameter `parameter`, picks, read
/// outside a rule's run.
pub(crate) fn selection_of(
    context: &RuleContext<'_>,
    parameter: &str,
    selector: &Selector,
) -> Result<MeasuredSelection, Unavailable> {
    selection(context, parameter, selector)
}

impl Arguments {
    /// What `rule`'s measured values bind: `@selection` the objects the
    /// rule itself selects, where it states no parameter of that name.
    pub(crate) fn of_rule(rule: &axioval_engine::CompiledRule) -> Self {
        Self {
            selector: Some(rule.selector.clone()),
            ..Self::default()
        }
    }

    /// The same, `@selection` the objects the rule selected already and
    /// those whose selection `outcomes` leave undecided.
    pub(crate) fn selected(
        self,
        selected: &[&axioval_ir::Object],
        outcomes: &axioval_engine::CapabilityEvaluation,
    ) -> Self {
        let mut undecided = BTreeSet::new();
        let mut first_undecided = None;
        let mut reasons = BTreeMap::new();
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => {
                    undecided.insert(object.clone());
                    first_undecided.get_or_insert_with(|| {
                        (outcome.reason().clone(), outcome.message().to_owned())
                    });
                    reasons.insert(object.clone(), outcome.message().to_owned());
                }
                // The selection cannot be listed whole: read it as bound.
                None => return self,
            }
        }
        self.selections.borrow_mut().insert(
            axioval_engine::template::SELECTION.to_owned(),
            Ok(Arc::new(MeasuredSelection {
                parameter: axioval_engine::template::SELECTION.to_owned(),
                matched: selected.iter().map(|object| object.id.clone()).collect(),
                undecided,
                first_undecided,
                reasons,
            })),
        );
        self
    }

    /// The measured name `name` parsed, bound and prepared for the rule,
    /// once per rule: `None` where it does not parse, binds nothing or
    /// names the anchor, which binds per object.
    pub(crate) fn call(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
    ) -> Option<Result<axioval_engine::PreparedRead, Unavailable>> {
        if let Some(bound) = self.calls.borrow().get(name) {
            return bound.clone();
        }
        let bound = Self::prepared(context, parameters, self, name);
        self.calls
            .borrow_mut()
            .insert(name.to_owned(), bound.clone());
        bound
    }

    fn prepared(
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        arguments: &Self,
        name: &str,
    ) -> Option<Result<axioval_engine::PreparedRead, Unavailable>> {
        let mut call = axioval_ir::measured::parse(name).ok()?;
        if call.is_bound()
            || call
                .references()
                .any(|(_, argument)| *argument == MeasuredArgument::Anchor)
        {
            return None;
        }
        // No anchor named: any object binds it alike.
        let anchor = context.project.objects().next()?.id.clone();
        Some(
            bind(context, parameters, Some(arguments), &anchor, &mut call)
                .map(|()| axioval_engine::PreparedRead::of(name, &call)),
        )
    }

    /// The measured name `name` (a member list where `list`) parsed, the
    /// rule's parameters it names bound, once per rule; each object binds
    /// the anchor it names ([`bind`]) alike. `None` where it does not
    /// parse or names no reference; a parameter that does not bind is
    /// refused as [`bind`] refuses it.
    pub(crate) fn anchored(
        &self,
        context: &RuleContext<'_>,
        parameters: Option<&BTreeMap<String, ParameterValue>>,
        name: &str,
        list: bool,
    ) -> Option<Result<MeasuredCall, Unavailable>> {
        let key = (list, name.to_owned());
        if let Some(call) = self.anchored.borrow().get(&key) {
            return call.clone();
        }
        let parsed = if list {
            axioval_ir::measured::parse_members(name)
        } else {
            axioval_ir::measured::parse(name)
        };
        let call = parsed.ok().filter(|call| !call.is_bound()).map(|mut call| {
            bind_references(context, parameters, Some(self), None, &mut call).map(|()| call)
        });
        self.anchored.borrow_mut().insert(key, call.clone());
        call
    }

    /// The list a template writes as `list`, as `write` writes it for the
    /// rule, once per rule.
    pub(crate) fn written(
        &self,
        list: &'static str,
        write: impl FnOnce() -> String,
    ) -> std::sync::Arc<str> {
        if let Some(written) = self.written.borrow().get(list) {
            return written.clone();
        }
        let written: std::sync::Arc<str> = std::sync::Arc::from(write());
        self.written.borrow_mut().insert(list, written.clone());
        written
    }

    /// The objects the selector parameter `parameter` picks, read once.
    pub(crate) fn selection_of(
        &self,
        context: &RuleContext<'_>,
        parameter: &str,
        selector: &Selector,
    ) -> Result<MeasuredSelection, Unavailable> {
        self.selection(context, parameter, selector)
            .map(|selection| selection.as_ref().clone())
    }

    /// The objects the selector parameter `parameter` picks, read once.
    fn selection(
        &self,
        context: &RuleContext<'_>,
        parameter: &str,
        selector: &Selector,
    ) -> Result<Arc<MeasuredSelection>, Unavailable> {
        if let Some(read) = self.selections.borrow().get(parameter) {
            return read.clone();
        }
        let read = selection(context, parameter, selector).map(Arc::new);
        self.selections
            .borrow_mut()
            .insert(parameter.to_owned(), read.clone());
        read
    }
}

/// A number of SI units or a quantity of `dimension`, at least `minimum`,
/// as `reference` binds it: `plain` and `quantity` name what it must be,
/// and `unit` its unit.
fn measure(
    reference: &Reference<'_>,
    value: &ParameterValue,
    (dimension, minimum): (QuantityDimension, f64),
    (plain, quantity, unit): (&str, &str, &str),
) -> Result<f64, Unavailable> {
    let not = |what: &str| invalid(format!("{reference} is not {what}"));
    #[allow(clippy::cast_precision_loss)]
    let measured = match value {
        ParameterValue::Number { value } => *value,
        ParameterValue::Integer { value } => *value as f64,
        ParameterValue::Quantity { value, unit } => match si_quantity(*value, unit) {
            Ok((measured, stated)) if stated == dimension => measured,
            _ => return Err(not(quantity)),
        },
        _ => return Err(not(plain)),
    };
    if !(measured.is_finite() && measured >= minimum) {
        return Err(invalid(format!(
            "{reference} is {measured} {unit}, not {quantity} of at least {minimum} {unit}"
        )));
    }
    Ok(measured)
}

/// The value the rule's parameter `parameter` binds for a measured
/// parameter of `kind`.
#[allow(clippy::too_many_lines)]
fn bound(
    context: &RuleContext<'_>,
    arguments: Option<&Arguments>,
    kind: MeasuredParameterKind,
    parameter: &str,
    value: &ParameterValue,
) -> Result<MeasuredArgument, Unavailable> {
    // Worded only when a refusal needs it.
    let reference = Reference(parameter);
    let not = |what: &str| invalid(format!("{reference} is not {what}"));
    Ok(match (kind, value) {
        (MeasuredParameterKind::Objects, ParameterValue::Selector { value: selector }) => {
            MeasuredArgument::Objects(match arguments {
                Some(arguments) => arguments.selection(context, parameter, selector)?,
                None => Arc::new(selection(context, parameter, selector)?),
            })
        }
        (MeasuredParameterKind::Objects, _) => return Err(not("a selector")),
        (MeasuredParameterKind::Length { minimum }, value) => MeasuredArgument::Length(measure(
            &reference,
            value,
            (QuantityDimension::Length, minimum),
            ("a number of metres or a length", "a length", "m"),
        )?),
        (MeasuredParameterKind::Path, ParameterValue::StringList { value: steps }) => {
            if steps.is_empty() || steps.iter().any(|step| step.trim().is_empty()) {
                return Err(invalid(format!("{reference} holds an empty step")));
            }
            MeasuredArgument::Path(steps.iter().map(|step| step.trim().to_owned()).collect())
        }
        (MeasuredParameterKind::Choices { options }, ParameterValue::StringList { value }) => {
            MeasuredArgument::Choices(
                axioval_ir::measured::choices(options, value.iter().map(String::as_str))
                    .map_err(|why| invalid(format!("{reference}: {why}")))?,
            )
        }
        (MeasuredParameterKind::Path | MeasuredParameterKind::Choices { .. }, _) => {
            return Err(not("a string list"));
        }
        // A pattern binds exactly as stated: its spaces match spaces.
        (MeasuredParameterKind::Pattern, ParameterValue::String { value: text }) => {
            if text.is_empty() {
                return Err(invalid(format!("{reference} is empty")));
            }
            MeasuredArgument::Text(text.clone())
        }
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
            | MeasuredParameterKind::Pattern
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
        (MeasuredParameterKind::Area { minimum }, value) => MeasuredArgument::Number(measure(
            &reference,
            value,
            (QuantityDimension::Area, minimum),
            ("a number of square metres or an area", "an area", "m²"),
        )?),
        (MeasuredParameterKind::Number { minimum }, value) => {
            #[allow(clippy::cast_precision_loss)]
            let number = match value {
                ParameterValue::Number { value } => *value,
                ParameterValue::Integer { value } => *value as f64,
                _ => return Err(not("a number")),
            };
            if !(number.is_finite() && number >= minimum) {
                return Err(invalid(format!(
                    "{reference} is {number}, not a number of at least {minimum}"
                )));
            }
            MeasuredArgument::Number(number)
        }
        (MeasuredParameterKind::Truth, ParameterValue::Boolean { value }) => {
            MeasuredArgument::Truth(*value)
        }
        (MeasuredParameterKind::Truth, _) => return Err(not("a boolean")),
        (MeasuredParameterKind::Angle { below }, value) => {
            let ParameterValue::Quantity { value, unit } = value else {
                return Err(not("a plane angle"));
            };
            let degrees = match si_quantity(*value, unit) {
                Ok((radians, QuantityDimension::PlaneAngle)) => radians.to_degrees(),
                _ => return Err(not("a plane angle")),
            };
            if !(0.0..below).contains(&degrees) {
                return Err(invalid(format!(
                    "{reference} is {degrees} degrees, not an angle of at least 0 and below \
                     {below} degrees"
                )));
            }
            MeasuredArgument::Number(degrees)
        }
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
    bind_references(context, parameters, arguments, Some(anchor), call)
}

/// [`bind`], the anchor left unbound where none is given.
fn bind_references(
    context: &RuleContext<'_>,
    parameters: Option<&BTreeMap<String, ParameterValue>>,
    arguments: Option<&Arguments>,
    anchor: Option<&ObjectId>,
    call: &mut MeasuredCall,
) -> Result<(), Unavailable> {
    let references: Vec<(&'static str, MeasuredArgument)> = call
        .references()
        .map(|(key, argument)| (key, argument.clone()))
        .collect();
    for (key, reference) in references {
        let argument = match &reference {
            MeasuredArgument::Anchor => match anchor {
                Some(anchor) => {
                    MeasuredArgument::Objects(Arc::new(MeasuredSelection::anchor(anchor.clone())))
                }
                None => continue,
            },
            MeasuredArgument::Parameter(parameter) => {
                let Some(parameters) = parameters else {
                    return Err(invalid(format!(
                        "`@{parameter}` names a rule parameter, which a selector never reads"
                    )));
                };
                // A template's own selection, as `@selection`.
                if parameter == axioval_engine::template::SELECTION
                    && !parameters.contains_key(parameter)
                    && let Some(arguments) = arguments
                    && let Some(selector) = &arguments.selector
                {
                    let selection = arguments.selection(context, parameter, selector)?;
                    call.bind(key, MeasuredArgument::Objects(selection))
                        .map_err(|error| {
                            (NotEvaluatedReason::InvalidDeclaration, error.to_string())
                        })?;
                    continue;
                }
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

/// A reference as a refusal words it, `` `@name` ``.
struct Reference<'a>(&'a str);

impl std::fmt::Display for Reference<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "`@{}`", self.0)
    }
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

    /// A number binds at least its minimum, an integer as a number, and a
    /// boolean as a truth.
    #[test]
    fn a_number_or_a_truth_binds_as_stated() {
        let project = Project::new(Vec::new()).unwrap();
        let services = ServiceRegistry::new();
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let number = MeasuredParameterKind::Number { minimum: 0.0 };
        assert_eq!(
            bound(
                &context,
                None,
                number,
                "share",
                &ParameterValue::Integer { value: 1 }
            ),
            Ok(MeasuredArgument::Number(1.0))
        );
        assert_eq!(
            bound(
                &context,
                None,
                number,
                "share",
                &ParameterValue::Number { value: -0.5 }
            ),
            Err((
                NotEvaluatedReason::InvalidDeclaration,
                "`@share` is -0.5, not a number of at least 0".into()
            ))
        );
        assert_eq!(
            bound(
                &context,
                None,
                MeasuredParameterKind::Truth,
                "chain",
                &ParameterValue::Boolean { value: true }
            ),
            Ok(MeasuredArgument::Truth(true))
        );
    }
}
