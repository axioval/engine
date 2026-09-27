//! Deterministic, fail-closed selector evaluation.

use axioval_engine::{
    BindingError, CapabilityEvaluation, ClassificationError, ClassificationServiceHandle,
    ConceptBindings, NotEvaluatedReason, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionServiceHandle, RuleContext, TypeHierarchyError,
    TypeHierarchyServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, Object, PropertyValue, QuantityDimension};
use regex::{Regex, RegexBuilder};
use std::cmp::Ordering;

use crate::support::{Tolerance, exact_f64, si_quantity};

pub(crate) fn select_objects<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, CapabilityEvaluation) {
    let mut selected = Vec::new();
    let mut evaluation = CapabilityEvaluation::default();
    for object in context.project.objects() {
        match selector_matches(context, selector, object, &mut Vec::new()) {
            Selection::Match => selected.push(object),
            Selection::NoMatch => {}
            Selection::NotEvaluated(reason, message) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
    (selected, evaluation)
}

#[derive(Clone, Debug)]
pub(crate) enum Selection {
    Match,
    NoMatch,
    NotEvaluated(NotEvaluatedReason, String),
}

/// Whether `object` is selected; property facts consulted are added to `evidence`.
pub(crate) fn selector_matches(
    context: &RuleContext<'_>,
    selector: &Selector,
    object: &Object,
    evidence: &mut Vec<Evidence>,
) -> Selection {
    match selector {
        Selector::All => Selection::Match,
        Selector::EntityType {
            object_type,
            include_subtypes,
        } => entity_type_matches(context, object, object_type, *include_subtypes),
        Selector::Classification {
            system,
            code,
            include_descendants,
        } => classification_matches(context, object, system, code, *include_descendants),
        Selector::AllOf { operands } => all_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object, evidence)),
        ),
        Selector::AnyOf { operands } => any_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object, evidence)),
        ),
        Selector::Not { operand } => match selector_matches(context, operand, object, evidence) {
            Selection::Match => Selection::NoMatch,
            Selection::NoMatch => Selection::Match,
            unavailable @ Selection::NotEvaluated(..) => unavailable,
        },
        Selector::Property {
            property_set,
            property,
            operator,
            value,
            case_sensitive,
            trim,
        } => property_selector_matches(
            context,
            object,
            property_set.as_deref(),
            property,
            (
                operator,
                value.as_ref(),
                TextOptions {
                    case_sensitive: *case_sensitive,
                    trim: *trim,
                },
            ),
            evidence,
        ),
    }
}

/// Vocabulary the names in a rule are written in.
///
/// A plan compiled from packages always runs with [`ConceptBindings`]: its
/// names are canonical concepts and must be translated per source. A trusted
/// host that evaluates a capability directly, without bindings, writes rules
/// in its own source vocabulary, and those names are used verbatim.
enum Vocabulary<'a> {
    Package(&'a ConceptBindings),
    Native,
}

fn vocabulary<'a>(context: &RuleContext<'a>) -> Vocabulary<'a> {
    context
        .services
        .get::<ConceptBindings>()
        .map_or(Vocabulary::Native, Vocabulary::Package)
}

fn binding_error(error: &BindingError) -> (NotEvaluatedReason, String) {
    // An unbound concept is a property of the package/source pairing, not of
    // the evidence: the package names nothing this source can express. A
    // concept no package declares is a broken declaration instead.
    let reason = match error {
        BindingError::UnknownConcept { .. } => NotEvaluatedReason::InvalidDeclaration,
        _ => NotEvaluatedReason::UnboundConcept,
    };
    (reason, error.to_string())
}

/// Builds a property request in the checked object's own source vocabulary.
///
/// Package rules reference canonical concepts, which are bound through the
/// object's declared type system first, for both the property and its
/// optional set qualifier. An unbindable concept is never passed through
/// verbatim: a source asked for a name it does not use would answer "absent"
/// and turn a vocabulary gap into a violation.
pub(crate) fn bound_property_request(
    context: &RuleContext<'_>,
    object: &Object,
    set: Option<&str>,
    name: &str,
) -> Result<PropertyRequest, (NotEvaluatedReason, String)> {
    let (property_set, property) = match vocabulary(context) {
        Vocabulary::Native => (set.map(ToOwned::to_owned), name),
        Vocabulary::Package(bindings) => {
            let source = &object.id.source;
            let property = bindings
                .property(name, source)
                .map_err(|error| binding_error(&error))?;
            // The attribute sets are engine vocabulary with one meaning in
            // every source, so they bind to themselves.
            let property_set = set
                .map(|set| {
                    if axioval_ir::is_reserved_set(set) {
                        Ok(set.to_owned())
                    } else {
                        bindings.property_set(set, source).map(ToOwned::to_owned)
                    }
                })
                .transpose()
                .map_err(|error| binding_error(&error))?;
            (property_set, property)
        }
    };
    PropertyRequest::try_new(object.id.clone(), property_set, property)
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))
}

/// Whether an object is an instance of an object type.
///
/// A kind always matches itself. Beyond that, `include_subtypes` needs the
/// source's own type hierarchy; the engine knows none, so without that service
/// a different kind is unknown rather than "no". Before this, subtypes were
/// ignored and every subtype instance was silently left out of scope.
fn entity_type_matches(
    context: &RuleContext<'_>,
    object: &Object,
    object_type: &str,
    include_subtypes: bool,
) -> Selection {
    let name = match vocabulary(context) {
        Vocabulary::Native => object_type,
        Vocabulary::Package(bindings) => match bindings.object_type(object_type, &object.id.source)
        {
            Ok(name) => name,
            Err(error) => {
                let (reason, message) = binding_error(&error);
                return Selection::NotEvaluated(reason, message);
            }
        },
    };
    // Source kinds are compared case-insensitively: STEP writes `IFCWALL` for
    // the schema's `IfcWall`, and both name the same entity.
    if object.kind().eq_ignore_ascii_case(name) {
        return Selection::Match;
    }
    if !include_subtypes {
        return Selection::NoMatch;
    }
    let Some(hierarchy) = context.services.get::<TypeHierarchyServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "type-hierarchy service is not registered; subtype membership is unknown".into(),
        );
    };
    match hierarchy.is_a(&object.id.source, object.kind(), name) {
        Ok(matches) => verdict(matches),
        Err(TypeHierarchyError::UnknownType(message)) => {
            Selection::NotEvaluated(NotEvaluatedReason::InvalidEvidence, message)
        }
        Err(error) => {
            Selection::NotEvaluated(NotEvaluatedReason::BackendUnavailable, error.to_string())
        }
    }
}

fn verdict(value: bool) -> Selection {
    if value {
        Selection::Match
    } else {
        Selection::NoMatch
    }
}

fn all_of(items: impl Iterator<Item = Selection>) -> Selection {
    let mut unavailable = None;
    for item in items {
        match item {
            Selection::NoMatch => return Selection::NoMatch,
            Selection::NotEvaluated(_, _) if unavailable.is_none() => unavailable = Some(item),
            _ => {}
        }
    }
    unavailable.unwrap_or(Selection::Match)
}

fn any_of(items: impl Iterator<Item = Selection>) -> Selection {
    let mut unavailable = None;
    for item in items {
        match item {
            Selection::Match => return Selection::Match,
            Selection::NotEvaluated(_, _) if unavailable.is_none() => unavailable = Some(item),
            _ => {}
        }
    }
    unavailable.unwrap_or(Selection::NoMatch)
}

/// Whether `object` carries `code` in `system`, as its source states.
///
/// The project's inline classification list is not consulted: an empty list
/// cannot tell "unclassified" from "never read", and treating the second as
/// the first made every classification rule over an IFC model select nothing
/// and pass. A source with no classification service is not evaluated.
fn classification_matches(
    context: &RuleContext<'_>,
    object: &Object,
    system: &str,
    code: &str,
    include_descendants: bool,
) -> Selection {
    let Some(service) = context.services.get::<ClassificationServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "classification service is not registered; classifications are unknown".into(),
        );
    };
    let assignments = match service.classifications(&object.id) {
        Ok(assignments) => assignments,
        Err(error @ ClassificationError::Unreadable(_)) => {
            return Selection::NotEvaluated(NotEvaluatedReason::InvalidEvidence, error.to_string());
        }
        Err(error) => {
            return Selection::NotEvaluated(
                NotEvaluatedReason::BackendUnavailable,
                error.to_string(),
            );
        }
    };
    let mut undecided = false;
    for assignment in &assignments {
        match assignment.matches(system, code, include_descendants) {
            Some(true) => return Selection::Match,
            Some(false) => {}
            None => undecided = true,
        }
    }
    if undecided {
        Selection::NotEvaluated(
            NotEvaluatedReason::IncompleteEvidence,
            "a classification of this object is not linked to a system".into(),
        )
    } else {
        Selection::NoMatch
    }
}

/// How a property selector compares text: case folding and trimming.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextOptions {
    pub(crate) case_sensitive: bool,
    pub(crate) trim: bool,
}

impl TextOptions {
    fn is_default(self) -> bool {
        self.case_sensitive && !self.trim
    }

    fn fold(self, text: &str) -> String {
        if self.case_sensitive {
            text.to_owned()
        } else {
            text.to_lowercase()
        }
    }

    /// The resolved value as compared: trimmed, then folded, as declared.
    fn prepare(self, text: &str) -> String {
        self.fold(if self.trim { text.trim() } else { text })
    }
}

fn property_selector_matches(
    context: &RuleContext<'_>,
    object: &Object,
    set: Option<&str>,
    name: &str,
    test: (&ComparisonOperator, Option<&ParameterValue>, TextOptions),
    evidence: &mut Vec<Evidence>,
) -> Selection {
    let (operator, expected, options) = test;
    let test = match Test::parse(operator, expected, options) {
        Ok(test) => test,
        Err(message) => {
            return Selection::NotEvaluated(
                NotEvaluatedReason::InvalidDeclaration,
                format!("property selector: {message}"),
            );
        }
    };
    let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        );
    };
    let request = match bound_property_request(context, object, set, name) {
        Ok(request) => request,
        Err((reason, message)) => return Selection::NotEvaluated(reason, message),
    };
    match service.resolve(&request) {
        Ok(PropertyResolution::Absent(proof)) => {
            evidence.push(proof.evidence().clone());
            Selection::NoMatch
        }
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.property();
            evidence.extend(property.evidence.iter().cloned());
            match test.holds(&property.value, options) {
                Ok(matches) => verdict(matches),
                Err(message) => Selection::NotEvaluated(
                    NotEvaluatedReason::InvalidEvidence,
                    format!("property selector on `{name}`: {message}"),
                ),
            }
        }
        Err(error) => unavailable(error),
    }
}

pub(crate) fn property_error(error: PropertyResolutionError) -> (NotEvaluatedReason, String) {
    match error {
        PropertyResolutionError::Unavailable(message) => {
            (NotEvaluatedReason::BackendUnavailable, message)
        }
        PropertyResolutionError::Incomplete(message) => {
            (NotEvaluatedReason::IncompleteEvidence, message)
        }
        error => (NotEvaluatedReason::InvalidEvidence, error.to_string()),
    }
}

fn unavailable(error: PropertyResolutionError) -> Selection {
    let (reason, message) = property_error(error);
    Selection::NotEvaluated(reason, message)
}

#[derive(Clone, Copy, Debug)]
enum Order {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl Order {
    fn of(operator: &ComparisonOperator) -> Option<Self> {
        Some(match operator {
            ComparisonOperator::Equals => Self::Equal,
            ComparisonOperator::NotEquals => Self::NotEqual,
            ComparisonOperator::LessThan => Self::Less,
            ComparisonOperator::LessThanOrEquals => Self::LessOrEqual,
            ComparisonOperator::GreaterThan => Self::Greater,
            ComparisonOperator::GreaterThanOrEquals => Self::GreaterOrEqual,
            _ => return None,
        })
    }

    fn is_equality(self) -> bool {
        matches!(self, Self::Equal | Self::NotEqual)
    }

    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Self::Equal => ordering.is_eq(),
            Self::NotEqual => !ordering.is_eq(),
            Self::Less => ordering.is_lt(),
            Self::LessOrEqual => ordering.is_le(),
            Self::Greater => ordering.is_gt(),
            Self::GreaterOrEqual => ordering.is_ge(),
        }
    }
}

/// The declared side of an ordered comparison, in canonical form.
#[derive(Debug)]
enum Expected {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    /// In SI, with its dimension.
    Quantity(f64, QuantityDimension),
    /// Folded as declared.
    Text(String),
}

/// A property selector's comparison, validated against its declaration.
#[derive(Debug)]
enum Test {
    Exists,
    Compare(Order, Expected),
    /// Folded as declared.
    Contains(String),
    /// `matches` or `like`, anchored to the whole value.
    Pattern(Regex),
    /// Folded as declared; `none` for `noneOf`.
    Member {
        texts: Vec<String>,
        none: bool,
    },
}

impl Test {
    fn parse(
        operator: &ComparisonOperator,
        expected: Option<&ParameterValue>,
        options: TextOptions,
    ) -> Result<Self, String> {
        let Some(expected) = expected else {
            return if matches!(operator, ComparisonOperator::Exists) {
                if options.is_default() {
                    Ok(Self::Exists)
                } else {
                    Err("`caseSensitive` and `trim` apply to text comparisons only".into())
                }
            } else {
                Err("comparison has no expected value".into())
            };
        };
        let string = |kind: &str| match expected {
            ParameterValue::String { value } => Ok(value.as_str()),
            _ => Err(format!("`{}` takes {kind}", operator_name(operator))),
        };
        let test = match operator {
            ComparisonOperator::Exists => {
                return Err("the exists operator must not have a value".into());
            }
            ComparisonOperator::Matches => Self::Pattern(
                RegexBuilder::new(&format!("^(?:{})$", string("a string")?))
                    .case_insensitive(!options.case_sensitive)
                    .build()
                    .map_err(|error| format!("invalid regular expression: {error}"))?,
            ),
            ComparisonOperator::Like => Self::Pattern(
                RegexBuilder::new(&wildcard(string("a string")?)?)
                    .case_insensitive(!options.case_sensitive)
                    .build()
                    .map_err(|error| format!("invalid wildcard pattern: {error}"))?,
            ),
            ComparisonOperator::Contains => Self::Contains(options.fold(string("a string")?)),
            ComparisonOperator::OneOf | ComparisonOperator::NoneOf => match expected {
                ParameterValue::StringList { value } => Self::Member {
                    texts: value.iter().map(|text| options.fold(text)).collect(),
                    none: matches!(operator, ComparisonOperator::NoneOf),
                },
                _ => {
                    return Err(format!("`{}` takes a string list", operator_name(operator)));
                }
            },
            ordered => {
                let order = Order::of(ordered).expect("every other operator is ordered");
                let value = match expected {
                    ParameterValue::Boolean { value } if order.is_equality() => {
                        Expected::Boolean(*value)
                    }
                    ParameterValue::Integer { value } => Expected::Integer(*value),
                    ParameterValue::Number { value } if value.is_finite() => {
                        Expected::Number(*value)
                    }
                    ParameterValue::Quantity { value, unit } => {
                        let (value, dimension) =
                            si_quantity(*value, unit).map_err(|(_, message)| message)?;
                        Expected::Quantity(value, dimension)
                    }
                    ParameterValue::String { value } => Expected::Text(options.fold(value)),
                    ParameterValue::Enum { value } | ParameterValue::Reference { value }
                        if order.is_equality() =>
                    {
                        Expected::Text(options.fold(value))
                    }
                    _ => {
                        return Err(format!(
                            "`{}` does not apply to this value",
                            operator_name(operator)
                        ));
                    }
                };
                if !matches!(value, Expected::Text(_)) && !options.is_default() {
                    return Err("`caseSensitive` and `trim` apply to text comparisons only".into());
                }
                Self::Compare(order, value)
            }
        };
        Ok(test)
    }

    /// Whether `actual` satisfies the test; `Err` when the value's type
    /// cannot be compared with the declared one, so the object is not
    /// evaluated rather than silently left out.
    fn holds(&self, actual: &PropertyValue, options: TextOptions) -> Result<bool, String> {
        // A comparison presupposes a value: null is no more a match than an
        // absent property.
        if matches!(actual, PropertyValue::Null) {
            return Ok(matches!(self, Self::Exists));
        }
        let mismatch = |declared: &str| {
            Err(format!(
                "the value is {} but the selector compares {declared}",
                kind(actual)
            ))
        };
        match self {
            Self::Exists => Ok(true),
            Self::Compare(order, expected) => {
                let ordering = match (expected, actual) {
                    (Expected::Boolean(expected), PropertyValue::Boolean(actual)) => {
                        actual.cmp(expected)
                    }
                    (Expected::Integer(expected), PropertyValue::Integer(actual)) => {
                        actual.cmp(expected)
                    }
                    (Expected::Integer(expected), PropertyValue::Decimal(actual)) => {
                        numeric(*actual, exact_f64(*expected))?
                    }
                    (Expected::Number(expected), PropertyValue::Decimal(actual)) => {
                        numeric(*actual, Some(*expected))?
                    }
                    (Expected::Number(expected), PropertyValue::Integer(actual)) => {
                        numeric_integer(*actual, *expected)?
                    }
                    (
                        Expected::Quantity(expected, dimension),
                        PropertyValue::Quantity {
                            value,
                            dimension: held,
                        },
                    ) => {
                        if held != dimension {
                            return Err(format!(
                                "a quantity in {} cannot be compared with one in {}",
                                held.unit_symbol(),
                                dimension.unit_symbol()
                            ));
                        }
                        Tolerance::unit_conversion()
                            .order(*value, *expected)
                            .ok_or("the quantity is not finite")?
                    }
                    (Expected::Text(expected), PropertyValue::String(actual)) => {
                        options.prepare(actual).as_str().cmp(expected.as_str())
                    }
                    (Expected::Boolean(_), _) => return mismatch("a boolean"),
                    (Expected::Integer(_) | Expected::Number(_), _) => {
                        return mismatch("a unit-less number");
                    }
                    (Expected::Quantity(_, dimension), _) => {
                        return mismatch(&format!("a quantity in {}", dimension.unit_symbol()));
                    }
                    (Expected::Text(_), _) => return mismatch("text"),
                };
                Ok(order.holds(ordering))
            }
            Self::Contains(text) => match actual {
                PropertyValue::String(actual) => Ok(options.prepare(actual).contains(text)),
                _ => mismatch("text"),
            },
            Self::Pattern(pattern) => match actual {
                PropertyValue::String(actual) => {
                    Ok(pattern.is_match(if options.trim { actual.trim() } else { actual }))
                }
                _ => mismatch("text"),
            },
            Self::Member { texts, none } => match actual {
                PropertyValue::String(actual) => {
                    Ok(texts.contains(&options.prepare(actual)) != *none)
                }
                _ => mismatch("text"),
            },
        }
    }
}

fn numeric(actual: f64, expected: Option<f64>) -> Result<Ordering, String> {
    match expected {
        Some(expected) => actual
            .partial_cmp(&expected)
            .ok_or_else(|| "the value is not a finite number".into()),
        None => Err("an integer beyond 2^53 cannot be compared with a number".into()),
    }
}

fn numeric_integer(actual: i64, expected: f64) -> Result<Ordering, String> {
    let actual = exact_f64(actual)
        .ok_or_else(|| "an integer beyond 2^53 cannot be compared with a number".to_owned())?;
    numeric(actual, Some(expected))
}

fn kind(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".into(),
        PropertyValue::Boolean(_) => "a boolean".into(),
        PropertyValue::Integer(_) => "an integer".into(),
        PropertyValue::Decimal(_) => "a number".into(),
        PropertyValue::Quantity { dimension, .. } => {
            format!("a quantity in {}", dimension.unit_symbol())
        }
        PropertyValue::String(_) => "text".into(),
    }
}

fn operator_name(operator: &ComparisonOperator) -> &'static str {
    match operator {
        ComparisonOperator::Equals => "equals",
        ComparisonOperator::NotEquals => "notEquals",
        ComparisonOperator::LessThan => "lessThan",
        ComparisonOperator::LessThanOrEquals => "lessThanOrEquals",
        ComparisonOperator::GreaterThan => "greaterThan",
        ComparisonOperator::GreaterThanOrEquals => "greaterThanOrEquals",
        ComparisonOperator::Matches => "matches",
        ComparisonOperator::Like => "like",
        ComparisonOperator::Contains => "contains",
        ComparisonOperator::OneOf => "oneOf",
        ComparisonOperator::NoneOf => "noneOf",
        ComparisonOperator::Exists => "exists",
    }
}

/// A wildcard pattern as an anchored regular expression.
///
/// `*` is any run of characters (none included), `?` exactly one, and a
/// backslash makes the next character literal (`\*`, `\?`, `\\`). Every
/// other character is literal.
fn wildcard(pattern: &str) -> Result<String, String> {
    let mut out = String::from("(?s)^");
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or("a wildcard pattern ends with a backslash")?;
                out.push_str(&regex::escape(escaped.encode_utf8(&mut [0; 4])));
            }
            other => out.push_str(&regex::escape(other.encode_utf8(&mut [0; 4]))),
        }
    }
    out.push('$');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::wildcard;

    #[test]
    fn wildcards_translate_to_anchored_literals() {
        assert_eq!(wildcard("W?-*.1").unwrap(), r"(?s)^W.\-.*\.1$");
        assert_eq!(wildcard(r"a\*b").unwrap(), r"(?s)^a\*b$");
        assert!(wildcard("a\\").is_err());
    }
}
