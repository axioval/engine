//! The `keyed-limit` implementation the template replaced, kept as the
//! template's parity reference.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, NotEvaluatedReason, ParameterDescriptor,
    ProximityServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Finding, Object, ObjectId};

use super::threshold::{self, Floor, Threshold, sides};
use super::{
    Declared, Keys, Limit, Measured, Measuring, Quantity, Sills, judge_as_displayed, parse, select,
    sills,
};
use crate::counts::Population;
use crate::level_spacing::{extent, extents};
use crate::plan_area::{Verdict, deviation, judge, shown};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding};

/// Checks a quantity of each object against the limits of the single row
/// of a keyed table that applies to it, as [`crate::KeyedLimit`] does.
pub struct KeyedLimit;

impl RuleCapability for KeyedLimit {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        let (keys, limits, quantity) = match parse(&Parameters(rule)) {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("keyed-limit: {message}"),
                );
            }
        };
        let population = match &quantity {
            Quantity::MemberPlanArea { members, .. } => Some(Population::of(context, members)),
            _ => None,
        };
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        for subject in subjects {
            let measuring = Measuring {
                quantity: &quantity,
                members: population.as_ref(),
            };
            match check(context, rule, &keys, &limits, &measuring, subject) {
                Ok(Some((found, deviation))) => evaluation.push_finding_deviating(found, deviation),
                Ok(None) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}
/// Undecided members can only add area: with any, only a sum already
/// above the maximum stands.
fn only_an_excess(
    measured: &Measured,
    undecided: usize,
    limit: &Limit,
    index: usize,
) -> Result<(), Unavailable> {
    if undecided > 0
        && !limit
            .maximum
            .is_some_and(|maximum| measured.lower > maximum)
    {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{undecided} reached object(s) may be members, so the {} is known only from \
                 below (limit row {index})",
                measured.what
            ),
        ));
    }
    Ok(())
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declared<'_>,
    limits: &[Limit],
    measuring: &Measuring<'_, '_>,
    subject: &Object,
) -> Result<Option<Graded>, Unavailable> {
    let quantity = measuring.quantity;
    let keys = Keys::read(context, declared, subject)?;
    let Some((index, limit)) = select(limits, declared, &keys)? else {
        return Ok(Some((
            finding(
                rule,
                &subject.id,
                format!("no limit defined for {}", keys.describe(declared)),
                keys.evidence,
                keys.sources,
            ),
            None,
        )));
    };
    if limit.minimum.is_none() && limit.maximum.is_none() {
        return Ok(None);
    }
    if let Quantity::SillHeight(floor) = quantity {
        let described = format!("limit row {index}: {}", keys.describe(declared));
        return sill_height(context, rule, floor, subject, limit, &described, keys);
    }
    if let Quantity::ThresholdStep(step) = quantity {
        let described = format!("limit row {index}: {}", keys.describe(declared));
        let limit = (limit.minimum, limit.maximum);
        return judge_step(
            step,
            context,
            rule,
            subject,
            limit,
            &described,
            keys.evidence,
            keys.sources,
        );
    }
    let (measured, members, undecided) = measuring.measure(context, subject)?;
    only_an_excess(&measured, undecided, limit, index)?;
    let unit = &measured.unit;
    let verdict = if matches!(
        quantity,
        Quantity::ClearWidth(_)
            | Quantity::ClearHeight(_)
            | Quantity::GlazingRatio(_)
            | Quantity::Measured(_)
    ) {
        judge_as_displayed(measured.lower, measured.upper, limit.minimum, limit.maximum)
    } else {
        judge(measured.lower, measured.upper, limit.minimum, limit.maximum)
    };
    match verdict {
        Verdict::Pass => Ok(None),
        Verdict::Fail(bound) => {
            let described = keys.describe(declared);
            let mut evidence = keys.evidence;
            evidence.extend(measured.evidence);
            let mut related = keys.sources;
            related.extend(members);
            Ok(Some((
                finding(
                    rule,
                    &subject.id,
                    format!(
                        "{} is {}{unit}; required {bound}{unit} (limit row {index}: {described})",
                        measured.what,
                        shown(measured.lower, measured.upper),
                    ),
                    evidence,
                    related,
                ),
                deviation(measured.lower, measured.upper, limit.minimum, limit.maximum),
            )))
        }
        Verdict::Undecided(bound) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} is {}{unit}, which straddles the bound {bound}{unit} (limit row {index})",
                measured.what,
                shown(measured.lower, measured.upper),
            ),
        )),
    }
}

/// Judges the sill height of `subject` above each floor `path` reaches from
/// it against `limit`. One failing floor is a finding; otherwise a floor that
/// cannot be measured or straddles a bound leaves the subject not evaluated.
fn sill_height(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    path: &Traversal,
    subject: &Object,
    limit: &Limit,
    described: &str,
    keys: Keys,
) -> Result<Option<Graded>, Unavailable> {
    let Sills {
        floors,
        window,
        cited,
    } = sills(context, path, subject)?;
    let mut failed = Vec::new();
    let mut worst: Option<Deviation> = None;
    let mut undecided = Vec::new();
    let mut evidence = keys.evidence;
    let mut related = keys.sources;
    for (floor, measured) in floors {
        let ((lower, upper), measured) = match measured {
            Ok(measured) => measured,
            Err(why) => {
                undecided.push(why);
                continue;
            }
        };
        let height = shown(lower, upper);
        match judge(lower, upper, limit.minimum, limit.maximum) {
            Verdict::Pass => {}
            Verdict::Fail(bound) => {
                failed.push(format!(
                    "sill height above the floor of {floor} is {height} m; required {bound} m"
                ));
                let missed = deviation(lower, upper, limit.minimum, limit.maximum);
                worst = match (worst, missed) {
                    (Some(worst), Some(missed)) => Some(worst.worst(missed)),
                    (worst, missed) => worst.or(missed),
                };
                evidence.push(measured);
                related.push(floor);
            }
            Verdict::Undecided(bound) => undecided.push(format!(
                "sill height above the floor of {floor} is {height} m, which straddles the \
                 bound {bound} m"
            )),
        }
    }
    if failed.is_empty() {
        return if undecided.is_empty() {
            Ok(None)
        } else {
            Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} ({described})", undecided.join("; ")),
            ))
        };
    }
    evidence.push(window);
    evidence.extend(cited);
    related.sort();
    related.dedup();
    Ok(Some((
        finding(
            rule,
            &subject.id,
            format!("{} ({described})", failed.join("; ")),
            evidence,
            related,
        ),
        worst,
    )))
}

/// A finding and how far its value misses the bound, when it has one.
type Graded = (axioval_ir::Finding, Option<Deviation>);

/// Judges the step on every side of `subject` against `(minimum,
/// maximum)`: one side failing is a finding naming its floor;
/// otherwise a side that cannot be decided leaves the door not
/// evaluated.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn judge_step(
    step: &threshold::ThresholdStep<'_>,
    context: &RuleContext<'_>,
    rule: &axioval_engine::CompiledRule,
    subject: &Object,
    (minimum, maximum): (Option<f64>, Option<f64>),
    described: &str,
    mut evidence: Vec<Evidence>,
    mut related: Vec<ObjectId>,
) -> Result<Option<(Finding, Option<Deviation>)>, Unavailable> {
    let service = extents(context)?;
    let door = extent(service, &subject.id)?;
    let threshold = step.threshold(context, subject, &mut evidence)?;
    let everything: Vec<&Object> = context.project.objects().collect();
    let (spaces, cited) = step.floor.related(context, &subject.id, &everything)?;
    if spaces.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} reaches no floor to measure from",
                step.floor.relationship
            ),
        ));
    }
    evidence.extend(cited);
    let ramps = step.ramps(context, service, &subject.id, &spaces);
    let proximity = context.services.get::<ProximityServiceHandle>();
    let bottom = (door.bottom().lower_metres(), door.bottom().upper_metres());
    let mut failed = Vec::new();
    let mut worst: Option<Deviation> = None;
    let mut open = Vec::new();
    for space in &spaces {
        let (sure, possible) = sides(proximity, service, space, &ramps);
        let mut fails = Vec::new();
        let mut passes = true;
        let mut undecided = Vec::new();
        let judged: Vec<(bool, &Floor, Judged)> = sure
            .iter()
            .map(|floor| {
                (
                    true,
                    floor,
                    floor_step(bottom, &threshold, floor, minimum, maximum),
                )
            })
            .chain(possible.iter().map(|floor| {
                (
                    false,
                    floor,
                    floor_step(bottom, &threshold, floor, minimum, maximum),
                )
            }))
            .collect();
        for (is_sure, floor, judged) in &judged {
            match judged {
                Judged::Pass => {}
                Judged::Fail(message, missed) => {
                    passes = false;
                    fails.push((*is_sure, *floor, message.clone(), *missed));
                }
                Judged::Undecided(message) => {
                    passes = false;
                    undecided.push(message.clone());
                }
            }
        }
        let sure_fails: Vec<_> = fails.iter().filter(|(is_sure, ..)| *is_sure).collect();
        let all_fail = sure.is_empty()
            && !possible.is_empty()
            && fails.len() == possible.len()
            && undecided.is_empty();
        if !sure_fails.is_empty() || all_fail {
            let chosen: Vec<_> = if sure_fails.is_empty() {
                fails.iter().collect()
            } else {
                sure_fails
            };
            for (_, floor, message, missed) in chosen {
                failed.push(message.clone());
                worst = match (worst, *missed) {
                    (Some(worst), Some(missed)) => Some(worst.worst(missed)),
                    (worst, missed) => worst.or(missed),
                };
                evidence.extend(floor.evidence.iter().cloned());
                related.push(floor.related.clone());
            }
        } else if !passes {
            if undecided.is_empty() {
                undecided.push(format!(
                    "the floor beside {space} may be any of several, and not every one fails"
                ));
            }
            open.extend(undecided);
        }
    }
    if failed.is_empty() {
        return if open.is_empty() {
            Ok(None)
        } else {
            Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} ({described})", open.join("; ")),
            ))
        };
    }
    evidence.push(door.evidence().clone());
    related.sort();
    related.dedup();
    Ok(Some((
        finding(
            rule,
            &subject.id,
            format!("{} ({described})", failed.join("; ")),
            evidence,
            related,
        ),
        worst,
    )))
}

enum Judged {
    Pass,
    /// A failing step and how far it misses its bound.
    Fail(String, Option<Deviation>),
    Undecided(String),
}

/// The step from `floor` to the door's `bottom` and threshold against the
/// bounds.
fn floor_step(
    bottom: (f64, f64),
    threshold: &Threshold,
    floor: &Floor,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Judged {
    let ((step_low, step_high), named) = match threshold::named_step(bottom, threshold, floor) {
        Ok(step) => step,
        Err(why) => return Judged::Undecided(why),
    };
    match judge_as_displayed(step_low, step_high, minimum, maximum) {
        Verdict::Pass => Judged::Pass,
        Verdict::Fail(bound) => Judged::Fail(
            format!("{named}; required {bound} m"),
            deviation(step_low, step_high, minimum, maximum),
        ),
        Verdict::Undecided(bound) => {
            Judged::Undecided(format!("{named}, which straddles the bound {bound} m"))
        }
    }
}
