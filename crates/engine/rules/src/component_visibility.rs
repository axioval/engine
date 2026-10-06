//! Visibility of targets from an eye above each component: a reception desk
//! that must see the entrance doors, or a device that must be seen from
//! nowhere.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use axioval_engine::template::Template;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    PlanSpanServiceHandle, RuleCapability, RuleContext, SightError, SightEvidence, SightOutcome,
    SightRequest, SightServiceHandle, VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, Object, ObjectId};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::ViewMeasures;

use crate::support::Unavailable;

pub(crate) const NAME: &str = "component-visibility";

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

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for ComponentVisibility {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

pub(crate) struct Services<'a> {
    sight: &'a SightServiceHandle,
    centres: &'a PlanSpanServiceHandle,
    extents: &'a VerticalExtentServiceHandle,
}

impl<'a> Services<'a> {
    pub(crate) fn of(context: &RuleContext<'a>) -> Result<Self, Unavailable> {
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
pub(crate) struct Picked {
    pub(crate) matched: BTreeSet<ObjectId>,
    pub(crate) undecided: BTreeSet<ObjectId>,
}

impl Picked {
    #[cfg(feature = "parity-reference")]
    pub(crate) fn of(context: &RuleContext<'_>, selector: &axioval_ir::contract::Selector) -> Self {
        let (matched, selection) = crate::selection::select_objects(context, selector);
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
pub(crate) struct View {
    pub(crate) visible: Vec<ObjectId>,
    /// Targets that may be in view, and why they are not known to be.
    pub(crate) unknown: Vec<(ObjectId, String)>,
    /// Targets in range, known hidden.
    pub(crate) hidden: Vec<ObjectId>,
    pub(crate) evidence: Vec<Evidence>,
}

impl View {
    /// The view from the eye `eye_height` above the component's base over
    /// its centre, of the targets within `radius`.
    pub(crate) fn of(
        services: &Services<'_>,
        (eye_height, radius): (f64, f64),
        targets: &Picked,
        blockers: &Picked,
        component: &Object,
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
        let eye = [x, y, base.lower_metres() + eye_height];
        let mut view = Self {
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
                SightRequest::try_new(eye, target.clone(), except(list), Some(radius))
                    .and_then(|request| services.sight.assess_sight(&request))
            };
            let (seen, why) = match ask(&all_blockers) {
                Err(error) => (Seen::Maybe, describe(&error)),
                Ok(answer) => view.classify(&answer, blockers, radius, || ask(&sure_blockers)),
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
}

fn describe(error: &SightError) -> String {
    format!("cannot be assessed: {error}")
}
