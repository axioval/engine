//! Member checks ([`Members::checks`]): each member of an anchor's first
//! population judged on its own, the member in scope, just before the
//! anchor reads the value the check names. A member found is a finding on
//! the member relating the anchor, reported once however many anchors
//! reach it, and leaves the anchor open once that value is read.
//!
//! [`Members::checks`]: axioval_engine::template::Members::checks

use std::cell::RefCell;
use std::collections::BTreeSet;

use axioval_engine::template::{MemberCheck, TemplateValue};
use axioval_engine::{CompiledRule, RuleContext};
use axioval_ir::contract::Expression;
use axioval_ir::{NotEvaluatedReason, Object, ObjectId};

use super::{Outcome, Plan, Read, Scope, effective_of, read_values, render, within};
use crate::expression_leaves::ObjectLeaves;
use crate::measured_arguments::Arguments;
use crate::plan_area::Verdict;

/// What member checks found about members over one rule's anchors: the
/// outcomes not yet reported, and every member already reported.
#[derive(Default)]
pub(super) struct Judged {
    found: RefCell<Vec<(ObjectId, Outcome)>>,
    reported: RefCell<BTreeSet<ObjectId>>,
}

impl Judged {
    /// The outcomes found since the last take, in the order found.
    pub(super) fn take(&self) -> Vec<(ObjectId, Outcome)> {
        std::mem::take(&mut *self.found.borrow_mut())
    }

    /// Keeps `outcome` about `member`, unless one was kept already.
    fn report(&self, member: &ObjectId, outcome: Outcome) {
        if self.reported.borrow_mut().insert(member.clone()) {
            self.found.borrow_mut().push((member.clone(), outcome));
        }
    }
}

/// Judges each of `decided`, the anchor's first population's members, by
/// every member check made before the plan's value `index`: the first
/// check that found a member, with its `failed` message, how many members
/// it found and the first of them.
pub(super) fn judge(
    plan: &Plan<'_>,
    scope: &Scope<'_>,
    (context, arguments): (&RuleContext<'_>, &Arguments),
    rule: &CompiledRule,
    anchor: &Object,
    decided: &[ObjectId],
    index: usize,
) -> Option<(&'static str, usize, ObjectId)> {
    let name = plan.form.values.get(index)?.name;
    let mut gate = None;
    for (check, values) in plan.member_checks() {
        if check.before != name {
            continue;
        }
        let mut failed: Vec<&ObjectId> = Vec::new();
        for id in decided {
            let Some(member) = crate::selection::object_by_id(context, id) else {
                continue;
            };
            let Some(outcome) = judge_member(
                plan,
                check,
                &values,
                (context, arguments),
                rule,
                member,
                anchor,
            ) else {
                continue;
            };
            if matches!(outcome, Outcome::Finding { .. }) {
                failed.push(id);
            }
            scope.judged.report(id, outcome);
        }
        if gate.is_none()
            && let Some(first) = failed.first()
        {
            gate = Some((check.failed, failed.len(), (*first).clone()));
        }
    }
    gate
}

/// One member judged by `check`: its finding or its open outcome, `None`
/// where it passes or is not judged (its first value `null` or unread).
fn judge_member(
    plan: &Plan<'_>,
    check: &MemberCheck,
    values: &[(&TemplateValue, &Expression)],
    (context, arguments): (&RuleContext<'_>, &Arguments),
    rule: &CompiledRule,
    member: &Object,
    anchor: &Object,
) -> Option<Outcome> {
    let mut leaves =
        ObjectLeaves::new(context, member, Some(&rule.parameters)).with_arguments(arguments);
    let mut read = Read::default();
    let (first, rest) = values.split_first()?;
    let not = |_: &str| false;
    if read_values(
        plan,
        std::iter::once(*first),
        &not,
        context,
        member,
        &mut leaves,
        &mut read,
    )
    .is_some()
    {
        return None;
    }
    if let Some(outcome) = read_values(
        plan,
        rest.iter().copied(),
        &not,
        context,
        member,
        &mut leaves,
        &mut read,
    ) {
        let reason = match &outcome {
            Outcome::Open(reason, message) => {
                if read.why.is_none() {
                    read.why = Some(message.clone());
                }
                reason.clone()
            }
            _ => NotEvaluatedReason::IncompleteEvidence,
        };
        return Some(Outcome::Open(reason, render(plan, &read, check.open)));
    }
    let Some(judged) = within(plan, &read, &effective_of(plan, &check.decision)) else {
        return Some(Outcome::Open(
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "{}: a value the member check reads is no number",
                plan.template.name
            ),
        ));
    };
    match judged.verdict {
        Verdict::Pass => None,
        Verdict::Fail(bound) => {
            read.bound = Some(bound);
            Some(Outcome::Finding {
                message: render(plan, &read, check.fail),
                evidence: read.evidence,
                related: vec![anchor.id.clone()],
                deviation: None,
                severity: None,
            })
        }
        Verdict::Undecided(bound) => {
            read.bound = Some(bound);
            Some(Outcome::Open(
                NotEvaluatedReason::IncompleteEvidence,
                render(plan, &read, check.undecided),
            ))
        }
    }
}
