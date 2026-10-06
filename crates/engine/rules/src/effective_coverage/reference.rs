//! `effective-coverage` as it was implemented before it became a template
//! (#282), kept only as the parity reference the template is held to in the
//! rules crate's tests (`parity-reference` feature). It is no capability of
//! any registry. It measures through the same [`Element`] the measured
//! values read, its broad phase run once over every element.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ObjectBounds, ParameterDescriptor,
    ProximityProjection, RuleCapability, RuleContext, projected_candidate_pairs,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use super::{Around, Capacity, Element, Mode, Multiplier, Services, Setting, Unmeasured};
use crate::near::bounds;
use crate::pairs::refuse_all;
use crate::plan_area::{Verdict, judge, shown};
use crate::selection::select_objects;
use crate::space_access::AccessDeclaration;
use crate::support::{Parameters, Unavailable, finding, invalid};

const NAME: &str = "effective-coverage";

/// Requires the union of sources' effect areas to cover enough of each
/// selected element's footprint, as `effective-coverage` judged it before
/// it became a template.
pub struct EffectiveCoverage;

struct Config<'a> {
    sources: &'a Selector,
    blockers: Option<&'a Selector>,
    setting: Setting<'a>,
    minimum: f64,
    capacity: Option<Capacity<'a>>,
    access: Option<AccessDeclaration<'a>>,
}

impl RuleCapability for EffectiveCoverage {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (elements, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context) {
            Ok(services) => services,
            Err((reason, message)) => {
                return refuse_all(&elements, evaluation, &reason, &message);
            }
        };
        let near = match Near::find(context, &config, &services, &elements) {
            Ok(near) => near,
            Err((reason, message)) => {
                return refuse_all(&elements, evaluation, &reason, &message);
            }
        };
        let index = config.access.as_ref().map(|access| access.index(context));
        let mut evaluation = evaluation;
        for object in elements {
            if let Some((reason, message)) = near.unbounded.get(&object.id) {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
                continue;
            }
            let along = |map: &BTreeMap<ObjectId, Vec<ObjectId>>| {
                map.get(&object.id).cloned().unwrap_or_default()
            };
            let (reaching, blocking) = (along(&near.reaching), along(&near.blocking));
            let element = Element {
                context,
                setting: &config.setting,
                services: &services,
                around: Around {
                    reaching: &reaching,
                    blocking: &blocking,
                    sources: &near.sources,
                    blockers: &near.blockers,
                    blind: near.blind.len(),
                    blind_blockers: near.blind_blockers.len(),
                    index: index.as_ref(),
                },
                object,
            };
            for check in checks(&element, &config) {
                match check {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => {
                        evaluation
                            .push_finding(finding(rule, &object.id, message, evidence, related));
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    }
                }
            }
        }
        evaluation
    }
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
    let length = |name: &str| match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value.is_finite() && value >= 0.0 => {
            Ok(Some(value))
        }
        Some(_) => Err(invalid(format!("{name} must be a non-negative length"))),
    };
    let mode = parameters.required_string("mode")?;
    let mode = Mode::of(mode).ok_or_else(|| {
        invalid(format!(
            "mode `{mode}` is unsupported; use `grown`, `touching`, `travel` or `visible`"
        ))
    })?;
    let range = length("range")?.ok_or_else(|| invalid("range is required"))?;
    let minimum = match parameters.number("minimum_ratio")? {
        Some(minimum) if minimum > 0.0 && minimum <= 1.0 => minimum,
        _ => return Err(invalid("minimum_ratio must lie in (0, 1]")),
    };
    let blockers = parameters.selector("blockers")?;
    if blockers.is_some() && matches!(mode, Mode::Grown | Mode::Touching) {
        return Err(invalid(
            "blockers apply only to modes `travel` and `visible`",
        ));
    }
    let touch = length("touch_tolerance")?;
    if touch.is_some() && mode != Mode::Touching {
        return Err(invalid("touch_tolerance applies only to mode `touching`"));
    }
    let access = AccessDeclaration::parse(parameters)?;
    if access.is_some() && matches!(mode, Mode::Grown | Mode::Touching) {
        return Err(invalid(
            "access_path applies only to modes `travel` and `visible`: a grown effect ignores \
             walls already",
        ));
    }
    Ok(Config {
        sources: parameters.required_selector("sources")?,
        blockers,
        setting: Setting {
            mode,
            range,
            touch: touch.unwrap_or(0.0),
            area: None,
        },
        minimum,
        capacity: capacity(parameters)?,
        access,
    })
    .and_then(|mut config| {
        config.setting.area = parameters.property("area_property")?;
        Ok(config)
    })
}

fn capacity<'a>(parameters: &Parameters<'a>) -> Result<Option<Capacity<'a>>, Unavailable> {
    let property = parameters.property("capacity_property")?;
    let constant = parameters.number("capacity_multiplier")?;
    let per_source = parameters.property("capacity_multiplier_property")?;
    let multiplier = match (constant, per_source) {
        (None, None) => None,
        (Some(multiplier), None) if multiplier.is_finite() && multiplier > 0.0 => {
            Some(Multiplier::Constant(multiplier))
        }
        (Some(_), None) => return Err(invalid("capacity_multiplier must be a positive number")),
        (None, Some(property)) => Some(Multiplier::Property(property)),
        (Some(_), Some(_)) => {
            return Err(invalid(
                "declare capacity_multiplier or capacity_multiplier_property, not both",
            ));
        }
    };
    match (property, multiplier) {
        (None, None) => Ok(None),
        (Some(property), Some(multiplier)) => Ok(Some(Capacity {
            property,
            multiplier,
        })),
        _ => Err(invalid(
            "capacity_property is declared together with capacity_multiplier or \
             capacity_multiplier_property",
        )),
    }
}

/// The sources and blockers near each element, from the plan broad phase.
struct Near {
    /// Sources the selector picks.
    sources: BTreeSet<ObjectId>,
    /// Blockers the selector picks.
    blockers: BTreeSet<ObjectId>,
    /// Sources within reach of each element, picked or undecided.
    reaching: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Blockers that may matter to each element, picked or undecided: those
    /// overlapping it in plan, or with connections those within range.
    blocking: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Sources whose extent cannot be read: they may reach any element.
    blind: BTreeSet<ObjectId>,
    /// Blockers whose extent cannot be read: they may block in any element.
    blind_blockers: BTreeSet<ObjectId>,
    /// Elements whose extent cannot be read.
    unbounded: BTreeMap<ObjectId, Unavailable>,
}

/// The objects a selector picks, and those it cannot decide.
fn picked(
    context: &RuleContext<'_>,
    selector: &Selector,
) -> (BTreeSet<ObjectId>, BTreeSet<ObjectId>) {
    let (matched, selection) = select_objects(context, selector);
    (
        matched.iter().map(|object| object.id.clone()).collect(),
        selection
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect(),
    )
}

impl Near {
    fn find(
        context: &RuleContext<'_>,
        config: &Config<'_>,
        services: &Services<'_>,
        elements: &[&Object],
    ) -> Result<Self, Unavailable> {
        let (sources, undecided_sources) = picked(context, config.sources);
        let (blockers, undecided_blockers) = config
            .blockers
            .map(|selector| picked(context, selector))
            .unwrap_or_default();
        let mut unbounded = BTreeMap::new();
        let mut element_bounds: Vec<ObjectBounds> = Vec::new();
        for element in elements {
            match bounds(services.proximity, &element.id) {
                Ok(extent) => element_bounds.push(extent),
                Err((reason, message)) => {
                    unbounded.insert(
                        element.id.clone(),
                        (reason, format!("{message}; its coverage was not checked")),
                    );
                }
            }
        }
        let gather = |candidates: &mut dyn Iterator<Item = &ObjectId>| {
            let mut found = Vec::new();
            let mut blind = BTreeSet::new();
            for candidate in candidates {
                match bounds(services.proximity, candidate) {
                    Ok(extent) => found.push(extent),
                    Err(_) => {
                        blind.insert(candidate.clone());
                    }
                }
            }
            (found, blind)
        };
        let (source_bounds, blind) = gather(&mut sources.iter().chain(&undecided_sources));
        let (blocker_bounds, blind_blockers) = gather(
            &mut blockers
                .iter()
                .chain(&undecided_blockers)
                .filter(|blocker| {
                    !sources.contains(*blocker) && !undecided_sources.contains(*blocker)
                }),
        );
        let margin = config.setting.margin();
        // A walk or a sight line reaching the element within the range
        // stays within the range of it, so only blockers there can cut it.
        let blocker_margin = if config.access.is_some() {
            config.setting.range
        } else {
            0.0
        };
        let pairs = |counterparts: &[ObjectBounds], margin: f64| {
            projected_candidate_pairs(
                &element_bounds,
                counterparts,
                ProximityProjection::Horizontal,
                margin,
            )
            .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))
            .map(|pairs| {
                let mut near: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
                for pair in pairs {
                    if pair.subject() != pair.counterpart() {
                        near.entry(pair.subject().clone())
                            .or_default()
                            .push(pair.counterpart().clone());
                    }
                }
                near
            })
        };
        Ok(Self {
            reaching: pairs(&source_bounds, margin)?,
            blocking: pairs(&blocker_bounds, blocker_margin)?,
            sources,
            blockers,
            blind,
            blind_blockers,
            unbounded,
        })
    }
}

/// The finding of one check, or `None` when it passes.
type Check = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

/// The element's checks, in the order the capability judged them.
fn checks(element: &Element<'_, '_>, config: &Config<'_>) -> Vec<Check> {
    let measured = match element.measure() {
        Ok(measured) => measured,
        Err(Unmeasured::Missing(message, evidence)) => {
            return vec![Ok(Some((message, evidence, Vec::new())))];
        }
        Err(Unmeasured::Unavailable(refused)) => return vec![Err(refused)],
    };
    let area = &measured.area;
    let named = match element.setting.area {
        Some(property) => format!("the stated area ({property})"),
        None => "the footprint".to_owned(),
    };
    let covered = element.covered(&measured);
    let (share, (lower, upper)) = (covered.share, covered.covered);
    let what = format!(
        "{} of {named} ({} of {} m²) lies within the sources' effect areas ({} by {} m)",
        shown(share.0, share.1),
        shown(lower, upper),
        shown(area.lower, area.upper),
        mode_name(element.setting.mode),
        element.setting.range,
    );
    let coverage = match judge(share.0, share.1, Some(config.minimum), None) {
        Verdict::Pass => Ok(None),
        Verdict::Fail(bound) => {
            let mut message = format!("{what}; required {bound}");
            if measured.asked.request.sources().is_empty() {
                message.push_str("; no source reaches it");
            }
            Ok(Some((
                message,
                covered.evidence,
                super::contributing(&measured.asked.request, &measured.coverage, true),
            )))
        }
        Verdict::Undecided(bound) => {
            let mut message = format!("{what}, which straddles the bound {bound}");
            for note in covered.notes.iter().take(3) {
                let _ = write!(message, "; {note}");
            }
            Err((NotEvaluatedReason::IncompleteEvidence, message))
        }
    };
    let mut checks = vec![coverage];
    if let Some(capacity) = config.capacity {
        checks.extend(capacity_checks(element, &measured, capacity, &named));
    }
    checks
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Grown => "grown",
        Mode::Touching => "touching",
        Mode::Travel => "travel",
        Mode::Visible => "visible",
    }
}

/// The summed capacity against the element's area, then a missing-value
/// finding per surely contributing source that states no capacity or
/// multiplier.
fn capacity_checks(
    element: &Element<'_, '_>,
    measured: &super::Measured,
    capacity: Capacity<'_>,
    named: &str,
) -> Vec<Check> {
    let summed = element.capacity(measured, capacity);
    let area = &measured.area;
    let upper = if summed.unread > 0 {
        f64::INFINITY
    } else {
        summed.upper
    };
    let (need_low, need_high) = (area.lower, area.upper);
    let against = if element.setting.area.is_some() {
        named
    } else {
        "a footprint"
    };
    let summed_words = match capacity.multiplier {
        Multiplier::Constant(multiplier) => format!(
            "{} summed over the sources reaching it, times {multiplier},",
            capacity.property
        ),
        Multiplier::Property(multiplier) => format!(
            "{} times {multiplier} summed over the sources reaching it",
            capacity.property
        ),
    };
    let what = format!(
        "capacity: {summed_words} is {} m² for {against} of {} m²",
        shown(summed.lower, upper),
        shown(need_low, need_high),
    );
    let check = if summed.lower >= need_high {
        Ok(None)
    } else if upper < need_low {
        Ok(Some((what, summed.evidence, summed.sure)))
    } else {
        let mut message = format!("{what}, which cannot be decided");
        for note in summed.unknown.iter().take(3) {
            let _ = write!(message, "; {note}");
        }
        Err((NotEvaluatedReason::IncompleteEvidence, message))
    };
    let mut checks = vec![check];
    for (source, _, message, cited) in summed.missing {
        checks.push(Ok(Some((message, cited, vec![source]))));
    }
    checks
}
