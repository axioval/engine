//! `related-count` as it was implemented before it became a template
//! (#290), kept only as the parity reference the template is held to in
//! the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;

use crate::counts::{Population, real, relation_text, same_ends, tally};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

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
/// With `same_ends`, a relationship path like `path`, a related object
/// counts only when that path reaches the same set of objects from it as
/// from the anchor: a revolving door needs a swing door between the same
/// spaces, not any door of one of them. An object whose ends cannot be read
/// counts as unknown; an anchor whose ends cannot be read or reach nothing
/// is not evaluated.
///
/// An object whose membership in `related_selector` cannot be decided is
/// counted as unknown. The anchor is judged when the verdict holds either
/// way, and is not evaluated otherwise.
pub struct RelatedCount;

impl RuleCapability for RelatedCount {
    fn id(&self) -> &'static str {
        "axioval:capability.related-count"
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("related_selector", ParameterType::Selector),
            ParameterDescriptor::optional("minimum", ParameterType::Integer),
            ParameterDescriptor::optional("maximum", ParameterType::Integer),
            ParameterDescriptor::optional("same_ends", ParameterType::StringList),
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
                parameters
                    .strings("same_ends")?
                    .map(Traversal::path)
                    .transpose()?,
            ))
        })();
        let (related, minimum, maximum, traversal, ends) = match parsed {
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
        let mut via = relation_text(traversal.as_ref());
        if let Some(ends) = &ends {
            via = format!("{via} with the same ends via {}", ends.relationship);
        }
        for anchor in anchors {
            let tallied = tally(context, traversal.as_ref(), anchor, &population).and_then(
                |tally| match &ends {
                    Some(ends) => same_ends(context, ends, anchor, tally),
                    None => Ok(tally),
                },
            );
            let tally = match tallied {
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
                // Undecided objects may still count: the count lies in
                // `[count, most]`.
                let (least, greatest) = (real(count), real(most));
                let deviation = match (too_few, too_many) {
                    (Some(minimum), _) => Deviation::below(real(minimum), least, greatest),
                    (None, Some(maximum)) => Deviation::above(real(maximum), least, greatest),
                    (None, None) => unreachable!("a bound was missed"),
                };
                evaluation.push_graded_finding(
                    finding(
                        rule,
                        &anchor.id,
                        format!("{count} related object(s) {via}; required {bound}"),
                        tally.evidence,
                        tally.decided,
                    ),
                    deviation,
                );
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
