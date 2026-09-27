//! Counts of objects related to each anchor.

use std::collections::BTreeSet;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Objects a selector picks, split into decided and undecided.
pub(crate) struct Population {
    pub(crate) matched: BTreeSet<ObjectId>,
    pub(crate) undecided: BTreeSet<ObjectId>,
}

impl Population {
    pub(crate) fn of(context: &RuleContext<'_>, selector: &Selector) -> Self {
        let (matched, outcomes) = select_objects(context, selector);
        Self {
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
    pub(crate) evidence: Vec<Evidence>,
}

/// The members of `population` an anchor reaches through the traversal, or
/// every member of the anchor's own source when there is none.
pub(crate) fn tally(
    context: &RuleContext<'_>,
    traversal: Option<&Traversal<'_>>,
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
    let decided: Vec<ObjectId> = reached
        .iter()
        .filter(|id| population.matched.contains(*id))
        .cloned()
        .collect();
    Ok(Tally {
        undecided: reached.len() - decided.len(),
        decided,
        evidence,
    })
}

pub(crate) fn relation_text(traversal: Option<&Traversal<'_>>) -> String {
    traversal.map_or_else(
        || "in the same source".to_owned(),
        |traversal| format!("via {}", traversal.relationship),
    )
}

/// Requires each selected anchor to have a bounded number of related objects.
///
/// Anchors are the rule's selection: a space whose doors are counted, a zone
/// whose member spaces are counted, a building that must contain walls.
/// Related objects are those `related_selector` picks (everything by
/// default) that the declared `relationship` reaches from the anchor; with
/// no relationship, every such object in the anchor's own source counts.
/// `minimum` and `maximum` bound the count, inclusive; at least one is
/// required.
///
/// An object whose membership in `related_selector` cannot be decided is
/// counted as unknown. The anchor is judged when the verdict holds either
/// way, and is not evaluated otherwise.
pub struct RelatedCount;

impl RuleCapability for RelatedCount {
    fn id(&self) -> &'static str {
        "axioval:capability.related-count"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("related_selector", ParameterType::Selector),
            ParameterDescriptor::optional("minimum", ParameterType::Integer),
            ParameterDescriptor::optional("maximum", ParameterType::Integer),
        ]
        .into_iter()
        .chain(crate::support::traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.integer("minimum")?;
            let maximum = parameters.integer("maximum")?;
            match (minimum, maximum) {
                (None, None) => return Err(invalid("minimum or maximum is required")),
                (Some(minimum), _) if minimum < 0 => {
                    return Err(invalid("minimum is negative"));
                }
                (_, Some(maximum)) if maximum < 0 => {
                    return Err(invalid("maximum is negative"));
                }
                (Some(minimum), Some(maximum)) if minimum > maximum => {
                    return Err(invalid("minimum exceeds maximum"));
                }
                _ => {}
            }
            Ok::<_, Unavailable>((
                parameters.selector("related_selector")?,
                minimum,
                maximum,
                parameters.traversal()?,
            ))
        })();
        let (related, minimum, maximum, traversal) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("related-count: {message}"),
                );
            }
        };
        let population = Population::of(context, related.unwrap_or(&Selector::All));
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let via = relation_text(traversal.as_ref());
        for anchor in anchors {
            let tally = match tally(context, traversal.as_ref(), anchor, &population) {
                Ok(tally) => tally,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                    continue;
                }
            };
            let count = i64::try_from(tally.decided.len()).unwrap_or(i64::MAX);
            let most = count.saturating_add(i64::try_from(tally.undecided).unwrap_or(i64::MAX));
            let too_few = minimum.filter(|minimum| most < *minimum);
            let too_many = maximum.filter(|maximum| count > *maximum);
            let undecided = minimum.is_some_and(|minimum| count < minimum)
                || maximum.is_some_and(|maximum| most > maximum);
            let bound = match (minimum, maximum) {
                (Some(minimum), Some(maximum)) => format!("between {minimum} and {maximum}"),
                (Some(minimum), None) => format!("at least {minimum}"),
                (None, Some(maximum)) => format!("at most {maximum}"),
                (None, None) => unreachable!("parsing requires a bound"),
            };
            if too_few.is_some() || too_many.is_some() {
                evaluation.push_finding(finding(
                    rule,
                    &anchor.id,
                    format!("{count} related object(s) {via}; required {bound}"),
                    tally.evidence,
                    tally.decided,
                ));
            } else if undecided {
                evaluation.push_object_not_evaluated(
                    anchor.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{count} related object(s) {via} and {} more that may count; required {bound}",
                        tally.undecided
                    ),
                );
            }
        }
        evaluation
    }
}
