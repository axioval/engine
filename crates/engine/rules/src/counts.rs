//! Counts of objects related to each anchor, absolute and relative.

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
struct Population {
    matched: BTreeSet<ObjectId>,
    undecided: BTreeSet<ObjectId>,
}

impl Population {
    fn of(context: &RuleContext<'_>, selector: &Selector) -> Self {
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

    fn contains(&self, id: &ObjectId) -> bool {
        self.matched.contains(id) || self.undecided.contains(id)
    }
}

/// How many of `population` belong to `anchor`: decided, undecided, and which.
struct Tally {
    decided: Vec<ObjectId>,
    undecided: usize,
    evidence: Vec<Evidence>,
}

/// The members of `population` an anchor reaches through the traversal, or
/// every member of the anchor's own source when there is none.
fn tally(
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

fn relation_text(traversal: Option<&Traversal<'_>>) -> String {
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
            ParameterDescriptor::optional("relationship", ParameterType::String),
            ParameterDescriptor::optional("direction", ParameterType::String),
            ParameterDescriptor::optional("follow_chain", ParameterType::Boolean),
            ParameterDescriptor::optional("skip_absent_relationship_ends", ParameterType::Boolean),
        ]
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

#[derive(Clone, Copy)]
enum Operator {
    Equal,
    NotEqual,
    Greater,
    AtLeast,
    Less,
    AtMost,
}

impl Operator {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "equal" => Self::Equal,
            "not_equal" => Self::NotEqual,
            "greater" => Self::Greater,
            "at_least" => Self::AtLeast,
            "less" => Self::Less,
            "at_most" => Self::AtMost,
            _ => return None,
        })
    }

    fn holds(self, left: i128, right: i128) -> bool {
        match self {
            Self::Equal => left == right,
            Self::NotEqual => left != right,
            Self::Greater => left > right,
            Self::AtLeast => left >= right,
            Self::Less => left < right,
            Self::AtMost => left <= right,
        }
    }
}

/// Requires a ratio between two related populations at each anchor.
///
/// With `provided_unit` p and `required_unit` r, the anchor passes when
/// `provided / p` stands in `operator` to `required / r`: "one washbasin
/// (provided, p = 1) per four workplaces (required, r = 4), at least" is
/// `washbasins * 4 >= workplaces * 1`. Integer arithmetic keeps it exact.
/// Anchors and the relationship work as in `related-count`; with no
/// relationship an anchor's whole source is counted, so selecting the
/// building checks the ratio for the whole model and selecting storeys
/// checks it storey by storey.
///
/// An anchor with any undecided member is not evaluated.
pub struct RelativeCount;

impl RuleCapability for RelativeCount {
    fn id(&self) -> &'static str {
        "axioval:capability.relative-count"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("provided_selector", ParameterType::Selector),
            ParameterDescriptor::required("required_selector", ParameterType::Selector),
            ParameterDescriptor::required("provided_unit", ParameterType::Integer),
            ParameterDescriptor::required("required_unit", ParameterType::Integer),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::optional("relationship", ParameterType::String),
            ParameterDescriptor::optional("direction", ParameterType::String),
            ParameterDescriptor::optional("follow_chain", ParameterType::Boolean),
            ParameterDescriptor::optional("skip_absent_relationship_ends", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let unit = |name| match parameters.integer(name)? {
                Some(value) if value > 0 => Ok(value),
                _ => Err(invalid(format!("{name} must be a positive integer"))),
            };
            let operator = parameters.required_string("operator")?;
            Ok::<_, Unavailable>((
                parameters.required_selector("provided_selector")?,
                parameters.required_selector("required_selector")?,
                unit("provided_unit")?,
                unit("required_unit")?,
                Operator::parse(operator)
                    .ok_or_else(|| invalid(format!("operator `{operator}` is unsupported")))?,
                operator,
                parameters.traversal()?,
            ))
        })();
        let (provided, required, provided_unit, required_unit, operator, word, traversal) =
            match parsed {
                Ok(parsed) => parsed,
                Err((reason, message)) => {
                    return CapabilityEvaluation::not_evaluated(
                        reason,
                        format!("relative-count: {message}"),
                    );
                }
            };
        let provided = Population::of(context, provided);
        let required = Population::of(context, required);
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let via = relation_text(traversal.as_ref());
        for anchor in anchors {
            let tallies = tally(context, traversal.as_ref(), anchor, &provided)
                .and_then(|p| Ok((p, tally(context, traversal.as_ref(), anchor, &required)?)));
            let (provided, required) = match tallies {
                Ok(tallies) => tallies,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                    continue;
                }
            };
            if provided.undecided + required.undecided > 0 {
                evaluation.push_object_not_evaluated(
                    anchor.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} related object(s) {via} cannot be assigned to either population",
                        provided.undecided + required.undecided
                    ),
                );
                continue;
            }
            let (n_provided, n_required) = (provided.decided.len(), required.decided.len());
            #[allow(clippy::cast_possible_wrap)]
            let holds = operator.holds(
                n_provided as i128 * i128::from(required_unit),
                n_required as i128 * i128::from(provided_unit),
            );
            if !holds {
                let mut evidence = provided.evidence;
                evidence.extend(required.evidence);
                evaluation.push_finding(finding(
                    rule,
                    &anchor.id,
                    format!(
                        "{n_provided} provided and {n_required} required object(s) {via}; \
                         required {n_provided}/{provided_unit} {word} {n_required}/{required_unit}"
                    ),
                    evidence,
                    provided
                        .decided
                        .into_iter()
                        .chain(required.decided)
                        .collect(),
                ));
            }
        }
        evaluation
    }
}
