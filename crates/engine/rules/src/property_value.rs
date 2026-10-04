//! Value constraints on an exactly resolved property.
//!
//! Constraints are written as lexical strings, the way XML Schema facets are,
//! and cast to the kind of the value the source resolved:
//!
//! - text compares exactly and case-sensitively, and alone takes `patterns`
//!   and lengths;
//! - a boolean accepts `true`/`1` and `false`/`0`;
//! - an integer takes integer literals, and bounds in any numeric form;
//! - a decimal takes `xs:double` literals and equals within the tolerance
//!   `|x - v| <= |v|·1e-6 + 1e-6`; bounds compare without tolerance;
//! - a date takes `xs:date` literals (`2026-09-27`) and a date-time
//!   `xs:dateTime` literals with a UTC offset, compared chronologically.
//!   With `precision` `day` both read as the calendar day they state, so a
//!   date-time value takes date literals and the reverse.
//!
//! A literal that cannot be cast to the value's kind, or a constraint the kind
//! does not take, makes the object not evaluated (`InvalidDeclaration`): the
//! declaration cannot be applied to this value, which is neither a pass nor a
//! violation. A quantity is compared only when `si_units` states that the
//! literals are in the coherent SI unit of its dimension; otherwise it is not
//! evaluated. A list, bounded value or table is judged by its stated values
//! under the declared `quantifier`, a range also by its open ends.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NamePattern, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PropertyResolution, PropertyResolutionError, PropertyResolutionServiceHandle,
    RuleCapability, RuleContext, UnreadableValue,
};
use axioval_ir::contract::{ParameterValue, Quantifier};
use axioval_ir::{
    Date, DateTime, Evidence, Finding, Object, Property, PropertyValue, Severity, TemporalPrecision,
};

use crate::support::temporal_order;

use crate::selection::{
    NameSpec, bound_property_request, enumerate, property_error, select_objects,
    sets_without_match, xsd_name_pattern,
};
use crate::xsd_pattern;

/// IDS equality tolerance for doubles, relative and absolute.
const EPSILON: f64 = 1.0e-6;

/// The declared constraints of one rule.
#[derive(Default)]
struct Constraints<'r> {
    data_type: Option<&'r str>,
    values: &'r [String],
    patterns: &'r [String],
    min_inclusive: Option<&'r str>,
    max_inclusive: Option<&'r str>,
    min_exclusive: Option<&'r str>,
    max_exclusive: Option<&'r str>,
    length: Option<i64>,
    min_length: Option<i64>,
    max_length: Option<i64>,
    total_digits: Option<i64>,
    fraction_digits: Option<i64>,
    optional: bool,
    precision: Option<TemporalPrecision>,
    /// How the stated values of a list, bounded value or table are judged.
    quantifier: Option<Quantifier>,
    /// Whether numeric literals compared with a quantity are in the
    /// coherent SI unit of its dimension.
    si_units: bool,
}

impl<'r> Constraints<'r> {
    fn read(rule: &'r CompiledRule) -> Result<Self, String> {
        let text = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::String { value }) => Some(value.as_str()),
            _ => None,
        };
        let list = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::StringList { value }) => value.as_slice(),
            _ => &[],
        };
        let count = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::Integer { value }) => Some(*value),
            _ => None,
        };
        let constraints = Self {
            data_type: text("data_type"),
            values: list("values"),
            patterns: list("patterns"),
            min_inclusive: text("min_inclusive"),
            max_inclusive: text("max_inclusive"),
            min_exclusive: text("min_exclusive"),
            max_exclusive: text("max_exclusive"),
            length: count("length"),
            min_length: count("min_length"),
            max_length: count("max_length"),
            total_digits: count("total_digits"),
            fraction_digits: count("fraction_digits"),
            optional: matches!(
                rule.parameters.get("optional"),
                Some(ParameterValue::Boolean { value: true })
            ),
            quantifier: match text("quantifier") {
                None => None,
                Some("any") => Some(Quantifier::Any),
                Some("all") => Some(Quantifier::All),
                Some(other) => {
                    return Err(format!("quantifier `{other}` is not `any` or `all`"));
                }
            },
            si_units: matches!(
                rule.parameters.get("si_units"),
                Some(ParameterValue::Boolean { value: true })
            ),
            precision: match text("precision") {
                None => None,
                Some("day") => Some(TemporalPrecision::Day),
                Some(other) => {
                    return Err(format!(
                        "precision `{other}` is unsupported; the only precision is `day`"
                    ));
                }
            },
        };
        if constraints
            .data_type
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err("data_type is blank".into());
        }
        if [
            constraints.length,
            constraints.min_length,
            constraints.max_length,
        ]
        .into_iter()
        .flatten()
        .any(|count| count < 0)
        {
            return Err("a length is negative".into());
        }
        // XML Schema: `totalDigits` is a positive integer, `fractionDigits`
        // a non-negative one.
        if constraints.total_digits.is_some_and(|digits| digits < 1) {
            return Err("total_digits is not positive".into());
        }
        if constraints.fraction_digits.is_some_and(|digits| digits < 0) {
            return Err("fraction_digits is negative".into());
        }
        if constraints.data_type.is_none() && !constraints.constrains_value() {
            return Err("no data type and no value constraint".into());
        }
        Ok(constraints)
    }

    fn constrains_value(&self) -> bool {
        !self.values.is_empty()
            || !self.patterns.is_empty()
            || self.has_bounds()
            || self.has_lengths()
            || self.has_digits()
    }

    fn has_digits(&self) -> bool {
        self.total_digits.is_some() || self.fraction_digits.is_some()
    }

    fn has_bounds(&self) -> bool {
        self.min_inclusive.is_some()
            || self.max_inclusive.is_some()
            || self.min_exclusive.is_some()
            || self.max_exclusive.is_some()
    }

    fn has_lengths(&self) -> bool {
        self.length.is_some() || self.min_length.is_some() || self.max_length.is_some()
    }
}

/// Whether a present value meets the constraints.
enum Verdict {
    Meets,
    Fails(String),
    /// The constraints cannot be applied to this value.
    Inapplicable(NotEvaluatedReason, String),
}

/// Requires a property's value to meet lexical constraints, cast to its kind.
///
/// Parameters: `property`, or `property_pattern` with an optional
/// `property_set_pattern` (XML Schema patterns over the source's own names,
/// matching them whole); optionally `data_type` (the source-declared type,
/// as in `property-data-type`), `values` (any of), `patterns` (XML Schema
/// regular expressions, any of, whole value), `min_inclusive`,
/// `max_inclusive`, `min_exclusive`, `max_exclusive`, `length`,
/// `min_length`, `max_length`, `total_digits`, `fraction_digits` (for a
/// number only), `optional`, `precision` (`day`, for a date or date-time
/// value only), `quantifier` (`any` or `all`) and `si_units`. All given
/// constraints must hold. A decimal's digits are
/// counted on the shortest decimal that reads back as the same double, the
/// form a model's literal has. Without `optional`, absence, `null`, blank
/// text and an empty list are violations; with it, an absent or `null`
/// property passes and any present value, empty text included, is checked.
///
/// With patterns, every matching property, enumerated exactly through the
/// property service, must meet the constraints, and one must match unless
/// the rule is optional; with `property_set_pattern`, one must match in
/// every set the pattern matches, as IDS requires. Each failing property
/// and each set without a match is its own finding.
///
/// A list, a bounded value or a table is judged by its stated values (see
/// `PropertyValue::stated_values`) under `quantifier`: `any` holds when one
/// of them meets every constraint, `all` when each does, and there is at
/// least one. A range holds every value between its bounds, so under `all`
/// a bounded value open on one side fails every bound on that side. Without
/// a quantifier such a value is not evaluated; a scalar under a quantifier
/// is judged as itself. `si_units` reads numeric literals compared with a
/// quantity in the coherent SI unit of its dimension (metres, square
/// metres, kilograms, ...), the unit the value is stated in; without it a
/// quantity is not evaluated.
pub struct PropertyValueConstraint;
impl RuleCapability for PropertyValueConstraint {
    fn id(&self) -> &'static str {
        "axioval:capability.property-value"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![ParameterDescriptor::optional(
            "property",
            ParameterType::PropertyReference,
        )];
        for name in [
            "property_set_pattern",
            "property_pattern",
            "data_type",
            "min_inclusive",
            "max_inclusive",
            "min_exclusive",
            "max_exclusive",
            "precision",
            "quantifier",
        ] {
            parameters.push(ParameterDescriptor::optional(name, ParameterType::String));
        }
        for name in ["values", "patterns"] {
            parameters.push(ParameterDescriptor::optional(
                name,
                ParameterType::StringList,
            ));
        }
        for name in [
            "length",
            "min_length",
            "max_length",
            "total_digits",
            "fraction_digits",
        ] {
            parameters.push(ParameterDescriptor::optional(name, ParameterType::Integer));
        }
        for name in ["optional", "si_units"] {
            parameters.push(ParameterDescriptor::optional(name, ParameterType::Boolean));
        }
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let target = match Target::read(rule) {
            Ok(target) => target,
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("property-value: {message}"),
                );
            }
        };
        let constraints = match Constraints::read(rule) {
            Ok(constraints) => constraints,
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("property-value parameters are invalid: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "property-resolution service is not registered",
                );
            }
            return evaluation;
        };
        for object in selected {
            match &target {
                Target::Exact { set, name } => {
                    check_exact(
                        context,
                        rule,
                        service,
                        object,
                        (*set, name),
                        &constraints,
                        &mut evaluation,
                    );
                }
                Target::Matched { set, name, shown } => {
                    check_matched(
                        context,
                        rule,
                        object,
                        (set.as_ref(), name, shown),
                        &constraints,
                        &mut evaluation,
                    );
                }
            }
        }
        evaluation
    }
}

/// The property a rule judges: one named exactly, or every one whose name
/// (and set) match XML Schema patterns.
enum Target<'r> {
    Exact {
        set: Option<&'r str>,
        name: &'r str,
    },
    Matched {
        set: Option<NamePattern>,
        name: NamePattern,
        shown: String,
    },
}

impl<'r> Target<'r> {
    fn read(rule: &'r CompiledRule) -> Result<Self, String> {
        let text = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::String { value }) => Ok(Some(value.as_str())),
            None => Ok(None),
            Some(_) => Err(format!("`{name}` is not a string")),
        };
        let reference = match rule.parameters.get("property") {
            Some(ParameterValue::PropertyReference {
                property,
                property_set,
            }) => Some((property_set.as_deref(), property.as_str())),
            None => None,
            Some(_) => return Err("`property` is not a property reference".into()),
        };
        let set_pattern = text("property_set_pattern")?;
        let name_pattern = text("property_pattern")?;
        let compile = |pattern: &str| {
            xsd_name_pattern(pattern).map_err(|why| format!("name pattern {pattern:?}: {why}"))
        };
        match (reference, set_pattern, name_pattern) {
            (Some((set, name)), None, None) => Ok(Self::Exact { set, name }),
            (None, set, Some(name)) => Ok(Self::Matched {
                shown: format!(
                    "{}/{name}/",
                    set.map_or_else(String::new, |set| format!("/{set}/."))
                ),
                set: set.map(compile).transpose()?,
                name: compile(name)?,
            }),
            (Some(_), _, Some(_)) => {
                Err("declare `property` or `property_pattern`, not both".into())
            }
            (_, Some(_), None) => Err("`property_set_pattern` needs `property_pattern`".into()),
            (None, None, None) => Err("declare `property` or `property_pattern`".into()),
        }
    }
}

fn check_exact(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    service: &PropertyResolutionServiceHandle,
    object: &Object,
    (set, name): (Option<&str>, &str),
    constraints: &Constraints<'_>,
    evaluation: &mut CapabilityEvaluation,
) {
    let request = match bound_property_request(context, object, set, name) {
        Ok(request) => request,
        Err((reason, message)) => {
            evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            return;
        }
    };
    match service.resolve(&request) {
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.property();
            match judge(property, name, constraints) {
                Verdict::Meets => {}
                Verdict::Fails(message) => evaluation.push_finding(finding(
                    rule,
                    object,
                    message,
                    property.evidence.clone().into_iter().collect(),
                )),
                Verdict::Inapplicable(reason, message) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        Ok(PropertyResolution::Absent(proof)) => {
            if !constraints.optional {
                evaluation.push_finding(finding(
                    rule,
                    object,
                    format!("missing required property {name}"),
                    vec![proof.evidence().clone()],
                ));
            }
        }
        Err(PropertyResolutionError::UnreadableValue(unreadable)) => {
            match judge_unreadable(&unreadable, name, constraints) {
                Verdict::Meets => {}
                Verdict::Fails(message) => evaluation.push_finding(finding(
                    rule,
                    object,
                    message,
                    vec![unreadable.evidence().clone()],
                )),
                Verdict::Inapplicable(reason, message) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        Err(error) => {
            let (reason, message) = property_error(error);
            evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
        }
    }
}

/// The verdict on a present property whose stated value cannot be read but
/// whose declared type is exact: another type than `data_type` fails, and
/// any constraint on the value is not evaluated. Presence and a matching
/// type alone are met, since a value is stated.
fn judge_unreadable(
    unreadable: &UnreadableValue,
    name: &str,
    constraints: &Constraints<'_>,
) -> Verdict {
    let actual = unreadable.data_type();
    if let Some(expected) = constraints.data_type
        && !actual.eq_ignore_ascii_case(expected)
    {
        return Verdict::Fails(format!("property {name} is {actual}, not {expected}"));
    }
    if constraints.constrains_value() {
        return Verdict::Inapplicable(
            NotEvaluatedReason::IncompleteEvidence,
            format!("property {name}: {}", unreadable.reason()),
        );
    }
    Verdict::Meets
}

/// Every matched property must meet the constraints, and one must match
/// unless the rule is optional. Each failing property is a finding; one
/// that cannot be judged leaves the object not evaluated as well.
fn check_matched(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    object: &Object,
    (set, name, shown): (Option<&NamePattern>, &NamePattern, &str),
    constraints: &Constraints<'_>,
    evaluation: &mut CapabilityEvaluation,
) {
    let set = set.map_or(NameSpec::Any, NameSpec::Pattern);
    let enumeration = match enumerate(context, object, set, NameSpec::Pattern(name)) {
        Ok(enumeration) => enumeration,
        Err((reason, message)) => {
            evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            return;
        }
    };
    if enumeration.properties().is_empty() {
        if !constraints.optional {
            evaluation.push_finding(finding(
                rule,
                object,
                format!("missing required property {shown}: no property matches"),
                vec![enumeration.evidence().clone()],
            ));
        }
        return;
    }
    if !constraints.optional && !matches!(set, NameSpec::Any) {
        match sets_without_match(context, object, set, &enumeration) {
            Ok((missing, evidence)) => {
                for set in missing {
                    evaluation.push_finding(finding(
                        rule,
                        object,
                        format!("missing required property {shown} in set {set}"),
                        vec![evidence.clone()],
                    ));
                }
            }
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
    for property in enumeration.properties() {
        let label = format!("{}.{}", property.property_set, property.name);
        match judge(property, &label, constraints) {
            Verdict::Meets => {}
            Verdict::Fails(message) => {
                let mut evidence = vec![enumeration.evidence().clone()];
                evidence.extend(property.evidence.iter().cloned());
                evaluation.push_finding(finding(rule, object, message, evidence));
            }
            Verdict::Inapplicable(reason, message) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
}

/// The verdict on a present property: emptiness, declared type, then value.
fn judge(property: &Property, name: &str, constraints: &Constraints<'_>) -> Verdict {
    let value = &property.value;
    if constraints.optional && matches!(value, PropertyValue::Null) {
        return Verdict::Meets;
    }
    if !constraints.optional && is_empty(value) {
        return Verdict::Fails(format!("missing required property {name}"));
    }
    // A complex property is present but holds no value of any type: its
    // presence meets the rule, any type or value it must have fails it.
    if matches!(value, PropertyValue::Complex) {
        if let Some(expected) = constraints.data_type {
            return Verdict::Fails(format!(
                "property {name} is a complex property, not {expected}"
            ));
        }
        if constraints.constrains_value() {
            return Verdict::Fails(format!(
                "property {name} is a complex property, which holds no value"
            ));
        }
        return Verdict::Meets;
    }
    let typed;
    let mut value = value;
    if let Some(expected) = constraints.data_type {
        match (property.data_type(), typed_cells(property, expected)) {
            (Some(actual), _) if actual.eq_ignore_ascii_case(expected) => {}
            (Some(actual), _) => {
                return Verdict::Fails(format!("property {name} is {actual}, not {expected}"));
            }
            // A table whose columns differ: the cells of the expected type
            // are its values.
            (None, Some(cells)) if cells.is_empty() => {
                return Verdict::Fails(format!("property {name} has no column of type {expected}"));
            }
            (None, Some(cells)) => {
                typed = PropertyValue::List(cells.into_iter().cloned().collect());
                value = &typed;
            }
            (None, None) => {
                return Verdict::Inapplicable(
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("the source does not report the type of property {name}"),
                );
            }
        }
    }
    match verdict(value, constraints) {
        Verdict::Fails(why) => Verdict::Fails(format!("property {name} {why}")),
        Verdict::Inapplicable(reason, message) => {
            Verdict::Inapplicable(reason, format!("property {name}: {message}"))
        }
        Verdict::Meets => Verdict::Meets,
    }
}

/// The cells of a table value's columns declared `expected`, when the
/// source reports its column types; `None` for any other value.
pub(crate) fn typed_cells<'p>(
    property: &'p Property,
    expected: &str,
) -> Option<Vec<&'p PropertyValue>> {
    let (PropertyValue::Table(rows), Some(types)) = (&property.value, property.column_types())
    else {
        return None;
    };
    let defining = types.defining.eq_ignore_ascii_case(expected);
    let defined = types.defined.eq_ignore_ascii_case(expected);
    Some(
        rows.iter()
            .flat_map(|row| {
                [
                    defining.then_some(&row.defining),
                    defined.then_some(&row.defined),
                ]
            })
            .flatten()
            .collect(),
    )
}

fn is_empty(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::Null => true,
        PropertyValue::String(text) => text.trim().is_empty(),
        PropertyValue::List(elements) => elements.is_empty(),
        _ => false,
    }
}

fn finding(
    rule: &CompiledRule,
    object: &Object,
    message: String,
    evidence: Vec<Evidence>,
) -> Finding {
    Finding {
        explanation: None,
        id: None,
        decision: None,
        rule_id: rule.id.clone(),
        scope: axioval_ir::Scope::Object(object.id.clone()),
        related: Vec::new(),
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence,
        location: None,
        categories: Vec::new(),
    }
}

fn invalid(message: impl Into<String>) -> Verdict {
    Verdict::Inapplicable(NotEvaluatedReason::InvalidDeclaration, message.into())
}

/// The verdict on a value: a scalar directly, the stated values of a list,
/// bounded value or table under the declared quantifier.
fn verdict(value: &PropertyValue, constraints: &Constraints<'_>) -> Verdict {
    let Some(stated) = value.stated_values() else {
        return scalar_verdict(value, constraints);
    };
    let shown = crate::support::display(Some(value));
    let Some(quantifier) = constraints.quantifier else {
        return Verdict::Inapplicable(
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "the value is {shown}; declare `quantifier` `any` or `all` to judge its values"
            ),
        );
    };
    let mut open = None;
    match quantifier {
        Quantifier::Any => {
            for part in stated {
                match scalar_verdict(part, constraints) {
                    Verdict::Meets => return Verdict::Meets,
                    Verdict::Fails(_) => {}
                    inapplicable @ Verdict::Inapplicable(..) => {
                        open.get_or_insert(inapplicable);
                    }
                }
            }
            open.unwrap_or_else(|| {
                Verdict::Fails(format!(
                    "is {shown}, and none of its values meets the constraints"
                ))
            })
        }
        Quantifier::All => {
            if stated.is_empty() {
                return Verdict::Fails(format!("is {shown}, which holds no value"));
            }
            // A range holds every value between its bounds: an open end
            // passes every bound on that side.
            if let PropertyValue::Bounded { lower, upper, .. } = value {
                let below =
                    constraints.min_inclusive.is_some() || constraints.min_exclusive.is_some();
                let above =
                    constraints.max_inclusive.is_some() || constraints.max_exclusive.is_some();
                if below && lower.is_none() {
                    return Verdict::Fails(format!(
                        "is {shown}, open below, so not all its values meet the lower bound"
                    ));
                }
                if above && upper.is_none() {
                    return Verdict::Fails(format!(
                        "is {shown}, open above, so not all its values meet the upper bound"
                    ));
                }
            }
            for part in stated {
                match scalar_verdict(part, constraints) {
                    Verdict::Meets => {}
                    Verdict::Fails(why) => {
                        return Verdict::Fails(format!("is {shown}: one of its values {why}"));
                    }
                    inapplicable @ Verdict::Inapplicable(..) => {
                        open.get_or_insert(inapplicable);
                    }
                }
            }
            open.unwrap_or(Verdict::Meets)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn scalar_verdict(value: &PropertyValue, constraints: &Constraints<'_>) -> Verdict {
    if let (PropertyValue::Quantity { value, .. }, true) = (value, constraints.si_units) {
        // In SI the quantity's number is the literal's number.
        return scalar_verdict(&PropertyValue::Decimal(*value), constraints);
    }
    if constraints.precision.is_some()
        && !matches!(value, PropertyValue::Date(_) | PropertyValue::DateTime(_))
    {
        return invalid("precision applies to a date or date-time value only");
    }
    if constraints.has_digits()
        && matches!(
            value,
            PropertyValue::String(_)
                | PropertyValue::Boolean(_)
                | PropertyValue::Date(_)
                | PropertyValue::DateTime(_)
        )
    {
        return invalid("total_digits and fraction_digits apply to a number only");
    }
    match value {
        PropertyValue::Date(_) | PropertyValue::DateTime(_) => temporal_verdict(value, constraints),
        PropertyValue::String(text) => text_verdict(text, constraints),
        PropertyValue::Boolean(actual) => {
            if !constraints.patterns.is_empty()
                || constraints.has_bounds()
                || constraints.has_lengths()
            {
                return invalid("a boolean takes only values");
            }
            one_of(
                constraints.values,
                |literal| match literal {
                    "true" | "1" => Ok(*actual),
                    "false" | "0" => Ok(!*actual),
                    other => Err(format!("{other:?} is not a boolean literal")),
                },
                &actual.to_string(),
            )
        }
        PropertyValue::Integer(actual) => {
            if !constraints.patterns.is_empty() || constraints.has_lengths() {
                return invalid("a number takes no patterns or lengths");
            }
            let equal = one_of(
                constraints.values,
                |literal| {
                    parse_integer(literal)
                        .map(|expected| expected == *actual)
                        .ok_or_else(|| format!("{literal:?} is not an integer literal"))
                },
                &actual.to_string(),
            );
            if !matches!(equal, Verdict::Meets) {
                return equal;
            }
            #[allow(clippy::cast_precision_loss)]
            let bounded = bounds(Number::Integer(*actual), *actual as f64, constraints);
            if !matches!(bounded, Verdict::Meets) {
                return bounded;
            }
            digits(&actual.unsigned_abs().to_string(), constraints)
        }
        PropertyValue::Decimal(actual) => {
            if !constraints.patterns.is_empty() || constraints.has_lengths() {
                return invalid("a number takes no patterns or lengths");
            }
            let equal = one_of(
                constraints.values,
                |literal| {
                    parse_double(literal)
                        .map(|expected| within_tolerance(*actual, expected))
                        .ok_or_else(|| format!("{literal:?} is not a number literal"))
                },
                &actual.to_string(),
            );
            if !matches!(equal, Verdict::Meets) {
                return equal;
            }
            let bounded = bounds(Number::Decimal, *actual, constraints);
            if !matches!(bounded, Verdict::Meets) || !constraints.has_digits() {
                return bounded;
            }
            if !actual.is_finite() {
                return invalid("a value that is not finite has no digits");
            }
            // Rust prints the shortest decimal that reads back as the same
            // double, never in exponent notation.
            digits(&actual.abs().to_string(), constraints)
        }
        PropertyValue::Quantity { .. } => Verdict::Inapplicable(
            NotEvaluatedReason::IncompleteEvidence,
            "comparing a quantity needs its unit; declare `si_units` to read literals in SI".into(),
        ),
        PropertyValue::Measured { .. } => Verdict::Inapplicable(
            NotEvaluatedReason::IncompleteEvidence,
            "a measured interval has no one value to check against literals".into(),
        ),
        PropertyValue::Null => invalid("null has no value to compare"),
        PropertyValue::Complex => {
            Verdict::Fails("is a complex property, which holds no value to compare".into())
        }
        PropertyValue::Reference(_) => Verdict::Inapplicable(
            NotEvaluatedReason::InvalidEvidence,
            "a reference to another instance has no value to check against literals".into(),
        ),
        PropertyValue::List(_) | PropertyValue::Bounded { .. } | PropertyValue::Table(_) => {
            Verdict::Inapplicable(
                NotEvaluatedReason::InvalidEvidence,
                "a composite value nested in a value cannot be compared".into(),
            )
        }
    }
}

/// `totalDigits` and `fractionDigits` of an unsigned decimal numeral.
///
/// As XML Schema counts them: the value is `i / 10^n` with `n` the
/// fraction digits and `|i|` below `10^totalDigits`, so leading zeros of the
/// whole part and trailing zeros of the fraction do not count.
fn digits(numeral: &str, constraints: &Constraints<'_>) -> Verdict {
    let (whole, fraction) = numeral.split_once('.').unwrap_or((numeral, ""));
    let whole = whole.trim_start_matches('0');
    let fraction = fraction.trim_end_matches('0');
    let count = |text: &str| i64::try_from(text.len()).unwrap_or(i64::MAX);
    let (fraction_digits, total_digits) = (count(fraction), count(whole) + count(fraction));
    if let Some(limit) = constraints.total_digits
        && total_digits > limit
    {
        return Verdict::Fails(format!(
            "is {numeral}, with {total_digits} digits, more than {limit}"
        ));
    }
    if let Some(limit) = constraints.fraction_digits
        && fraction_digits > limit
    {
        return Verdict::Fails(format!(
            "is {numeral}, with {fraction_digits} fraction digits, more than {limit}"
        ));
    }
    Verdict::Meets
}

/// A date or date-time against lexical date and date-time literals.
fn temporal_verdict(value: &PropertyValue, constraints: &Constraints<'_>) -> Verdict {
    if !constraints.patterns.is_empty() || constraints.has_lengths() {
        return invalid("a date takes no patterns or lengths");
    }
    let order = |literal: &str| {
        let literal_value = literal
            .parse::<Date>()
            .map(PropertyValue::Date)
            .or_else(|_| literal.parse::<DateTime>().map(PropertyValue::DateTime))
            .map_err(|_| format!("{literal:?} is not a date or date-time literal"))?;
        temporal_order(value, &literal_value, constraints.precision)
            .unwrap_or_else(|| Err("not a date".into()))
            .map_err(|message| format!("{literal:?}: {message}"))
    };
    let shown = crate::support::display(Some(value));
    // A zoned and an unzoned date XML Schema cannot order are unequal.
    let equal = one_of(
        constraints.values,
        |literal| order(literal).map(|ordering| ordering.is_some_and(std::cmp::Ordering::is_eq)),
        &shown,
    );
    if !matches!(equal, Verdict::Meets) {
        return equal;
    }
    let checks: [Bound<'_>; 4] = [
        (constraints.min_inclusive, std::cmp::Ordering::is_ge, ">="),
        (constraints.max_inclusive, std::cmp::Ordering::is_le, "<="),
        (constraints.min_exclusive, std::cmp::Ordering::is_gt, ">"),
        (constraints.max_exclusive, std::cmp::Ordering::is_lt, "<"),
    ];
    for (bound, holds, symbol) in checks {
        let Some(bound) = bound else { continue };
        match order(bound) {
            Ok(Some(ordering)) if holds(ordering) => {}
            Ok(Some(_)) => return Verdict::Fails(format!("is {shown}, not {symbol} {bound}")),
            Ok(None) => {
                return Verdict::Inapplicable(
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("{bound:?}: {}", crate::support::INCOMPARABLE_DATES),
                );
            }
            Err(message) => return invalid(message),
        }
    }
    Verdict::Meets
}

fn text_verdict(text: &str, constraints: &Constraints<'_>) -> Verdict {
    if constraints.has_bounds() {
        return invalid("text takes no numeric bounds");
    }
    if !constraints.values.is_empty() && !constraints.values.iter().any(|value| value == text) {
        return Verdict::Fails(format!("is {text:?}, not one of the required values"));
    }
    if !constraints.patterns.is_empty() {
        let mut any = false;
        for pattern in constraints.patterns {
            match xsd_pattern::compile(pattern) {
                Ok(regex) => any |= regex.is_match(text),
                Err(error) => return invalid(format!("pattern {pattern:?}: {error}")),
            }
        }
        if !any {
            return Verdict::Fails(format!("is {text:?}, which matches no required pattern"));
        }
    }
    let length = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
    let fails_length = constraints
        .length
        .is_some_and(|expected| length != expected)
        || constraints.min_length.is_some_and(|min| length < min)
        || constraints.max_length.is_some_and(|max| length > max);
    if fails_length {
        return Verdict::Fails(format!(
            "is {length} characters long, outside the required length"
        ));
    }
    Verdict::Meets
}

/// Whether the value equals any literal; no literals constrain nothing.
fn one_of(
    literals: &[String],
    equals: impl Fn(&str) -> Result<bool, String>,
    shown: &str,
) -> Verdict {
    if literals.is_empty() {
        return Verdict::Meets;
    }
    let mut any = false;
    for literal in literals {
        match equals(literal) {
            Ok(equal) => any |= equal,
            Err(message) => return invalid(message),
        }
    }
    if any {
        Verdict::Meets
    } else {
        Verdict::Fails(format!("is {shown}, not one of the required values"))
    }
}

#[derive(Clone, Copy)]
enum Number {
    Integer(i64),
    Decimal,
}

/// A range facet: its literal, the ordering it accepts, and its symbol.
type Bound<'r> = (
    Option<&'r str>,
    fn(std::cmp::Ordering) -> bool,
    &'static str,
);

/// Range facets, compared exactly: IDS applies no tolerance to ranges.
fn bounds(number: Number, actual: f64, constraints: &Constraints<'_>) -> Verdict {
    let checks: [Bound<'_>; 4] = [
        (constraints.min_inclusive, std::cmp::Ordering::is_ge, ">="),
        (constraints.max_inclusive, std::cmp::Ordering::is_le, "<="),
        (constraints.min_exclusive, std::cmp::Ordering::is_gt, ">"),
        (constraints.max_exclusive, std::cmp::Ordering::is_lt, "<"),
    ];
    for (bound, holds, symbol) in checks {
        let Some(bound) = bound else { continue };
        let ordering = match (number, parse_integer(bound)) {
            // Integer against integer compares exactly, beyond 2^53 too.
            (Number::Integer(actual), Some(bound)) => Some(actual.cmp(&bound)),
            _ => parse_double(bound).and_then(|bound| actual.partial_cmp(&bound)),
        };
        if ordering.is_none() && parse_double(bound).is_none() {
            return invalid(format!("{bound:?} is not a number literal"));
        }
        // An unordered pair (NaN) meets no bound.
        if !ordering.is_some_and(holds) {
            return Verdict::Fails(format!("is {actual}, not {symbol} {bound}"));
        }
    }
    Verdict::Meets
}

/// `x == v` in IDS: `v - |v|ε - ε <= x <= v + |v|ε + ε`.
///
/// The IDS tolerance note writes strict inequalities, but the buildingSMART
/// test cases pass a value exactly on either boundary and fail one just past
/// it, so the boundaries are included. A boundary written in decimal is
/// rarely a binary double, so the comparison also allows a few ulps of
/// rounding (`1e-15` relative), far below the `1e-7` steps those cases test.
fn within_tolerance(actual: f64, expected: f64) -> bool {
    let margin = expected.abs() * EPSILON + EPSILON;
    let rounding = (expected.abs() + margin) * 1.0e-15;
    expected - margin - rounding <= actual && actual <= expected + margin + rounding
}

/// The `xs:integer` lexical form: an optional sign and digits.
fn parse_integer(literal: &str) -> Option<i64> {
    let digits = literal.strip_prefix(['+', '-']).unwrap_or(literal);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    literal.strip_prefix('+').unwrap_or(literal).parse().ok()
}

/// The `xs:double` lexical form, including `INF`, `-INF` and `NaN`.
fn parse_double(literal: &str) -> Option<f64> {
    match literal {
        "INF" | "+INF" => return Some(f64::INFINITY),
        "-INF" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    let unsigned = literal.strip_prefix(['+', '-']).unwrap_or(literal);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok =
        digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !exponent.is_empty() && digits(exponent)
    });
    if mantissa_ok && exponent_ok {
        literal.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Constraints, Verdict, digits, parse_double, parse_integer, within_tolerance};

    #[test]
    fn digits_are_counted_as_xml_schema_counts_them() {
        let limits = |total, fraction| Constraints {
            total_digits: total,
            fraction_digits: fraction,
            ..Constraints::default()
        };
        let meets = |numeral: &str, constraints: &Constraints<'_>| {
            matches!(digits(numeral, constraints), Verdict::Meets)
        };
        assert!(meets("123", &limits(Some(3), None)));
        assert!(!meets("1234", &limits(Some(3), None)));
        assert!(meets("120", &limits(Some(3), Some(0))));
        assert!(meets("0.0012", &limits(Some(4), Some(4))));
        assert!(!meets("0.0012", &limits(Some(3), None)));
        assert!(!meets("1.25", &limits(None, Some(1))));
        assert!(meets("1.25", &limits(Some(3), Some(2))));
        assert!(meets("0", &limits(Some(1), Some(0))));
    }

    #[test]
    fn number_literals_follow_xml_schema() {
        assert_eq!(parse_integer("+42"), Some(42));
        assert_eq!(parse_integer("-7"), Some(-7));
        for bad in ["42.0", "4 2", "", "+", "0x1"] {
            assert_eq!(parse_integer(bad), None, "{bad}");
        }
        assert_eq!(parse_double("1.2345e3"), Some(1234.5));
        assert_eq!(parse_double("1.2345E3"), Some(1234.5));
        assert_eq!(parse_double(".5"), Some(0.5));
        assert_eq!(parse_double("5."), Some(5.0));
        assert_eq!(parse_double("-INF"), Some(f64::NEG_INFINITY));
        for bad in ["42,3", "123,4.5", "inf", "nan", "e3", ".", "1e", "1.2.3"] {
            assert_eq!(parse_double(bad), None, "{bad}");
        }
    }

    #[test]
    fn equality_uses_the_ids_tolerance() {
        assert!(within_tolerance(100_000.1, 100_000.0));
        assert!(!within_tolerance(100_000.2, 100_000.0));
        assert!(within_tolerance(0.000_000_5, 0.0));
        assert!(!within_tolerance(0.000_001_1, 0.0));
        // The boundary itself is equal, as the buildingSMART cases require.
        assert!(within_tolerance(0.000_001, 0.0));
        assert!(within_tolerance(99_999.899_999, 100_000.0));
        assert!(!within_tolerance(99_999.899_998_9, 100_000.0));
        assert!(within_tolerance(0.000_000_900_000_1, -0.000_000_1));
        assert!(!within_tolerance(0.000_000_900_000_11, -0.000_000_1));
        assert!(!within_tolerance(-1_000_001.000_001_1, -1_000_000.0));
        assert!(within_tolerance(-1.000_001, -1.0));
        assert!(!within_tolerance(f64::NAN, f64::NAN));
    }
}
