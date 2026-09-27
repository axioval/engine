//! Visibility of targets from an eye above each component: a reception desk
//! that must see the entrance doors, or a device that must be seen from
//! nowhere.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PlanSpanServiceHandle, RuleCapability, RuleContext, SightError, SightEvidence, SightOutcome,
    SightRequest, SightServiceHandle, VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::pairs::refuse_all;
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

const NAME: &str = "component-visibility";

/// Requires targets within a radius to be in view from an eye point above
/// each selected component.
///
/// The eye stands `eye_height` above the component's base (the bottom of
/// its vertical extent), over the centre of its footprint. Every object
/// `targets` picks whose nearest point lies within `radius` of the eye is a
/// target; `blockers` picks the objects that may hide one. The component
/// itself and the target never block.
///
/// - `mode: at-least` requires at least `minimum` targets in view (one when
///   not declared).
/// - `mode: none` requires no target in view.
///
/// A target is in view when a straight segment from the eye reaches it past
/// every blocker (a witness point on it proves this), and hidden when the
/// blockers cover every ray to it. Neither may be provable, for a target
/// only grazed or covered only where two blockers meet; such a target is
/// undecided. Undecided targets, targets whose selection or distance is
/// undecided, and undecided blockers leave the component not evaluated
/// unless they cannot change the verdict. An eye whose centre or base is
/// not known exactly is not evaluated.
///
/// Transparent blockers are left out by the `blockers` selection, through
/// `axioval:presentation.Transparency`; the rule has no threshold of its own.
pub struct ComponentVisibility;

/// What the rule requires.
#[derive(Clone, Copy)]
enum Mode {
    AtLeast(u64),
    None,
}

struct Config<'a> {
    targets: &'a Selector,
    blockers: &'a Selector,
    eye_height: f64,
    radius: f64,
    mode: Mode,
}

impl RuleCapability for ComponentVisibility {
    fn id(&self) -> &'static str {
        "axioval:capability.component-visibility"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("targets", ParameterType::Selector),
            ParameterDescriptor::required("blockers", ParameterType::Selector),
            ParameterDescriptor::required("eye_height", ParameterType::Quantity),
            ParameterDescriptor::required("radius", ParameterType::Quantity),
            ParameterDescriptor::required("mode", ParameterType::String),
            ParameterDescriptor::optional("minimum", ParameterType::Integer),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (components, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context) {
            Ok(services) => services,
            Err((reason, message)) => {
                return refuse_all(&components, evaluation, &reason, &message);
            }
        };
        let targets = Picked::of(context, config.targets);
        let blockers = Picked::of(context, config.blockers);
        let mut evaluation = evaluation;
        for component in components {
            match View::of(&services, &config, &targets, &blockers, component) {
                Ok(view) => view.judge(rule, &config, &mut evaluation),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(component.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
    let length = |name: &str| match parameters.quantity(name)? {
        Some((value, QuantityDimension::Length)) if value.is_finite() && value >= 0.0 => Ok(value),
        Some(_) => Err(invalid(format!("{name} must be a non-negative length"))),
        None => Err(invalid(format!("{name} is required"))),
    };
    let minimum = parameters.integer("minimum")?;
    let mode = match parameters.string("mode")? {
        Some("at-least") => match minimum {
            None => Mode::AtLeast(1),
            Some(minimum) => Mode::AtLeast(
                u64::try_from(minimum)
                    .ok()
                    .filter(|minimum| *minimum > 0)
                    .ok_or_else(|| invalid("minimum must be a positive count"))?,
            ),
        },
        Some("none") if minimum.is_some() => {
            return Err(invalid("minimum applies only to mode `at-least`"));
        }
        Some("none") => Mode::None,
        Some(other) => {
            return Err(invalid(format!(
                "mode `{other}` is unsupported; use `at-least` or `none`"
            )));
        }
        None => return Err(invalid("mode is required")),
    };
    Ok(Config {
        targets: parameters.required_selector("targets")?,
        blockers: parameters.required_selector("blockers")?,
        eye_height: length("eye_height")?,
        radius: length("radius")?,
        mode,
    })
}

struct Services<'a> {
    sight: &'a SightServiceHandle,
    centres: &'a PlanSpanServiceHandle,
    extents: &'a VerticalExtentServiceHandle,
}

impl<'a> Services<'a> {
    fn of(context: &RuleContext<'a>) -> Result<Self, Unavailable> {
        let missing = |what: &str| {
            (
                NotEvaluatedReason::MissingService,
                format!("{what} service is not registered"),
            )
        };
        Ok(Self {
            sight: context
                .services
                .get::<SightServiceHandle>()
                .ok_or_else(|| missing("line-of-sight"))?,
            centres: context
                .services
                .get::<PlanSpanServiceHandle>()
                .ok_or_else(|| missing("plan-span"))?,
            extents: context
                .services
                .get::<VerticalExtentServiceHandle>()
                .ok_or_else(|| missing("vertical-extent"))?,
        })
    }
}

/// The objects a selector picks, and those it cannot decide.
struct Picked {
    matched: BTreeSet<ObjectId>,
    undecided: BTreeSet<ObjectId>,
}

impl Picked {
    fn of(context: &RuleContext<'_>, selector: &Selector) -> Self {
        let (matched, selection) = select_objects(context, selector);
        Self {
            matched: matched.iter().map(|object| object.id.clone()).collect(),
            undecided: selection
                .not_evaluated_outcomes()
                .iter()
                .filter_map(|outcome| outcome.object_id().cloned())
                .collect(),
        }
    }

    fn all(&self) -> impl Iterator<Item = &ObjectId> {
        self.matched.iter().chain(&self.undecided)
    }
}

/// Whether one target is in view.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seen {
    Surely,
    Maybe,
    No,
}

/// The targets of one component, sorted by what is known of them.
struct View<'o> {
    component: &'o Object,
    visible: Vec<ObjectId>,
    /// Targets that may be in view, and why they are not known to be.
    unknown: Vec<(ObjectId, String)>,
    /// Targets in range, known hidden.
    hidden: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

impl<'o> View<'o> {
    fn of(
        services: &Services<'_>,
        config: &Config<'_>,
        targets: &Picked,
        blockers: &Picked,
        component: &'o Object,
    ) -> Result<Self, Unavailable> {
        let own = &component.id;
        let centre = services.centres.measure_centre(own).map_err(|error| {
            (
                NotEvaluatedReason::BackendUnavailable,
                format!("the centre of {own} cannot be located: {error}"),
            )
        })?;
        if !centre.is_exact() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the centre of {own} is known only within {} m, so the eye is not a point",
                    centre.radius_metres()
                ),
            ));
        }
        let extent = services
            .extents
            .measure_vertical_extent(own)
            .map_err(|error| {
                (
                    NotEvaluatedReason::BackendUnavailable,
                    format!("the base of {own} cannot be measured: {error}"),
                )
            })?;
        let base = extent.bottom();
        if !base.is_exact() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the base of {own} is not known exactly, so the eye is not a point"),
            ));
        }
        let [x, y] = centre.point();
        let eye = [x, y, base.lower_metres() + config.eye_height];
        let mut view = Self {
            component,
            visible: Vec::new(),
            unknown: Vec::new(),
            hidden: Vec::new(),
            evidence: vec![centre.evidence().clone(), extent.evidence().clone()],
        };
        let all_blockers: Vec<ObjectId> = blockers.all().cloned().collect();
        let sure_blockers: Vec<ObjectId> = blockers.matched.iter().cloned().collect();
        for target in targets.all().filter(|target| *target != own) {
            let selected = targets.matched.contains(target);
            let except = |list: &[ObjectId]| -> Vec<ObjectId> {
                list.iter()
                    .filter(|blocker| *blocker != target && *blocker != own)
                    .cloned()
                    .collect()
            };
            let ask = |list: &[ObjectId]| {
                SightRequest::try_new(eye, target.clone(), except(list), Some(config.radius))
                    .and_then(|request| services.sight.assess_sight(&request))
            };
            let (seen, why) = match ask(&all_blockers) {
                Err(error) => (Seen::Maybe, describe(&error)),
                Ok(answer) => {
                    view.classify(&answer, blockers, config.radius, || ask(&sure_blockers))
                }
            };
            match (seen, selected) {
                (Seen::No, _) => {
                    if !why.is_empty() {
                        view.hidden.push(target.clone());
                    }
                }
                (Seen::Surely, true) => view.visible.push(target.clone()),
                (Seen::Surely, false) => view.unknown.push((
                    target.clone(),
                    "is in view, but whether it is a target is undecided".to_owned(),
                )),
                (Seen::Maybe, _) => view.unknown.push((target.clone(), why)),
            }
        }
        Ok(view)
    }

    /// What one answer shows, with a reason when the target is undecided
    /// or, for a target known not in view, `hidden` when it is in range.
    fn classify(
        &mut self,
        answer: &SightEvidence,
        blockers: &Picked,
        radius: f64,
        without_undecided: impl Fn() -> Result<SightEvidence, SightError>,
    ) -> (Seen, String) {
        let (near, far) = answer.distance_metres();
        let Some(outcome) = answer.outcome() else {
            return (Seen::No, String::new());
        };
        if near > radius {
            return (Seen::No, String::new());
        }
        let in_range = far <= radius;
        let straddles = || format!("lies between {near:.3} and {far:.3} m from the eye");
        match outcome {
            SightOutcome::Visible { .. } => {
                self.evidence.push(answer.evidence().clone());
                if in_range {
                    (Seen::Surely, String::new())
                } else {
                    (Seen::Maybe, format!("is in view but {}", straddles()))
                }
            }
            SightOutcome::Hidden { occluders }
                if occluders
                    .iter()
                    .all(|occluder| blockers.matched.contains(occluder)) =>
            {
                self.evidence.push(answer.evidence().clone());
                (Seen::No, "hidden".into())
            }
            SightOutcome::Hidden { .. } => match without_undecided() {
                Ok(again) if matches!(again.outcome(), Some(SightOutcome::Hidden { .. })) => {
                    self.evidence.push(again.evidence().clone());
                    (Seen::No, "hidden".into())
                }
                Ok(_) => (
                    Seen::Maybe,
                    "is hidden only by blockers whose selection is undecided".into(),
                ),
                Err(error) => (Seen::Maybe, describe(&error)),
            },
            SightOutcome::Undecided => (
                Seen::Maybe,
                "can be proven neither in view nor hidden (grazed, or covered only where \
                 blockers meet)"
                    .into(),
            ),
        }
    }

    fn judge(
        self,
        rule: &CompiledRule,
        config: &Config<'_>,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let id = &self.component.id;
        let sure = self.visible.len() as u64;
        let most = sure + self.unknown.len() as u64;
        let within = format!(
            "within {} m of the eye {} m above the base of {id}",
            config.radius, config.eye_height
        );
        let undecided = || {
            let mut message = format!("{} target(s) {within} are undecided:", self.unknown.len());
            for (target, why) in self.unknown.iter().take(3) {
                let _ = write!(message, " {target} {why};");
            }
            message.pop();
            message
        };
        match config.mode {
            Mode::AtLeast(minimum) => {
                if sure >= minimum {
                    return;
                }
                if most >= minimum {
                    evaluation.push_object_not_evaluated(
                        id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "{sure} target(s) are in view, {minimum} required; {}",
                            undecided()
                        ),
                    );
                    return;
                }
                let mut message =
                    format!("{sure} target(s) {within} are in view; required at least {minimum}");
                if !self.hidden.is_empty() {
                    let _ = write!(message, "; {} hidden", self.hidden.len());
                }
                let mut related = self.visible.clone();
                related.extend(self.hidden.iter().cloned());
                evaluation.push_finding(finding(rule, id, message, self.evidence, related));
            }
            Mode::None => {
                if sure > 0 {
                    let named = self
                        .visible
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    evaluation.push_finding(finding(
                        rule,
                        id,
                        format!("{sure} target(s) {within} are in view, none allowed: {named}"),
                        self.evidence,
                        self.visible,
                    ));
                } else if most > 0 {
                    evaluation.push_object_not_evaluated(
                        id.clone(),
                        NotEvaluatedReason::IncompleteEvidence,
                        undecided(),
                    );
                }
            }
        }
    }
}

fn describe(error: &SightError) -> String {
    format!("cannot be assessed: {error}")
}
