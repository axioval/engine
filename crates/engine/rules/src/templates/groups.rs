//! Group decisions ([`axioval_engine::template::Decision::Unique`],
//! [`axioval_engine::template::Decision::Consistent`]): each object's
//! stated value compared with those of the other objects of its group.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::template::{Consistent, ConsistentMessages, Unique};
use axioval_engine::{CapabilityEvaluation, CompiledRule, RuleContext};
use axioval_ir::contract::ScalarValue;
use axioval_ir::{Evidence, NotEvaluatedReason, Object, PropertyValue, QuantityDimension};

use super::{Constant, Outcome, Plan, Read, read_values, render};
use crate::expression_leaves::ObjectLeaves;
use crate::selection::select_objects;
use crate::support::{Parameters, display, finding, scope_key, undefined, value_key};

/// An object holding a value: the object, the value, its evidence.
type Holder<'a> = (&'a Object, PropertyValue, Vec<Evidence>);

/// A boolean parameter of the plan (or its default), false where unstated.
fn flag(plan: &Plan<'_>, name: &str) -> bool {
    match plan.constants.get(name) {
        Some(Constant::Scalar(ScalarValue::Boolean { value })) => *value,
        _ => false,
    }
}

/// A number's comparison domain (unit-less, or one quantity dimension) and
/// its value, in SI for a quantity; `None` for anything that is not a
/// number.
fn numeric(value: &PropertyValue) -> Option<Result<(String, f64), String>> {
    match value {
        PropertyValue::Decimal(value) => Some(Ok(("number".into(), *value))),
        PropertyValue::Integer(value) => Some(if value.unsigned_abs() <= 1 << 53 {
            #[allow(clippy::cast_precision_loss)]
            Ok(("number".into(), *value as f64))
        } else {
            Err("integer cannot be represented exactly as a decimal".into())
        }),
        PropertyValue::Quantity { value, dimension } => Some(Ok((
            format!("quantity:{}", dimension.unit_symbol()),
            *value,
        ))),
        _ => None,
    }
}

/// Runs a form deciding each object against its group.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    plan: &Plan<'_>,
    value: &'static str,
    unique: &Unique,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let decision = &plan.form.decision;
    let across = flag(plan, unique.across);
    let trim = flag(plan, unique.trim);
    let case_sensitive = flag(plan, unique.case_sensitive);
    let require = flag(plan, unique.require);
    // Checked by the declaration (`Check::Tolerance`).
    let tolerance = Parameters(rule).tolerance().unwrap_or_default();
    let traversal = Parameters(rule).traversal().ok().flatten();
    let (selected, mut evaluation) = select_objects(context, &rule.selector);
    // (group, value) -> objects holding it.
    let mut holders: BTreeMap<(String, String), Vec<Holder<'_>>> = BTreeMap::new();
    // (group, domain) -> numbers compared pair by pair under a tolerance.
    let mut near: BTreeMap<(String, String), Vec<(Holder<'_>, f64)>> = BTreeMap::new();
    for object in selected {
        let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
        let mut read = Read::default();
        match read_values(
            plan,
            plan.values(),
            &|name| super::judges_stated(decision, name),
            context,
            object,
            &mut leaves,
            &mut read,
        ) {
            None => {}
            Some(Outcome::Open(reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                continue;
            }
            Some(Outcome::Finding {
                message, evidence, ..
            }) => {
                evaluation.push_finding(finding(rule, &object.id, message, evidence, vec![]));
                continue;
            }
            Some(Outcome::Passed) => continue,
        }
        let stated = read.stated.get(value).cloned().flatten();
        if undefined(stated.as_ref()) {
            if require {
                evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    render(plan, &read, unique.missing),
                    read.evidence,
                    vec![],
                ));
            }
            continue;
        }
        let (group, mut evidence) = match scope_key(context, traversal.as_ref(), across, object) {
            Ok(group) => group,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                continue;
            }
        };
        evidence.extend(read.evidence);
        let stated = stated.expect("an undefined value was handled above");
        let number = if tolerance.is_exact() {
            None
        } else {
            numeric(&stated)
        };
        let key = value_key(&stated, trim, case_sensitive);
        let holder = (object, stated, evidence);
        match number {
            None => holders.entry((group, key)).or_default().push(holder),
            Some(Err(message)) => evaluation.push_object_not_evaluated(
                object.id.clone(),
                NotEvaluatedReason::InvalidEvidence,
                message,
            ),
            // Rounding is transitive: the rounded value is the key.
            Some(Ok((domain, number))) if tolerance.rounds() => holders
                .entry((group, format!("{domain}:{}", tolerance.round(number))))
                .or_default()
                .push(holder),
            Some(Ok((domain, number))) => {
                near.entry((group, domain))
                    .or_default()
                    .push((holder, number));
            }
        }
    }
    let suffix = tolerance.suffix();
    for group in holders.into_values() {
        report(
            plan,
            rule,
            value,
            &suffix,
            &group,
            |_, _| true,
            &mut evaluation,
        );
    }
    for group in near.into_values() {
        let (group, numbers): (Vec<_>, Vec<_>) = group.into_iter().unzip();
        let near = |a: usize, b: usize| tolerance.equal(numbers[a], numbers[b]);
        report(plan, rule, value, &suffix, &group, near, &mut evaluation);
    }
    evaluation
}

/// Reports each holder that is `near` another one of its group, relating
/// exactly those: every other one for an equal key, and for a tolerance
/// the ones within it of this holder's own value.
fn report(
    plan: &Plan<'_>,
    rule: &CompiledRule,
    value: &'static str,
    suffix: &str,
    group: &[Holder<'_>],
    near: impl Fn(usize, usize) -> bool,
    evaluation: &mut CapabilityEvaluation,
) {
    for (index, (object, stated, own)) in group.iter().enumerate() {
        let neighbours: Vec<&Holder<'_>> = group
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index && near(index, *other))
            .map(|(_, holder)| holder)
            .collect();
        if neighbours.is_empty() {
            continue;
        }
        let evidence = own
            .iter()
            .chain(neighbours.iter().flat_map(|(_, _, evidence)| evidence))
            .cloned()
            .collect();
        let related = std::iter::once(object.id.clone())
            .chain(neighbours.iter().map(|(other, _, _)| other.id.clone()))
            .collect();
        let mut read = Read::default();
        read.stated.insert(value, Some(stated.clone()));
        read.named.insert("others", neighbours.len().to_string());
        read.named.insert("tolerance:suffix", suffix.to_owned());
        evaluation.push_finding(finding(
            rule,
            &object.id,
            render(plan, &read, plan.form.fail),
            evidence,
            related,
        ));
    }
}

/// A declared tolerance on the numeric values of a [`Consistent`] group.
#[derive(Clone, Copy)]
enum Spread {
    /// Applies to numbers, and to quantities in SI units.
    Number(f64),
    /// Applies to quantities of its dimension only, in SI units.
    Quantity(f64, QuantityDimension),
}

impl Spread {
    /// The rule's tolerance, where it declares one.
    fn of(plan: &Plan<'_>, consistent: &Consistent) -> Option<Self> {
        match (
            plan.constants.get(consistent.tolerance),
            plan.constants.get(consistent.tolerance_quantity),
        ) {
            (Some(Constant::Scalar(ScalarValue::Number { value })), _) => {
                Some(Self::Number(*value))
            }
            (_, Some(Constant::Quantity(value, dimension))) => {
                Some(Self::Quantity(*value, *dimension))
            }
            _ => None,
        }
    }

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
    /// compared exactly, the message naming why where the tolerance cannot
    /// apply.
    fn span(
        self,
        value: &PropertyValue,
        messages: &ConsistentMessages,
    ) -> Option<Result<(String, f64, f64), &'static str>> {
        let (dimension, lower, upper) = match value {
            PropertyValue::Integer(number) => match crate::support::exact_f64(*number) {
                Some(number) => (None, number, number),
                None => return Some(Err(messages.inexact)),
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
        if let Self::Quantity(_, wanted) = self
            && dimension != Some(wanted)
        {
            return Some(Err(messages.inapplicable));
        }
        if !(lower.is_finite() && upper.is_finite() && lower <= upper) {
            return Some(Err(messages.not_finite));
        }
        let domain = dimension.map_or_else(
            || "number".to_owned(),
            |dimension| format!("quantity:{}", dimension.unit_symbol()),
        );
        Some(Ok((domain, lower, upper)))
    }
}

/// The group key of objects whose key is absent, `null` or blank. Real
/// keys carry a `text:` or `value:` prefix, so this cannot collide.
const NO_KEY: &str = "no key";

/// One object of a [`Consistent`] group.
struct Member<'a> {
    object: &'a Object,
    /// The value's class: its exact key, or with a tolerance its numeric
    /// domain.
    class: String,
    stated: Option<PropertyValue>,
    /// With a tolerance, the interval a numeric value lies in.
    span: Option<(f64, f64)>,
    evidence: Vec<Evidence>,
}

/// A [`Consistent`] group: the key as the first object stated it (`None`
/// without one) and its members.
type Group<'a> = (Option<PropertyValue>, Vec<Member<'a>>);

/// How a [`Consistent`] decision words one member of a group: the names it
/// reads, each object's stated key and value, and its message parts.
struct Words<'p, 't> {
    plan: &'p Plan<'t>,
    names: (&'static str, &'static str),
    messages: &'p ConsistentMessages,
    spread: Option<Spread>,
}

impl Words<'_, '_> {
    /// What the messages read of `member` in the group keyed `key`.
    fn read(&self, key: Option<&PropertyValue>, member: &Member<'_>) -> Read {
        let mut read = Read::default();
        read.stated.insert(self.names.0, key.cloned());
        read.stated.insert(self.names.1, member.stated.clone());
        if let Some(spread) = self.spread {
            read.named.insert("spread", spread.shown());
        }
        let objects = render(
            self.plan,
            &read,
            if key.is_some() {
                self.messages.keyed
            } else {
                self.messages.unkeyed
            },
        );
        read.named.insert("objects", objects);
        read
    }
}

/// Runs a form deciding whether the objects sharing a key share a value.
#[allow(clippy::too_many_lines)]
pub(super) fn consistent(
    plan: &Plan<'_>,
    (key, value): (&'static str, &'static str),
    consistent: &Consistent,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let decision = &plan.form.decision;
    let messages = &consistent.messages;
    let across = flag(plan, consistent.across);
    let case_sensitive = flag(plan, consistent.case_sensitive);
    let same_kind = flag(plan, consistent.same_kind);
    let spread = Spread::of(plan, consistent);
    // Checked by the declaration (`Check::Traversal`).
    let traversal = Parameters(rule).traversal().ok().flatten();
    let (selected, mut evaluation) = select_objects(context, &rule.selector);
    let mut groups: BTreeMap<(String, String, String), Group<'_>> = BTreeMap::new();
    for object in selected {
        let mut leaves = ObjectLeaves::new(context, object, Some(&rule.parameters));
        let mut read = Read::default();
        match read_values(
            plan,
            plan.values(),
            &|name| super::judges_stated(decision, name),
            context,
            object,
            &mut leaves,
            &mut read,
        ) {
            None => {}
            Some(Outcome::Open(reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                continue;
            }
            Some(Outcome::Finding {
                message, evidence, ..
            }) => {
                evaluation.push_finding(finding(rule, &object.id, message, evidence, vec![]));
                continue;
            }
            Some(Outcome::Passed) => continue,
        }
        let (scope, mut evidence) = match scope_key(context, traversal.as_ref(), across, object) {
            Ok(scope) => scope,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                continue;
            }
        };
        evidence.append(&mut read.evidence);
        let kind = if same_kind {
            object.kind().to_ascii_lowercase()
        } else {
            String::new()
        };
        let found_key = read
            .stated
            .get(key)
            .cloned()
            .flatten()
            .filter(|held| !undefined(Some(held)));
        let stated = read.stated.get(value).cloned().flatten();
        let numeric = spread.and_then(|spread| spread.span(stated.as_ref()?, messages));
        let (class, span) = match numeric {
            Some(Ok((domain, lower, upper))) => (format!("numeric:{domain}"), Some((lower, upper))),
            Some(Err(message)) => {
                if let Some(spread) = spread {
                    read.named.insert("spread", spread.shown());
                }
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidEvidence,
                    render(plan, &read, message),
                );
                continue;
            }
            None => (
                stated.as_ref().map_or_else(
                    || "absent".to_owned(),
                    |held| value_key(held, true, case_sensitive),
                ),
                None,
            ),
        };
        // Objects without a key form one group of their own, so a missing
        // key is reported only where their values conflict.
        let group_key = found_key.as_ref().map_or_else(
            || NO_KEY.to_owned(),
            |held| value_key(held, true, case_sensitive),
        );
        groups
            .entry((scope, kind, group_key))
            .or_insert_with(|| (found_key, Vec::new()))
            .1
            .push(Member {
                object,
                class,
                stated,
                span,
                evidence,
            });
    }
    let words = Words {
        plan,
        names: (key, value),
        messages,
        spread,
    };
    for (shown_key, members) in groups.into_values() {
        let distinct: BTreeSet<&str> = members.iter().map(|m| m.class.as_str()).collect();
        if distinct.len() < 2 {
            if let (Some(spread), Some(first)) = (spread, members.first())
                && first.span.is_some()
            {
                let unit = first
                    .class
                    .strip_prefix("numeric:quantity:")
                    .map(|unit| format!(" {unit}"))
                    .unwrap_or_default();
                let group = Spreading {
                    words: &words,
                    rule,
                    key: shown_key.as_ref(),
                    members: &members,
                    spread,
                    unit: &unit,
                };
                group.judge(&mut evaluation);
            }
            continue;
        }
        for member in &members {
            let others: Vec<&Member<'_>> = members
                .iter()
                .filter(|other| other.class != member.class)
                .collect();
            let mut seen = BTreeSet::new();
            // Numbers of one domain are one class under a tolerance; each
            // is still shown.
            let other_values: Vec<String> = others
                .iter()
                .filter(|other| {
                    seen.insert(if other.span.is_some() {
                        display(other.stated.as_ref())
                    } else {
                        other.class.clone()
                    })
                })
                .map(|other| display(other.stated.as_ref()))
                .collect();
            let evidence = member
                .evidence
                .iter()
                .chain(others.iter().flat_map(|other| &other.evidence))
                .cloned()
                .collect();
            let mut read = words.read(shown_key.as_ref(), member);
            read.named.insert("others", other_values.join(", "));
            let message = if shown_key.is_some() {
                messages.differs
            } else {
                messages.differs_unkeyed
            };
            evaluation.push_finding(finding(
                rule,
                &member.object.id,
                render(plan, &read, message),
                evidence,
                others.iter().map(|other| other.object.id.clone()).collect(),
            ));
        }
    }
    evaluation
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

/// One group of numeric values of one domain under a tolerance.
struct Spreading<'w, 'p, 't, 'a> {
    words: &'w Words<'p, 't>,
    rule: &'w CompiledRule,
    key: Option<&'w PropertyValue>,
    members: &'w [Member<'a>],
    spread: Spread,
    /// The domain's unit as shown, with a leading space, or empty.
    unit: &'w str,
}

impl Spreading<'_, '_, '_, '_> {
    #[allow(clippy::too_many_lines)]
    fn judge(&self, evaluation: &mut CapabilityEvaluation) {
        let (plan, messages) = (self.words.plan, self.words.messages);
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
        if !beyond(greatest_lower - least_upper) {
            for member in self.members {
                evaluation.push_object_not_evaluated(
                    member.object.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    render(plan, &self.words.read(self.key, member), messages.straddles),
                );
            }
            return;
        }
        let unit = self.unit;
        let median_lower = median(spans.iter().map(|span| span.0).collect());
        let median_upper = median(spans.iter().map(|span| span.1).collect());
        let median = if median_lower.total_cmp(&median_upper).is_eq() {
            format!("{median_lower}{unit}")
        } else {
            format!("{median_lower} to {median_upper}{unit}")
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
            messages.at_end
        } else {
            messages.beyond
        };
        let range = format!("{least_lower} to {greatest_upper}{unit}");
        for &index in &outliers {
            let member = &self.members[index];
            let others: Vec<&Member<'_>> = self
                .members
                .iter()
                .filter(|other| other.object.id != member.object.id)
                .collect();
            let evidence = member
                .evidence
                .iter()
                .chain(others.iter().flat_map(|other| &other.evidence))
                .cloned()
                .collect();
            let mut read = self.words.read(self.key, member);
            read.named.insert("median", median.clone());
            read.named.insert("range", range.clone());
            evaluation.push_finding(finding(
                self.rule,
                &member.object.id,
                render(plan, &read, message),
                evidence,
                others.iter().map(|other| other.object.id.clone()).collect(),
            ));
        }
        for index in undecided {
            let member = &self.members[index];
            let mut read = self.words.read(self.key, member);
            read.named.insert("median", median.clone());
            evaluation.push_object_not_evaluated(
                member.object.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, messages.undecided),
            );
        }
    }
}
