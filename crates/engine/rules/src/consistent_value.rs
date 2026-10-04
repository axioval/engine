//! Objects that agree on one property must agree on another.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Object, PropertyValue, QuantityDimension};

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Unavailable, display, exact_f64, finding, invalid, resolve, scope_key,
    undefined, value_key,
};

/// Requires objects sharing a `key` value to share their `value` too.
///
/// Doors of one type mark must have one fire rating; walls of one
/// construction type one thickness class. Objects are grouped by their key
/// value, compared without regard to case unless `case_sensitive` is `true`,
/// within one source (or the project with `across_sources`), of one kind
/// unless `same_kind` is `false`, and optionally within the objects a
/// declared `relationship` reaches, such as one storey.
///
/// A group whose members disagree raises one finding per member, naming the
/// members that hold another value. An absent value is a value of its own:
/// "some doors of this type state a rating and some do not" is a
/// disagreement. Objects with no key value (absent, null or blank) form one
/// group of their own in each scope, so a missing key is reported only when
/// those objects disagree on the value.
///
/// With `tolerance` (a number, applied to numbers and to quantities in SI
/// units) or `tolerance_quantity` (a quantity, applied to quantities of its
/// dimension), numeric values agree when the group's range, its greatest
/// value less its least, is within the tolerance. A measured interval counts
/// with its full width, and a group whose range may lie on either side of
/// the tolerance is not evaluated. A group beyond it reports each member
/// farther than the tolerance from the group's median, naming the others; a
/// member whose distance straddles the tolerance is not evaluated. A value
/// the tolerance does not apply to (a quantity of another dimension than
/// `tolerance_quantity`, a number under it) leaves its object not evaluated.
/// Numbers against text, or against an absent value, still disagree.
pub struct ConsistentValue;

/// The group key of objects whose key is absent, null or blank. Real keys
/// carry a `text:` or `value:` prefix, so this cannot collide with one.
const NO_KEY: &str = "no key";

/// A group's key as shown (`None` without a key) and its members.
type Group<'a> = (Option<String>, Vec<Member<'a>>);

struct Member<'a> {
    object: &'a Object,
    /// The value's class: its exact key, or with a tolerance its numeric
    /// domain.
    value: String,
    shown: String,
    /// With a tolerance, the interval a numeric value lies in.
    span: Option<(f64, f64)>,
    evidence: Vec<Evidence>,
}

/// A declared tolerance on numeric values.
#[derive(Clone, Copy)]
enum Spread {
    /// Applies to numbers, and to quantities in SI units.
    Number(f64),
    /// Applies to quantities of its dimension only, in SI units.
    Quantity(f64, QuantityDimension),
}

impl Spread {
    fn amount(self) -> f64 {
        match self {
            Self::Number(amount) | Self::Quantity(amount, _) => amount,
        }
    }

    fn shown(self) -> String {
        match self {
            Self::Number(amount) => amount.to_string(),
            Self::Quantity(amount, dimension) => format!("{amount} {}", dimension.unit_symbol()),
        }
    }

    /// The numeric domain and interval of `value`; `None` for a value
    /// compared exactly, an error where the tolerance cannot apply.
    fn span(self, value: &PropertyValue) -> Option<Result<(String, f64, f64), String>> {
        let (dimension, lower, upper) = match value {
            PropertyValue::Integer(number) => match exact_f64(*number) {
                Some(number) => (None, number, number),
                None => {
                    return Some(Err(
                        "integer cannot be represented exactly as a decimal".into()
                    ));
                }
            },
            PropertyValue::Decimal(number) => (None, *number, *number),
            PropertyValue::Quantity { value, dimension } => (Some(*dimension), *value, *value),
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            } => (*dimension, *lower, *upper),
            _ => return None,
        };
        if let Self::Quantity(_, wanted) = self {
            if dimension != Some(wanted) {
                return Some(Err(format!(
                    "the tolerance {} does not apply to {}",
                    self.shown(),
                    display(Some(value))
                )));
            }
        }
        if !(lower.is_finite() && upper.is_finite() && lower <= upper) {
            return Some(Err(format!(
                "{} is not a finite value",
                display(Some(value))
            )));
        }
        let domain = dimension.map_or_else(
            || "number".to_owned(),
            |dimension| format!("quantity:{}", dimension.unit_symbol()),
        );
        Some(Ok((domain, lower, upper)))
    }
}

impl RuleCapability for ConsistentValue {
    fn id(&self) -> &'static str {
        "axioval:capability.consistent-value"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("key", ParameterType::PropertyReference),
            ParameterDescriptor::required("value", ParameterType::PropertyReference),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("same_kind", ParameterType::Boolean),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
            ParameterDescriptor::optional("tolerance", ParameterType::Number),
            ParameterDescriptor::optional("tolerance_quantity", ParameterType::Quantity),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .collect()
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let spread = match (
                parameters.number("tolerance")?,
                parameters.quantity("tolerance_quantity")?,
            ) {
                (None, None) => None,
                (Some(amount), None) => Some(Spread::Number(amount)),
                (None, Some((amount, dimension))) => Some(Spread::Quantity(amount, dimension)),
                (Some(_), Some(_)) => {
                    return Err(invalid(
                        "declare either `tolerance` or `tolerance_quantity`, not both",
                    ));
                }
            };
            if spread.is_some_and(|spread| spread.amount() < 0.0) {
                return Err(invalid("the tolerance is negative"));
            }
            Ok::<_, Unavailable>((
                parameters.required_property("key")?,
                parameters.required_property("value")?,
                parameters.boolean("case_sensitive")?.unwrap_or(false),
                parameters.boolean("same_kind")?.unwrap_or(true),
                parameters.boolean("across_sources")?.unwrap_or(false),
                parameters.traversal()?,
                spread,
            ))
        })();
        let (key, value, case_sensitive, same_kind, across_sources, traversal, spread) =
            match parsed {
                Ok(parsed) => parsed,
                Err((reason, message)) => {
                    return CapabilityEvaluation::not_evaluated(
                        reason,
                        format!("consistent-value: {message}"),
                    );
                }
            };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let mut groups: BTreeMap<(String, String, String), Group<'_>> = BTreeMap::new();
        for object in selected {
            let answers = resolve(context, object, key)
                .and_then(|found_key| Ok((found_key, resolve(context, object, value)?)));
            let (found_key, found_value) = match answers {
                Ok(answers) => answers,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let (scope, mut evidence) =
                match scope_key(context, traversal.as_ref(), across_sources, object) {
                    Ok(scope) => scope,
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                        continue;
                    }
                };
            evidence.extend(found_key.evidence());
            evidence.extend(found_value.evidence());
            let kind = if same_kind {
                object.kind().to_ascii_lowercase()
            } else {
                String::new()
            };
            let key_value = found_key.value().filter(|held| !undefined(Some(held)));
            let numeric = spread.and_then(|spread| spread.span(found_value.value()?));
            let (class, span) = match numeric {
                Some(Ok((domain, lower, upper))) => {
                    (format!("numeric:{domain}"), Some((lower, upper)))
                }
                Some(Err(message)) => {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        NotEvaluatedReason::InvalidEvidence,
                        format!("consistent-value: {message}"),
                    );
                    continue;
                }
                None => (
                    found_value.value().map_or_else(
                        || "absent".to_owned(),
                        |held| value_key(held, true, case_sensitive),
                    ),
                    None,
                ),
            };
            let member = Member {
                object,
                value: class,
                shown: display(found_value.value()),
                span,
                evidence,
            };
            // Objects without a key form one group of their own, so a missing
            // key is reported only where their values conflict.
            let group_key = key_value.map_or_else(
                || NO_KEY.to_owned(),
                |held| value_key(held, true, case_sensitive),
            );
            groups
                .entry((scope, kind, group_key))
                .or_insert_with(|| (key_value.map(|held| display(Some(held))), Vec::new()))
                .1
                .push(member);
        }
        let names = Names { key, value };
        for (shown_key, members) in groups.into_values() {
            let distinct: BTreeSet<&str> = members.iter().map(|m| m.value.as_str()).collect();
            if distinct.len() < 2 {
                if let (Some(spread), Some(first)) = (spread, members.first()) {
                    if first.span.is_some() {
                        let unit = first.value.strip_prefix("numeric:quantity:");
                        let unit = unit.map(|unit| format!(" {unit}")).unwrap_or_default();
                        let group = SpreadGroup {
                            rule,
                            names,
                            shown_key: shown_key.as_deref(),
                            members: &members,
                            spread,
                            unit: &unit,
                        };
                        group.judge(&mut evaluation);
                    }
                }
                continue;
            }
            for member in &members {
                let others: Vec<&Member<'_>> = members
                    .iter()
                    .filter(|other| other.value != member.value)
                    .collect();
                let mut seen = BTreeSet::new();
                // Numbers of one domain are one class under a tolerance;
                // each is still shown.
                let other_values: Vec<&str> = others
                    .iter()
                    .filter(|other| {
                        seen.insert(if other.span.is_some() {
                            other.shown.as_str()
                        } else {
                            other.value.as_str()
                        })
                    })
                    .map(|other| other.shown.as_str())
                    .collect();
                let mut evidence = member.evidence.clone();
                evidence.extend(
                    others
                        .iter()
                        .flat_map(|other| other.evidence.iter().cloned()),
                );
                evaluation.push_finding(finding(
                    rule,
                    &member.object.id,
                    match &shown_key {
                        Some(shown_key) => format!(
                            "{value} is {} where other objects with {key} {shown_key} have {}",
                            member.shown,
                            other_values.join(", ")
                        ),
                        None => format!(
                            "{key} has no value, and {value} is {} where other objects \
                             without {key} have {}",
                            member.shown,
                            other_values.join(", ")
                        ),
                    },
                    evidence,
                    others.iter().map(|other| other.object.id.clone()).collect(),
                ));
            }
        }
        evaluation
    }
}

/// The key and value properties, for messages.
#[derive(Clone, Copy)]
struct Names<'a> {
    key: PropertyRef<'a>,
    value: PropertyRef<'a>,
}

/// One group of numeric values of one domain under a tolerance.
struct SpreadGroup<'r, 'a> {
    rule: &'r CompiledRule,
    names: Names<'r>,
    shown_key: Option<&'r str>,
    members: &'r [Member<'a>],
    spread: Spread,
    /// The domain's unit as shown, with a leading space, or empty.
    unit: &'r str,
}

impl SpreadGroup<'_, '_> {
    fn judge(&self, evaluation: &mut CapabilityEvaluation) {
        let spans: Vec<(f64, f64)> = self
            .members
            .iter()
            .map(|member| member.span.expect("a numeric member has a span"))
            .collect();
        let tolerance = self.spread.amount();
        // Binary rounding of decimal inputs and of the difference itself.
        let magnitude = spans
            .iter()
            .flat_map(|(lower, upper)| [lower.abs(), upper.abs()])
            .fold(tolerance, f64::max);
        let slack = 4.0 * f64::EPSILON * magnitude;
        let beyond = |distance: f64| distance > tolerance + slack;
        let lowers = spans.iter().map(|span| span.0);
        let uppers = spans.iter().map(|span| span.1);
        let (least_lower, greatest_lower) = (
            lowers.clone().fold(f64::INFINITY, f64::min),
            lowers.fold(f64::NEG_INFINITY, f64::max),
        );
        let (least_upper, greatest_upper) = (
            uppers.clone().fold(f64::INFINITY, f64::min),
            uppers.fold(f64::NEG_INFINITY, f64::max),
        );
        if !beyond(greatest_upper - least_lower) {
            return;
        }
        let (key, value) = (self.names.key, self.names.value);
        let objects = match self.shown_key {
            Some(shown_key) => format!("objects with {key} {shown_key}"),
            None => format!("objects without {key}"),
        };
        if !beyond(greatest_lower - least_upper) {
            for member in self.members {
                evaluation.push_object_not_evaluated(
                    member.object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "consistent-value: the range of {value} over the {objects} may lie on \
                         either side of the tolerance {}",
                        self.spread.shown()
                    ),
                );
            }
            return;
        }
        let median_lower = median(spans.iter().map(|span| span.0).collect());
        let median_upper = median(spans.iter().map(|span| span.1).collect());
        let median = if median_lower.total_cmp(&median_upper).is_eq() {
            format!("{median_lower}{}", self.unit)
        } else {
            format!("{median_lower} to {median_upper}{}", self.unit)
        };
        let mut outliers = Vec::new();
        let mut undecided = Vec::new();
        for (index, (lower, upper)) in spans.iter().enumerate() {
            let least = (lower - median_upper).max(median_lower - upper).max(0.0);
            let greatest = (upper - median_lower).max(median_upper - lower);
            if beyond(least) {
                outliers.push(index);
            } else if beyond(greatest) {
                undecided.push(index);
            }
        }
        let message = if outliers.is_empty() && undecided.is_empty() {
            // Every value lies within the tolerance of the median, yet the
            // range exceeds it: the members at its ends span it.
            outliers = spans
                .iter()
                .enumerate()
                .filter(|(_, (lower, upper))| {
                    lower.total_cmp(&least_lower).is_eq()
                        || upper.total_cmp(&greatest_upper).is_eq()
                })
                .map(|(index, _)| index)
                .collect();
            format!(
                "at an end of the range {least_lower} to {greatest_upper}{unit} of {value} over \
                 the {objects}, which exceeds the tolerance {tolerance}",
                unit = self.unit,
                tolerance = self.spread.shown()
            )
        } else {
            format!(
                "farther than the tolerance {} from the median {median} of the {objects}",
                self.spread.shown()
            )
        };
        self.report(&outliers, &message, evaluation);
        for index in undecided {
            evaluation.push_object_not_evaluated(
                self.members[index].object.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "consistent-value: {value} {} may lie within the tolerance {} of the median \
                     {median} of the {objects} or beyond it",
                    self.members[index].shown,
                    self.spread.shown()
                ),
            );
        }
    }

    /// One finding per member at `outliers`, relating every other member.
    fn report(&self, outliers: &[usize], message: &str, evaluation: &mut CapabilityEvaluation) {
        let value = self.names.value;
        for &index in outliers {
            let member = &self.members[index];
            let others: Vec<&Member<'_>> = self
                .members
                .iter()
                .filter(|other| other.object.id != member.object.id)
                .collect();
            let mut evidence = member.evidence.clone();
            evidence.extend(
                others
                    .iter()
                    .flat_map(|other| other.evidence.iter().cloned()),
            );
            evaluation.push_finding(finding(
                self.rule,
                &member.object.id,
                format!("{value} is {}, {message}", member.shown),
                evidence,
                others.iter().map(|other| other.object.id.clone()).collect(),
            ));
        }
    }
}

/// The median of `values`: the middle one, or the mean of the middle two.
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        f64::midpoint(values[middle - 1], values[middle])
    }
}
