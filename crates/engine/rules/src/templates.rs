//! Built-in capabilities run as templates
//! ([`axioval_engine::template`]): a rule bound to one is bound into its
//! composition's plan (parameters folded in as constants, slots filled),
//! its values are read for each selected object by the shared expression
//! evaluator, exactly as an `expression` rule reads them, and its
//! decision and messages keep the capability's outside contract.
//!
//! [`fork`] copies a rule bound to a template into the `expression` rule
//! it composes: the starting point for a stricter or extended rule.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

mod compare;
mod each;
mod groups;
mod scopes;

use axioval_engine::expression::{
    Evaluation, ExpressionContext, NotEvaluated, Reason, Value, evaluate_untraced,
};
use axioval_engine::template::{
    Check, Condition, Decision, End, Expect, Form, Members, Operand, Sign, Template, TemplateValue,
    Term, UndecidedMembers,
};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, MeasuredValues, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::{AggregateSource, Expression, ParameterValue, ScalarValue, Selector};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, PropertyValue, QuantityDimension, ReportColumn,
    ReportTable, ReportValue,
};
use serde_json::Value as Json;

use crate::body_extent::rounding_slack;
use crate::counts::{Population, Tally, relation_text, same_ends, tally};
use crate::expression_leaves::{Candidate, ObjectLeaves, Prefetch};
use crate::expression_requirement::reason_of;
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, deviation, judge};
use crate::selection::{bound_property_request, property_error, select_objects};
use crate::support::{Parameters, Traversal, Unavailable, display, finding, invalid};

/// A built-in capability run as its template.
pub struct Templated(Template, Plans);

impl Templated {
    /// The capability `template` describes.
    #[must_use]
    pub fn new(template: Template) -> Self {
        Self(template, Plans::new())
    }
}

impl RuleCapability for Templated {
    fn id(&self) -> &'static str {
        self.0.id
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        self.0.parameters.clone()
    }

    fn grades_deviation(&self) -> bool {
        self.0.grades
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        run((&self.0, &self.1), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&self.0)
    }
}

/// A rule parameter folded into the plan as a constant.
#[derive(Clone, Debug)]
enum Constant {
    /// A string, as stated.
    Text(String),
    /// A property reference.
    Property { set: Option<String>, name: String },
    /// A quantity in coherent SI units, converted as the capability always
    /// converted it.
    Quantity(f64, QuantityDimension),
    /// Any other scalar, as stated.
    Scalar(ScalarValue),
    /// A value no expression reads as a literal (a selector, a list of
    /// strings, a table): the runner reads it where the template says.
    Other(ParameterValue),
}

impl Constant {
    /// The constant as a number, in coherent units.
    fn number(&self) -> Option<f64> {
        match self {
            Self::Quantity(value, _) | Self::Scalar(ScalarValue::Number { value }) => Some(*value),
            #[allow(clippy::cast_precision_loss)]
            Self::Scalar(ScalarValue::Integer { value }) => Some(*value as f64),
            _ => None,
        }
    }

    /// The literal an expression reads in a `parameter` read's place.
    fn literal(&self) -> Option<ScalarValue> {
        match self {
            Self::Text(value) => Some(ScalarValue::String {
                value: value.clone(),
            }),
            Self::Quantity(value, dimension) => Some(ScalarValue::Quantity {
                value: *value,
                unit: dimension.unit_symbol().replace('²', "2").replace('³', "3"),
            }),
            Self::Scalar(scalar) => Some(scalar.clone()),
            Self::Property { .. } | Self::Other(_) => None,
        }
    }

    /// The constant as a message shows it.
    fn shown(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Property { set, name } => match set {
                Some(set) => format!("{set}.{name}"),
                None => name.clone(),
            },
            Self::Quantity(value, QuantityDimension::Length) => metres(*value),
            Self::Quantity(value, dimension) => format!("{value} {}", dimension.unit_symbol()),
            Self::Scalar(scalar) => axioval_engine::expression::Value::from_literal(scalar)
                .map_or_else(|why| why, |value| value.to_string()),
            Self::Other(_) => String::new(),
        }
    }
}

/// A rule bound into one form of its template.
struct Plan<'t> {
    template: &'t Template,
    form: &'t Form,
    bound: Arc<Bound>,
}

/// What binding a rule into its template comes to. A pure function of the
/// template and the rule's parameters, so it is kept ([`Plans`]).
struct Bound {
    /// The form's place among the template's.
    form: usize,
    constants: BTreeMap<String, Constant>,
    /// Each value step's expression, the rule's parameters bound in, in the
    /// form's order.
    expressions: Vec<Expression>,
    /// The comparison a [`Decision::Compare`] judges, bound.
    comparison: Option<compare::Bound>,
    /// A [`Decision::Each`]'s member values' expressions, then each
    /// nested population's, the rule's parameters bound in.
    each: Vec<Vec<Expression>>,
}

impl std::ops::Deref for Plan<'_> {
    type Target = Bound;

    fn deref(&self) -> &Bound {
        &self.bound
    }
}

impl Plan<'_> {
    /// Each value step with its expression, the rule's parameters bound in.
    fn values(&self) -> impl Iterator<Item = (&TemplateValue, &Expression)> {
        self.form.values.iter().zip(&self.bound.expressions)
    }
}

/// The plans of a template's rules, bound once and kept: a rule binds the
/// same way every time it runs, so a host checking model after model binds
/// it once. Kept by the rule's parameters; at most [`PLANS_KEPT`], the
/// oldest dropped first. A rule that does not bind is never kept.
#[derive(Default)]
pub(crate) struct Plans(Mutex<Vec<Kept>>);

/// A bound plan kept with the parameters it was bound from.
type Kept = (BTreeMap<String, ParameterValue>, Arc<Bound>);

/// How many bound plans a template keeps.
const PLANS_KEPT: usize = 64;

impl Plans {
    /// No plan kept yet.
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }
}

/// `rule`'s plan in `template`: kept in `plans`, or bound now and kept.
fn plan<'t>(
    template: &'t Template,
    plans: &Plans,
    rule: &CompiledRule,
) -> Result<Plan<'t>, Unavailable> {
    let kept = plans.0.lock().ok().and_then(|kept| {
        kept.iter()
            .find(|(parameters, _)| *parameters == rule.parameters)
            .map(|(_, bound)| bound.clone())
    });
    let bound = if let Some(bound) = kept {
        bound
    } else {
        let bound = Arc::new(bind(template, rule)?);
        if let Ok(mut kept) = plans.0.lock() {
            if kept.len() >= PLANS_KEPT {
                kept.remove(0);
            }
            kept.push((rule.parameters.clone(), bound.clone()));
        }
        bound
    };
    Ok(Plan {
        template,
        form: &template.forms[bound.form],
        bound,
    })
}

fn stated(rule: &CompiledRule, name: &str) -> bool {
    rule.parameters.contains_key(name)
}

/// A length parameter in metres, as the capability read it.
fn length(rule: &CompiledRule, name: &str) -> Result<Option<f64>, Unavailable> {
    match Parameters(rule).quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some((_, QuantityDimension::Length)) => Err(invalid(format!("`{name}` is negative"))),
        Some(_) => Err(invalid(format!("`{name}` is not a length"))),
    }
}

/// `a`, `b` or `c`, each in backquotes.
fn alternatives(options: &[&str]) -> String {
    let quoted: Vec<String> = options.iter().map(|option| format!("`{option}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

/// Whether the descriptor requires `parameter`.
fn required(template: &Template, parameter: &str) -> bool {
    template
        .parameters
        .iter()
        .any(|descriptor| descriptor.name == parameter && descriptor.required)
}

/// A string parameter among `options`.
fn choice(
    rule: &CompiledRule,
    template: &Template,
    parameter: &str,
    options: &[&str],
) -> Result<(), Unavailable> {
    let value = if required(template, parameter) {
        Some(Parameters(rule).required_string(parameter)?)
    } else {
        Parameters(rule).string(parameter)?
    };
    match value {
        Some(value) if !options.contains(&value) => Err(invalid(format!(
            "{parameter} `{value}` is unsupported; use {}",
            alternatives(options)
        ))),
        _ => Ok(()),
    }
}

/// An integer parameter of at least zero, stated where required.
fn count(rule: &CompiledRule, template: &Template, parameter: &str) -> Result<(), Unavailable> {
    match Parameters(rule).integer(parameter)? {
        None if required(template, parameter) => {
            Err(invalid(format!("parameter `{parameter}` is required")))
        }
        Some(value) if value < 0 => Err(invalid(format!("`{parameter}` is negative"))),
        _ => Ok(()),
    }
}

/// Runs `check` over the rule's parameters.
fn check(check: &Check, rule: &CompiledRule, template: &Template) -> Result<(), Unavailable> {
    let any = |names: &[&str]| names.iter().any(|name| stated(rule, name));
    match check {
        Check::Choice { parameter, options } => choice(rule, template, parameter, options),
        Check::Length { parameter } => length(rule, parameter).map(|_| ()),
        Check::Count { parameter } => count(rule, template, parameter),
        Check::Exclusive {
            one,
            other,
            message,
        } => {
            if any(one) && any(other) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::AnyOf {
            parameters,
            message,
        } => {
            if any(parameters) {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::Requires {
            parameter,
            with,
            message,
        } => {
            if stated(rule, parameter) && !any(with) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Ordered { low, high, message } => {
            match (
                numeric(rule, template, low)?,
                numeric(rule, template, high)?,
            ) {
                (Some(low), Some(high)) if low > high => Err(invalid(*message)),
                _ => Ok(()),
            }
        }
        Check::NonNegative {
            parameters,
            message,
        } => {
            let mut values = Vec::new();
            for name in *parameters {
                values.push(numeric(rule, template, name)?);
            }
            if values.into_iter().flatten().any(|value| value < 0.0) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Kind { parameter } => {
            if let Some(descriptor) = template
                .parameters
                .iter()
                .find(|descriptor| descriptor.name == *parameter)
            {
                constant(rule, descriptor)?;
            }
            Ok(())
        }
        Check::Traversal { with, message } => {
            if Parameters(rule).traversal()?.is_some() && !with.is_empty() && !any(with) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Tolerance => Parameters(rule).tolerance().map(|_| ()),
        Check::NonNegativeLength { parameter, message } => {
            match Parameters(rule).quantity(parameter)? {
                None => Ok(()),
                Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(()),
                Some(_) => Err(invalid(*message)),
            }
        }
        Check::Together {
            parameters,
            message,
        } => {
            let stated = parameters.iter().filter(|name| stated(rule, name)).count();
            if stated == 0 || stated == parameters.len() {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::Among {
            parameter,
            options,
            message,
        } => match Parameters(rule).string(parameter)? {
            Some(value) if !options.contains(&value) => {
                Err(invalid(message.replace("{value}", value)))
            }
            _ => Ok(()),
        },
        Check::Declares {
            parameters,
            message,
        } => {
            let declared = parameters.iter().any(|name| {
                !matches!(
                    rule.parameters.get(*name),
                    None | Some(ParameterValue::Boolean { value: false })
                )
            });
            if declared {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::FalseRequires {
            flag,
            with,
            message,
        } => {
            if Parameters(rule).boolean(flag)? == Some(false) && !any(with) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Required { parameter } => {
            let descriptor = template
                .parameters
                .iter()
                .find(|descriptor| descriptor.name == *parameter);
            match descriptor
                .map(|descriptor| constant(rule, descriptor))
                .transpose()?
            {
                Some(Some(_)) => Ok(()),
                _ => Err(invalid(format!("parameter `{parameter}` is required"))),
            }
        }
        Check::Path { parameter } => Parameters(rule)
            .strings(parameter)?
            .map(Traversal::path)
            .transpose()
            .map(|_| ()),
        Check::Disciplines { parameter } => match Parameters(rule).strings(parameter)? {
            Some([]) => Err(invalid(format!("`{parameter}` is empty"))),
            Some(names) => names
                .iter()
                .try_for_each(|name| axioval_ir::Discipline::new(name.as_str()).map(|_| ()))
                .map_err(|error| invalid(error.to_string())),
            None => Ok(()),
        },
    }
}

/// The bounds a range states, as a requirement reads them: `between 1 and
/// 3`, `at least 1`, `at most 3`, and, `exactly` asked, `exactly 2` for
/// equal ones.
fn requirement(minimum: Option<f64>, maximum: Option<f64>, exactly: bool) -> Option<String> {
    Some(match (minimum, maximum) {
        #[allow(clippy::float_cmp)]
        (Some(minimum), Some(maximum)) if exactly && minimum == maximum => {
            format!("exactly {minimum}")
        }
        (Some(minimum), Some(maximum)) => format!("between {minimum} and {maximum}"),
        (Some(minimum), None) => format!("at least {minimum}"),
        (None, Some(maximum)) => format!("at most {maximum}"),
        (None, None) => return None,
    })
}

/// A numeric parameter as its descriptor types it: a number, an integer,
/// or a quantity in coherent SI units.
fn numeric(
    rule: &CompiledRule,
    template: &Template,
    name: &str,
) -> Result<Option<f64>, Unavailable> {
    let kind = template
        .parameters
        .iter()
        .find(|descriptor| descriptor.name == name)
        .map(|descriptor| descriptor.parameter_type);
    let parameters = Parameters(rule);
    match kind {
        Some(ParameterType::Number) => parameters.number(name),
        #[allow(clippy::cast_precision_loss)]
        Some(ParameterType::Integer) => Ok(parameters.integer(name)?.map(|value| value as f64)),
        _ => Ok(parameters.quantity(name)?.map(|(value, _)| value)),
    }
}

/// The constant a parameter of `descriptor`'s type states.
fn constant(
    rule: &CompiledRule,
    descriptor: &ParameterDescriptor,
) -> Result<Option<Constant>, Unavailable> {
    let parameters = Parameters(rule);
    let name = descriptor.name.as_str();
    Ok(match descriptor.parameter_type {
        ParameterType::String => parameters
            .string(name)?
            .map(|value| Constant::Text(value.to_owned())),
        ParameterType::PropertyReference => {
            parameters
                .property(name)?
                .map(|property| Constant::Property {
                    set: property.set.map(str::to_owned),
                    name: property.name.to_owned(),
                })
        }
        ParameterType::Quantity => parameters
            .quantity(name)?
            .map(|(value, dimension)| Constant::Quantity(value, dimension)),
        ParameterType::Number => parameters
            .number(name)?
            .map(|value| Constant::Scalar(ScalarValue::Number { value })),
        ParameterType::Selector => parameters.selector(name)?.map(|selector| {
            Constant::Other(ParameterValue::Selector {
                value: Box::new(selector.clone()),
            })
        }),
        ParameterType::StringList => parameters.strings(name)?.map(|value| {
            Constant::Other(ParameterValue::StringList {
                value: value.to_vec(),
            })
        }),
        // Any other parameter: a scalar as stated, or a value only the
        // runner reads (a table, an expression).
        _ => rule.parameters.get(name).cloned().map(|value| {
            match ScalarValue::try_from(value.clone()) {
                Ok(scalar) => Constant::Scalar(scalar),
                Err(_) => Constant::Other(value),
            }
        }),
    })
}

/// `expression` with the rule's parameters bound in: each `parameter` read
/// of a constant replaced by its literal, and each slot in a string field
/// (`{axis}`, `{target_property.set}`, `{target_property.name}`) filled.
/// A field holding only the set slot of a reference without a set is
/// dropped.
fn bound(expression: &Expression, constants: &BTreeMap<String, Constant>) -> Expression {
    // A plain property read, the shape of most values, is filled in place;
    // anything else through its JSON form, every string field alike.
    if let Some(bound) = bound_read(expression, constants) {
        return bound;
    }
    let mut json = serde_json::to_value(expression).unwrap_or_default();
    fill(&mut json, constants);
    serde_json::from_value(json).unwrap_or_else(|_| expression.clone())
}

/// A plain property read with its string fields filled as [`fill`] fills
/// them; `None` for any other expression.
fn bound_read(
    expression: &Expression,
    constants: &BTreeMap<String, Constant>,
) -> Option<Expression> {
    let Expression::Property {
        property_set,
        property,
        of,
        label,
    } = expression
    else {
        return None;
    };
    let filled = |text: &str| match slot(text, constants) {
        Some(Slot::Filled(filled)) => Some(Some(filled)),
        Some(Slot::Dropped) => Some(None),
        None => None,
    };
    let optional = |field: &Option<String>| match field.as_deref().map(filled) {
        Some(Some(filled)) => filled,
        _ => field.clone(),
    };
    let property = match filled(property) {
        Some(Some(filled)) => filled,
        // A required field dropped leaves no expression to bind into.
        Some(None) => return Some(expression.clone()),
        None => property.clone(),
    };
    Some(Expression::Property {
        property_set: optional(property_set),
        property,
        of: *of,
        label: optional(label),
    })
}

/// `json`, an expression's JSON form, with every `parameter` read of a
/// constant replaced by its literal and every slot in a string field
/// filled.
fn fill(json: &mut Json, constants: &BTreeMap<String, Constant>) {
    match json {
        Json::Object(fields) => {
            if fields.get("kind").and_then(Json::as_str) == Some("parameter")
                && let Some(literal) = fields
                    .get("name")
                    .and_then(Json::as_str)
                    .and_then(|name| constants.get(name))
                    .and_then(Constant::literal)
            {
                let mut replaced = serde_json::Map::new();
                replaced.insert("kind".into(), "literal".into());
                replaced.insert(
                    "value".into(),
                    serde_json::to_value(literal).unwrap_or_default(),
                );
                if let Some(label) = fields.get("label") {
                    replaced.insert("label".into(), label.clone());
                }
                *fields = replaced;
                return;
            }
            let mut dropped = Vec::new();
            for (key, value) in fields.iter_mut() {
                if let Json::String(text) = value {
                    match slot(text, constants) {
                        Some(Slot::Filled(filled)) => *text = filled,
                        Some(Slot::Dropped) => dropped.push(key.clone()),
                        None => {}
                    }
                } else {
                    fill(value, constants);
                }
            }
            for key in dropped {
                fields.remove(&key);
            }
        }
        Json::Array(items) => {
            for item in items {
                fill(item, constants);
            }
        }
        _ => {}
    }
}

/// A string field with its slots filled.
enum Slot {
    /// The text, every slot filled.
    Filled(String),
    /// The field held only the set slot of a reference without a set.
    Dropped,
}

/// `text` with its slots filled; `None` where it holds none.
fn slot(text: &str, constants: &BTreeMap<String, Constant>) -> Option<Slot> {
    if !text.contains('{') {
        return None;
    }
    let mut filled = String::new();
    let mut rest = text;
    let mut changed = false;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let key = &rest[open + 1..open + close];
        let (name, part) = key.split_once('.').unwrap_or((key, ""));
        let value = match (constants.get(name), part) {
            (Some(Constant::Property { set, .. }), "set") => {
                if key.len() + 2 == text.len() && set.is_none() {
                    return Some(Slot::Dropped);
                }
                set.clone()
            }
            (Some(Constant::Property { name, .. }), "name") => Some(name.clone()),
            (Some(constant @ (Constant::Text(_) | Constant::Scalar(_))), "") => {
                Some(constant.shown())
            }
            _ => None,
        };
        filled.push_str(&rest[..open]);
        match value {
            Some(value) => {
                filled.push_str(&value);
                changed = true;
            }
            None => filled.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    filled.push_str(rest);
    changed.then_some(Slot::Filled(filled))
}

/// Binds `rule` into `template`: checks its declaration, folds its
/// parameters into constants and chooses its form.
fn bind(template: &Template, rule: &CompiledRule) -> Result<Bound, Unavailable> {
    for each in &template.declaration {
        check(each, rule, template)?;
    }
    let mut constants = BTreeMap::new();
    for descriptor in &template.parameters {
        if let Some(constant) = constant(rule, descriptor)? {
            constants.insert(descriptor.name.clone(), constant);
        }
    }
    for default in &template.defaults {
        if !constants.contains_key(default.parameter) {
            let constant = match &default.value {
                ScalarValue::Quantity { value, unit } => {
                    let (value, dimension) = crate::support::si_quantity(*value, unit)?;
                    Constant::Quantity(value, dimension)
                }
                ScalarValue::String { value } => Constant::Text(value.clone()),
                other => Constant::Scalar(other.clone()),
            };
            constants.insert(default.parameter.to_owned(), constant);
        }
    }
    let index = template
        .forms
        .iter()
        .position(|form| form.when.iter().all(|name| stated(rule, name)))
        .ok_or_else(|| invalid("no form of the template applies to the rule's parameters"))?;
    let form = &template.forms[index];
    let expressions = form
        .values
        .iter()
        .map(|step| bound(&step.expression, &constants))
        .collect();
    let comparison = match &form.decision {
        Decision::Compare { comparison, .. } => Some(compare::bind(rule, comparison)?),
        _ => None,
    };
    let each = match &form.decision {
        Decision::Each(each) => std::iter::once(&each.values)
            .chain(each.nested.iter().map(|nested| &nested.values))
            .map(|values| {
                values
                    .iter()
                    .map(|step| bound(&step.expression, &constants))
                    .collect()
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(Bound {
        form: index,
        constants,
        expressions,
        comparison,
        each,
    })
}

/// The form's decision with a bound left out where it sums a parameter
/// the rule leaves unstated.
fn effective(plan: &Plan<'_>) -> Decision {
    effective_of(plan, &plan.form.decision)
}

/// `decision` with a bound left out where it sums a parameter the rule
/// leaves unstated.
fn effective_of(plan: &Plan<'_>, decision: &Decision) -> Decision {
    match decision {
        Decision::Within {
            value,
            minimum,
            maximum,
            rounding,
        } => {
            let known = |terms: &Vec<Term>| {
                terms.iter().all(|term| match term.operand {
                    Operand::Parameter(name) => plan.constants.contains_key(name),
                    Operand::Value(_) => true,
                })
            };
            Decision::Within {
                value,
                minimum: minimum.clone().filter(known),
                maximum: maximum.clone().filter(known),
                rounding: rounding
                    .iter()
                    .copied()
                    .filter(|magnitude| match magnitude.operand {
                        Operand::Parameter(name) => plan.constants.contains_key(name),
                        Operand::Value(_) => true,
                    })
                    .collect(),
            }
        }
        decision => decision.clone(),
    }
}

/// What one object's values were read as.
#[derive(Default)]
struct Read {
    values: Named<Value>,
    stated: Named<Option<PropertyValue>>,
    evidence: Vec<Evidence>,
    /// The values read from evidence that is not exact.
    inexact: Named<()>,
    /// The bound a decision failed or straddled, as the judge words it
    /// (`at least 6`).
    bound: Option<String>,
    why: Option<String>,
    /// Further placeholders the runner states: an anchor's `{undecided}`
    /// members and how they are reached (`{relation}`).
    named: Named<String>,
    /// The values of the member of a nested member in scope, which
    /// messages read as `member:<name>`.
    outer: Named<Value>,
    /// The declared minimum and maximum a range judge read (`{required}`).
    bounds: Option<(Option<f64>, Option<f64>)>,
}

/// The few values of one object a form names, in reading order: a list
/// kept inline, since a form reads a handful and each object reads them
/// anew.
#[derive(Clone)]
struct Named<V>(smallvec::SmallVec<[(&'static str, V); 6]>);

impl<V> Default for Named<V> {
    fn default() -> Self {
        Self(smallvec::SmallVec::new())
    }
}

impl<V> Named<V> {
    /// Every name with its value, in naming order.
    fn iter(&self) -> impl Iterator<Item = (&'static str, &V)> {
        self.0.iter().map(|(name, value)| (*name, value))
    }

    fn get(&self, name: &str) -> Option<&V> {
        self.0
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }

    /// Names `value`, replacing what the name held.
    fn insert(&mut self, name: &'static str, value: V) {
        match self.0.iter_mut().find(|(key, _)| *key == name) {
            Some(slot) => slot.1 = value,
            None => self.0.push((name, value)),
        }
    }
}

/// An interval of a value or a constant.
fn interval(plan: &Plan<'_>, read: &Read, operand: Operand) -> Option<(f64, f64)> {
    match operand {
        Operand::Value(name) => match read.values.get(name) {
            Some(Value::Number { value, .. }) => Some((value.lower, value.upper)),
            _ => None,
        },
        Operand::Parameter(name) => plan
            .constants
            .get(name)
            .and_then(Constant::number)
            .map(|value| (value, value)),
    }
}

/// `at least 0.24` as `at least 0.24 m`, rounded as lengths are shown.
fn bound_text(bound: &str) -> String {
    match bound.rsplit_once(' ') {
        Some((words, value)) => match value.parse::<f64>() {
            Ok(value) => format!("{words} {}", metres(value)),
            Err(_) => bound.to_owned(),
        },
        None => bound.to_owned(),
    }
}

/// What the range judge decided over one object's values.
struct Judged {
    verdict: Verdict,
    /// The value's interval.
    lower: f64,
    upper: f64,
    /// The bounds as declared, before the rounding allowance widened them:
    /// what a graded finding's deviation is measured from.
    minimum: Option<f64>,
    maximum: Option<f64>,
}

/// The generic range judge over one object's values, in plain binary
/// arithmetic as the capabilities judged: the verdict, or `None` where an
/// operand is no number.
fn within(plan: &Plan<'_>, read: &Read, decision: &Decision) -> Option<Judged> {
    let Decision::Within {
        value,
        minimum,
        maximum,
        rounding,
    } = decision
    else {
        return None;
    };
    let (lower, upper) = interval(plan, read, Operand::Value(value))?;
    let mut magnitudes = Vec::new();
    for magnitude in rounding {
        let (low, high) = interval(plan, read, magnitude.operand)?;
        magnitudes.push(match magnitude.end {
            End::Lower => low,
            End::Upper => high,
        });
    }
    let slack = rounding_slack(&magnitudes);
    let sum = |terms: &[Term]| -> Option<f64> {
        let mut total: Option<f64> = None;
        for term in terms {
            let (value, _) = interval(plan, read, term.operand)?;
            total = Some(match (total, term.sign) {
                (None, Sign::Plus) => value,
                (None, Sign::Minus) => -value,
                (Some(total), Sign::Plus) => total + value,
                (Some(total), Sign::Minus) => total - value,
            });
        }
        total
    };
    let minimum = match minimum {
        Some(terms) => Some(sum(terms)?),
        None => None,
    };
    let maximum = match maximum {
        Some(terms) => Some(sum(terms)?),
        None => None,
    };
    Some(Judged {
        verdict: judge(
            lower,
            upper,
            minimum.map(|bound| bound - slack),
            maximum.map(|bound| bound + slack),
        ),
        lower,
        upper,
        minimum,
        maximum,
    })
}

/// Whether a value is of the kind `expect` names, judged on what the
/// source states where it was read as stated.
fn expected(expect: Expect, value: &Value, stated: Option<&Option<PropertyValue>>) -> bool {
    match expect {
        Expect::Length => match stated {
            Some(Some(PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Length,
            })) => value.is_finite(),
            Some(Some(_)) => false,
            _ => matches!(
                value,
                Value::Number { unit, .. }
                    if *unit == axioval_engine::expression::Unit::of(Some(QuantityDimension::Length))
            ),
        },
    }
}

/// The measured value a value step reads: itself, each member's value of
/// an aggregate, or either restated in a unit (divided by a literal).
fn measured_read(expression: &Expression) -> Option<&str> {
    match expression {
        Expression::Aggregate {
            value: Some(value), ..
        } => measured_read(value),
        Expression::Divide { left, right, .. }
            if matches!(right.as_ref(), Expression::Literal { .. }) =>
        {
            measured_read(left)
        }
        _ => match property_read(expression) {
            Some((Some(axioval_ir::MEASURED_SET), call)) => {
                Some(call.split(';').next().unwrap_or(call))
            }
            _ => None,
        },
    }
}

/// The property a value step reads, when it reads one stated property.
fn property_read(expression: &Expression) -> Option<(Option<&str>, &str)> {
    match expression {
        Expression::Property {
            property_set,
            property,
            of: None,
            ..
        } => Some((property_set.as_deref(), property.as_str())),
        _ => None,
    }
}

/// A refusal as the measured value words it: without the property
/// resolution's prefix and the call and object a measured value names
/// (the object itself, or the member of an aggregate whose value it is).
fn refusal(message: &str, expression: &Expression, object: &Object) -> String {
    let Some(name) = measured_read(expression) else {
        return message.to_owned();
    };
    let message = message
        .strip_prefix("property evidence conflicts: ")
        .unwrap_or(message);
    // An engine-measured value's refusal names the set too.
    let message = message
        .strip_prefix(&format!("`{}` value ", axioval_ir::MEASURED_SET))
        .unwrap_or(message);
    if let Some(rest) = message.strip_prefix(&format!("`{name}` of {}: ", object.id)) {
        return rest.to_owned();
    }
    // A member's refusal, read through an aggregate.
    message
        .strip_prefix(&format!("`{name}` of "))
        .and_then(|rest| rest.split_once(": "))
        .map_or(message, |(_, why)| why)
        .to_owned()
}

/// `template` with its placeholders rendered.
fn render(plan: &Plan<'_>, read: &Read, template: &str) -> String {
    // Room for the placeholders' words, so a message grows once at most.
    let mut out = String::with_capacity(template.len() * 2);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push_str(&rest[..open]);
        let key = &rest[open + 1..open + close];
        match placeholder(plan, read, key) {
            Some(text) => out.push_str(&text),
            None => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

fn placeholder(plan: &Plan<'_>, read: &Read, key: &str) -> Option<String> {
    // The format follows the last colon: `member:height:length` formats
    // the member's value `height`.
    let (name, format) = key.rsplit_once(':').unwrap_or((key, ""));
    if format.is_empty() {
        let mut named = plan
            .template
            .texts
            .iter()
            .filter(|text| text.name == name)
            .peekable();
        if named.peek().is_some() {
            // The first text of the name whose condition holds, or nothing.
            return Some(
                named
                    .find(|text| holds(plan, read, text.when))
                    .map(|text| render(plan, read, text.text))
                    .unwrap_or_default(),
            );
        }
        match name {
            "bound" => return read.bound.as_deref().map(bound_text),
            "why" => return read.why.clone(),
            "target" if plan.comparison.is_some() => {
                return plan.comparison.as_ref().map(|bound| bound.target.clone());
            }
            _ => {}
        }
        if let Some(text) = read.named.get(name) {
            return Some(text.clone());
        }
    }
    if let ("tolerance", "suffix") = (name, format) {
        // A comparison's declared tolerance, or the one a group decision
        // read.
        return plan
            .comparison
            .as_ref()
            .map(compare::Bound::tolerance_suffix)
            .or_else(|| read.named.get("tolerance:suffix").cloned());
    }
    match format {
        // The bound as the judge words it, its number as declared.
        "plain" if name == "bound" => read.bound.clone(),
        // The declared range as a requirement reads it.
        "" | "exactly" if name == "required" => read
            .bounds
            .and_then(|(minimum, maximum)| requirement(minimum, maximum, format == "exactly")),
        // What surely counts: the value's lower end.
        "least" => match read.values.get(name) {
            Some(Value::Number { value, .. }) => Some(value.lower.to_string()),
            _ => None,
        },
        // An area as the area capabilities show it, rounded to 1e-4 m².
        "area" => match read.values.get(name) {
            Some(Value::Number { value, .. }) => {
                Some(crate::plan_area::shown(value.lower, value.upper))
            }
            _ => plan
                .constants
                .get(name)
                .and_then(Constant::number)
                .map(|value| crate::plan_area::shown(value, value)),
        },
        "length" => {
            let value = match name.strip_prefix("member:") {
                Some(outer) => read.outer.get(outer),
                None => read.values.get(name),
            };
            if let Some(Value::Number { value, .. }) = value {
                return Some(shown(value.lower, value.upper));
            }
            plan.constants
                .get(name)
                .and_then(Constant::number)
                .map(metres)
        }
        "stated" => read
            .stated
            .get(name)
            .map(|value| display(value.as_ref()))
            .or_else(|| read.values.get(name).map(ToString::to_string)),
        "" => plan
            .constants
            .get(name)
            .map(Constant::shown)
            .or_else(|| read.values.get(name).map(ToString::to_string)),
        _ => None,
    }
}

/// Whether a text's condition holds.
fn holds(plan: &Plan<'_>, read: &Read, condition: Option<Condition>) -> bool {
    match condition {
        None => true,
        Some(Condition::Positive { parameter }) => plan
            .constants
            .get(parameter)
            .and_then(Constant::number)
            .is_some_and(|value| value > 0.0),
        Some(Condition::Inexact { value }) => read.inexact.get(value).is_some(),
        Some(Condition::Equals { parameter, value }) => matches!(
            plan.constants.get(parameter),
            Some(Constant::Text(stated)) if stated == value
        ),
        Some(Condition::OneOf { parameter, values }) => matches!(
            plan.constants.get(parameter),
            Some(Constant::Text(stated)) if values.contains(&stated.as_str())
        ),
        Some(Condition::Zero { value }) => matches!(
            read.values.get(value),
            Some(Value::Number { value, .. }) if value.lower == 0.0
        ),
    }
}

fn evidence_of(evaluation: &Evaluation) -> impl Iterator<Item = Evidence> + '_ {
    evaluation
        .reads
        .iter()
        .flat_map(|read| read.leaf.evidence.iter().cloned())
}

/// What one object comes to.
enum Outcome {
    Passed,
    Finding {
        message: String,
        evidence: Vec<Evidence>,
        related: Vec<ObjectId>,
        deviation: Option<Deviation>,
    },
    Open(NotEvaluatedReason, String),
}

impl Outcome {
    fn finding(message: String, evidence: Vec<Evidence>) -> Self {
        Self::Finding {
            message,
            evidence,
            related: Vec::new(),
            deviation: None,
        }
    }
}

/// A value not of the kind its step expects: invalid evidence.
fn mismatch(plan: &Plan<'_>, read: &Read, step: &TemplateValue) -> Outcome {
    let message = match step.mismatch {
        Some(mismatch) => render(plan, read, mismatch),
        None => format!("`{}` is not of the kind the check needs", step.name),
    };
    Outcome::Open(NotEvaluatedReason::InvalidEvidence, message)
}

/// The members of a form's anchors, read once per rule: the population
/// the member selector picks and the traversal reaching it.
struct Scope<'t> {
    members: &'t Members,
    population: Population,
    traversal: Option<Traversal>,
    /// The path whose ends a member must share with its anchor.
    ends: Option<Traversal>,
}

impl<'t> Scope<'t> {
    fn of(
        plan: &Plan<'t>,
        context: &RuleContext<'_>,
        rule: &CompiledRule,
    ) -> Result<Option<Self>, Unavailable> {
        let Some(members) = &plan.form.members else {
            return Ok(None);
        };
        let population = match plan.constants.get(members.selector) {
            Some(Constant::Other(ParameterValue::Selector { value: selector })) => {
                Population::of(context, selector)
            }
            _ if members.every_when_unstated => Population::of(context, &Selector::All),
            _ => {
                return Err(invalid(format!(
                    "parameter `{}` is required",
                    members.selector
                )));
            }
        };
        let ends = match members.same_ends {
            Some(parameter) => Parameters(rule)
                .strings(parameter)?
                .map(Traversal::path)
                .transpose()?,
            None => None,
        };
        Ok(Some(Self {
            members,
            population,
            traversal: Parameters(rule).traversal()?,
            ends,
        }))
    }

    /// How a message names the way members are reached (`{relation}`).
    fn relation(&self) -> String {
        let via = relation_text(self.traversal.as_ref());
        match &self.ends {
            Some(ends) => format!("{via} with the same ends via {}", ends.relationship),
            None => via,
        }
    }

    /// The members of `anchor`: reached, then kept where they share its
    /// ends.
    fn tally(&self, context: &RuleContext<'_>, anchor: &Object) -> Result<Tally, Unavailable> {
        let tallied = tally(context, self.traversal.as_ref(), anchor, &self.population)?;
        match &self.ends {
            Some(ends) => same_ends(context, ends, anchor, tallied),
            None => Ok(tallied),
        }
    }
}

/// The members' source and filter an aggregate over the form's members
/// reads in their place.
type Retarget<'a> = &'a dyn Fn(Option<&Selector>) -> (AggregateSource, Option<Box<Selector>>);

/// `expression` with every aggregate over the form's members (itself, or
/// an operand of a division restating it in a unit) reading `retarget`'s
/// source and filter instead.
fn over_members(expression: &Expression, selector: &str, retarget: Retarget<'_>) -> Expression {
    match expression {
        Expression::Aggregate {
            function,
            over,
            filter,
            value,
            label,
        } if *over == Members::source(selector) => {
            let (over, filter) = retarget(filter.as_deref());
            Expression::Aggregate {
                function: *function,
                over,
                filter,
                value: value.clone(),
                label: label.clone(),
            }
        }
        Expression::Divide { left, right, label } => Expression::Divide {
            left: Box::new(over_members(left, selector, retarget)),
            right: Box::new(over_members(right, selector, retarget)),
            label: label.clone(),
        },
        other => other.clone(),
    }
}

/// What one object's values come to, and the table row they fill.
struct Judgement {
    outcome: Outcome,
    row: Option<Vec<ReportValue>>,
}

impl From<Outcome> for Judgement {
    fn from(outcome: Outcome) -> Self {
        Self { outcome, row: None }
    }
}

/// The candidates an aggregate over a form's members reads: `sure` and
/// `possible` objects, in the project's order, as a scan of the project
/// would list them.
fn candidates<'a>(
    context: &RuleContext<'a>,
    sure: &[ObjectId],
    possible: &[ObjectId],
) -> Vec<Candidate<'a>> {
    let mut candidates: Vec<Candidate<'a>> = sure
        .iter()
        .map(|id| (id, true))
        .chain(possible.iter().map(|id| (id, false)))
        .filter_map(|(id, certain)| {
            crate::selection::object_by_id(context, id).map(|object| (object, certain, Vec::new()))
        })
        .collect();
    candidates.sort_by(|left, right| left.0.id.cmp(&right.0.id));
    candidates
}

/// Reads the plan's values for `object` with `leaves`, in order, into
/// `read`: the outcome where one leaves the object decided or open
/// before the decision.
fn read_values<'v>(
    plan: &Plan<'_>,
    values: impl IntoIterator<Item = (&'v TemplateValue, &'v Expression)>,
    as_stated: &dyn Fn(&str) -> bool,
    context: &RuleContext<'_>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
    read: &mut Read,
) -> Option<Outcome> {
    for (step, expression) in values {
        let (outcome, evidence) = read_step(expression, step.name, leaves);
        let before = read.evidence.len();
        read.evidence.extend(evidence);
        if read.evidence[before..]
            .iter()
            .any(|evidence| !evidence.exact)
        {
            read.inexact.insert(step.name, ());
        }
        let stated = property_read(expression)
            .and_then(|(set, name)| leaves.stated(set, name))
            .map(|stated| stated.0);
        if let Some(value) = &stated {
            read.stated.insert(step.name, value.clone());
        }
        // The value a comparison judges is judged as the source states it:
        // an absence, `null` and a value of any kind reach the comparison.
        if as_stated(step.name) {
            if stated.is_some() {
                continue;
            }
            // A reserved set the evaluator reads apart (`axioval:value`):
            // judge what the property resolution states for it.
            if let Some((set, name)) = property_read(expression) {
                match crate::support::resolve(
                    context,
                    object,
                    crate::support::PropertyRef { set, name },
                ) {
                    Ok(resolved) => {
                        read.evidence.truncate(before);
                        read.evidence.extend(resolved.evidence());
                        read.stated.insert(step.name, resolved.value().cloned());
                        continue;
                    }
                    Err((reason, message)) => return Some(Outcome::Open(reason, message)),
                }
            }
        }
        match outcome {
            Ok(Value::Null) => {
                let message = step.absent.map_or_else(
                    || format!("`{}` is stated absent", step.name),
                    |absent| render(plan, read, absent),
                );
                return Some(Outcome::finding(
                    message,
                    std::mem::take(&mut read.evidence),
                ));
            }
            Ok(value) => {
                let mismatched = step
                    .expect
                    .is_some_and(|expect| !expected(expect, &value, stated.as_ref()));
                read.values.insert(step.name, value);
                if mismatched {
                    return Some(mismatch(plan, read, step));
                }
            }
            // Stated, but no single value of any kind: not the kind the
            // step needs, worded as the source states it.
            Err(_) if step.expect.is_some() && matches!(stated, Some(Some(_))) => {
                return Some(mismatch(plan, read, step));
            }
            Err(why) => {
                let reason = leaves
                    .first_reason()
                    .filter(|_| matches!(why.reason, Reason::Unreadable(_)))
                    .unwrap_or_else(|| reason_of(&why));
                read.why = Some(match &why.reason {
                    Reason::Unreadable(message) => refusal(message, expression, object),
                    other => other.to_string(),
                });
                return Some(Outcome::Open(reason, read.why.clone().unwrap_or_default()));
            }
        }
    }
    None
}

/// Whether `decision` judges the value `name` as the source states it,
/// rather than as the evaluator reads it.
fn judges_stated(decision: &Decision, name: &str) -> bool {
    matches!(decision, Decision::Compare { value, .. } | Decision::Unique { value, .. } if *value == name)
}

/// Reads the plan's values for `object` and decides.
fn judge_object(
    (plan, decision, scope): (&Plan<'_>, &Decision, Option<&Scope<'_>>),
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    object: &Object,
    prefetched: Prefetch,
) -> Judgement {
    let mut leaves =
        ObjectLeaves::new(context, object, Some(&rule.parameters)).with_prefetched(prefetched);
    let mut read = Read::default();
    let mut members = None;
    if let Some(scope) = scope {
        match scope.tally(context, object) {
            Ok(tally) => {
                read.named.insert("undecided", tally.undecided.to_string());
                read.named.insert("relation", scope.relation());
                // Undecided members widen the aggregate only where the
                // form says they may.
                let possible = match scope.members.undecided {
                    UndecidedMembers::Widen => tally.possible.as_slice(),
                    UndecidedMembers::OnlyExcess { .. } | UndecidedMembers::Refuse { .. } => &[],
                };
                leaves = leaves.supplying(
                    Members::source(scope.members.selector),
                    candidates(context, &tally.decided, possible),
                );
                members = Some(tally);
            }
            Err((reason, message)) => return Outcome::Open(reason, message).into(),
        }
    }
    if let Some(outcome) = read_values(
        plan,
        plan.values(),
        &|name| judges_stated(decision, name),
        context,
        object,
        &mut leaves,
        &mut read,
    ) {
        return outcome.into();
    }
    let undecided = members.as_ref().map_or(0, |members| members.undecided);
    let related = members
        .as_ref()
        .map(|members| members.decided.clone())
        .unwrap_or_default();
    if let Some(members) = &members {
        read.evidence.extend(members.evidence.iter().cloned());
    }
    // A value known only from below has no row.
    let row = plan
        .form
        .table
        .as_ref()
        .filter(|_| undecided == 0)
        .map(|table| {
            table
                .columns
                .iter()
                .map(|column| match read.values.get(column.value) {
                    Some(Value::Number { value, .. }) => {
                        ReportValue::measured(value.lower, value.upper)
                    }
                    _ => ReportValue::Unknown,
                })
                .collect()
        });
    if let (Decision::Compare { value, .. }, Some(comparison)) = (decision, &plan.comparison) {
        let stated = read.stated.get(value).cloned().flatten();
        let outcome = match comparison.holds(stated.as_ref()) {
            Ok(true) => Outcome::Passed,
            Ok(false) => Outcome::finding(render(plan, &read, plan.form.fail), read.evidence),
            Err(why) => Outcome::Open(NotEvaluatedReason::InvalidEvidence, why),
        };
        return Judgement { outcome, row };
    }
    let Some(judged) = within(plan, &read, decision) else {
        return Judgement {
            outcome: Outcome::Open(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "{}: a value the decision reads is no number",
                    plan.template.name
                ),
            ),
            row,
        };
    };
    read.bounds = Some((judged.minimum, judged.maximum));
    if undecided > 0
        && let Some(scope) = scope
        && let UndecidedMembers::OnlyExcess { message } = &scope.members.undecided
    {
        // Undecided members can only add: only an excess stands.
        if !judged.maximum.is_some_and(|maximum| judged.lower > maximum) {
            return Judgement {
                outcome: Outcome::Open(
                    NotEvaluatedReason::IncompleteEvidence,
                    render(plan, &read, message),
                ),
                row,
            };
        }
    }
    let outcome = ranged(plan, read, &judged, related);
    Judgement { outcome, row }
}

/// What a range judge's verdict comes to, worded with the form's
/// messages: a finding relating `related` (graded where the template
/// grades), or the object open naming the bound it straddles.
fn ranged(plan: &Plan<'_>, mut read: Read, judged: &Judged, related: Vec<ObjectId>) -> Outcome {
    match &judged.verdict {
        Verdict::Pass => Outcome::Passed,
        Verdict::Fail(bound) => {
            read.bound = Some(bound.clone());
            Outcome::Finding {
                message: render(plan, &read, plan.form.fail),
                evidence: read.evidence,
                related,
                deviation: if plan.template.grades {
                    deviation(judged.lower, judged.upper, judged.minimum, judged.maximum)
                } else {
                    None
                },
            }
        }
        Verdict::Undecided(bound) => {
            read.bound = Some(bound.clone());
            Outcome::Open(
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, plan.form.undecided),
            )
        }
    }
}

/// The report table a form fills, its column ids rendered.
fn report_table(plan: &Plan<'_>, rule: &CompiledRule) -> Option<ReportTable> {
    let table = plan.form.table.as_ref()?;
    let columns = table
        .columns
        .iter()
        .map(|column| {
            ReportColumn::quantity(render(plan, &Read::default(), column.id), column.dimension)
        })
        .collect();
    ReportTable::new(rule.id.clone(), table.name, columns).ok()
}

/// Runs `template` for `rule`.
pub(crate) fn run(
    (template, plans): (&Template, &Plans),
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let plan = match plan(template, plans, rule) {
        Ok(plan) => plan,
        Err((reason, message)) => {
            return CapabilityEvaluation::not_evaluated(
                reason,
                format!("{}: {message}", template.name),
            );
        }
    };
    if let Some(services) = &template.services
        && !services
            .needs
            .iter()
            .all(|service| service.registered(context.services))
    {
        return CapabilityEvaluation::not_evaluated(
            NotEvaluatedReason::MissingService,
            services.message,
        );
    }
    if let Decision::Each(each) = &plan.form.decision {
        return each::run(&plan, each, context, rule);
    }
    if let Decision::Unique { value, unique } = &plan.form.decision {
        return groups::run(&plan, value, unique, context, rule);
    }
    if let Some(scopes) = &plan.form.scope {
        return scopes::run(&plan, &effective(&plan), scopes, context, rule);
    }
    let scope = match Scope::of(&plan, context, rule) {
        Ok(scope) => scope,
        Err((reason, message)) => {
            return CapabilityEvaluation::not_evaluated(
                reason,
                format!("{}: {message}", template.name),
            );
        }
    };
    let decision = effective(&plan);
    let mut table = report_table(&plan, rule);
    let (selected, mut evaluation) = select_objects(context, &rule.selector);
    let batched = batched(&plan, context, selected.first().copied());
    let values = context.services.get::<MeasuredValues>();
    for chunk in selected.chunks(BATCH) {
        // Each measured value of every object of the chunk, measured
        // together; an object reads them as it would resolve them alone.
        let ids: Vec<&ObjectId> = chunk.iter().map(|object| &object.id).collect();
        let mut columns: Vec<_> = match values {
            Some(values) => batched
                .iter()
                .map(|(_, name)| {
                    values
                        .read_batch(name, &ids)
                        .into_iter()
                        .map(|read| read.map_err(property_error))
                })
                .collect(),
            None => Vec::new(),
        };
        for object in chunk {
            let prefetched = batched
                .iter()
                .zip(columns.iter_mut())
                .filter_map(|((set, name), column)| {
                    column.next().map(|read| (set.clone(), name.clone(), read))
                })
                .collect();
            let Judgement { outcome, row } = judge_object(
                (&plan, &decision, scope.as_ref()),
                context,
                rule,
                object,
                prefetched,
            );
            if let (Some(table), Some(row)) = (&mut table, row) {
                // Selected objects are distinct, so rows never collide.
                let _ = table.push_row(object.id.clone(), row);
            }
            match outcome {
                Outcome::Passed => {}
                Outcome::Finding {
                    message,
                    evidence,
                    related,
                    deviation,
                } => {
                    evaluation.push_finding_deviating(
                        finding(rule, &object.id, message, evidence, related),
                        deviation,
                    );
                }
                Outcome::Open(reason, message) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
    }
    if let Some(table) = table {
        evaluation.push_table(table);
    }
    evaluation
}

/// How many objects' values are measured together: enough to share each
/// value's parsing and provider set-up, few enough that what is measured
/// ahead stays small beside the run.
const BATCH: usize = 32;

/// The measured values the plan reads directly, as `(set, name)`: each is
/// read for a chunk of objects together. Any other read (a stated
/// property, a value inside an arithmetic) is resolved as it is read, and
/// so is a measured value whose request does not bind (the read refuses it
/// as resolving it would).
fn batched(
    plan: &Plan<'_>,
    context: &RuleContext<'_>,
    first: Option<&Object>,
) -> Vec<(Option<Arc<str>>, Arc<str>)> {
    let Some(first) = first else {
        return Vec::new();
    };
    plan.values()
        .filter_map(|(_, expression)| match property_read(expression) {
            Some((Some(set), name))
                if set == axioval_ir::MEASURED_SET
                    && bound_property_request(context, first, Some(set), name).is_ok() =>
            {
                Some((Some(Arc::from(set)), Arc::from(name)))
            }
            _ => None,
        })
        .collect()
}

/// One value step of `object`: a plain property read straight through the
/// object's leaves, anything else through the evaluator. Both read the same
/// leaf, spend the same budget and answer the same value, evidence and
/// refusal; the plain read only skips recording the step. A plain read is
/// never narrowed to an anchor's members, so its prefetched value holds.
fn read_step(
    expression: &Expression,
    root: &str,
    leaves: &mut ObjectLeaves<'_>,
) -> (Result<Value, NotEvaluated>, Vec<Evidence>) {
    if let Expression::Property {
        property_set,
        property,
        of: None,
        ..
    } = expression
    {
        let here = |reason| NotEvaluated {
            path: root.to_owned(),
            label: expression.label().map(str::to_owned),
            reason,
        };
        if !leaves.spend() {
            return (Err(here(Reason::BudgetExhausted)), Vec::new());
        }
        let leaf = leaves.property(property_set.as_deref(), property);
        return (
            leaf.value.map_err(|why| here(Reason::Unreadable(why))),
            leaf.evidence,
        );
    }
    let evaluation = evaluate_untraced(expression, root, leaves);
    let evidence = evidence_of(&evaluation).collect();
    (evaluation.outcome, evidence)
}

/// A rule bound to a template, copied into the `expression` rule it
/// composes: a starting point for a stricter or extended rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Fork {
    /// The requirement: the template's form with the rule's parameters
    /// bound in, as a block editor shows it.
    pub requirement: Expression,
}

impl Fork {
    /// The capability a forked rule is bound to.
    pub const CAPABILITY: &'static str = "axioval:capability.expression";

    /// The forked rule's parameters.
    #[must_use]
    pub fn parameters(&self) -> BTreeMap<String, ParameterValue> {
        BTreeMap::from([(
            "requirement".to_owned(),
            ParameterValue::Expression {
                value: Box::new(self.requirement.clone()),
            },
        )])
    }
}

/// The path a forked rule's aggregates reach an anchor's members along:
/// the rule's `path`, or its `relationship` in its `direction`. Members
/// everywhere in the anchor's source, a followed chain and skipped absent
/// ends have no aggregate path.
fn member_path(rule: &CompiledRule) -> Result<AggregateSource, ForkError> {
    let parameters = Parameters(rule);
    let inexpressible = |why: &str| Err(ForkError::Inexpressible(why.to_owned()));
    if parameters.boolean("follow_chain").ok().flatten() == Some(true) {
        return inexpressible("an aggregate path follows no chain of a relationship");
    }
    if parameters
        .boolean("skip_absent_relationship_ends")
        .ok()
        .flatten()
        == Some(true)
    {
        return inexpressible("an aggregate path skips no absent relationship end");
    }
    let path = match (
        parameters.strings("path").ok().flatten(),
        parameters.string("relationship").ok().flatten(),
    ) {
        (Some(path), _) => path.to_vec(),
        (None, Some(relationship)) => {
            let direction = parameters
                .string("direction")
                .ok()
                .flatten()
                .unwrap_or("forward");
            vec![format!("{relationship}:{direction}")]
        }
        (None, None) => {
            return inexpressible(
                "members everywhere in an anchor's source are no aggregate's members",
            );
        }
    };
    Ok(AggregateSource::Path { path })
}

/// `expression`, its aggregates over the form's members as aggregates
/// along `over` of the objects `selector` picks.
fn along(
    expression: &Expression,
    members: &str,
    over: &AggregateSource,
    selector: &Selector,
) -> Expression {
    over_members(expression, members, &|filter| {
        (
            over.clone(),
            Some(Box::new(match filter {
                None => selector.clone(),
                Some(filter) => Selector::AllOf {
                    operands: vec![selector.clone(), filter.clone()],
                },
            })),
        )
    })
}

/// Why a rule cannot be forked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ForkError {
    /// The capability is implemented in code, not as a template.
    #[error("capability `{0}` is not a template")]
    NotATemplate(String),
    /// The rule's parameters do not bind into the template.
    #[error("{0}")]
    Declaration(String),
    /// The rule binds, but its composition has no `expression` form.
    #[error("the rule has no expression form: {0}")]
    Inexpressible(String),
}

/// `rule`, bound to the template `capability`, as the `expression` rule
/// it composes. The form's decision becomes its expression form
/// ([`Decision::expression`]), every value inlined and every parameter
/// folded in, so the fork reads the same measured and stated values
/// through the same evaluator. Its findings are worded as an `expression`
/// rule's, and its bounds are decided by sound interval arithmetic, so it
/// reaches the template's verdicts except where a value lies within a
/// unit in the last place of a bound's rounding allowance.
///
/// # Errors
///
/// [`ForkError`] when `capability` is no template or `rule`'s parameters
/// do not bind into it.
pub fn fork(capability: &dyn RuleCapability, rule: &CompiledRule) -> Result<Fork, ForkError> {
    let template = capability
        .template()
        .ok_or_else(|| ForkError::NotATemplate(capability.id().to_owned()))?;
    let binding = bind(template, rule)
        .map_err(|(_, message)| ForkError::Declaration(format!("{}: {message}", template.name)))?;
    let plan = Plan {
        template,
        form: &template.forms[binding.form],
        bound: Arc::new(binding),
    };
    if matches!(plan.form.decision, Decision::Each(_)) {
        return Err(ForkError::Inexpressible(
            "members judged one by one against their neighbours have no expression form".to_owned(),
        ));
    }
    if matches!(plan.form.decision, Decision::Unique { .. }) {
        return Err(ForkError::Inexpressible(
            "an expression rule judges each object on its own, not against the values of its group"
                .to_owned(),
        ));
    }
    if plan.form.scope.is_some() {
        return Err(ForkError::Inexpressible(
            "an expression rule judges objects, not a source or the project as a whole".to_owned(),
        ));
    }
    if let (Decision::Compare { value: subject, .. }, Some(comparison)) =
        (&plan.form.decision, &plan.comparison)
    {
        let value = plan
            .values()
            .find(|(step, _)| step.name == *subject)
            .map_or_else(
                || Expression::Derived {
                    name: (*subject).to_owned(),
                    label: None,
                },
                |(_, expression)| expression.clone(),
            );
        let requirement = comparison
            .expression(value)
            .map_err(ForkError::Inexpressible)?;
        return Ok(Fork {
            requirement: bound(&requirement, &plan.constants),
        });
    }
    let values = match &plan.form.members {
        None => plan
            .values()
            .map(|(step, expression)| (step, expression.clone()))
            .collect::<Vec<_>>(),
        Some(members) => {
            if members
                .same_ends
                .is_some_and(|parameter| plan.constants.contains_key(parameter))
            {
                return Err(ForkError::Inexpressible(
                    "an aggregate does not compare the ends a path reaches from each member \
                     with the anchor's"
                        .to_owned(),
                ));
            }
            let over = member_path(rule)?;
            let every = Selector::All;
            let selector: &Selector = match plan.constants.get(members.selector) {
                Some(Constant::Other(ParameterValue::Selector { value: selector })) => selector,
                _ if members.every_when_unstated => &every,
                _ => {
                    return Err(ForkError::Declaration(format!(
                        "{}: parameter `{}` is required",
                        template.name, members.selector
                    )));
                }
            };
            plan.values()
                .map(|(step, expression)| {
                    (step, along(expression, members.selector, &over, selector))
                })
                .collect()
        }
    };
    let requirement = effective(&plan).expression(&|name| {
        values
            .iter()
            .find(|(step, _)| step.name == name)
            .map_or_else(
                || Expression::Derived {
                    name: name.to_owned(),
                    label: None,
                },
                |(_, expression)| expression.clone(),
            )
    });
    Ok(Fork {
        requirement: bound(&requirement, &plan.constants),
    })
}
