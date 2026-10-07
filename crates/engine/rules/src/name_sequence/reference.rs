//! `name-sequence` as it was implemented before it became a template
//! (#283), kept only as the parity reference the template is held to in
//! the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry. It reads the members through the same
//! `members` the template's measured `name_sequence` reads.

use std::cmp::Ordering;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};
use axioval_ir::ObjectId;
use axioval_ir::PropertyValue;

use super::{Config, Member, NAME, members, number};
use crate::selection::select_objects;
use crate::support::{Parameters, finding, undefined};

/// Requires the members of each anchor to be numbered consecutively in
/// order, as `name-sequence` judged it before it became a template.
pub struct NameSequence;

impl RuleCapability for NameSequence {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::read(&Parameters(rule), true) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let selector = config.members.expect("read with the declaration");
        for anchor in anchors {
            let (candidates, outcomes) = select_objects(context, selector);
            let undecided = outcomes
                .not_evaluated_outcomes()
                .first()
                .map(|outcome| (outcome.reason().clone(), outcome.message().to_owned()));
            let candidates: Vec<ObjectId> =
                candidates.iter().map(|object| object.id.clone()).collect();
            match members(context, &config, anchor, (&candidates, undecided)) {
                Ok(members) => check(rule, &config, &members, &mut evaluation),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
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
        if value < config.first {
            // Its own result: a number below the start is not an order break,
            // and like a non-number it does not interrupt the sequence.
            evaluation.push_finding(report(
                format!("{name} {value} is below the start {}", config.first),
                None,
            ));
            continue;
        }
        let message = match previous {
            None if value > config.first => Some((
                format!(
                    "{name} of the first member is {value}; expected {}",
                    config.first
                ),
                None,
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
