//! Existence and cardinality of the rule's selection, per source or project.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext, SourceDisciplines,
};
use axioval_ir::{Discipline, Evidence, Finding, ObjectId, Scope, SourceId};

use crate::pairs::severity;
use crate::selection::{Selection, selector_matches};
use crate::support::{Parameters, Unavailable, invalid, sources};

/// Requires the rule's selection to hold a bounded number of objects in each
/// source, or in the whole project with `across_sources`.
///
/// This is the check an object rule cannot make: "the model has a building",
/// "at most one site", "an IDS specification requires at least one wall".
/// An object rule over an empty selection reports nothing, which reads as
/// compliance; this one reports the empty selection itself, against the
/// source or the project rather than an object.
///
/// Every source of the session is counted, including one that holds no
/// objects at all: an empty model does not contain a building, and says so.
///
/// `disciplines` counts only the sources playing one of the listed
/// disciplines: a per-source duct count limited to `mep` judges the MEP
/// models alone, and the architecture model raises no finding. A source that
/// declares no discipline may or may not count: per source it is not
/// evaluated (once, `NotRecorded`), and across sources its matching objects
/// are undecided. No source playing a listed discipline leaves the rule not
/// evaluated rather than passed.
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
            ParameterDescriptor::optional("disciplines", ParameterType::StringList),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parsed = parse(rule);
        let (bounds, across_sources, disciplines) = match parsed {
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
        // A source with no objects at all still gets its tally, so an empty
        // model is reported as holding nothing instead of never being judged.
        let mut evaluation = CapabilityEvaluation::default();
        let membership = match membership(context, disciplines.as_ref()) {
            Ok(membership) => membership,
            Err(unavailable) => return unavailable,
        };
        let mut tallies: BTreeMap<Scope, Tally> = BTreeMap::new();
        if across_sources {
            tallies.insert(Scope::Project, Tally::default());
        } else {
            for source in sources(context) {
                if membership.left_out.contains(&source) {
                    continue;
                }
                if membership.unknown.contains(&source) {
                    evaluation.push_source_not_evaluated(
                        source.clone(),
                        NotEvaluatedReason::NotRecorded,
                        format!(
                            "object-count: source `{source}` declares no discipline, so whether it is counted is unknown"
                        ),
                    );
                    continue;
                }
                tallies.insert(Scope::Source(source), Tally::default());
            }
        }
        for object in context.project.objects() {
            let source = &object.id.source;
            if membership.left_out.contains(source)
                || (!across_sources && membership.unknown.contains(source))
            {
                continue;
            }
            let scope = if across_sources {
                Scope::Project
            } else {
                Scope::Source(source.clone())
            };
            let tally = tallies.entry(scope).or_default();
            let mut evidence = Vec::new();
            let selection = selector_matches(context, &rule.selector, object, &mut evidence);
            match (selection, membership.unknown.contains(source)) {
                (Selection::NoMatch, _) => {}
                (Selection::Match, false) => {
                    tally.matched.push(object.id.clone());
                    tally.evidence.extend(evidence);
                }
                // Across sources, a match in a source of no declared
                // discipline may count or not.
                (Selection::Match, true) => tally.undecided.push((
                    object.id.clone(),
                    NotEvaluatedReason::NotRecorded,
                    format!(
                        "source `{source}` declares no discipline, so whether this object is counted is unknown"
                    ),
                )),
                (Selection::NotEvaluated(reason, message), _) => {
                    tally.undecided.push((object.id.clone(), reason, message));
                }
            }
        }

        if tallies.is_empty() && membership.unknown.is_empty() {
            // Per source, and no source counts: there is no scope to judge,
            // and saying nothing would read as a pass.
            let message = if membership.left_out.is_empty() {
                "object-count: the project has no source to count in".to_owned()
            } else {
                format!(
                    "object-count: no source plays {}, so there is no source to count in",
                    listed(disciplines.iter().flatten())
                )
            };
            evaluation.push_not_evaluated(NotEvaluatedReason::IncompleteEvidence, message);
        }
        for (scope, tally) in tallies {
            judge(rule, &bounds, scope, tally, &mut evaluation);
        }
        evaluation
    }
}

type Parsed = (Bounds, bool, Option<BTreeSet<Discipline>>);

fn parse(rule: &CompiledRule) -> Result<Parsed, Unavailable> {
    let parameters = Parameters(rule);
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
    let disciplines = parameters
        .strings("disciplines")?
        .map(|names| {
            if names.is_empty() {
                return Err(invalid("`disciplines` is empty"));
            }
            names
                .iter()
                .map(|name| {
                    Discipline::new(name.as_str()).map_err(|error| invalid(error.to_string()))
                })
                .collect::<Result<BTreeSet<_>, _>>()
        })
        .transpose()?;
    Ok((
        bounds,
        parameters.boolean("across_sources")?.unwrap_or(false),
        disciplines,
    ))
}

/// Which sources count: every one, or those playing a listed discipline. A
/// source declaring none may or may not count.
fn membership(
    context: &RuleContext<'_>,
    disciplines: Option<&BTreeSet<Discipline>>,
) -> Result<Membership, CapabilityEvaluation> {
    let Some(disciplines) = disciplines else {
        return Ok(Membership::default());
    };
    let Some(declared) = context.services.get::<SourceDisciplines>() else {
        return Err(CapabilityEvaluation::not_evaluated(
            NotEvaluatedReason::MissingService,
            "object-count: source disciplines are not available outside an evidence session",
        ));
    };
    let mut membership = Membership::default();
    for source in sources(context) {
        match declared.of(&source) {
            Some(discipline) if disciplines.contains(discipline) => {}
            Some(_) => {
                membership.left_out.insert(source);
            }
            None => {
                membership.unknown.insert(source);
            }
        }
    }
    Ok(membership)
}

/// Disciplines as a reader reads them: `` `mep` `` or `` `mep` or `hvac` ``.
fn listed<'a>(disciplines: impl Iterator<Item = &'a Discipline>) -> String {
    disciplines
        .map(|discipline| format!("`{discipline}`"))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Sources a `disciplines` list leaves out, and sources whose discipline is
/// unknown; both empty without the list.
#[derive(Default)]
struct Membership {
    left_out: BTreeSet<SourceId>,
    unknown: BTreeSet<SourceId>,
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
