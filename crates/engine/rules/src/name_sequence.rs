//! Numbered names that must count up in a declared order.

use std::cmp::Ordering;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, finding, invalid, resolve, undefined,
};

/// Requires the members of each anchor to be numbered consecutively in order.
///
/// Storeys of a building named `1`, `2`, `3` from the lowest up: the anchors
/// are the rule's selection (buildings), the members are the objects
/// `member_selector` picks that the declared `relationship` reaches from an
/// anchor (or, with none, every such object in the anchor's source), ordered
/// by the numeric `order` property (elevation), ties broken by name. The
/// first numbered member must be `first` (default 1), and each next one the
/// previous plus `increment` (default 1).
///
/// A name counts as a number only when it is one exactly: an optional sign
/// and digits, nothing else, so ` 1` and `1a` are not numbers. A member
/// without a numeric name gets its own finding and does not interrupt the
/// sequence. Ordering needs every member's order value, so a member without
/// one makes the whole anchor not evaluated.
pub struct NameSequence;

struct Member<'a> {
    object: &'a Object,
    order: f64,
    name: Option<PropertyValue>,
    evidence: Vec<Evidence>,
}

impl RuleCapability for NameSequence {
    fn id(&self) -> &'static str {
        "axioval:capability.name-sequence"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("member_selector", ParameterType::Selector),
            ParameterDescriptor::required("name", ParameterType::PropertyReference),
            ParameterDescriptor::required("order", ParameterType::PropertyReference),
            ParameterDescriptor::optional("first", ParameterType::Integer),
            ParameterDescriptor::optional("increment", ParameterType::Integer),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let increment = parameters.integer("increment")?.unwrap_or(1);
            if increment <= 0 {
                return Err(invalid("increment must be positive"));
            }
            Ok::<_, Unavailable>(Config {
                members: parameters.required_selector("member_selector")?,
                name: parameters.required_property("name")?,
                order: parameters.required_property("order")?,
                first: parameters.integer("first")?.unwrap_or(1),
                increment,
                traversal: parameters.traversal()?,
            })
        })();
        let config = match parsed {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("name-sequence: {message}"),
                );
            }
        };
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        for anchor in anchors {
            match members(context, &config, anchor) {
                Ok(members) => check(rule, &config, &members, &mut evaluation),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

struct Config<'a> {
    members: &'a Selector,
    name: PropertyRef<'a>,
    order: PropertyRef<'a>,
    first: i64,
    increment: i64,
    traversal: Option<Traversal<'a>>,
}

/// The anchor's members in order, or why they cannot be ordered.
fn members<'a>(
    context: &RuleContext<'a>,
    config: &Config<'_>,
    anchor: &Object,
) -> Result<Vec<Member<'a>>, Unavailable> {
    let (candidates, outcomes) = select_objects(context, config.members);
    if let Some(outcome) = outcomes.not_evaluated_outcomes().first() {
        return Err((
            outcome.reason().clone(),
            format!("member selection is undecided: {}", outcome.message()),
        ));
    }
    let (reached, relation_evidence): (Vec<ObjectId>, Vec<Evidence>) = match &config.traversal {
        Some(traversal) => traversal.related(context, &anchor.id, &candidates)?,
        None => (
            candidates
                .iter()
                .filter(|member| member.id.source == anchor.id.source && member.id != anchor.id)
                .map(|member| member.id.clone())
                .collect(),
            Vec::new(),
        ),
    };
    let mut members = Vec::new();
    for id in reached {
        let object = context
            .project
            .object(&id)
            .ok_or_else(|| invalid(format!("member {id} is not in the project")))?;
        let order = resolve(context, object, config.order)?;
        let order_value = match order.value() {
            Some(PropertyValue::Integer(value)) => {
                #[allow(clippy::cast_precision_loss)]
                let value = *value as f64;
                value
            }
            Some(PropertyValue::Decimal(value) | PropertyValue::Quantity { value, .. }) => *value,
            other => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{id} has no numeric {} ({}), so the order is unknown",
                        config.order,
                        crate::support::display(other)
                    ),
                ));
            }
        };
        let name = resolve(context, object, config.name)?;
        let mut evidence = relation_evidence.clone();
        evidence.extend(order.evidence());
        evidence.extend(name.evidence());
        members.push(Member {
            object,
            order: order_value,
            name: name.value().cloned(),
            evidence,
        });
    }
    members.sort_by(|left, right| {
        left.order
            .total_cmp(&right.order)
            .then_with(|| name_text(left).cmp(&name_text(right)))
            .then_with(|| left.object.id.cmp(&right.object.id))
    });
    Ok(members)
}

fn name_text(member: &Member<'_>) -> String {
    match &member.name {
        Some(PropertyValue::String(text)) => text.clone(),
        other => crate::support::display(other.as_ref()),
    }
}

/// A strict whole number: optional sign and ASCII digits, nothing else.
fn number(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn check(
    rule: &CompiledRule,
    config: &Config<'_>,
    members: &[Member<'_>],
    evaluation: &mut CapabilityEvaluation,
) {
    let name = config.name;
    let mut previous: Option<(i64, &Member<'_>)> = None;
    for member in members {
        let report = |message: String, related: Option<&Member<'_>>| {
            let mut evidence = member.evidence.clone();
            if let Some(related) = related {
                evidence.extend(related.evidence.iter().cloned());
            }
            finding(
                rule,
                &member.object.id,
                message,
                evidence,
                related
                    .map(|related| related.object.id.clone())
                    .into_iter()
                    .collect(),
            )
        };
        let value = match &member.name {
            _ if undefined(member.name.as_ref()) => {
                evaluation.push_finding(report(format!("{name} is not set"), None));
                continue;
            }
            Some(PropertyValue::String(text)) => number(text),
            Some(PropertyValue::Integer(value)) => Some(*value),
            _ => None,
        };
        let Some(value) = value else {
            evaluation.push_finding(report(
                format!(
                    "{name} {} is not a whole number",
                    crate::support::display(member.name.as_ref())
                ),
                None,
            ));
            continue;
        };
        let message = match previous {
            None if value > config.first => Some((
                format!(
                    "{name} of the first member is {value}; expected {}",
                    config.first
                ),
                None,
            )),
            _ if value < config.first => Some((
                format!("{name} {value} is less than {}", config.first),
                previous.map(|(_, member)| member),
            )),
            Some((before, below)) => match value.cmp(&before) {
                Ordering::Less | Ordering::Equal => Some((
                    format!("{name} {value} is not above {before}, the member below it"),
                    Some(below),
                )),
                Ordering::Greater if Some(value) != before.checked_add(config.increment) => Some((
                    format!(
                        "{name} {value} does not follow {before}; expected {}",
                        before.saturating_add(config.increment)
                    ),
                    Some(below),
                )),
                Ordering::Greater => None,
            },
            None => None,
        };
        if let Some((message, related)) = message {
            evaluation.push_finding(report(message, related));
        }
        previous = Some((value, member));
    }
}
