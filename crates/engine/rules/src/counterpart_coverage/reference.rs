//! `counterpart-coverage` as it was implemented before it became a
//! template (#282), kept only as the parity reference the template is held
//! to in the rules crate's tests (`parity-reference` feature). It is no
//! capability of any registry. It measures through the same [`Subject`]
//! the measured values read, its broad phase run once over every subject.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, NotEvaluatedReason, ObjectBounds,
    ParameterDescriptor, ProximityProjection, RuleCapability, RuleContext,
    projected_candidate_pairs,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension, Severity};

use super::{Candidates, Config, Counterparts, Cover, Infill, Services, Share, Subject, bounds};
use crate::orientation::{Tri, angle_tolerance};
use crate::pairs::refuse_all;
use crate::plan_area::{footprint, shown};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

const NAME: &str = "counterpart-coverage";

/// Requires each selected element to be covered by its counterparts, as
/// `counterpart-coverage` judged it before it became a template.
pub struct CounterpartCoverage;

/// A declared threshold and the severity of the band above it.
type Band = (f64, Severity);

/// The rule's declaration.
struct Declared<'a> {
    counterparts: &'a Selector,
    config: Config,
    /// Ascending thresholds.
    bands: Vec<Band>,
    /// Frame members whose infill covers.
    infill: Option<&'a Selector>,
}

impl RuleCapability for CounterpartCoverage {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match parse(&Parameters(rule)) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let config = &declared.config;
        let (subjects, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context, config) {
            Ok(services) => services,
            Err((reason, message)) => return refuse_all(&subjects, evaluation, &reason, &message),
        };
        let margin = config.margin();
        let found = |selector: &Selector| find(context, selector, margin, &services, &subjects);
        let (counterparts, unbounded) = match found(declared.counterparts) {
            Ok(counterparts) => counterparts,
            Err((reason, message)) => return refuse_all(&subjects, evaluation, &reason, &message),
        };
        let frame = match declared.infill.map(found).transpose() {
            Ok(frame) => frame.map(|(frame, _)| frame),
            Err((reason, message)) => return refuse_all(&subjects, evaluation, &reason, &message),
        };
        let mut evaluation = evaluation;
        for subject in subjects {
            if let Some((reason, message)) = unbounded.get(&subject.id) {
                evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
                continue;
            }
            let subject = Subject {
                config,
                services: &services,
                counterparts: &counterparts,
                frame: frame.as_ref(),
                object: subject,
            };
            for check in checks(context, &subject, &declared.bands) {
                match check {
                    Ok(None) => {}
                    Ok(Some(Graded {
                        severity,
                        message,
                        evidence,
                        related,
                        deviation,
                    })) => {
                        let mut found =
                            finding(rule, &subject.object.id, message, evidence, related);
                        found.severity = severity;
                        evaluation.push_graded_finding(found, deviation);
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(
                            subject.object.id.clone(),
                            reason,
                            message,
                        );
                    }
                }
            }
        }
        evaluation
    }
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Declared<'a>, Unavailable> {
    let length = |name: &str| match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) => Ok(Some(value)),
        Some(_) => Err(invalid(format!("{name} must be a length"))),
    };
    let (horizontal, vertical) = match (
        length("tolerance")?,
        length("horizontal_tolerance")?,
        length("vertical_tolerance")?,
    ) {
        (Some(both), None, None) => (both, both),
        (None, Some(horizontal), Some(vertical)) => (horizontal, vertical),
        (None, None, None) => {
            return Err(invalid(
                "declare `tolerance`, or `horizontal_tolerance` and `vertical_tolerance`",
            ));
        }
        (Some(_), _, _) => {
            return Err(invalid(
                "`tolerance` cannot be combined with `horizontal_tolerance` or \
                 `vertical_tolerance`",
            ));
        }
        _ => {
            return Err(invalid(
                "`horizontal_tolerance` and `vertical_tolerance` are declared together",
            ));
        }
    };
    let on = |tolerance: f64| (tolerance >= 0.0).then_some(tolerance);
    let (horizontal, vertical) = (on(horizontal), on(vertical));
    if horizontal.is_none() && vertical.is_none() {
        return Err(invalid(
            "a negative tolerance switches its check off, and every check is off",
        ));
    }
    let mut bands: Vec<Band> = Vec::new();
    for (name, severity) in [
        ("info_above", Severity::Info),
        ("warning_above", Severity::Warning),
        ("error_above", Severity::Error),
    ] {
        let Some(threshold) = parameters.number(name)? else {
            continue;
        };
        if !(0.0..1.0).contains(&threshold) {
            return Err(invalid(format!("{name} must lie in [0, 1)")));
        }
        if bands.last().is_some_and(|(below, _)| *below >= threshold) {
            return Err(invalid(format!(
                "{name} must exceed the thresholds of less severe bands"
            )));
        }
        bands.push((threshold, severity));
    }
    if bands.is_empty() {
        return Err(invalid(
            "declare at least one of `info_above`, `warning_above` and `error_above`",
        ));
    }
    let elevation = match parameters.string("measure")? {
        None | Some("plan_and_height") => false,
        Some("elevation") => true,
        Some(other) => {
            return Err(invalid(format!(
                "measure `{other}` is unsupported; use `plan_and_height` or `elevation`"
            )));
        }
    };
    if elevation && (horizontal.is_none() || vertical.is_none()) {
        return Err(invalid(
            "the elevation is one check measured with both tolerances; neither may be negative",
        ));
    }
    let counterparts = parameters.required_selector("counterparts")?;
    let axis = angle_tolerance(parameters, "axis_tolerance")?;
    let (infill, above) = match infill(parameters, elevation)? {
        Some((selector, above)) => (Some(selector), Some(above)),
        None => (None, None),
    };
    Ok(Declared {
        counterparts,
        config: Config {
            horizontal,
            vertical,
            axis,
            elevation,
            infill: above,
        },
        bands,
        infill,
    })
}

/// The frame members whose infill covers, and the share above which.
fn infill<'a>(
    parameters: &Parameters<'a>,
    elevation: bool,
) -> Result<Option<(&'a Selector, f64)>, Unavailable> {
    match (
        parameters.selector("infill_counterparts")?,
        parameters.number("infill_above")?,
    ) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(invalid("`infill_above` needs `infill_counterparts`")),
        (Some(_), _) if !elevation => Err(invalid(
            "`infill_counterparts` applies only to `measure` `elevation`",
        )),
        (Some(selector), above) => {
            let above = above.unwrap_or(0.5);
            if (0.0..1.0).contains(&above) {
                Ok(Some((selector, above)))
            } else {
                Err(invalid("infill_above must lie in [0, 1)"))
            }
        }
    }
}

/// The counterparts `selector` picks near each subject, from one plan broad
/// phase over every subject, and the subjects whose extent cannot be read.
fn find(
    context: &RuleContext<'_>,
    selector: &Selector,
    margin: f64,
    services: &Services<'_>,
    subjects: &[&Object],
) -> Result<(Counterparts, BTreeMap<ObjectId, Unavailable>), Unavailable> {
    let (matched, selection) = select_objects(context, selector);
    let matched: BTreeSet<ObjectId> = matched.iter().map(|object| object.id.clone()).collect();
    let undecided: BTreeSet<ObjectId> = selection
        .not_evaluated_outcomes()
        .iter()
        .filter_map(|outcome| outcome.object_id().cloned())
        .collect();
    let mut unbounded = BTreeMap::new();
    let mut subject_bounds: Vec<ObjectBounds> = Vec::new();
    for subject in subjects {
        match bounds(services.proximity, &subject.id) {
            Ok(extent) => subject_bounds.push(extent),
            Err((reason, message)) => {
                unbounded.insert(
                    subject.id.clone(),
                    (reason, format!("{message}; its coverage was not checked")),
                );
            }
        }
    }
    let (candidates, extents) = Candidates::read(services.proximity, matched, &undecided);
    let pairs = projected_candidate_pairs(
        &subject_bounds,
        &extents,
        ProximityProjection::Horizontal,
        margin,
    )
    .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))?;
    let mut near: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    for pair in pairs {
        if pair.subject() != pair.counterpart() {
            near.entry(pair.subject().clone())
                .or_default()
                .push(pair.counterpart().clone());
        }
    }
    Ok((
        Counterparts {
            candidates: Arc::new(candidates),
            near,
        },
        unbounded,
    ))
}

/// A finding graded into a band, with how far it misses the lowest.
struct Graded {
    severity: Severity,
    message: String,
    evidence: Vec<Evidence>,
    related: Vec<ObjectId>,
    deviation: Deviation,
}

/// The finding of one check, or `None` when it passes.
type Check = Result<Option<Graded>, Unavailable>;

/// The subject's checks, each graded.
fn checks(context: &RuleContext<'_>, subject: &Subject<'_, '_>, bands: &[Band]) -> Vec<Check> {
    let config = subject.config;
    if config.elevation {
        return vec![subject.elevation_share().and_then(|(share, cover, infill)| {
            let infill = match infill {
                Some(Infill {
                    applies: Tri::Yes,
                    members,
                }) => format!(" or the infill of the frame of {}", named(&members)),
                Some(Infill { members, .. }) => format!(
                    " or, should more than {} be uncovered (undecided), the infill of the frame \
                     of {}",
                    config.infill.unwrap_or(0.5),
                    named(&members)
                ),
                None => String::new(),
            };
            let what = format!(
                "elevation: {} of the elevation ({} of {} m²) lies outside every \
                 counterpart{infill}, grown by {} m along its axis and {} m in height",
                shown(share.interval.0, share.interval.1),
                shown(share.uncovered.0, share.uncovered.1),
                shown(share.whole.0, share.whole.1),
                config.horizontal.unwrap_or(0.0),
                config.vertical.unwrap_or(0.0),
            );
            grade((share, what), &cover, bands)
        })];
    }
    let area = match footprint(context, &subject.object.id) {
        Ok(area) => area,
        Err(error) => return vec![Err(error)],
    };
    let cover = subject.cover(&area);
    let mut checks = Vec::new();
    if let Some(growth) = subject.config.horizontal {
        checks.push(subject.plan_share(&area, &cover, growth).and_then(|share| {
            let what = format!(
                "plan: {} of the footprint ({} of {} m²) lies outside every counterpart \
                         grown by {growth} m",
                shown(share.interval.0, share.interval.1),
                shown(share.uncovered.0, share.uncovered.1),
                shown(share.whole.0, share.whole.1),
            );
            grade((share, what), &cover, bands)
        }));
    }
    if let (Some(growth), Some(extents)) = (subject.config.vertical, subject.services.extents) {
        checks.push(
            subject
                .height_share(extents, &cover, growth)
                .and_then(|share| {
                    let what = format!(
                        "height: {} of the height ({} of {} m) lies outside every counterpart \
                         overlapping it in plan, grown by {growth} m",
                        shown(share.interval.0, share.interval.1),
                        shown(share.uncovered.0, share.uncovered.1),
                        shown(share.whole.0, share.whole.1),
                    );
                    grade((share, what), &cover, bands)
                }),
        );
    }
    checks
}

/// Object identities for a message, comma-separated.
fn named(objects: &[ObjectId]) -> String {
    objects
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Grades an uncovered share, and what it measured, against the declared
/// bands.
fn grade((measured, what): (Share, String), cover: &Cover, bands: &[Band]) -> Check {
    let Share {
        interval: (lower, upper),
        evidence,
        ..
    } = measured;
    let lowest = bands[0].0;
    let reached = |share: f64| {
        bands
            .iter()
            .rev()
            .find(|(threshold, _)| share > *threshold)
            .map(|(_, severity)| severity.clone())
    };
    if upper <= lowest {
        return Ok(None);
    }
    let Some(surely) = reached(lower) else {
        let mut message = format!("{what}, which straddles the threshold {lowest}");
        for unknown in cover.unknown.iter().chain(&cover.axes) {
            message.push_str("; ");
            message.push_str(unknown);
        }
        return Err((NotEvaluatedReason::IncompleteEvidence, message));
    };
    let severity = reached(upper).unwrap_or_else(|| surely.clone());
    let mut message = what;
    if cover.most.is_empty() && cover.unknown.is_empty() {
        message.push_str("; no counterpart overlaps it");
    }
    if severity != surely {
        let _ = write!(
            message,
            "; graded {} by its upper bound, at least {}",
            label(&severity),
            label(&surely)
        );
    }
    Ok(Some(Graded {
        severity,
        message,
        evidence,
        related: cover.least.clone(),
        deviation: Deviation::above(lowest, lower, upper),
    }))
}

fn label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}
