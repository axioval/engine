//! Group decisions ([`axioval_engine::template::Decision::Unique`]): each
//! object's stated value compared with those of the other objects of its
//! group.

use std::collections::BTreeMap;

use axioval_engine::template::Unique;
use axioval_engine::{CapabilityEvaluation, CompiledRule, RuleContext};
use axioval_ir::contract::ScalarValue;
use axioval_ir::{Evidence, NotEvaluatedReason, Object, PropertyValue};

use super::{Constant, Outcome, Plan, Read, read_values, render};
use crate::expression_leaves::ObjectLeaves;
use crate::selection::select_objects;
use crate::support::{Parameters, finding, scope_key, undefined, value_key};

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
        match read_values(plan, decision, context, object, &mut leaves, &mut read) {
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
