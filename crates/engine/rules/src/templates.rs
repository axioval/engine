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
mod compared;
mod each;
pub(crate) mod facets;
mod groups;
mod items;
mod joined;
mod members;
mod pairs;
mod parts;
mod proportion;
mod requirements;
mod scopes;

use axioval_engine::expression::{
    Evaluation, ExpressionContext, NotEvaluated, Reason, Value, evaluate_untraced,
};
use axioval_engine::template::{
    Check, Condition, Decision, Derived, End, Expect, Form, Members, Operand, Refusals, Sign,
    Template, TemplateValue, Term, UndecidedMembers,
};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, MeasuredValues, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::{AggregateSource, Expression, ParameterValue, ScalarValue, Selector};
use axioval_ir::{
    Evidence, NotEvaluatedReason, Object, ObjectId, PropertyValue, QuantityDimension, ReportColumn,
    ReportTable, ReportValue, Severity,
};
use serde_json::Value as Json;

use crate::body_extent::rounding_slack;
use crate::counts::{Population, Tally, relation_text, same_ends, tally};
use crate::expression_leaves::{BoundPrefetched, Candidate, ObjectLeaves, Prefetch};
use crate::expression_requirement::reason_of;
use crate::level_spacing::{metres, shown};
use crate::measured_arguments::Arguments;
use crate::plan_area::{Verdict, deviation, judge};
use crate::selection::{bound_property_request, property_error, select_shared};
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
    /// The rule's parameters, those the template defaults added: what a
    /// measured value's references bind.
    parameters: BTreeMap<String, ParameterValue>,
    constants: BTreeMap<String, Constant>,
    /// Each value step's expression, the rule's parameters bound in, in the
    /// form's order.
    expressions: Vec<Expression>,
    /// The comparison a [`Decision::Compare`] judges, bound.
    comparison: Option<compare::Bound>,
    /// The proportion a [`Decision::Proportion`] judges, bound.
    proportion: Option<proportion::Mode>,
    /// A [`Decision::Each`]'s member values' expressions, then each
    /// nested population's, the rule's parameters bound in.
    each: Vec<Vec<Expression>>,
    /// Each of the form's checks' value expressions, bound likewise.
    checks: Vec<Vec<Expression>>,
    /// Each of the form's member checks' value expressions, bound likewise.
    member_checks: Vec<Vec<Expression>>,
    /// Each of the form's `unless` values' expressions, bound likewise.
    unless: Vec<Expression>,
    /// The form's grading values' expressions, then each check's, bound
    /// likewise.
    grading: Vec<Vec<Expression>>,
    /// The expressions of the values read once per rule, bound likewise.
    once: Vec<Expression>,
    /// Each of the form's checks of the project's value expressions, bound
    /// likewise.
    project: Vec<Vec<Expression>>,
    /// Whether the form names `@selection` anywhere: only then is the
    /// rule's selection made a bound selection.
    reads_selection: bool,
    /// Each member list the form reads, as written for the rule: written
    /// once and kept with the plan, since it depends only on the template
    /// and the rule's parameters.
    written: Mutex<BTreeMap<&'static str, Arc<str>>>,
    /// What every run of the rule reads of its measured names, parsed and
    /// bound once ([`Planned`](crate::measured_arguments::Planned)).
    measured: Arc<crate::measured_arguments::Planned>,
    /// Each constant as a message shows it, worded once.
    shown: BTreeMap<String, String>,
    /// Whether each of the form's `unless` values applies to the rule,
    /// decided once.
    unless_applies: std::sync::OnceLock<Vec<bool>>,
}

impl Bound {
    /// The member list a template writes as `list`, without the arguments
    /// naming parameters the rule leaves unstated ([`unstated_dropped`]),
    /// written once for the rule.
    fn written_list(&self, list: &'static str) -> Arc<str> {
        let mut written = self
            .written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        written
            .entry(list)
            .or_insert_with(|| Arc::from(unstated_dropped_list(list, &self.constants)))
            .clone()
    }
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

    /// The value steps read for every object: all of them, but for a truth
    /// judge only those up to the truth, since the ones after it are read
    /// only where it fails.
    fn eager_values(&self) -> impl Iterator<Item = (&TemplateValue, &Expression)> {
        let last = match self.form.decision {
            Decision::Holds { value } => self
                .form
                .values
                .iter()
                .position(|step| step.name == value)
                .map_or(usize::MAX, |index| index + 1),
            _ => usize::MAX,
        };
        self.values().take(last)
    }

    /// Each value step of the form's check `index`, bound.
    fn check_values(&self, index: usize) -> impl Iterator<Item = (&TemplateValue, &Expression)> {
        self.form.checks[index]
            .values
            .iter()
            .zip(&self.bound.checks[index])
    }

    /// The form's member checks, each with its value steps, bound.
    fn member_checks(
        &self,
    ) -> impl Iterator<
        Item = (
            &axioval_engine::template::MemberCheck,
            Vec<(&TemplateValue, &Expression)>,
        ),
    > {
        self.form
            .members
            .iter()
            .flat_map(|members| &members.checks)
            .zip(&self.bound.member_checks)
            .map(|(check, expressions)| (check, check.values.iter().zip(expressions).collect()))
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
#[allow(clippy::too_many_lines)]
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
        Check::NonNegativeQuantity {
            parameter,
            dimension,
            message,
        } => match Parameters(rule).quantity(parameter)? {
            Some((value, stated)) if stated != *dimension || value.is_nan() || value < 0.0 => {
                Err(invalid(*message))
            }
            _ => Ok(()),
        },
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
            if parameters.iter().any(|name| declared(rule, name)) {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::Proportion(names) => proportion::parse(rule, names).map(|_| ()),
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
        Check::Finite {
            parameters,
            above,
            at_least,
            message,
        } => finite(rule, parameters, (*above, *at_least), message),
        Check::Increasing { low, high, message } => {
            match (stated_number(rule, low), stated_number(rule, high)) {
                (Some(low), Some(high)) if low < high => Ok(()),
                _ => Err(invalid(*message)),
            }
        }
        Check::AtMost {
            parameters,
            value,
            message,
        } => {
            if parameters
                .iter()
                .any(|name| stated_number(rule, name).is_some_and(|stated| stated > *value))
            {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Positive { parameter, message } => match length(rule, parameter)? {
            Some(value) if value <= 0.0 => Err(invalid(
                message.map_or_else(|| format!("`{parameter}` must be positive"), str::to_owned),
            )),
            _ => Ok(()),
        },
        Check::Needs {
            all,
            any: declaring,
            missing,
            unused,
        } => {
            let declared = declaring.iter().any(|name| declared(rule, name));
            let every = all.iter().all(|name| stated(rule, name));
            match (declared, every, any(all)) {
                (true, false, _) => Err(invalid(*missing)),
                (false, _, true) => Err(invalid(*unused)),
                _ => Ok(()),
            }
        }
        Check::Below { low, high, message } => match (length(rule, low)?, length(rule, high)?) {
            (Some(low), Some(high)) if low >= high => Err(invalid(*message)),
            _ => Ok(()),
        },
        Check::RequiresValue {
            parameter,
            with,
            value,
            message,
        } => {
            if stated(rule, parameter) && Parameters(rule).string(with)? != Some(*value) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Excludes {
            when,
            parameters,
            value,
            message,
        } => {
            if !stated(rule, when) {
                return Ok(());
            }
            for name in *parameters {
                if Parameters(rule).string(name)? == Some(*value) {
                    return Err(invalid(*message));
                }
            }
            Ok(())
        }
        Check::Arguments { when, value } => {
            if when.iter().all(|name| stated(rule, name)) {
                arguments_checked(rule, value)
            } else {
                Ok(())
            }
        }
        Check::RequiresDeclared {
            parameter,
            with,
            message,
        } => {
            if stated(rule, parameter) && !with.iter().any(|name| declared(rule, name)) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Rows { parameter, columns } => rows(rule, parameter, columns),
        Check::Angle { parameter } => match Parameters(rule).quantity(parameter)? {
            None => Ok(()),
            Some((value, QuantityDimension::PlaneAngle)) if value >= 0.0 => Ok(()),
            Some((_, QuantityDimension::PlaneAngle)) => {
                Err(invalid(format!("`{parameter}` is negative")))
            }
            Some(_) => Err(invalid(format!("`{parameter}` is not a plane angle"))),
        },
        Check::ValueRequires {
            parameter,
            value,
            with,
            message,
        } => {
            if Parameters(rule).string(parameter)? == Some(*value) && !any(with) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::TogetherExcept {
            parameters,
            stated: excepted,
            unstated,
            message,
        } => {
            let count = parameters.iter().filter(|name| stated(rule, name)).count();
            let except = excepted.iter().all(|name| stated(rule, name))
                && !unstated.iter().any(|name| stated(rule, name));
            if count == 0 || count == parameters.len() || except {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::AnyRequires {
            any: declaring,
            with,
            message,
        } => {
            if declaring.iter().any(|name| declared(rule, name)) && !stated(rule, with) {
                Err(invalid(*message))
            } else {
                Ok(())
            }
        }
        Check::Listed {
            parameter,
            options,
            unknown,
            repeated,
        } => {
            let mut seen: Vec<&str> = Vec::new();
            for listed in Parameters(rule).strings(parameter)?.unwrap_or_default() {
                if !options.contains(&listed.as_str()) {
                    return Err(invalid(unknown.replace("{value}", listed)));
                }
                if seen.contains(&listed.as_str()) {
                    return Err(invalid(repeated.replace("{value}", listed)));
                }
                seen.push(listed);
            }
            Ok(())
        }
        Check::ListedNeeds { parameter, needs } => {
            for listed in Parameters(rule).strings(parameter)?.unwrap_or_default() {
                for needed in needs.iter().filter(|needed| needed.value == listed) {
                    if !needed.parameters.iter().all(|name| stated(rule, name)) {
                        return Err(invalid(needed.message));
                    }
                }
            }
            Ok(())
        }
        Check::AmongEach {
            parameter,
            options,
            message,
        } => {
            for listed in Parameters(rule).strings(parameter)?.unwrap_or_default() {
                if !options.contains(&listed.trim()) {
                    return Err(invalid(message.replace("{value}", listed)));
                }
            }
            Ok(())
        }
        Check::DeclaresListed {
            parameters,
            message,
        } => {
            let listed = |name: &&str| match rule.parameters.get(*name) {
                Some(ParameterValue::StringList { value }) => !value.is_empty(),
                _ => declared(rule, name),
            };
            if parameters.iter().any(listed) {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::Holds { condition, message } => {
            let (constants, _) = constants(template, rule, false)?;
            if holds_over(&constants, &Read::default(), Some(*condition)) {
                Ok(())
            } else {
                Err(invalid(*message))
            }
        }
        Check::Quantity {
            parameter,
            dimension,
            message,
        } => match Parameters(rule).quantity(parameter)? {
            Some((_, stated)) if stated != *dimension => Err(invalid(*message)),
            _ => Ok(()),
        },
        Check::Exceeds {
            parameter,
            earlier,
            message,
        } => {
            let Some(value) = numeric(rule, template, parameter)? else {
                return Ok(());
            };
            for name in *earlier {
                if numeric(rule, template, name)?.is_some_and(|earlier| value <= earlier) {
                    return Err(invalid(*message));
                }
            }
            Ok(())
        }
        Check::AngleBelow {
            parameter,
            below,
            range,
            angle,
        } => match Parameters(rule).quantity(parameter)? {
            None => Ok(()),
            Some((radians, QuantityDimension::PlaneAngle)) => {
                if (0.0..*below).contains(&radians.to_degrees()) {
                    Ok(())
                } else {
                    Err(invalid(*range))
                }
            }
            Some(_) => Err(invalid(*angle)),
        },
        Check::FiniteLength { parameter } => match Parameters(rule).quantity(parameter)? {
            None => Ok(()),
            Some((value, QuantityDimension::Length)) if value.is_finite() && value >= 0.0 => Ok(()),
            Some((_, QuantityDimension::Length)) => Err(invalid(format!(
                "`{parameter}` must be a finite length, not negative"
            ))),
            Some(_) => Err(invalid(format!("`{parameter}` is not a length"))),
        },
        Check::IfStated {
            parameter,
            check: inner,
        } => {
            if stated(rule, parameter) {
                self::check(inner, rule, template)
            } else {
                Ok(())
            }
        }
        Check::When { flags, check: then } => {
            let mut on = false;
            for flag in *flags {
                on |= Parameters(rule).boolean(flag)? == Some(true);
            }
            if on {
                self::check(then, rule, template)
            } else {
                Ok(())
            }
        }
    }
}

/// The rule parameters the measured value `value` names, as stated and
/// keyed by the value's keys, checked by the value's own argument check.
fn arguments_checked(rule: &CompiledRule, value: &str) -> Result<(), Unavailable> {
    use axioval_ir::measured::MeasuredArgument;
    // A measured value, or a measured member list.
    let call = axioval_ir::measured::parse(value)
        .or_else(|_| axioval_ir::measured::parse_members(value))
        .map_err(|error| invalid(format!("the template's value `{value}`: {error}")))?;
    let Some(check) = crate::measured_kinds::argument_check(call.name()) else {
        return Ok(());
    };
    let stated = call
        .references()
        .filter_map(|(key, argument)| match argument {
            MeasuredArgument::Parameter(name) => rule
                .parameters
                .get(name)
                .map(|stated| (key.to_owned(), stated.clone())),
            _ => None,
        })
        .collect();
    check(&stated)
}

/// Whether the rule declares `name`: states it, a boolean true, a table
/// with a row.
fn declared(rule: &CompiledRule, name: &str) -> bool {
    match rule.parameters.get(name) {
        None | Some(ParameterValue::Boolean { value: false }) => false,
        Some(ParameterValue::Table { value }) => !value.is_empty(),
        Some(_) => true,
    }
}

/// Every row of the table `parameter` against `columns`, row by row.
fn rows(
    rule: &CompiledRule,
    parameter: &str,
    columns: &[axioval_engine::template::RowCheck],
) -> Result<(), Unavailable> {
    use axioval_engine::template::RowCheck;
    for row in Parameters(rule).table(parameter)?.unwrap_or_default() {
        for column in columns {
            match column {
                RowCheck::Number {
                    column,
                    missing,
                    negative,
                } => match row.number(column)? {
                    None => return Err(invalid(*missing)),
                    Some(value) if value < 0.0 => return Err(invalid(*negative)),
                    Some(_) => {}
                },
                RowCheck::Length { column, message } => {
                    if let Some((value, unit)) = row.quantity(column)? {
                        match crate::support::si_quantity(value, unit)? {
                            (value, QuantityDimension::Length) if value >= 0.0 => {}
                            _ => return Err(invalid(*message)),
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// The parameter `name` where stated as a `number`.
fn stated_number(rule: &CompiledRule, name: &str) -> Option<f64> {
    match rule.parameters.get(name) {
        Some(ParameterValue::Number { value }) => Some(*value),
        _ => None,
    }
}

/// Every one of `parameters` stated as a finite `number`, above and at
/// least the bounds given: otherwise `message`.
fn finite(
    rule: &CompiledRule,
    parameters: &[&str],
    (above, at_least): (Option<f64>, Option<f64>),
    message: &str,
) -> Result<(), Unavailable> {
    let fits = parameters.iter().all(|name| {
        stated_number(rule, name).is_some_and(|value| {
            value.is_finite()
                && above.is_none_or(|above| value > above)
                && at_least.is_none_or(|least| value >= least)
        })
    });
    if fits { Ok(()) } else { Err(invalid(message)) }
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
        // Read as the capabilities read them, so a value of another kind
        // is refused as theirs was.
        ParameterType::Boolean => parameters
            .boolean(name)?
            .map(|value| Constant::Scalar(ScalarValue::Boolean { value })),
        ParameterType::Integer => parameters
            .integer(name)?
            .map(|value| Constant::Scalar(ScalarValue::Integer { value })),
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
        // A table is read as the capabilities read it, refused when it is
        // no table.
        ParameterType::Table(_) => parameters.table(name)?.map(|rows| {
            Constant::Other(ParameterValue::Table {
                value: rows.iter().map(|row| row.0.clone()).collect(),
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
    let property = if property_set.as_deref() == Some(axioval_ir::MEASURED_SET) {
        unstated_dropped(&property, constants)
    } else {
        property
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
            if fields.get("kind").and_then(Json::as_str) == Some("property")
                && fields.get("propertySet").and_then(Json::as_str)
                    == Some(axioval_ir::MEASURED_SET)
                && let Some(Json::String(name)) = fields.get_mut("property")
            {
                *name = unstated_dropped(name, constants);
            }
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

/// The measured name `name` without the arguments naming an optional
/// rule parameter the rule leaves unstated, where its measured parameter is
/// optional too and has no default: the value measured as if the argument
/// were not written. Any other reference stays, and binding it fails
/// closed.
fn unstated_dropped(name: &str, constants: &BTreeMap<String, Constant>) -> String {
    unstated_dropped_from(name, constants, axioval_ir::measured::parse)
}

/// [`unstated_dropped`] of a member list a template's check reads.
fn unstated_dropped_list(name: &str, constants: &BTreeMap<String, Constant>) -> String {
    unstated_dropped_from(name, constants, axioval_ir::measured::parse_members)
}

fn unstated_dropped_from(
    name: &str,
    constants: &BTreeMap<String, Constant>,
    parse: fn(
        &str,
    )
        -> Result<axioval_ir::measured::MeasuredCall, axioval_ir::measured::MeasuredError>,
) -> String {
    use axioval_ir::measured::MeasuredArgument;
    if !name.contains('@') {
        return name.to_owned();
    }
    let Ok(call) = parse(name) else {
        return name.to_owned();
    };
    let dropped: Vec<&str> = call
        .references()
        .filter_map(|(key, argument)| match argument {
            // The template's own selection is always bound.
            MeasuredArgument::Parameter(parameter)
                if parameter != axioval_engine::template::SELECTION
                    && !constants.contains_key(parameter.as_str())
                    && call.parameter(key).is_some_and(|declared| {
                        !declared.required && declared.default.is_none()
                    }) =>
            {
                Some(key)
            }
            _ => None,
        })
        .collect();
    if dropped.is_empty() {
        return name.to_owned();
    }
    name.split(';')
        .filter(|part| {
            part.split_once('=').is_none_or(|(key, _)| {
                !dropped
                    .iter()
                    .any(|dropped| key.trim().eq_ignore_ascii_case(dropped))
            })
        })
        .collect::<Vec<_>>()
        .join(";")
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

/// A rule's parameters as constants, and as a measured value's references
/// bind them.
type Bindings = (BTreeMap<String, Constant>, BTreeMap<String, ParameterValue>);

/// The rule's parameters as constants, each of its descriptor's kind, and
/// the template's defaults applied: a default taking another parameter's
/// value takes the first of them stated, its literal where none is. With
/// `strict`, a parameter of another kind refuses the rule; otherwise it is
/// left out (a declaration check reading the constants refuses it in its
/// place). Returns the constants and the parameters a measured value's
/// references bind, the defaults added.
fn constants(
    template: &Template,
    rule: &CompiledRule,
    strict: bool,
) -> Result<Bindings, Unavailable> {
    let mut constants = BTreeMap::new();
    for descriptor in &template.parameters {
        let read = match constant(rule, descriptor) {
            Ok(read) => read,
            Err(refused) if strict => return Err(refused),
            Err(_) => None,
        };
        if let Some(constant) = read {
            constants.insert(descriptor.name.clone(), constant);
        }
    }
    let mut parameters = rule.parameters.clone();
    for default in &template.defaults {
        if constants.contains_key(default.parameter) {
            continue;
        }
        // Another parameter's value, the first of them stated.
        if let Some(from) = default
            .from
            .iter()
            .find(|name| constants.contains_key(**name))
        {
            let taken = constants[*from].clone();
            if let Some(stated) = parameters.get(*from).cloned() {
                parameters.insert(default.parameter.to_owned(), stated);
            }
            constants.insert(default.parameter.to_owned(), taken);
            continue;
        }
        parameters.insert(
            default.parameter.to_owned(),
            ParameterValue::from(default.value.clone()),
        );
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
    Ok((constants, parameters))
}

/// Binds `rule` into `template`: checks its declaration, folds its
/// parameters into constants and chooses its form.
#[allow(clippy::too_many_lines)]
fn bind(template: &Template, rule: &CompiledRule) -> Result<Bound, Unavailable> {
    for each in &template.declaration {
        check(each, rule, template)?;
    }
    // A judge checking its own declaration refuses it first, as the
    // capability did, before any parameter is read as a constant.
    for form in &template.forms {
        match &form.decision {
            Decision::Facets(names) => facets::check(rule, names)?,
            Decision::Requirements(names) => requirements::bind(rule, names)?,
            Decision::Compared(names) => compared::bind(rule, names)?,
            _ => {}
        }
    }
    let (constants, parameters) = constants(template, rule, true)?;
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
    let proportion = match &form.decision {
        Decision::Proportion(decided) => Some(proportion::parse(rule, &decided.parameters)?),
        _ => None,
    };
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
    let checks = form
        .checks
        .iter()
        .map(|check| {
            check
                .values
                .iter()
                .map(|step| bound(&step.expression, &constants))
                .collect()
        })
        .collect();
    let member_checks = form
        .members
        .iter()
        .flat_map(|members| &members.checks)
        .map(|check| {
            check
                .values
                .iter()
                .map(|step| bound(&step.expression, &constants))
                .collect()
        })
        .collect();
    let unless = form
        .unless
        .iter()
        .map(|unless| bound(&unless.value.expression, &constants))
        .collect();
    let grading = std::iter::once(&form.grading)
        .chain(form.checks.iter().map(|check| &check.grading))
        .chain(form.project.iter().map(|check| &check.grading))
        .map(|grading| {
            grading
                .iter()
                .flat_map(|grading| grading.values.iter().chain(&grading.undecided))
                .map(|step| bound(&step.expression, &constants))
                .collect()
        })
        .collect();
    let once = form
        .once
        .iter()
        .map(|once| bound(&once.value.expression, &constants))
        .collect();
    let project = form
        .project
        .iter()
        .map(|check| {
            check
                .values
                .iter()
                .map(|step| bound(&step.expression, &constants))
                .collect()
        })
        .collect();
    Ok(Bound {
        written: Mutex::new(BTreeMap::new()),
        form: index,
        parameters,
        expressions,
        comparison,
        proportion,
        each,
        checks,
        member_checks,
        unless,
        grading,
        once,
        project,
        reads_selection: serde_json::to_string(form).map_or(true, |written| {
            written.contains(&format!("@{}", axioval_engine::template::SELECTION))
        }),
        measured: Arc::default(),
        unless_applies: std::sync::OnceLock::new(),
        shown: constants
            .iter()
            .map(|(name, constant)| (name.clone(), constant.shown()))
            .collect(),
        constants,
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
    /// The objects each value's measured reads were measured against, as
    /// their providers cite them.
    related: Vec<(&'static str, Vec<ObjectId>)>,
    /// What each value's measured reads noted, as their providers cite it.
    notes: Vec<(&'static str, Vec<String>)>,
    /// The sources each value's measured reads were measured against (a
    /// reference source), as their providers cite them.
    sources: Vec<(&'static str, Vec<axioval_ir::SourceId>)>,
    /// Whether a stated absence is read as `null` rather than found: the
    /// decision judges it (a truth judge).
    keep_null: bool,
}

impl Clone for Read {
    fn clone(&self) -> Self {
        let mut read = Self::default();
        read.clone_from(self);
        read
    }

    /// Field by field, keeping what `self` allocated: each of an object's
    /// checks starts over from the object's read in one buffer.
    fn clone_from(&mut self, source: &Self) {
        // Every field named, so a new one is not left behind.
        let Self {
            values,
            stated,
            evidence,
            inexact,
            bound,
            why,
            named,
            outer,
            bounds,
            related,
            notes,
            sources,
            keep_null,
        } = source;
        self.values.clone_from(values);
        self.stated.clone_from(stated);
        self.evidence.clone_from(evidence);
        self.inexact.clone_from(inexact);
        self.bound.clone_from(bound);
        self.why.clone_from(why);
        self.named.clone_from(named);
        self.outer.clone_from(outer);
        self.bounds = *bounds;
        self.related.clone_from(related);
        self.notes.clone_from(notes);
        self.sources.clone_from(sources);
        self.keep_null = *keep_null;
    }
}

/// The few values of one object a form names, in reading order: a list
/// kept inline, since a form reads a handful and each object reads them
/// anew.
struct Named<V>(smallvec::SmallVec<[(&'static str, V); 6]>);

impl<V> Default for Named<V> {
    fn default() -> Self {
        Self(smallvec::SmallVec::new())
    }
}

impl<V: Clone> Clone for Named<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }

    fn clone_from(&mut self, source: &Self) {
        self.0.clone_from(&source.0);
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

/// The value `value` against the value `reference` within `tolerance`
/// ([`each::near`]): what a check's `Near` decision decides.
fn near_judged(
    plan: &Plan<'_>,
    read: &Read,
    (value, reference): (&'static str, &'static str),
    tolerance: Operand,
) -> Option<Judged> {
    let (lower, upper) = interval(plan, read, Operand::Value(value))?;
    let reference = interval(plan, read, Operand::Value(reference))?;
    let (tolerance, _) = interval(plan, read, tolerance)?;
    Some(Judged {
        verdict: each::near((lower, upper), reference, tolerance),
        lower,
        upper,
        minimum: None,
        maximum: None,
    })
}

/// Whether a value is of the kind `expect` names, judged on what the
/// source states where it was read as stated.
fn expected(expect: Expect, value: &Value, stated: Option<&Option<PropertyValue>>) -> bool {
    let dimension = match expect {
        Expect::Length => QuantityDimension::Length,
        Expect::Area => QuantityDimension::Area,
        Expect::Optional | Expect::Words => return true,
    };
    match stated {
        Some(Some(PropertyValue::Quantity {
            value,
            dimension: stated,
        })) => *stated == dimension && value.is_finite(),
        Some(Some(_)) => false,
        _ => matches!(
            value,
            Value::Number { unit, .. }
                if *unit == axioval_engine::expression::Unit::of(Some(dimension))
        ),
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
/// Matched without formatting: a refusal is worded for every object a
/// guard refuses.
fn refusal(mut message: String, expression: &Expression, object: &Object) -> String {
    // What is kept is the message's tail: cut in place.
    let kept = refused_tail(&message, expression, object);
    let start = kept.as_ptr() as usize - message.as_ptr() as usize;
    message.drain(..start);
    message
}

/// The tail of `message` [`refusal`] keeps.
fn refused_tail<'m>(message: &'m str, expression: &Expression, object: &Object) -> &'m str {
    let message = message
        .strip_prefix("property evidence conflicts: ")
        .unwrap_or(message);
    // An engine-measured value's refusal names the set too.
    let message = message
        .strip_prefix('`')
        .and_then(|rest| rest.strip_prefix(axioval_ir::MEASURED_SET))
        .and_then(|rest| rest.strip_prefix("` value "))
        .unwrap_or(message);
    let Some(name) = measured_read(expression) else {
        // A composition: worded as the measured value that refused it
        // words it, whichever of those it reads that was.
        return match message.strip_prefix('`').and_then(|rest| {
            let (name, rest) = rest.split_once("` of ")?;
            Some((name, of_object(rest, &object.id)?))
        }) {
            // A measured value's name: snake case, as the registry names
            // them.
            Some((name, why))
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
            {
                why
            }
            _ => message,
        };
    };
    let Some(rest) = message
        .strip_prefix('`')
        .and_then(|rest| rest.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix("` of "))
    else {
        return message;
    };
    // The object itself, or a member's refusal read through an aggregate.
    of_object(rest, &object.id)
        .or_else(|| rest.split_once(": ").map(|(_, why)| why))
        .unwrap_or(message)
}

/// What follows `<object>: ` at the start of `text`, `object` as it
/// displays (`system:document/local`), matched without formatting it.
fn of_object<'t>(text: &'t str, object: &ObjectId) -> Option<&'t str> {
    text.strip_prefix(object.source.system.as_str())?
        .strip_prefix(':')?
        .strip_prefix(object.source.document.as_str())?
        .strip_prefix('/')?
        .strip_prefix(object.local_id.as_str())?
        .strip_prefix(": ")
}

/// `template` with its placeholders rendered.
fn render(plan: &Plan<'_>, read: &Read, template: &str) -> String {
    // Room for the placeholders' words (a refusal's whole), so a message
    // grows once at most.
    let mut out =
        String::with_capacity(template.len() * 2 + read.why.as_ref().map_or(0, String::len));
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push_str(&rest[..open]);
        let key = &rest[open + 1..open + close];
        // A refusal, the most worded placeholder, copied once.
        if key == "why"
            && let Some(why) = &read.why
            && !plan.template.texts.iter().any(|text| text.name == "why")
        {
            out.push_str(why);
            rest = &rest[open + close + 1..];
            continue;
        }
        // A rule's constant, worded once for the plan, where nothing of the
        // object's names it first (as `placeholder` reads them).
        if !key.contains(':')
            && !matches!(key, "bound" | "why" | "target" | "required")
            && let Some(shown) = plan.shown.get(key)
            && read.named.get(key).is_none()
            && !plan.template.texts.iter().any(|text| text.name == key)
        {
            out.push_str(shown);
            rest = &rest[open + close + 1..];
            continue;
        }
        match placeholder(plan, read, key) {
            Some(text) => out.push_str(&text),
            None => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

#[allow(clippy::too_many_lines)]
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
        // What surely counts: the value's lower end, or that to two
        // decimals, as the ratio capabilities showed the areas divided.
        "least" | "least2" => match read.values.get(name) {
            Some(Value::Number { value, .. }) if format == "least2" => {
                Some(((value.lower * 100.0).round() / 100.0).to_string())
            }
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
        // A share as the coverage capabilities showed it: the value's upper
        // end, at most the whole, to four decimals.
        // The objects the value's measured reads were measured against.
        "cited" => Some(
            cited(read, name)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        // What the value's measured reads noted, each after `; `: nothing
        // where they noted nothing; the first few (`notes3`) where a
        // number follows.
        _ if format.starts_with("notes") => {
            let most = match &format["notes".len()..] {
                "" => usize::MAX,
                count => count.parse::<usize>().ok()?,
            };
            Some(
                read.notes
                    .iter()
                    .filter(|(read, _)| *read == name)
                    .flat_map(|(_, notes)| notes)
                    .take(most)
                    .fold(String::new(), |mut out, note| {
                        out.push_str("; ");
                        out.push_str(note);
                        out
                    }),
            )
        }
        // A constant's number in coherent SI units, as Rust shows it
        // (`0.02`): a tolerance as the capability wrote it after its own
        // unit.
        "si" => plan
            .constants
            .get(name)
            .and_then(Constant::number)
            .map(|value| value.to_string()),
        // The sources the value's measured reads were measured against (the
        // reference source).
        "source" => {
            let sources = sources(read, name);
            (!sources.is_empty()).then(|| {
                sources
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        }
        // An area with its unit, to the micrometre squared, as the boundary
        // capabilities showed it: `7.5 m²`, or `between … and …`.
        "m2" => spanned(plan, read, name, square_metres),
        // A share as a percentage to two decimals: `87.29%`, or `between …
        // and …`.
        "percent" => spanned(plan, read, name, percent),
        // What the value's measured reads noted, joined by `; `: nothing
        // where they noted nothing.
        "noted" => Some(
            read.notes
                .iter()
                .filter(|(read, _)| *read == name)
                .flat_map(|(_, notes)| notes)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("; "),
        ),
        "share" => match read.values.get(name) {
            Some(Value::Number { value, .. }) => {
                Some(((value.upper.min(1.0) * 1e4).round() / 1e4).to_string())
            }
            _ => None,
        },
        // A share a hundredfold: a value's lower end or a constant with a
        // fixed number of decimals (`hundred3`), or a constant as Rust
        // shows it (`hundred`).
        _ if format.starts_with("hundred") => match read.values.get(name) {
            Some(Value::Number { value, .. }) => formatted(format, (value.lower, value.upper)),
            Some(_) => None,
            None => plan
                .constants
                .get(name)
                .and_then(Constant::number)
                .and_then(|value| formatted(format, (value, value))),
        },
        // A number with a fixed number of decimals (`fixed3`): a constant,
        // or a value known as one point; or a value's lower or upper end,
        // or a constant (`lower4`, `upper3`): what a value surely below a
        // minimum is shown as.
        _ if decimals(format).is_some() => {
            let (end, places) = decimals(format)?;
            match read.values.get(name) {
                Some(Value::Number { value, .. }) => match end {
                    "fixed" if value.lower.to_bits() != value.upper.to_bits() => None,
                    "lower" => Some(format!("{:.places$}", value.lower)),
                    _ => Some(format!("{:.places$}", value.upper)),
                },
                Some(_) => None,
                None => plan
                    .constants
                    .get(name)
                    .and_then(Constant::number)
                    .map(|value| format!("{value:.places$}")),
            }
        }
        // The objects a value's measured reads cite, by their local ids.
        "cited_ids" => Some(
            cited(read, name)
                .iter()
                .map(|object| object.local_id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
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

/// A value's interval, or a constant, each end shown by `show`: one end
/// where both show alike, `between … and …` otherwise.
fn spanned(plan: &Plan<'_>, read: &Read, name: &str, show: fn(f64) -> String) -> Option<String> {
    let (lower, upper) = match read.values.get(name) {
        Some(Value::Number { value, .. }) => (value.lower, value.upper),
        Some(_) => return None,
        None => {
            let value = plan.constants.get(name).and_then(Constant::number)?;
            (value, value)
        }
    };
    let (low, high) = (show(lower), show(upper));
    Some(if low == high {
        low
    } else {
        format!("between {low} and {high}")
    })
}

/// An area in square metres, rounded to the micrometre squared: `7.5 m²`.
fn square_metres(value: f64) -> String {
    format!("{} m²", (value * 1e6).round() / 1e6)
}

/// A share as a percentage, rounded to two decimals: `87.29%`.
fn percent(share: f64) -> String {
    format!("{}%", (share * 1e4).round() / 1e2)
}

/// A number shown by a numeric format: `fixedN` (a point, N decimals),
/// `lowerN` or `upperN` (that end), `hundredN` (the lower end a
/// hundredfold, N decimals) or `hundred` (a point a hundredfold, as Rust
/// shows it); `None` for any other format, or `fixed` or `hundred` of an
/// interval.
pub(super) fn formatted(format: &str, (lower, upper): (f64, f64)) -> Option<String> {
    if let Some(places) = format.strip_prefix("hundred") {
        if places.is_empty() {
            return (lower.to_bits() == upper.to_bits()).then(|| (lower * 100.0).to_string());
        }
        let places = places.parse::<usize>().ok().filter(|places| *places <= 9)?;
        return Some(format!("{:.places$}", lower * 100.0));
    }
    let (end, places) = decimals(format)?;
    match end {
        "fixed" if lower.to_bits() != upper.to_bits() => None,
        "lower" | "fixed" => Some(format!("{lower:.places$}")),
        _ => Some(format!("{upper:.places$}")),
    }
}

/// A format showing a number with a fixed number of decimals, `fixed3`,
/// `lower4` or `upper3`: which end of the value it shows, and how many.
fn decimals(format: &str) -> Option<(&str, usize)> {
    ["fixed", "lower", "upper"].into_iter().find_map(|end| {
        format
            .strip_prefix(end)
            .and_then(|places| places.parse::<usize>().ok())
            .filter(|places| *places <= 9)
            .map(|places| (end, places))
    })
}

/// Whether a text's condition holds.
fn holds(plan: &Plan<'_>, read: &Read, condition: Option<Condition>) -> bool {
    holds_over(&plan.constants, read, condition)
}

/// Whether `condition` holds over `constants` and the values `read`.
fn holds_over(
    constants: &BTreeMap<String, Constant>,
    read: &Read,
    condition: Option<Condition>,
) -> bool {
    let number = |parameter: &str| constants.get(parameter).and_then(Constant::number);
    let value = |name: &str| match read.values.get(name) {
        Some(Value::Number { value, .. }) => Some((value.lower, value.upper)),
        _ => None,
    };
    match condition {
        None => true,
        Some(Condition::Positive { parameter }) => {
            number(parameter).is_some_and(|value| value > 0.0)
        }
        Some(Condition::Inexact { value }) => read.inexact.get(value).is_some(),
        Some(Condition::Equals { parameter, value }) => matches!(
            constants.get(parameter),
            Some(Constant::Text(stated)) if stated == value
        ),
        Some(Condition::OneOf { parameter, values }) => matches!(
            constants.get(parameter),
            Some(Constant::Text(stated)) if values.contains(&stated.as_str())
        ),
        Some(Condition::Zero { value: name }) => value(name).is_some_and(|(lower, _)| lower == 0.0),
        Some(Condition::All { conditions }) => conditions
            .iter()
            .all(|condition| holds_over(constants, read, Some(*condition))),
        Some(Condition::Not { condition }) => !holds_over(constants, read, Some(*condition)),
        Some(Condition::Cites { value }) => read
            .related
            .iter()
            .any(|(name, objects)| *name == value && !objects.is_empty()),
        Some(Condition::Absent { value }) => matches!(read.values.get(value), Some(Value::Null)),
        Some(Condition::Below { value: name, than }) => {
            value(name).is_some_and(|(_, upper)| upper < than)
        }
        Some(Condition::Above { value: name, than }) => {
            value(name).is_some_and(|(lower, _)| lower > than)
        }
        Some(Condition::Lists { parameter, value }) => matches!(
            constants.get(parameter),
            Some(Constant::Other(ParameterValue::StringList { value: listed }))
                if listed.iter().any(|listed| listed.trim() == value)
        ),
        Some(Condition::Stated { parameter }) => constants.contains_key(parameter),
        Some(Condition::AtLeast { parameter, than }) => {
            number(parameter).is_some_and(|value| value >= than)
        }
        Some(Condition::Under { parameter, than }) => {
            number(parameter).is_some_and(|value| value < than)
        }
        Some(Condition::Exceeds {
            value: name,
            parameter,
            end,
        }) => match (value(name), number(parameter)) {
            (Some((lower, upper)), Some(threshold)) => {
                (if end == End::Lower { lower } else { upper }) > threshold
            }
            _ => false,
        },
        Some(Condition::Measured { value }) => read.values.get(value).is_some(),
        Some(Condition::Scope { value }) => read.named.get("source").is_some_and(|scope| {
            sources(read, value)
                .iter()
                .any(|source| source.to_string() == *scope)
        }),
        Some(Condition::Noted { value }) => read
            .notes
            .iter()
            .any(|(name, notes)| *name == value && !notes.is_empty()),
    }
}

/// The sources `name`'s measured reads cite.
fn sources<'r>(read: &'r Read, name: &str) -> &'r [axioval_ir::SourceId] {
    read.sources
        .iter()
        .find(|(cited, _)| *cited == name)
        .map_or(&[], |(_, sources)| sources.as_slice())
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
        /// The severity a grading gave it, in place of the rule's own.
        severity: Option<Severity>,
    },
    Open(NotEvaluatedReason, String),
    /// An outcome on another object than the one judged: an item's, on the
    /// object it names ([`axioval_engine::template::Items::at`]).
    Placed(ObjectId, Box<Outcome>),
}

impl Outcome {
    fn finding(message: String, evidence: Vec<Evidence>) -> Self {
        Self::Finding {
            message,
            evidence,
            related: Vec::new(),
            deviation: None,
            severity: None,
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
    /// The further populations (`Members::more`), each its selector
    /// parameter and the objects it picks; an unstated one picks none.
    more: Vec<(&'t str, Option<Population>)>,
    /// What member checks found about members, not yet reported, and the
    /// members already reported, each once however many anchors reach it.
    judged: members::Judged,
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
        let more = members
            .more
            .iter()
            .map(|name| {
                let population = match plan.constants.get(*name) {
                    Some(Constant::Other(ParameterValue::Selector { value: selector })) => {
                        Some(Population::of(context, selector))
                    }
                    _ => None,
                };
                (*name, population)
            })
            .collect();
        Ok(Some(Self {
            members,
            population,
            traversal: Parameters(rule).traversal()?,
            ends,
            more,
            judged: members::Judged::default(),
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
        self.tally_of(context, anchor, &self.population)
    }

    fn tally_of(
        &self,
        context: &RuleContext<'_>,
        anchor: &Object,
        population: &Population,
    ) -> Result<Tally, Unavailable> {
        let tallied = tally(context, self.traversal.as_ref(), anchor, population)?;
        match &self.ends {
            Some(ends) => same_ends(context, ends, anchor, tallied),
            None => Ok(tallied),
        }
    }

    /// Whether undecided members widen the aggregates over them.
    fn widens(&self) -> bool {
        matches!(self.members.undecided, UndecidedMembers::Widen)
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
    /// Each of the form's checks' outcomes, in order, before `outcome`.
    checks: Vec<Outcome>,
}

impl From<Outcome> for Judgement {
    fn from(outcome: Outcome) -> Self {
        Self {
            outcome,
            row: None,
            checks: Vec::new(),
        }
    }
}

/// The objects the measured reads of the value `name` were measured
/// against, in the order cited, each once.
fn cited(read: &Read, name: &str) -> Vec<ObjectId> {
    let mut related: Vec<ObjectId> = Vec::new();
    // Each once, in the order cited: a source's walls may be many.
    let mut seen: std::collections::BTreeSet<&ObjectId> = std::collections::BTreeSet::new();
    let cited = read
        .related
        .iter()
        .filter(|(read, _)| *read == name)
        .flat_map(|(_, objects)| objects);
    for object in cited {
        if seen.insert(object) {
            related.push(object.clone());
        }
    }
    related
}

/// What an object's values were read ahead as.
struct Ahead {
    prefetched: Prefetch,
    bound: Vec<BoundPrefetched>,
}

/// The form's `unless` values of `object`, each read where it applies:
/// the outcome where one passes the object or leaves it open, `None` where
/// the object is judged on.
fn unless(
    plan: &Plan<'_>,
    context: &RuleContext<'_>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
) -> Option<Outcome> {
    let applying = plan.bound.unless_applies.get_or_init(|| {
        plan.form
            .unless
            .iter()
            .map(|unless| each::applies(plan, &unless.applies))
            .collect()
    });
    for (index, unless) in plan.form.unless.iter().enumerate() {
        if !applying.get(index).copied().unwrap_or(false) {
            continue;
        }
        let mut read = Read::default();
        if let Some(outcome) = read_values(
            plan,
            std::iter::once((&unless.value, &plan.bound.unless[index])),
            &|_| false,
            context,
            object,
            leaves,
            &mut read,
        ) {
            return Some(outcome);
        }
        // A guard read is all it asks.
        if unless.guard {
            continue;
        }
        match read.values.get(unless.value.name) {
            Some(Value::Boolean(true)) => return Some(Outcome::Passed),
            Some(Value::Number { value, .. }) if value.lower > 0.0 || value.upper < 0.0 => {
                return Some(Outcome::Passed);
            }
            Some(Value::Boolean(false)) => {}
            Some(Value::Number { value, .. }) if value.lower == 0.0 && value.upper == 0.0 => {}
            _ => {
                return Some(Outcome::Open(
                    NotEvaluatedReason::IncompleteEvidence,
                    render(plan, &read, plan.form.undecided),
                ));
            }
        }
    }
    None
}

/// Grades a finding the form (`index` 0) or its check `index − 1` is about
/// to report: reads the grading's values into `read`, `null` kept, and
/// returns the severity of the first band that holds; `Err` the outcome
/// where a value cannot be read.
fn grade(
    plan: &Plan<'_>,
    index: usize,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
    read: &mut Read,
) -> Result<Option<Severity>, Outcome> {
    let grading = if index == 0 {
        plan.form.grading.as_ref()
    } else {
        checked_grading(plan, index)
    };
    let Some(grading) = grading else {
        return Ok(None);
    };
    read_graded(
        grading.values.iter().zip(&plan.bound.grading[index]),
        object,
        leaves,
        read,
    )?;
    if let Some(outcome) = derive_of(plan, &grading.derived, read) {
        return Err(outcome);
    }
    Ok(grading
        .bands
        .iter()
        .find(|band| band.when.is_none() || holds(plan, read, band.when))
        .map(|band| band.severity.clone()))
}

/// Reads the values the form's grading (`index` 0) or its check's
/// (`index − 1`) reads to word an undecided outcome, if any.
fn word_undecided(
    plan: &Plan<'_>,
    index: usize,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
    read: &mut Read,
) -> Result<(), Outcome> {
    let grading = if index == 0 {
        plan.form.grading.as_ref()
    } else {
        checked_grading(plan, index)
    };
    let Some(grading) = grading else {
        return Ok(());
    };
    read_graded(
        grading
            .undecided
            .iter()
            .zip(&plan.bound.grading[index][grading.values.len()..]),
        object,
        leaves,
        read,
    )
}

/// Reads grading values into `read`, a `null` kept as one: `Err` the
/// outcome where one cannot be read.
fn read_graded<'v>(
    values: impl Iterator<Item = (&'v TemplateValue, &'v Expression)>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
    read: &mut Read,
) -> Result<(), Outcome> {
    for (step, expression) in values {
        let (outcome, evidence) = read_step(expression, step.name, leaves);
        let cited = leaves.take_related();
        if !cited.is_empty() {
            read.related.push((step.name, cited));
        }
        let noted = leaves.take_notes();
        if !noted.is_empty() {
            read.notes.push((step.name, noted));
        }
        let before = read.evidence.len();
        read.evidence.extend(evidence);
        if read.evidence[before..]
            .iter()
            .any(|evidence| !evidence.exact)
        {
            read.inexact.insert(step.name, ());
        }
        match outcome {
            Ok(value) => read.values.insert(step.name, value),
            Err(why) => {
                let reason = leaves
                    .first_reason()
                    .filter(|_| matches!(why.reason, Reason::Unreadable(_)))
                    .unwrap_or_else(|| reason_of(&why));
                let message = match why.reason {
                    Reason::Unreadable(message) => refusal(message, expression, object),
                    other => other.to_string(),
                };
                return Err(Outcome::Open(reason, message));
            }
        }
    }
    Ok(())
}

/// Each of the form's checks judged on its own over `read` and its own
/// values: an outcome per check, in order. A check's value that cannot be
/// read leaves only that check open.
fn judge_checks(
    plan: &Plan<'_>,
    read: &Read,
    context: &RuleContext<'_>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
) -> Vec<Outcome> {
    judge_checks_in(
        plan,
        (&plan.form.checks, &plan.bound.checks),
        (read, None),
        context,
        object,
        leaves,
    )
}

/// Whether each of the form's checks applies over `read`: what
/// [`judge_checks_alike`] judges every object by, decided once.
fn checks_applying(plan: &Plan<'_>, read: &Read) -> Vec<bool> {
    plan.form
        .checks
        .iter()
        .map(|check| {
            check
                .applies
                .as_ref()
                .is_none_or(|applies| each::applies_reading(plan, applies, read))
        })
        .collect()
}

/// [`judge_checks`] of objects judged over one `read` alike, which checks
/// apply decided once ([`checks_applying`]).
fn judge_checks_alike(
    plan: &Plan<'_>,
    (read, applying): (&Read, &[bool]),
    context: &RuleContext<'_>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
) -> Vec<Outcome> {
    judge_checks_in(
        plan,
        (&plan.form.checks, &plan.bound.checks),
        (read, Some(applying)),
        context,
        object,
        leaves,
    )
}

/// [`judge_checks`] of `checks`, their values bound in `values`.
#[allow(clippy::too_many_lines)]
fn judge_checks_in(
    plan: &Plan<'_>,
    (checks, values): (&[axioval_engine::template::FormCheck], &[Vec<Expression>]),
    (read, applying): (&Read, Option<&[bool]>),
    context: &RuleContext<'_>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let offset = graded_from(plan, checks);
    // Each check reads on from the object's read, in one buffer.
    let mut checked = Read::default();
    for (position, (check, bound)) in checks.iter().zip(values).enumerate() {
        let index = offset.map(|offset| position + offset);
        let applies = match applying.and_then(|applying| applying.get(position)) {
            Some(applies) => *applies,
            None => check
                .applies
                .as_ref()
                .is_none_or(|applies| each::applies_reading(plan, applies, read)),
        };
        if !applies {
            continue;
        }
        checked.clone_from(read);
        if let Some(outcome) = read_values(
            plan,
            check.values.iter().zip(bound),
            &|_| false,
            context,
            object,
            leaves,
            &mut checked,
        ) {
            // Another check reading the value reports its refusal.
            if !(check.quiet && matches!(outcome, Outcome::Open(..))) {
                outcomes.push(outcome);
            }
            continue;
        }
        // Where its condition holds over the values, the check passes
        // without deciding.
        if check.unless.is_some() && holds(plan, &checked, check.unless) {
            continue;
        }
        if let Some(outcome) = derive_of(plan, &check.derived, &mut checked) {
            outcomes.push(outcome);
            continue;
        }
        if let Decision::Items(judged) = &check.decision {
            let open_already = judged.once
                && outcomes
                    .iter()
                    .any(|outcome| matches!(outcome, Outcome::Open(..)));
            let found = items::judge_items(plan, judged, &checked, open_already, object, leaves);
            // The check's grading grades its items' findings too.
            let found = match index {
                Some(index) if check.grading.is_some() => {
                    graded_items(plan, index, found, object, leaves, &mut checked)
                }
                _ => found,
            };
            if judged.once {
                // Left open once: the first open outcome, and none after an
                // earlier check's.
                let mut open = outcomes
                    .iter()
                    .any(|outcome| matches!(outcome, Outcome::Open(..)));
                for outcome in found {
                    if matches!(outcome, Outcome::Open(..)) {
                        if open {
                            continue;
                        }
                        open = true;
                    }
                    outcomes.push(outcome);
                }
            } else {
                outcomes.extend(found);
            }
            continue;
        }
        let decision = effective_of(plan, &check.decision);
        let judged = match &decision {
            Decision::Near {
                value,
                reference: axioval_engine::template::Reference::Value(reference),
                tolerance,
            } => near_judged(plan, &checked, (value, reference), *tolerance),
            _ => within(plan, &checked, &decision),
        };
        let Some(judged) = judged else {
            outcomes.push(Outcome::Open(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "{}: a value the decision reads is no number",
                    plan.template.name
                ),
            ));
            continue;
        };
        checked.bounds = Some((judged.minimum, judged.maximum));
        let related = check
            .related
            .map(|value| cited(&checked, value))
            .unwrap_or_default();
        let severity = match (&judged.verdict, index) {
            (Verdict::Fail(_), Some(index)) => {
                match grade(plan, index, object, leaves, &mut checked) {
                    Ok(severity) => severity,
                    Err(outcome) => {
                        outcomes.push(outcome);
                        continue;
                    }
                }
            }
            (Verdict::Undecided(_), Some(index)) => {
                if let Err(outcome) = word_undecided(plan, index, object, leaves, &mut checked) {
                    outcomes.push(outcome);
                    continue;
                }
                None
            }
            _ => None,
        };
        let outcome = ranged_as(
            plan,
            &mut checked,
            &judged,
            related,
            (check.fail, check.undecided),
        );
        let outcome = graded(outcome, severity);
        outcomes.push(if check.ungraded {
            ungraded(outcome)
        } else {
            outcome
        });
    }
    outcomes
}

/// Where the gradings of `checks` begin among the form's (its own at 0):
/// the form's own checks are graded, and its checks of the project after
/// them; checks of an item are not.
fn graded_from(plan: &Plan<'_>, checks: &[axioval_engine::template::FormCheck]) -> Option<usize> {
    if std::ptr::eq(checks, plan.form.checks.as_slice()) {
        Some(1)
    } else if std::ptr::eq(checks, plan.form.project.as_slice()) {
        Some(1 + plan.form.checks.len())
    } else {
        None
    }
}

/// The grading of the form's check `index − 1`, its checks of the project
/// numbered after them.
fn checked_grading<'p>(
    plan: &'p Plan<'_>,
    index: usize,
) -> Option<&'p axioval_engine::template::Grading> {
    let checks = plan.form.checks.len();
    if index <= checks {
        plan.form.checks[index - 1].grading.as_ref()
    } else {
        plan.form.project[index - 1 - checks].grading.as_ref()
    }
}

/// `outcome` stating no deviation.
fn ungraded(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Finding {
            message,
            evidence,
            related,
            severity,
            ..
        } => Outcome::Finding {
            message,
            evidence,
            related,
            deviation: None,
            severity,
        },
        Outcome::Placed(placed, outcome) => Outcome::Placed(placed, Box::new(ungraded(*outcome))),
        outcome => outcome,
    }
}

/// The findings of a check's items graded by the check's grading (`index`
/// its place, the form's 0), or the outcome leaving it open where its
/// values cannot be read.
fn graded_items(
    plan: &Plan<'_>,
    index: usize,
    found: Vec<Outcome>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
    read: &mut Read,
) -> Vec<Outcome> {
    let found_one = |outcome: &Outcome| match outcome {
        Outcome::Finding { .. } => true,
        Outcome::Placed(_, outcome) => matches!(**outcome, Outcome::Finding { .. }),
        _ => false,
    };
    if !found.iter().any(found_one) {
        return found;
    }
    match grade(plan, index, object, leaves, read) {
        Ok(severity) => found
            .into_iter()
            .map(|outcome| graded(outcome, severity.clone()))
            .collect(),
        Err(outcome) => vec![outcome],
    }
}

/// `outcome` with the severity a grading gave it, if any.
fn graded(outcome: Outcome, severity: Option<Severity>) -> Outcome {
    match (outcome, severity) {
        (
            Outcome::Finding {
                message,
                evidence,
                related,
                deviation,
                ..
            },
            Some(severity),
        ) => Outcome::Finding {
            message,
            evidence,
            related,
            deviation,
            severity: Some(severity),
        },
        // An outcome placed on another object, graded alike.
        (Outcome::Placed(placed, outcome), severity) => {
            Outcome::Placed(placed, Box::new(graded(*outcome, severity)))
        }
        (outcome, _) => outcome,
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
#[allow(clippy::too_many_lines)]
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
        let cited = leaves.take_related();
        if !cited.is_empty() {
            read.related.push((step.name, cited));
        }
        let noted = leaves.take_notes();
        if !noted.is_empty() {
            read.notes.push((step.name, noted));
        }
        let sources = leaves.take_sources();
        if !sources.is_empty() {
            read.sources.push((step.name, sources));
        }
        let before = read.evidence.len();
        if evidence.iter().any(|evidence| !evidence.exact) {
            read.inexact.insert(step.name, ());
        }
        // A value read only to word the outcome cites nothing of its own.
        if step.expect != Some(Expect::Words) {
            read.evidence.extend(evidence);
        }
        let stated = property_read(expression)
            .and_then(|(set, name)| leaves.stated(set, name))
            .map(|stated| stated.0);
        let (was_stated, states_value) = (stated.is_some(), matches!(stated, Some(Some(_))));
        // Kept once, and read back where a kind is expected.
        if let Some(value) = stated {
            read.stated.insert(step.name, value);
        }
        // The value a comparison judges is judged as the source states it:
        // an absence, `null` and a value of any kind reach the comparison.
        if as_stated(step.name) {
            if was_stated {
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
            // Nothing to judge.
            Ok(Value::Null) if step.expect == Some(Expect::Optional) => {
                return Some(Outcome::Passed);
            }
            // A value stated `null` where a kind is expected is of the wrong
            // kind; an absence is `null` the decision judges.
            Ok(Value::Null) if read.keep_null => {
                let mismatched = step.expect.is_some_and(|expect| {
                    states_value
                        && !expected(
                            expect,
                            &Value::Null,
                            read.stated.get(step.name).filter(|_| was_stated),
                        )
                });
                read.values.insert(step.name, Value::Null);
                if mismatched {
                    return Some(mismatch(plan, read, step));
                }
            }
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
                let mismatched = step.expect.is_some_and(|expect| {
                    !expected(
                        expect,
                        &value,
                        read.stated.get(step.name).filter(|_| was_stated),
                    )
                });
                read.values.insert(step.name, value);
                if mismatched {
                    return Some(mismatch(plan, read, step));
                }
            }
            // Stated, but no single value of any kind: not the kind the
            // step needs, worded as the source states it.
            Err(_) if step.expect.is_some() && states_value => {
                return Some(mismatch(plan, read, step));
            }
            Err(why) => {
                let reason = leaves
                    .first_reason()
                    .filter(|_| matches!(why.reason, Reason::Unreadable(_)))
                    .unwrap_or_else(|| reason_of(&why));
                let why = match why.reason {
                    Reason::Unreadable(message) => refusal(message, expression, object),
                    other => other.to_string(),
                };
                let message = if let Some(refused) = step.refused {
                    read.why = Some(why);
                    render(plan, read, refused)
                } else {
                    read.why = None;
                    why
                };
                return Some(Outcome::Open(reason, message));
            }
        }
    }
    None
}

/// Derives the form's `derived` values from those read, in plain binary
/// arithmetic over their intervals: the outcome where one leaves the
/// object open.
fn derive(plan: &Plan<'_>, read: &mut Read) -> Option<Outcome> {
    derive_of(plan, &plan.form.derived, read)
}

/// [`derive`] of `derived`.
fn derive_of(plan: &Plan<'_>, derived: &[Derived], read: &mut Read) -> Option<Outcome> {
    let span = |read: &Read, name: &str| match read.values.get(name) {
        Some(Value::Number { value, unit }) => Some((value.lower, value.upper, unit.clone())),
        _ => None,
    };
    for derived in derived {
        let (name, lower, upper, unit) = match derived {
            Derived::Difference(difference) => {
                let (Some(minuend), Some(subtrahend)) = (
                    span(read, difference.minuend),
                    span(read, difference.subtrahend),
                ) else {
                    continue;
                };
                (
                    difference.name,
                    minuend.0 - subtrahend.1,
                    minuend.1 - subtrahend.0,
                    minuend.2,
                )
            }
            Derived::Ratio {
                name,
                numerator,
                denominator,
                zero,
            } => {
                let (Some(top), Some(bottom)) = (span(read, numerator), span(read, denominator))
                else {
                    continue;
                };
                if bottom.1 <= 0.0 {
                    return Some(Outcome::Open(
                        NotEvaluatedReason::IncompleteEvidence,
                        render(plan, read, zero),
                    ));
                }
                let upper = if bottom.0 > 0.0 {
                    top.1 / bottom.0
                } else {
                    f64::INFINITY
                };
                (
                    *name,
                    top.0 / bottom.1,
                    upper,
                    axioval_engine::expression::Unit::NONE,
                )
            }
            Derived::Open { name, value, open } => {
                let Some(summed) = span(read, value) else {
                    continue;
                };
                let unread = span(read, open).is_some_and(|(lower, _, _)| lower > 0.0);
                (
                    *name,
                    summed.0,
                    if unread { f64::INFINITY } else { summed.1 },
                    summed.2,
                )
            }
        };
        read.values.insert(
            name,
            Value::Number {
                value: axioval_engine::expression::Interval { lower, upper },
                unit,
            },
        );
    }
    None
}

/// Whether `decision` judges the value `name` as the source states it,
/// rather than as the evaluator reads it.
fn judges_stated(decision: &Decision, name: &str) -> bool {
    match decision {
        Decision::Compare { value, .. } | Decision::Unique { value, .. } => *value == name,
        Decision::Consistent { key, value, .. } => *key == name || *value == name,
        Decision::Proportion(decided) => decided
            .groups
            .as_ref()
            .is_some_and(|groups| groups.value == name),
        _ => false,
    }
}

/// Reads the plan's values for `object` and decides.
#[allow(clippy::too_many_lines)]
fn judge_object(
    (plan, decision, scope): (&Plan<'_>, &Decision, Option<&Scope<'_>>),
    (context, arguments): (&RuleContext<'_>, &Arguments),
    rule: &CompiledRule,
    (object, once): (&Object, &Read),
    ahead: Ahead,
) -> Judgement {
    // Values composing reads (a truth over properties other values read)
    // read them again: keep each once resolved.
    let composed = plan
        .form
        .values
        .iter()
        .any(|step| property_read(&step.expression).is_none());
    let mut leaves = ObjectLeaves::read_ahead(
        context,
        (object, &plan.bound.parameters),
        arguments,
        (ahead.prefetched, ahead.bound),
        composed,
    );
    if let Some(outcome) = unless(plan, context, object, &mut leaves) {
        return outcome.into();
    }
    // What was read once per rule, every object's messages read too.
    let mut read = once.clone();
    let mut members = None;
    // Each population's members surely picked, the first population's first.
    let mut populations: Vec<(&str, Vec<ObjectId>)> = Vec::new();
    if let Some(scope) = scope {
        match scope.tally(context, object) {
            Ok(mut tally) => {
                populations.push((scope.members.selector, tally.decided.clone()));
                read.named.insert("relation", scope.relation());
                // Undecided members widen the aggregate only where the
                // form says they may.
                let possible: &[ObjectId] = if scope.widens() { &tally.possible } else { &[] };
                leaves = leaves.supplying(
                    Members::source(scope.members.selector),
                    candidates(context, &tally.decided, possible),
                );
                // Further populations, reached alike.
                for (name, population) in &scope.more {
                    let further = match population {
                        Some(population) => match scope.tally_of(context, object, population) {
                            Ok(further) => further,
                            Err((reason, message)) => {
                                return Outcome::Open(reason, message).into();
                            }
                        },
                        None => Tally {
                            decided: Vec::new(),
                            undecided: 0,
                            possible: Vec::new(),
                            evidence: Vec::new(),
                        },
                    };
                    let possible: &[ObjectId] = if scope.widens() {
                        &further.possible
                    } else {
                        &[]
                    };
                    leaves = leaves.supplying(
                        Members::source(name),
                        candidates(context, &further.decided, possible),
                    );
                    populations.push((name, further.decided.clone()));
                    tally.undecided += further.undecided;
                    tally.decided.extend(further.decided);
                    tally.possible.extend(further.possible);
                    tally.evidence.extend(further.evidence);
                }
                read.named.insert("undecided", tally.undecided.to_string());
                if tally.undecided > 0
                    && let UndecidedMembers::Open { message } = &scope.members.undecided
                {
                    // A value over members that may be there is never judged.
                    return Outcome::Open(
                        NotEvaluatedReason::IncompleteEvidence,
                        render(plan, &read, message),
                    )
                    .into();
                }
                members = Some(tally);
            }
            Err((reason, message)) => return Outcome::Open(reason, message).into(),
        }
    }
    // A truth judge decides what a stated absence means itself.
    let truth = match decision {
        Decision::Holds { value } => {
            read.keep_null = true;
            Some(*value)
        }
        _ => None,
    };
    for (index, value) in plan.values().enumerate() {
        // The values after a truth only word its failure.
        if let Some(truth) = truth
            && plan.form.values[..index]
                .iter()
                .any(|step| step.name == truth)
            && !matches!(read.values.get(truth), Some(Value::Boolean(false)))
        {
            break;
        }
        // Members judged on their own before this value, where a check
        // says so; an anchor with a member found is open once it is read.
        let found = match (scope, populations.first()) {
            (Some(scope), Some((_, first))) => members::judge(
                plan,
                scope,
                (context, arguments),
                rule,
                object,
                first,
                index,
            ),
            _ => None,
        };
        if let Some(outcome) = read_values(
            plan,
            std::iter::once(value),
            &|name| judges_stated(decision, name),
            context,
            object,
            &mut leaves,
            &mut read,
        ) {
            // A finding on a stated absence cites what reached the members.
            return match (outcome, &members) {
                (
                    Outcome::Finding {
                        message,
                        mut evidence,
                        related,
                        deviation,
                        severity,
                    },
                    Some(tally),
                ) => {
                    evidence.extend(tally.evidence.iter().cloned());
                    Outcome::Finding {
                        message,
                        evidence,
                        related,
                        deviation,
                        severity,
                    }
                }
                (outcome, _) => outcome,
            }
            .into();
        }
        if let Some((message, failed, first)) = found {
            read.named.insert("failed", failed.to_string());
            read.named.insert("first", first.to_string());
            return Outcome::Open(
                NotEvaluatedReason::InvalidEvidence,
                render(plan, &read, message),
            )
            .into();
        }
    }
    if let Some(outcome) = derive(plan, &mut read) {
        return outcome.into();
    }
    let undecided = members.as_ref().map_or(0, |members| members.undecided);
    let related = match plan.form.related {
        Some(value) => match value.strip_prefix("members:") {
            Some(selector) => populations
                .iter()
                .find(|(name, _)| *name == selector)
                .map(|(_, decided)| decided.clone())
                .unwrap_or_default(),
            None => cited(&read, value),
        },
        None => members
            .as_ref()
            .map(|members| members.decided.clone())
            .unwrap_or_default(),
    };
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
    let checks = judge_checks(plan, &read, context, object, &mut leaves);
    if let Decision::Proportion(decided) = decision {
        let outcome = proportion::judged(plan, read, (decided.provided, decided.required), related)
            .unwrap_or_else(|| {
                Outcome::Open(
                    NotEvaluatedReason::InvalidEvidence,
                    format!(
                        "{}: a count the decision reads is no count",
                        plan.template.name
                    ),
                )
            });
        return Judgement {
            outcome,
            row,
            checks,
        };
    }
    // The object judged by the words of its own list's items.
    if let Decision::Joined(joined) = decision {
        return Judgement {
            outcome: joined::judge(plan, joined, &mut read, &leaves),
            row,
            checks,
        };
    }
    if let Decision::Holds { value } = decision {
        let outcome = match read.values.get(value) {
            Some(Value::Boolean(true)) => Outcome::Passed,
            Some(Value::Boolean(false)) => Outcome::Finding {
                message: render(plan, &read, plan.form.fail),
                evidence: read.evidence,
                related,
                deviation: None,
                severity: None,
            },
            Some(Value::Null) => Outcome::Open(
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, plan.form.undecided),
            ),
            _ => Outcome::Open(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "{}: the value the decision reads is no truth",
                    plan.template.name
                ),
            ),
        };
        return Judgement {
            outcome,
            row,
            checks,
        };
    }
    if let (Decision::Compare { value, .. }, Some(comparison)) = (decision, &plan.comparison) {
        let stated = read.stated.get(value).cloned().flatten();
        let outcome = match comparison.holds(stated.as_ref()) {
            Ok(true) => Outcome::Passed,
            Ok(false) => Outcome::finding(render(plan, &read, plan.form.fail), read.evidence),
            Err(why) => Outcome::Open(NotEvaluatedReason::InvalidEvidence, why),
        };
        return Judgement {
            outcome,
            row,
            checks,
        };
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
            checks,
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
                checks,
            };
        }
    }
    let severity = match judged.verdict {
        Verdict::Fail(_) => match grade(plan, 0, object, &mut leaves, &mut read) {
            Ok(severity) => severity,
            Err(outcome) => {
                return Judgement {
                    outcome,
                    row,
                    checks,
                };
            }
        },
        Verdict::Undecided(_) => {
            if let Err(outcome) = word_undecided(plan, 0, object, &mut leaves, &mut read) {
                return Judgement {
                    outcome,
                    row,
                    checks,
                };
            }
            None
        }
        Verdict::Pass => None,
    };
    let outcome = graded(ranged(plan, &mut read, &judged, related), severity);
    Judgement {
        outcome,
        row,
        checks,
    }
}

/// What a range judge's verdict comes to, worded with the form's
/// messages: a finding relating `related` (graded where the template
/// grades), or the object open naming the bound it straddles.
fn ranged(plan: &Plan<'_>, read: &mut Read, judged: &Judged, related: Vec<ObjectId>) -> Outcome {
    ranged_as(
        plan,
        read,
        judged,
        related,
        (plan.form.fail, plan.form.undecided),
    )
}

/// [`ranged`], worded with `fail` and `undecided`.
///
/// It takes what it words from `read` (its evidence), leaving the rest:
/// a read is kept in place rather than copied into each verdict.
fn ranged_as(
    plan: &Plan<'_>,
    read: &mut Read,
    judged: &Judged,
    related: Vec<ObjectId>,
    (fail, undecided): (&str, &str),
) -> Outcome {
    match &judged.verdict {
        Verdict::Pass => Outcome::Passed,
        Verdict::Fail(bound) => {
            read.bound = Some(bound.clone());
            Outcome::Finding {
                message: render(plan, read, fail),
                evidence: std::mem::take(&mut read.evidence),
                related,
                deviation: if plan.template.grades {
                    deviation(judged.lower, judged.upper, judged.minimum, judged.maximum)
                } else {
                    None
                },
                severity: None,
            }
        }
        Verdict::Undecided(bound) => {
            read.bound = Some(bound.clone());
            Outcome::Open(
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, read, undecided),
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
            let id = render(plan, &Read::default(), column.id);
            if column.dimension == axioval_engine::template::NUMBER {
                ReportColumn::number(id)
            } else {
                ReportColumn::quantity(id, column.dimension)
            }
        })
        .collect();
    ReportTable::new(rule.id.clone(), table.name, columns).ok()
}

/// A refusal of the rule's declaration or of the host's services, reported
/// where the template says: for the rule as a whole, or for each selected
/// object.
fn refused(
    template: &Template,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    reason: NotEvaluatedReason,
    message: String,
) -> CapabilityEvaluation {
    match template.refusals {
        Refusals::Rule | Refusals::Worded | Refusals::Selected => {
            CapabilityEvaluation::not_evaluated(reason, message)
        }
        Refusals::Objects | Refusals::Prefixed { .. } | Refusals::ServicesPerObject => {
            let (selected, mut evaluation) = select_shared(context, &rule.selector);
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
            }
            evaluation
        }
    }
}

/// Pushes one outcome about `object` into `evaluation`.
fn push(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    object: &Object,
    outcome: Outcome,
) {
    push_on(evaluation, rule, &object.id, outcome);
}

/// Pushes one outcome about the object `id` into `evaluation`.
fn push_on(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    id: &ObjectId,
    outcome: Outcome,
) {
    match outcome {
        Outcome::Passed => {}
        Outcome::Finding {
            message,
            evidence,
            related,
            deviation,
            severity,
        } => {
            let mut cited: Vec<Evidence> = Vec::with_capacity(evidence.len());
            for evidence in evidence {
                if !cited.contains(&evidence) {
                    cited.push(evidence);
                }
            }
            let mut found = finding(rule, id, message, cited, related);
            if let Some(severity) = severity {
                found.severity = severity;
            }
            evaluation.push_finding_deviating(found, deviation);
        }
        Outcome::Open(reason, message) => {
            evaluation.push_object_not_evaluated(id.clone(), reason, message);
        }
        Outcome::Placed(placed, outcome) => push_on(evaluation, rule, &placed, *outcome),
    }
}

/// Judges the form's checks of the project ([`Form::project`]) once for
/// the rule: an outcome its items place on an object there, any other the
/// rule's own.
fn judge_project(
    plan: &Plan<'_>,
    (context, arguments): (&RuleContext<'_>, &Arguments),
    rule: &CompiledRule,
    evaluation: &mut CapabilityEvaluation,
) {
    if plan.form.project.is_empty() {
        return;
    }
    let project = project_object();
    let mut leaves = ObjectLeaves::new(context, &project, Some(&plan.bound.parameters))
        .with_arguments(arguments);
    let outcomes = judge_checks_in(
        plan,
        (&plan.form.project, &plan.bound.project),
        (&Read::default(), None),
        context,
        &project,
        &mut leaves,
    );
    for outcome in outcomes {
        match outcome {
            Outcome::Passed => {}
            Outcome::Placed(placed, outcome) => push_on(evaluation, rule, &placed, *outcome),
            Outcome::Open(reason, message) => evaluation.push_not_evaluated(reason, message),
            Outcome::Finding { message, .. } => evaluation.push_not_evaluated(
                NotEvaluatedReason::InvalidEvidence,
                format!(
                    "{}: a finding of the project names no object: {message}",
                    plan.template.name
                ),
            ),
        }
    }
}

/// The project as the object a value of the project is read of: no object
/// of the model, so a value of the project reads none.
fn project_object() -> Object {
    Object::new(
        ObjectId::new(
            axioval_ir::SourceId::new("axioval", "project").expect("a valid source id"),
            axioval_engine::template::SELECTION,
        )
        .expect("a valid object id"),
        axioval_engine::template::SELECTION,
    )
}

/// What a run read before judging scopes and objects: the values read once
/// per rule, the rule's open outcomes their refusals left, and the
/// selection a template reporting its refusals after selecting made first.
struct Ran<'a> {
    once: Read,
    opened: Vec<(NotEvaluatedReason, String)>,
    selected: Option<(Vec<&'a Object>, CapabilityEvaluation)>,
    /// What the rule's measured values bind, shared by the values read
    /// once and those read per object.
    arguments: Arguments,
}

impl<'a> Ran<'a> {
    /// The rule's selection, made now unless it was made first, with the
    /// rule's open outcomes the values read once per rule left.
    fn selection(
        &mut self,
        context: &RuleContext<'a>,
        rule: &CompiledRule,
    ) -> (Vec<&'a Object>, CapabilityEvaluation) {
        let (selected, mut evaluation) = self
            .selected
            .take()
            .unwrap_or_else(|| select_shared(context, &rule.selector));
        for (reason, message) in self.opened.drain(..) {
            evaluation.push_not_evaluated(reason, message);
        }
        (selected, evaluation)
    }
}

/// What the values read once per rule come to: the values and their
/// citations, and the rule's open outcomes their refusals leave.
type ReadOnce = (Read, Vec<(NotEvaluatedReason, String)>);

/// The values the form reads once per rule ([`Form::once`]), of the
/// project, and the rule's open outcomes their refusals leave; a required
/// value's refusal instead, which leaves nothing else judged.
fn read_once(
    plan: &Plan<'_>,
    context: &RuleContext<'_>,
    arguments: &Arguments,
) -> Result<ReadOnce, (NotEvaluatedReason, String)> {
    let mut read = Read::default();
    let mut opened = Vec::new();
    if plan.form.once.is_empty() {
        return Ok((read, opened));
    }
    // The project is no object of the model: a value of the project reads
    // none.
    let project = Object::new(
        ObjectId::new(
            axioval_ir::SourceId::new("axioval", "project").expect("a valid source id"),
            axioval_engine::template::SELECTION,
        )
        .expect("a valid object id"),
        axioval_engine::template::SELECTION,
    );
    // A value read where a list parameter lists a word is read in the
    // order the rule lists the words (each derivation as the rule lists
    // it); any other in the template's order.
    let mut ordered: Vec<(&axioval_engine::template::Once, &Expression)> =
        plan.form.once.iter().zip(&plan.bound.once).collect();
    ordered.sort_by_key(|(once, _)| {
        match once.applies.as_ref().and_then(|applies| applies.condition) {
            Some(Condition::Lists { parameter, value }) => match plan.constants.get(parameter) {
                Some(Constant::Other(ParameterValue::StringList { value: listed })) => listed
                    .iter()
                    .position(|listed| listed.trim() == value)
                    .unwrap_or(usize::MAX),
                _ => usize::MAX,
            },
            _ => usize::MAX,
        }
    });
    for (once, expression) in ordered {
        if let Some(applies) = &once.applies
            && !each::applies(plan, applies)
        {
            continue;
        }
        // Leaves of their own, so a refusal is worded by its own reason.
        let mut leaves = ObjectLeaves::new(context, &project, Some(&plan.bound.parameters))
            .with_arguments(arguments);
        // A measured value is read with what it cites, whatever it names.
        if let Some((Some(axioval_ir::MEASURED_SET), name)) = property_read(expression) {
            let leaf = leaves.measured_cited(name);
            let related = leaves.take_related();
            let sources = leaves.take_sources();
            match leaf.value {
                Ok(value) => {
                    read.values.insert(once.value.name, value);
                    if !related.is_empty() {
                        read.related.push((once.value.name, related));
                    }
                    if !sources.is_empty() {
                        read.sources.push((once.value.name, sources));
                    }
                }
                Err(why) => {
                    let reason = leaves
                        .first_reason()
                        .unwrap_or(NotEvaluatedReason::IncompleteEvidence);
                    let step = Read {
                        why: Some(refusal(why, expression, &project)),
                        ..Read::default()
                    };
                    let message = render(plan, &step, once.refused);
                    if once.required {
                        return Err((reason, message));
                    }
                    opened.push((reason, message));
                }
            }
            continue;
        }
        let mut step = Read::default();
        if let Some(Outcome::Open(reason, why)) = read_values(
            plan,
            std::iter::once((&once.value, expression)),
            &|_| false,
            context,
            &project,
            &mut leaves,
            &mut step,
        ) {
            step.why = Some(why);
            let message = render(plan, &step, once.refused);
            if once.required {
                return Err((reason, message));
            }
            opened.push((reason, message));
            continue;
        }
        for (name, value) in step.values.iter() {
            read.values.insert(name, value.clone());
        }
        read.related.extend(step.related);
        read.sources.extend(step.sources);
    }
    Ok((read, opened))
}

/// A refusal of the rule worded after the template's name, unless the
/// check's message names the capability itself (`horizontal-guard
/// declaration is …`), as a capability wording one refusal so did.
fn named(template: &Template, message: String) -> String {
    if message.starts_with(&format!("{} ", template.name)) {
        message
    } else {
        format!("{}: {message}", template.name)
    }
}

/// Pushes an object's outcomes into `evaluation`: each as it is, or, with
/// a `joined` separator, everything left open as one outcome after the
/// findings, its messages joined in order, for the first one's reason.
fn push_all(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    object: &Object,
    outcomes: impl Iterator<Item = Outcome>,
    joined: Option<&str>,
) {
    let Some(separator) = joined else {
        for outcome in outcomes {
            push(evaluation, rule, object, outcome);
        }
        return;
    };
    let mut open: Vec<(NotEvaluatedReason, String)> = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Open(reason, message) => open.push((reason, message)),
            outcome => push(evaluation, rule, object, outcome),
        }
    }
    if let Some((reason, _)) = open.first() {
        let reason = reason.clone();
        let messages: Vec<String> = open.into_iter().map(|(_, message)| message).collect();
        push(
            evaluation,
            rule,
            object,
            Outcome::Open(reason, messages.join(separator)),
        );
    }
}

/// Runs `template` for `rule`.
#[allow(clippy::too_many_lines)]
pub(crate) fn run(
    (template, plans): (&Template, &Plans),
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    // A template reporting its refusals after selecting selects first, and
    // says nothing more where nothing is selected.
    let mut selected = None;
    if template.refusals == Refusals::Selected {
        let (objects, evaluation) = select_shared(context, &rule.selector);
        if objects.is_empty() {
            return evaluation;
        }
        selected = Some((objects, evaluation));
    }
    let plan = match plan(template, plans, rule) {
        Ok(plan) => plan,
        Err((reason, message)) => {
            // A rule-scoped refusal names the capability; one reported per
            // object is worded as the check states it.
            let message = match template.refusals {
                Refusals::Rule => named(template, message),
                Refusals::Selected => {
                    let (_, mut evaluation) = selected.unwrap_or_default();
                    evaluation.push_not_evaluated(reason, named(template, message));
                    return evaluation;
                }
                Refusals::ServicesPerObject => {
                    return CapabilityEvaluation::not_evaluated(reason, named(template, message));
                }
                Refusals::Objects | Refusals::Worded => message,
                Refusals::Prefixed { prefix } => format!("{prefix}: {message}"),
            };
            return refused(template, context, rule, reason, message);
        }
    };
    if let Some(services) = &template.services
        && services.only.is_none_or(|(parameter, value)| {
            matches!(
                rule.parameters.get(parameter),
                Some(
                    ParameterValue::String { value: stated }
                        | ParameterValue::Enum { value: stated }
                        | ParameterValue::Reference { value: stated }
                ) if stated == value
            )
        })
        && !services
            .needs
            .iter()
            .all(|service| service.registered(context.services))
    {
        if services.whole {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                named(template, services.message.to_owned()),
            );
        }
        if let Some((_, mut evaluation)) = selected {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::MissingService,
                services.message.to_owned(),
            );
            return evaluation;
        }
        return refused(
            template,
            context,
            rule,
            NotEvaluatedReason::MissingService,
            services.message.to_owned(),
        );
    }
    let arguments = Arguments::of_rule(rule).planned(&plan.bound.measured);
    let mut ran = match read_once(&plan, context, &arguments) {
        Ok((once, opened)) => Ran {
            once,
            opened,
            selected,
            arguments,
        },
        Err((reason, message)) => {
            return match template.refusals {
                // Each selected object open, as the capability reported a
                // refusal of what all its objects needed.
                Refusals::Objects | Refusals::Prefixed { .. } | Refusals::ServicesPerObject => {
                    refused(template, context, rule, reason, message)
                }
                _ => {
                    let (_, mut evaluation) = selected.unwrap_or_default();
                    evaluation.push_not_evaluated(reason, message);
                    evaluation
                }
            };
        }
    };
    if let Some(mut evaluation) = run_apart(&plan, context, rule, &mut ran) {
        for (reason, message) in ran.opened {
            evaluation.push_not_evaluated(reason, message);
        }
        return evaluation;
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
    let (selected, mut evaluation) = ran.selection(context, rule);
    // `@selection` is the selection just made, where the form names it.
    let arguments = std::mem::take(&mut ran.arguments);
    let arguments = if plan.reads_selection {
        arguments.selected(&selected, &evaluation)
    } else {
        arguments
    };
    let (batched, bound) = read_ahead(&plan, context, &arguments, selected.first().copied());
    for chunk in selected.chunks(BATCH) {
        let prefetched = prefetch(context, &batched, chunk);
        let bound_reads = prefetch_bound(context, &bound, chunk);
        for ((object, prefetched), bound) in chunk.iter().zip(prefetched).zip(bound_reads) {
            let Judgement {
                outcome,
                row,
                checks,
            } = judge_object(
                (&plan, &decision, scope.as_ref()),
                (context, &arguments),
                rule,
                (object, &ran.once),
                Ahead { prefetched, bound },
            );
            if let (Some(table), Some(row)) = (&mut table, row) {
                // Selected objects are distinct, so rows never collide.
                let _ = table.push_row(object.id.clone(), row);
            }
            if let Some(scope) = &scope {
                for (member, outcome) in scope.judged.take() {
                    if let Some(member) = crate::selection::object_by_id(context, &member) {
                        push(&mut evaluation, rule, member, outcome);
                    }
                }
            }
            push_all(
                &mut evaluation,
                rule,
                object,
                checks.into_iter().chain([outcome]),
                plan.form.joined,
            );
        }
    }
    judge_project(&plan, (context, &arguments), rule, &mut evaluation);
    if let Some(table) = table {
        evaluation.push_table(table);
    }
    evaluation
}

/// The evaluation of a form deciding otherwise than object by object over
/// its values (members one by one, groups, facets, a requirements table,
/// scopes); `None` for a form judged per selected object.
fn run_apart<'a>(
    plan: &Plan<'_>,
    context: &RuleContext<'a>,
    rule: &CompiledRule,
    ran: &mut Ran<'a>,
) -> Option<CapabilityEvaluation> {
    Some(match &plan.form.decision {
        Decision::Each(each) => each::run(plan, each, context, rule),
        Decision::Parts(parts) => parts::run(plan, parts, context, rule),
        Decision::Unique { value, unique } => groups::run(plan, value, unique, context, rule),
        Decision::Consistent {
            key,
            value,
            consistent,
        } => groups::consistent(plan, (key, value), consistent, context, rule),
        Decision::Facets(names) => facets::run(names, context, rule),
        Decision::Requirements(names) => requirements::run(names, context, rule),
        Decision::Compared(names) => compared::run(names, context, rule),
        Decision::Conforms(conformance) => groups::conforms(plan, conformance, context, rule),
        Decision::Pairs(decided) => pairs::run(plan, decided, context, rule),
        Decision::Proportion(decided) if decided.groups.is_some() => {
            let groups = decided.groups.as_ref()?;
            proportion::groups(plan, groups, context, rule)
        }
        _ => {
            let scopes = plan.form.scope.as_ref()?;
            scopes::run(plan, &effective(plan), scopes, context, rule, ran)
        }
    })
}

/// What a run reads ahead for chunks of objects: the measured values the
/// form and its checks read, and those naming the rule's parameters. An
/// object a value may leave unjudged is measured no further than that
/// value, so where such values apply to the rule only they are read ahead
/// (they are read for every object).
fn read_ahead(
    plan: &Plan<'_>,
    context: &RuleContext<'_>,
    arguments: &Arguments,
    first: Option<&Object>,
) -> (Batched, Vec<(Arc<str>, axioval_engine::PreparedRead)>) {
    // Nothing selected, nothing to read ahead.
    if first.is_none() {
        return (Vec::new(), Vec::new());
    }
    let unless: Vec<&Expression> = plan
        .form
        .unless
        .iter()
        .zip(&plan.bound.unless)
        .filter(|(unless, _)| each::applies(plan, &unless.applies))
        .map(|(_, expression)| expression)
        .collect();
    if !unless.is_empty() {
        return (
            batched(unless.iter().copied(), context, first),
            bound_batched(
                unless.into_iter(),
                context,
                &plan.bound.parameters,
                arguments,
            ),
        );
    }
    // A check the rule's parameters leave out is never read.
    let applying = |index: &usize| {
        plan.form.checks[*index]
            .applies
            .as_ref()
            .is_none_or(|applies| each::applies(plan, applies))
    };
    // The checks' values too, as those naming the rule's parameters are.
    let batched = batched(
        plan.eager_values()
            .chain(
                (0..plan.form.checks.len())
                    .filter(applying)
                    .flat_map(|index| plan.check_values(index)),
            )
            .map(|(_, expression)| expression),
        context,
        first,
    );
    // The form's grading values too: read together they cost less than
    // one by one, and one that cannot be read counts only where a finding
    // is graded.
    let graded = plan
        .form
        .grading
        .as_ref()
        .map_or(0, |grading| grading.values.len());
    let bound = bound_batched(
        plan.eager_values()
            .chain(
                (0..plan.form.checks.len())
                    .filter(applying)
                    .flat_map(|index| plan.check_values(index)),
            )
            .map(|(_, expression)| expression)
            .chain(
                plan.bound
                    .grading
                    .first()
                    .into_iter()
                    .flatten()
                    .take(graded),
            ),
        context,
        &plan.bound.parameters,
        arguments,
    );
    (batched, bound)
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
fn batched<'e>(
    expressions: impl Iterator<Item = &'e Expression>,
    context: &RuleContext<'_>,
    first: Option<&Object>,
) -> Batched {
    let Some(first) = first else {
        return Vec::new();
    };
    let mut batched: Batched = Vec::new();
    for expression in expressions {
        if let Some((Some(set), name)) = property_read(expression)
            && set == axioval_ir::MEASURED_SET
            && !name.contains('@')
            // Read ahead once, however many values read it.
            && !batched.iter().any(|(_, read)| &**read == name)
            && bound_property_request(context, first, Some(set), name).is_ok()
        {
            batched.push((Some(Arc::from(set)), Arc::from(name)));
        }
    }
    batched
}

/// The measured values read for many objects together, as `(set, name)`.
type Batched = Vec<(Option<Arc<str>>, Arc<str>)>;

/// The measured values naming the rule's parameters (not the anchor) that
/// the plan's form and checks read directly, each bound once for the rule:
/// they are read for a chunk of objects together. One naming the anchor,
/// or one that does not bind, is read as it is read (and refused there).
fn bound_batched<'e>(
    reads: impl Iterator<Item = &'e Expression>,
    context: &RuleContext<'_>,
    parameters: &BTreeMap<String, ParameterValue>,
    arguments: &Arguments,
) -> Vec<(Arc<str>, axioval_engine::PreparedRead)> {
    let mut bound: Vec<(Arc<str>, axioval_engine::PreparedRead)> = Vec::new();
    for expression in reads {
        let Some((Some(set), name)) = property_read(expression) else {
            continue;
        };
        if set != axioval_ir::MEASURED_SET
            || !name.contains('@')
            || bound.iter().any(|(read, _)| &**read == name)
        {
            continue;
        }
        // No anchor named: the same arguments for every object, bound as
        // each object's read binds them.
        if let Some(Ok(prepared)) = arguments.call(context, Some(parameters), name) {
            bound.push((Arc::from(name), prepared));
        }
    }
    bound
}

/// Each of `bound` measured for every object of `chunk` together, as each
/// object's bound reads, in `chunk`'s order.
fn prefetch_bound(
    context: &RuleContext<'_>,
    bound: &[(Arc<str>, axioval_engine::PreparedRead)],
    chunk: &[&Object],
) -> Vec<Vec<BoundPrefetched>> {
    let ids: Vec<&ObjectId> = chunk.iter().map(|object| &object.id).collect();
    let mut columns: Vec<_> = match context.services.get::<MeasuredValues>() {
        Some(values) => bound
            .iter()
            .map(|(_, call)| {
                values
                    .read_prepared(call, &ids)
                    .into_iter()
                    .map(|read| read.map_err(property_error))
            })
            .collect(),
        None => Vec::new(),
    };
    chunk
        .iter()
        .map(|_| {
            bound
                .iter()
                .zip(columns.iter_mut())
                .filter_map(|((name, _), column)| column.next().map(|read| (name.clone(), read)))
                .collect()
        })
        .collect()
}

/// Each of `batched` measured for every object of `chunk` together, as
/// each object's prefetched reads, in `chunk`'s order: an object reads
/// them as it would resolve them alone.
fn prefetch(context: &RuleContext<'_>, batched: &Batched, chunk: &[&Object]) -> Vec<Prefetch> {
    let ids: Vec<&ObjectId> = chunk.iter().map(|object| &object.id).collect();
    let mut columns: Vec<_> = match context.services.get::<MeasuredValues>() {
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
    chunk
        .iter()
        .map(|_| {
            batched
                .iter()
                .zip(columns.iter_mut())
                .filter_map(|((set, name), column)| {
                    column.next().map(|read| (set.clone(), name.clone(), read))
                })
                .collect()
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
        // The runner words a refusal by its reason alone: no path to build.
        let here = |reason| NotEvaluated {
            path: String::new(),
            label: None,
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
    // A constant needs no evaluator.
    if let Expression::Literal { value, .. } = expression {
        return (
            Value::from_literal(value).map_err(|why| NotEvaluated {
                path: root.to_owned(),
                label: expression.label().map(str::to_owned),
                reason: Reason::Mismatch(why),
            }),
            Vec::new(),
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
    /// The rule's parameters the requirement's measured values name
    /// (`@door_selector`), carried into the forked rule as they are, so it
    /// binds them as the template did.
    pub carried: BTreeMap<String, ParameterValue>,
}

impl Fork {
    /// The capability a forked rule is bound to.
    pub const CAPABILITY: &'static str = "axioval:capability.expression";

    /// The forked rule's parameters.
    #[must_use]
    pub fn parameters(&self) -> BTreeMap<String, ParameterValue> {
        let mut parameters = self.carried.clone();
        parameters.insert(
            "requirement".to_owned(),
            ParameterValue::Expression {
                value: Box::new(self.requirement.clone()),
            },
        );
        parameters
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
#[allow(clippy::too_many_lines)]
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
    // The checks applying to the rule: one that does not is never judged.
    let applying: Vec<usize> = (0..plan.form.checks.len())
        .filter(|index| {
            plan.form.checks[*index]
                .applies
                .as_ref()
                .is_none_or(|applies| each::applies(&plan, applies))
        })
        .collect();
    let checks = || applying.iter().map(|index| &plan.form.checks[*index]);
    if !plan.form.derived.is_empty() || checks().any(|check| !check.derived.is_empty()) {
        return Err(ForkError::Inexpressible(
            "a value derived in plain binary arithmetic (a difference, or a ratio whose \
             denominator may be zero) has no expression form the evaluator decides alike"
                .to_owned(),
        ));
    }
    if matches!(plan.form.decision, Decision::Proportion(_)) {
        return Err(ForkError::Inexpressible(
            "a proportion is judged in exact integer arithmetic, its small counts and table \
             steps by the runner, and per group where it groups"
                .to_owned(),
        ));
    }
    if matches!(plan.form.decision, Decision::Each(_)) {
        return Err(ForkError::Inexpressible(
            "members judged one by one against their neighbours have no expression form".to_owned(),
        ));
    }
    if checks().any(|check| matches!(check.decision, Decision::Items(_))) {
        return Err(ForkError::Inexpressible(
            "items of a measured list judged one by one, each its own outcome, have no \
             expression form"
                .to_owned(),
        ));
    }
    if matches!(plan.form.decision, Decision::Parts(_)) {
        return Err(ForkError::Inexpressible(
            "an expression rule judges the objects it selects, not their parts as objects of \
             their own"
                .to_owned(),
        ));
    }
    if matches!(
        plan.form.decision,
        Decision::Unique { .. }
            | Decision::Consistent { .. }
            | Decision::Conforms(_)
            | Decision::Facets(_)
            | Decision::Requirements(_)
            | Decision::Compared(_)
            | Decision::Pairs(_)
    ) {
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
    if plan.form.checks.iter().any(|check| check.unless.is_some()) {
        return Err(ForkError::Inexpressible(
            "a check passing where a condition holds over its values has no expression form"
                .to_owned(),
        ));
    }
    if !plan.form.once.is_empty() || matches!(plan.form.decision, Decision::Joined(_)) {
        return Err(ForkError::Inexpressible(
            "an expression rule reads its values per object: none read once for the rule, \
             whose refusal leaves the rule open, and no message joining the words of a list"
                .to_owned(),
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
        let requirement = bound(&requirement, &plan.constants);
        return Ok(Fork {
            carried: carried(&requirement, &plan.bound.parameters),
            requirement,
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
                    // Each population's aggregates along the same path,
                    // filtered by its own selector (none where unstated).
                    let mut expression = along(expression, members.selector, &over, selector);
                    for name in members.more {
                        let none = Selector::Not {
                            operand: Box::new(Selector::All),
                        };
                        let picks = match plan.constants.get(*name) {
                            Some(Constant::Other(ParameterValue::Selector { value })) => value,
                            _ => &none,
                        };
                        expression = along(&expression, name, &over, picks);
                    }
                    (step, expression)
                })
                .collect()
        }
    };
    let inline = |values: &[(&TemplateValue, Expression)], name: &str| {
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
    };
    let own = match plan.form.required_read() {
        Some(read) => Expression::IsDefined {
            operand: Box::new(inline(&values, read)),
            label: Some("read".into()),
        },
        None => effective(&plan).expression(&|name| inline(&values, name)),
    };
    // An object a value applying to the rule leaves unjudged passes.
    let unless: Vec<Expression> = plan
        .form
        .unless
        .iter()
        .zip(&plan.bound.unless)
        .filter(|(unless, _)| each::applies(&plan, &unless.applies))
        .map(|(unless, expression)| Expression::Compare {
            operator: axioval_ir::contract::ExpressionComparison::NotEquals,
            left: Box::new(expression.clone()),
            right: Box::new(Expression::Literal {
                value: ScalarValue::Integer { value: 0 },
                label: None,
            }),
            case_sensitive: true,
            label: Some(format!("unless {}", unless.value.name)),
        })
        .collect();
    // Every check applying to the rule is required beside the form's own
    // decision, as the template finds each on its own.
    let requirement = if applying.is_empty() {
        own
    } else {
        let mut operands: Vec<Expression> = applying
            .iter()
            .map(|&index| {
                let check = &plan.form.checks[index];
                let read: Vec<(&TemplateValue, Expression)> = values
                    .iter()
                    .cloned()
                    .chain(
                        plan.check_values(index)
                            .map(|(step, expression)| (step, expression.clone())),
                    )
                    .collect();
                effective_of(&plan, &check.decision).expression(&|name| inline(&read, name))
            })
            .collect();
        operands.push(own);
        Expression::And {
            operands,
            label: None,
        }
    };
    let requirement = if unless.is_empty() {
        requirement
    } else {
        Expression::Or {
            operands: unless.into_iter().chain([requirement]).collect(),
            label: None,
        }
    };
    let requirement = bound(&requirement, &plan.constants);
    if measured_references(&requirement).contains(axioval_engine::template::SELECTION) {
        return Err(ForkError::Inexpressible(
            "a value measured over the rule's own selection has no expression form".to_owned(),
        ));
    }
    Ok(Fork {
        carried: carried(&requirement, &plan.bound.parameters),
        requirement,
    })
}

/// The parameters the measured values `requirement` reads name, as the
/// rule states them or the template's defaults give them.
fn carried(
    requirement: &Expression,
    parameters: &BTreeMap<String, ParameterValue>,
) -> BTreeMap<String, ParameterValue> {
    measured_references(requirement)
        .into_iter()
        .filter_map(|name| {
            parameters
                .get(&name)
                .map(|value| (name.clone(), value.clone()))
        })
        .collect()
}

/// Every rule parameter the measured values `expression` reads name
/// (`@name`), sorted.
fn measured_references(expression: &Expression) -> std::collections::BTreeSet<String> {
    use axioval_ir::measured::MeasuredArgument;
    let mut names = std::collections::BTreeSet::new();
    let mut pending = vec![expression];
    while let Some(node) = pending.pop() {
        if let Expression::Property {
            property_set: Some(set),
            property,
            ..
        } = node
            && set == axioval_ir::MEASURED_SET
            && property.contains('@')
            && let Ok(call) = axioval_ir::measured::parse(property)
        {
            for (_, argument) in call.references() {
                if let MeasuredArgument::Parameter(name) = argument {
                    names.insert(name.clone());
                }
            }
        }
        pending.extend(node.children());
    }
    names
}
