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
//!   `|x - v| <= |v|·1e-6 + 1e-6`; bounds compare without tolerance.
//!
//! A literal that cannot be cast to the value's kind, or a constraint the kind
//! does not take, makes the object not evaluated (`InvalidDeclaration`): the
//! declaration cannot be applied to this value, which is neither a pass nor a
//! violation. A quantity is not evaluated: comparing it needs units.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PropertyResolution, PropertyResolutionServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, Finding, Object, PropertyValue, Severity};

use crate::selection::{bound_property_request, property_error, select_objects};
use crate::xsd_pattern;

/// IDS equality tolerance for doubles, relative and absolute.
const EPSILON: f64 = 1.0e-6;

/// The declared constraints of one rule.
#[derive(Default)]
pub(crate) struct Constraints<'r> {
    pub(crate) data_type: Option<&'r str>,
    values: &'r [String],
    patterns: &'r [String],
    min_inclusive: Option<&'r str>,
    max_inclusive: Option<&'r str>,
    min_exclusive: Option<&'r str>,
    max_exclusive: Option<&'r str>,
    length: Option<i64>,
    min_length: Option<i64>,
    max_length: Option<i64>,
    pub(crate) optional: bool,
    /// Meeting the requirement is the violation.
    pub(crate) prohibited: bool,
}

impl<'r> Constraints<'r> {
    /// Reads the constraints; `presence_suffices` admits a rule with none,
    /// which then only requires a value to be there.
    pub(crate) fn read(rule: &'r CompiledRule, presence_suffices: bool) -> Result<Self, String> {
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
            optional: matches!(
                rule.parameters.get("optional"),
                Some(ParameterValue::Boolean { value: true })
            ),
            prohibited: matches!(
                rule.parameters.get("prohibited"),
                Some(ParameterValue::Boolean { value: true })
            ),
        };
        if constraints.optional && constraints.prohibited {
            return Err("optional and prohibited exclude each other".into());
        }
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
        if !presence_suffices
            && !constraints.prohibited
            && constraints.data_type.is_none()
            && !constraints.constrains_value()
        {
            return Err("no data type and no value constraint".into());
        }
        Ok(constraints)
    }

    pub(crate) fn constrains_value(&self) -> bool {
        !self.values.is_empty()
            || !self.patterns.is_empty()
            || self.has_bounds()
            || self.has_lengths()
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
pub(crate) enum Verdict {
    Meets,
    Fails(String),
    /// The constraints cannot be applied to this value.
    Inapplicable(NotEvaluatedReason, String),
}

/// Requires a property's value to meet lexical constraints, cast to its kind.
///
/// Parameters: `property`; optionally `data_type` (the source-declared type,
/// as in `property-data-type`), `values` (any of), `patterns` (XML Schema
/// regular expressions, any of, whole value), `min_inclusive`,
/// `max_inclusive`, `min_exclusive`, `max_exclusive`, `length`,
/// `min_length`, `max_length`, and `optional`. All given constraints must
/// hold. Without `optional`, absence, `null` and blank text are violations;
/// with it, an absent or `null` property passes and any present value,
/// empty text included, is checked.
pub struct PropertyValueConstraint;
impl RuleCapability for PropertyValueConstraint {
    fn id(&self) -> &'static str {
        "axioval:capability.property-value"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )];
        for name in [
            "data_type",
            "min_inclusive",
            "max_inclusive",
            "min_exclusive",
            "max_exclusive",
        ] {
            parameters.push(ParameterDescriptor::optional(name, ParameterType::String));
        }
        for name in ["values", "patterns"] {
            parameters.push(ParameterDescriptor::optional(
                name,
                ParameterType::StringList,
            ));
        }
        for name in ["length", "min_length", "max_length"] {
            parameters.push(ParameterDescriptor::optional(name, ParameterType::Integer));
        }
        for name in ["optional", "prohibited"] {
            parameters.push(ParameterDescriptor::optional(name, ParameterType::Boolean));
        }
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some(ParameterValue::PropertyReference {
            property: name,
            property_set: set,
        }) = rule.parameters.get("property")
        else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-value has no valid property reference",
            );
        };
        let constraints = match Constraints::read(rule, false) {
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
            let request = match bound_property_request(context, object, set.as_deref(), name) {
                Ok(request) => request,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            match service.resolve(&request) {
                Ok(PropertyResolution::Present(resolved)) => {
                    let property = resolved.property();
                    let verdict = judge(
                        &property.value,
                        property.data_type(),
                        "property",
                        name,
                        &constraints,
                    );
                    match forbid_if(&constraints, verdict, "property", name) {
                        Verdict::Meets => {}
                        Verdict::Fails(message) => evaluation.push_finding(finding(
                            rule,
                            object,
                            message,
                            property.evidence.clone().into_iter().collect(),
                        )),
                        Verdict::Inapplicable(reason, message) => {
                            evaluation.push_object_not_evaluated(
                                object.id.clone(),
                                reason,
                                message,
                            );
                        }
                    }
                }
                Ok(PropertyResolution::Absent(proof)) => {
                    if !constraints.optional && !constraints.prohibited {
                        evaluation.push_finding(finding(
                            rule,
                            object,
                            format!("missing required property {name}"),
                            vec![proof.evidence().clone()],
                        ));
                    }
                }
                Err(error) => {
                    let (reason, message) = property_error(error);
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

/// The verdict on a value that is there: emptiness, declared type, then
/// the constraints. `kind` names what holds it (`property`, `attribute`).
pub(crate) fn judge(
    value: &PropertyValue,
    declared: Option<&str>,
    kind: &str,
    name: &str,
    constraints: &Constraints<'_>,
) -> Verdict {
    if constraints.optional && matches!(value, PropertyValue::Null) {
        return Verdict::Meets;
    }
    if !constraints.optional && is_empty(value) {
        return Verdict::Fails(format!("missing required {kind} {name}"));
    }
    if let Some(expected) = constraints.data_type {
        match declared {
            Some(actual) if actual.eq_ignore_ascii_case(expected) => {}
            Some(actual) => {
                return Verdict::Fails(format!("{kind} {name} is {actual}, not {expected}"));
            }
            None => {
                return Verdict::Inapplicable(
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("the source does not report the type of {kind} {name}"),
                );
            }
        }
    }
    match verdict(value, constraints) {
        Verdict::Fails(why) => Verdict::Fails(format!("{kind} {name} {why}")),
        Verdict::Inapplicable(reason, message) => {
            Verdict::Inapplicable(reason, format!("{kind} {name}: {message}"))
        }
        Verdict::Meets => Verdict::Meets,
    }
}

/// For a prohibited requirement, meeting it is the violation and failing it
/// passes; a verdict that could not be reached stays undecided.
pub(crate) fn forbid_if(
    constraints: &Constraints<'_>,
    verdict: Verdict,
    kind: &str,
    name: &str,
) -> Verdict {
    if !constraints.prohibited {
        return verdict;
    }
    match verdict {
        Verdict::Meets => Verdict::Fails(format!("{kind} {name} meets a prohibited requirement")),
        Verdict::Fails(_) => Verdict::Meets,
        undecided @ Verdict::Inapplicable(..) => undecided,
    }
}

fn is_empty(value: &PropertyValue) -> bool {
    matches!(value, PropertyValue::Null)
        || matches!(value, PropertyValue::String(text) if text.trim().is_empty())
}

pub(crate) fn finding(
    rule: &CompiledRule,
    object: &Object,
    message: String,
    evidence: Vec<Evidence>,
) -> Finding {
    Finding {
        rule_id: rule.id.clone(),
        object_id: object.id.clone(),
        related: Vec::new(),
        severity: severity_of(rule),
        message,
        evidence,
    }
}

/// The report severity a rule's declared severity stands for.
pub(crate) fn severity_of(rule: &CompiledRule) -> Severity {
    match rule.severity {
        axioval_ir::contract::Severity::Error => Severity::Error,
        axioval_ir::contract::Severity::Warning => Severity::Warning,
        axioval_ir::contract::Severity::Info => Severity::Info,
    }
}

fn invalid(message: impl Into<String>) -> Verdict {
    Verdict::Inapplicable(NotEvaluatedReason::InvalidDeclaration, message.into())
}

fn verdict(value: &PropertyValue, constraints: &Constraints<'_>) -> Verdict {
    match value {
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
            bounds(Number::Integer(*actual), *actual as f64, constraints)
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
            bounds(Number::Decimal, *actual, constraints)
        }
        PropertyValue::Quantity { .. } => Verdict::Inapplicable(
            NotEvaluatedReason::IncompleteEvidence,
            "comparing a quantity needs its unit".into(),
        ),
        PropertyValue::Null => invalid("null has no value to compare"),
    }
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
    use super::{parse_double, parse_integer, within_tolerance};

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
