//! Counts of objects related to each anchor.

use std::collections::BTreeSet;
use std::sync::Arc;

use axioval_engine::{MeasuredMemo, NotEvaluatedReason, RuleContext};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::selection::select_objects;
use crate::support::{Traversal, Unavailable};

/// The key of every object of the project in the run's memo.
#[derive(Hash, PartialEq, Eq)]
struct EveryObject;

/// Every object of the project as a population, made once per run: what a
/// measured value naming no objects reads.
pub(crate) fn every_object(context: &RuleContext<'_>) -> Arc<Population> {
    MeasuredMemo::of(context.services, EveryObject, || {
        Arc::new(Population::of(context, &Selector::All))
    })
}

/// Objects a selector picks, split into decided and undecided.
pub(crate) struct Population {
    pub(crate) matched: BTreeSet<ObjectId>,
    pub(crate) undecided: BTreeSet<ObjectId>,
    /// The first outcome the selection left open, of an object or a
    /// source: why the selector cannot decide everything it might pick.
    pub(crate) first: Option<(NotEvaluatedReason, String)>,
}

impl Population {
    pub(crate) fn of(context: &RuleContext<'_>, selector: &Selector) -> Self {
        let (matched, outcomes) = select_objects(context, selector);
        Self {
            first: outcomes
                .not_evaluated_outcomes()
                .first()
                .map(|outcome| (outcome.reason().clone(), outcome.message().to_owned())),
            matched: matched
                .into_iter()
                .map(|object| object.id.clone())
                .collect(),
            undecided: outcomes
                .not_evaluated_outcomes()
                .iter()
                .filter_map(|outcome| outcome.object_id().cloned())
                .collect(),
        }
    }

    pub(crate) fn contains(&self, id: &ObjectId) -> bool {
        self.matched.contains(id) || self.undecided.contains(id)
    }
}

/// How many of `population` belong to `anchor`: decided, undecided, and which.
pub(crate) struct Tally {
    pub(crate) decided: Vec<ObjectId>,
    pub(crate) undecided: usize,
    /// The reached objects whose selection is undecided.
    pub(crate) possible: Vec<ObjectId>,
    pub(crate) evidence: Vec<Evidence>,
}

/// The members of `population` an anchor reaches through the traversal, or
/// every member of the anchor's own source when there is none.
pub(crate) fn tally(
    context: &RuleContext<'_>,
    traversal: Option<&Traversal>,
    anchor: &Object,
    population: &Population,
) -> Result<Tally, Unavailable> {
    let (reached, evidence) = match traversal {
        Some(traversal) => {
            let universe: Vec<&Object> = context
                .project
                .objects()
                .filter(|object| population.contains(&object.id))
                .collect();
            traversal.related(context, &anchor.id, &universe)?
        }
        None => (
            context
                .project
                .objects()
                .filter(|object| {
                    object.id != anchor.id
                        && object.id.source == anchor.id.source
                        && population.contains(&object.id)
                })
                .map(|object| object.id.clone())
                .collect(),
            Vec::new(),
        ),
    };
    let (decided, possible): (Vec<ObjectId>, Vec<ObjectId>) = reached
        .into_iter()
        .partition(|id| population.matched.contains(id));
    Ok(Tally {
        undecided: possible.len(),
        decided,
        possible,
        evidence,
    })
}

/// Keeps the reached objects of `tally` from which `ends` reaches the same
/// set of objects as from `anchor`: a revolving door's swing door between
/// the same pair of spaces. An object whose ends cannot be read may share
/// them, so it moves to the undecided; one whose ends differ is dropped.
/// An anchor whose ends cannot be read, or reach nothing, is unavailable.
pub(crate) fn same_ends(
    context: &RuleContext<'_>,
    ends: &Traversal,
    anchor: &Object,
    tally: Tally,
) -> Result<Tally, Unavailable> {
    let everything: Vec<&Object> = context.project.objects().collect();
    let (own, mut evidence) = ends.related(context, &anchor.id, &everything)?;
    if own.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("`same_ends` {} reaches nothing from it", ends.relationship),
        ));
    }
    let own: BTreeSet<ObjectId> = own.into_iter().collect();
    evidence.extend(tally.evidence);
    let mut kept = Tally {
        decided: Vec::new(),
        undecided: 0,
        possible: Vec::new(),
        evidence,
    };
    for (object, sure) in tally
        .decided
        .into_iter()
        .map(|object| (object, true))
        .chain(tally.possible.into_iter().map(|object| (object, false)))
    {
        match ends.related(context, &object, &everything) {
            Ok((reached, cited)) => {
                if reached.into_iter().collect::<BTreeSet<_>>() != own {
                    continue;
                }
                kept.evidence.extend(cited);
                if sure {
                    kept.decided.push(object);
                } else {
                    kept.possible.push(object);
                }
            }
            Err(_) => kept.possible.push(object),
        }
    }
    kept.undecided = kept.possible.len();
    Ok(kept)
}

pub(crate) fn relation_text(traversal: Option<&Traversal>) -> String {
    traversal.map_or_else(
        || "in the same source".to_owned(),
        |traversal| format!("via {}", traversal.relationship),
    )
}

/// A count as a real number for its relative deviation; counts beyond 2^53
/// round, which moves a deviation by far less than any band.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn real(count: i64) -> f64 {
    count as f64
}
