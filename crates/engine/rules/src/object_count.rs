//! Existence and cardinality of the rule's selection, per source or project.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Finding, ObjectId, Scope};

use crate::pairs::severity;
use crate::selection::{Selection, selector_matches};
use crate::support::{Parameters, Unavailable, invalid};

/// Requires the rule's selection to hold a bounded number of objects in each
/// source, or in the whole project with `across_sources`.
///
/// This is the check an object rule cannot make: "the model has a building",
/// "at most one site", "an IDS specification requires at least one wall".
/// An object rule over an empty selection reports nothing, which reads as
/// compliance; this one reports the empty selection itself, against the
/// source or the project rather than an object.
///
/// `minimum` and `maximum` bound the count, inclusive. With neither, the
/// rule is an existence check: at least one object must match. An object
/// whose selection cannot be decided may or may not count; the scope is
/// judged only when those objects cannot change the verdict, and is not
/// evaluated otherwise, together with each undecided object.
pub struct ObjectCount;

impl RuleCapability for ObjectCount {
    fn id(&self) -> &'static str {
        "axioval:capability.object-count"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("minimum", ParameterType::Integer),
            ParameterDescriptor::optional("maximum", ParameterType::Integer),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.integer("minimum")?;
            let maximum = parameters.integer("maximum")?;
            let bounds = match (minimum, maximum) {
                (Some(minimum), _) if minimum < 0 => return Err(invalid("minimum is negative")),
                (_, Some(maximum)) if maximum < 0 => return Err(invalid("maximum is negative")),
                (Some(minimum), Some(maximum)) if minimum > maximum => {
                    return Err(invalid("minimum exceeds maximum"));
                }
                // No bound at all: the selection must not be empty.
                (None, None) => Bounds {
                    minimum: Some(1),
                    maximum: None,
                },
                (minimum, maximum) => Bounds { minimum, maximum },
            };
            Ok::<_, Unavailable>((
                bounds,
                parameters.boolean("across_sources")?.unwrap_or(false),
            ))
        })();
        let (bounds, across_sources) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("object-count: {message}"),
                );
            }
        };

        // Every scope is counted, including one where nothing matches: that
        // is the case this capability exists to report.
        let mut tallies: BTreeMap<Scope, Tally> = BTreeMap::new();
        if across_sources {
            tallies.insert(Scope::Project, Tally::default());
        }
        for object in context.project.objects() {
            let scope = if across_sources {
                Scope::Project
            } else {
                Scope::Source(object.id.source.clone())
            };
            let tally = tallies.entry(scope).or_default();
            let mut evidence = Vec::new();
            match selector_matches(context, &rule.selector, object, &mut evidence) {
                Selection::Match => {
                    tally.matched.push(object.id.clone());
                    tally.evidence.extend(evidence);
                }
                Selection::NoMatch => {}
                Selection::NotEvaluated(reason, message) => {
                    tally.undecided.push((object.id.clone(), reason, message));
                }
            }
        }

        let mut evaluation = CapabilityEvaluation::default();
        if tallies.is_empty() {
            // Per source, and no source contributes an object: there is no
            // scope to judge, and saying nothing would read as a pass.
            evaluation.push_not_evaluated(
                NotEvaluatedReason::IncompleteEvidence,
                "object-count: the project has no source to count in",
            );
        }
        for (scope, tally) in tallies {
            judge(rule, &bounds, scope, tally, &mut evaluation);
        }
        evaluation
    }
}

struct Bounds {
    minimum: Option<i64>,
    maximum: Option<i64>,
}

impl Bounds {
    fn text(&self) -> String {
        match (self.minimum, self.maximum) {
            (Some(minimum), Some(maximum)) if minimum == maximum => format!("exactly {minimum}"),
            (Some(minimum), Some(maximum)) => format!("between {minimum} and {maximum}"),
            (Some(minimum), None) => format!("at least {minimum}"),
            (None, Some(maximum)) => format!("at most {maximum}"),
            (None, None) => unreachable!("parsing always sets a bound"),
        }
    }
}

/// The selection within one scope.
#[derive(Default)]
struct Tally {
    matched: Vec<ObjectId>,
    undecided: Vec<(ObjectId, NotEvaluatedReason, String)>,
    evidence: Vec<Evidence>,
}

fn judge(
    rule: &CompiledRule,
    bounds: &Bounds,
    scope: Scope,
    tally: Tally,
    evaluation: &mut CapabilityEvaluation,
) {
    let place = match &scope {
        Scope::Source(source) => format!("in source `{source}`"),
        Scope::Project | Scope::Object(_) => "in the project".to_owned(),
    };
    let count = i64::try_from(tally.matched.len()).unwrap_or(i64::MAX);
    let most = count.saturating_add(i64::try_from(tally.undecided.len()).unwrap_or(i64::MAX));
    let too_few = bounds.minimum.is_some_and(|minimum| most < minimum);
    let too_many = bounds.maximum.is_some_and(|maximum| count > maximum);
    let undecided = bounds.minimum.is_some_and(|minimum| count < minimum)
        || bounds.maximum.is_some_and(|maximum| most > maximum);
    let required = bounds.text();
    if too_few || too_many {
        let message = if count == 0 {
            format!("no object matches the selection {place}; required {required}")
        } else {
            format!("{count} object(s) match the selection {place}; required {required}")
        };
        evaluation.push_finding(
            Finding::new(rule.id.clone(), scope, severity(rule), message)
                .with_evidence(tally.evidence)
                .with_related(tally.matched),
        );
    } else if undecided {
        let message = format!(
            "object-count: {count} object(s) match the selection {place} and {} more may; required {required}",
            tally.undecided.len()
        );
        for (object, reason, detail) in tally.undecided {
            evaluation.push_object_not_evaluated(object, reason, detail);
        }
        match scope {
            Scope::Source(source) => evaluation.push_source_not_evaluated(
                source,
                NotEvaluatedReason::IncompleteEvidence,
                message,
            ),
            Scope::Project | Scope::Object(_) => {
                evaluation.push_not_evaluated(NotEvaluatedReason::IncompleteEvidence, message);
            }
        }
    }
}
