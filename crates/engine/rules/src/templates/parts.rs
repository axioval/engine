//! The parts of each selected object judged as objects of their own
//! ([`Parts`]): each part once, by the first anchor reaching it, its own
//! values and checks on it; then the anchor by the form's.

use std::collections::BTreeSet;

use axioval_engine::template::{FormCheck, Parts};
use axioval_engine::{CapabilityEvaluation, CompiledRule, RuleContext};
use axioval_ir::contract::{Expression, ParameterValue};
use axioval_ir::{NotEvaluatedReason, Object};

use super::{Constant, Outcome, Plan, Read, bound, judge_checks_in, push, read_values};
use crate::counts::Population;
use crate::expression_leaves::ObjectLeaves;
use crate::measured_arguments::Arguments;
use crate::selection::{object_by_id, select_objects};
use crate::support::Traversal;

/// Each check's values, bound.
fn bound_checks(plan: &Plan<'_>, checks: &[FormCheck]) -> Vec<Vec<Expression>> {
    checks
        .iter()
        .map(|check| {
            check
                .values
                .iter()
                .map(|step| bound(&step.expression, &plan.constants))
                .collect()
        })
        .collect()
}

/// Reads `values` of `object` (the part or the anchor), then judges
/// `checks`: the outcomes, in order.
fn judged(
    plan: &Plan<'_>,
    (values, bound_values): (&[axioval_engine::template::TemplateValue], &[Expression]),
    (checks, bound_checks): (&[FormCheck], &[Vec<Expression>]),
    context: &RuleContext<'_>,
    object: &Object,
    leaves: &mut ObjectLeaves<'_>,
) -> Vec<Outcome> {
    let mut read = Read::default();
    if let Some(outcome) = read_values(
        plan,
        values.iter().zip(bound_values),
        &|_| false,
        context,
        object,
        leaves,
        &mut read,
    ) {
        return vec![outcome];
    }
    judge_checks_in(plan, (checks, bound_checks), &read, context, object, leaves)
}

/// Runs a form deciding `parts` for `rule`.
pub(super) fn run(
    plan: &Plan<'_>,
    parts: &Parts,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> CapabilityEvaluation {
    let refused = |message: String| {
        CapabilityEvaluation::not_evaluated(
            NotEvaluatedReason::InvalidDeclaration,
            format!("{}: {message}", plan.template.name),
        )
    };
    let Some(Constant::Other(ParameterValue::Selector { value: selector })) =
        plan.constants.get(parts.selector)
    else {
        return refused(format!("parameter `{}` is required", parts.selector));
    };
    let Some(Constant::Other(ParameterValue::StringList { value: steps })) =
        plan.constants.get(parts.path)
    else {
        return refused(format!("parameter `{}` is required", parts.path));
    };
    let traversal = match Traversal::path(steps) {
        Ok(traversal) => traversal,
        Err((reason, message)) => {
            return CapabilityEvaluation::not_evaluated(
                reason,
                format!("{}: {message}", plan.template.name),
            );
        }
    };
    let population = Population::of(context, selector);
    let everything: Vec<&Object> = context.project.objects().collect();
    let arguments = Arguments::default();
    let part_values: Vec<Expression> = parts
        .values
        .iter()
        .map(|step| bound(&step.expression, &plan.constants))
        .collect();
    let part_checks = bound_checks(plan, &parts.checks);
    let (selected, mut evaluation) = select_objects(context, &rule.selector);
    let mut reported = BTreeSet::new();
    for anchor in selected {
        let reached = match traversal.related(context, &anchor.id, &everything) {
            Ok((reached, _)) => reached,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                continue;
            }
        };
        for id in reached
            .iter()
            .filter(|part| population.matched.contains(*part))
        {
            if !reported.insert(id.clone()) {
                continue;
            }
            let Some(part) = object_by_id(context, id) else {
                continue;
            };
            let mut leaves = ObjectLeaves::new(context, part, Some(&rule.parameters))
                .with_subject(anchor)
                .with_arguments(&arguments);
            for outcome in judged(
                plan,
                (&parts.values, &part_values),
                (&parts.checks, &part_checks),
                context,
                part,
                &mut leaves,
            ) {
                push(&mut evaluation, rule, part, outcome);
            }
        }
        let mut leaves =
            ObjectLeaves::new(context, anchor, Some(&rule.parameters)).with_arguments(&arguments);
        for outcome in judged(
            plan,
            (&plan.form.values, &plan.bound.expressions),
            (&plan.form.checks, &plan.bound.checks),
            context,
            anchor,
            &mut leaves,
        ) {
            push(&mut evaluation, rule, anchor, outcome);
        }
    }
    evaluation
}
