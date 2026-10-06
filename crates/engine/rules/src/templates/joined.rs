//! One outcome over the items of a measured member list, its message
//! joining the words of the items found, or of those undecided
//! ([`Joined`]).

use axioval_engine::MemberValue;
use axioval_engine::template::Joined;
use axioval_ir::NotEvaluatedReason;

use super::{Outcome, Plan, Read, render, unstated_dropped_list};
use crate::expression_leaves::ObjectLeaves;

/// Judges the subject `leaves` read (a scope's source, or an object) by
/// the items of `joined`'s list: a finding joining the words of the items
/// found, else an open outcome joining those of the items undecided, else
/// a pass. The finding cites the list's evidence beside `read`'s.
pub(super) fn judge(
    plan: &Plan<'_>,
    joined: &Joined,
    read: &mut Read,
    leaves: &ObjectLeaves<'_>,
) -> Outcome {
    let list = unstated_dropped_list(joined.list, &plan.constants);
    let listed = leaves.bound_members(&list);
    let (members, evidence) = match &*listed {
        Ok(listed) => listed,
        Err((reason, why)) => {
            read.why = Some(why.clone());
            return Outcome::Open(reason.clone(), render(plan, read, joined.refused));
        }
    };
    let mut found: Vec<String> = Vec::new();
    let mut open: Vec<String> = Vec::new();
    // Whether every undecided item is open for a statement not recorded.
    let mut unrecorded = true;
    for member in members {
        let words = match member.fields.get(joined.words) {
            Some(MemberValue::Text { text }) => text.clone(),
            _ => String::new(),
        };
        match member.fields.get(joined.found) {
            Some(MemberValue::Truth { value: true, .. }) if member.certain => found.push(words),
            Some(MemberValue::Truth { value: false, .. }) => {}
            // Undecided, or an item that may not be one.
            _ => {
                let recorded = joined.recorded.is_none_or(|field| {
                    !matches!(
                        member.fields.get(field),
                        Some(MemberValue::Truth { value: false, .. })
                    )
                });
                unrecorded &= !recorded;
                if !open.contains(&words) {
                    open.push(words);
                }
            }
        }
    }
    if !found.is_empty() {
        read.named.insert("found", found.join(joined.separator));
        read.evidence.extend(evidence.iter().cloned());
        return Outcome::finding(
            render(plan, read, plan.form.fail),
            std::mem::take(&mut read.evidence),
        );
    }
    if open.is_empty() {
        return Outcome::Passed;
    }
    read.named.insert("open", open.join(joined.separator));
    let reason = if unrecorded {
        NotEvaluatedReason::NotRecorded
    } else {
        NotEvaluatedReason::IncompleteEvidence
    };
    Outcome::Open(reason, render(plan, read, plan.form.undecided))
}
