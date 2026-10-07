//! Exposed edges as measured members: what guards each edge the guard
//! service samples, measured under the searches and gaps the list states,
//! with the thresholds that vary by use (barrier height, fall height, how
//! low a climbable object may be) left to the expression or template
//! judging them.
//!
//! - `guard_edges`, a member list: each edge of the object, with the
//!   height its barriers reach along all of it (`guarded_height`), the
//!   share barriers reach along at all (`barrier_share`), the tallest such
//!   barrier (`tallest_barrier`, `tallest`, `tallest_top`), the tallest
//!   barrier along part of it (`partial_height`), the fall onto landings
//!   covering it (`landing_fall`), the landing nearest it (`nearest_gap`,
//!   `nearest_fall`, `nearest_width`, `nearest`) and the lowest object
//!   beside a barrier to climb (`climbable_height`, and, with
//!   `climb_height`, the first no taller, `climbable`);
//! - `guard_surfaces`: how many surfaces the one request measured, refused
//!   as `horizontal-guard` leaves its rule open.
//!
//! With `surfaces`, the surfaces are measured together in one request,
//! kept for the run, from which each surface's edges are listed; without
//! it, the object alone. Role selections (`barriers`, `landings`,
//! `climbables`) are kinds or a rule's selector, whose undecided objects
//! refuse the search. An edge is guarded by `horizontal-guard` exactly
//! when `guarded_height` reaches the barrier height and no
//! `climbable_height` defeats it, or, with less than half of it reached by
//! barriers (`barrier_share`), when `landing_fall` is within the fall
//! allowed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axioval_engine::{
    Citation, GuardCandidate, GuardEdge, GuardServiceHandle, MeasuredMember, MeasuredMemo,
    MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{
    EPSILON_M, REQUIRED_COVERAGE, admitted, barrier_height, coverage_gap, guard_search, may_climb,
    nearest_landing, reaching_barriers, tallest_reaching_barrier, unmeasured_reason, wide_enough,
};
use crate::measured_kinds::{interval, refused, resolution_error, selection};
use crate::support::Unavailable;

/// Measures the exposed edges of walking surfaces.
pub(crate) struct GuardMeasures;

const EDGES: &str = "guard_edges";
const SURFACES: &str = "guard_surfaces";

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => *value,
        _ => 0.0,
    }
}

/// The objects an argument names, as a memo keys them.
#[derive(Clone, Hash, PartialEq, Eq)]
enum Named {
    Object(ObjectId),
    Kinds(String),
    Selection(BTreeSet<ObjectId>, BTreeSet<ObjectId>),
}

fn named(call: &MeasuredCall, key: &str) -> Option<Named> {
    Some(match call.argument(key)? {
        // By the objects picked: the rule's selection read once for the
        // rule and again for its objects is one search.
        MeasuredArgument::Objects(picked) => {
            Named::Selection(picked.matched.clone(), picked.undecided.clone())
        }
        argument => Named::Kinds(format!("{argument:?}")),
    })
}

/// What one search measured, for every surface it names.
struct Measured {
    /// The edges of each surface, every candidate the search does not
    /// admit for its role removed.
    edges: BTreeMap<ObjectId, Vec<GuardEdge>>,
    evaluated: usize,
    exact: bool,
    locator: String,
}

/// The search `call` states for `object`, measured once per run for every
/// value and list reading it.
fn measured(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: Option<&ObjectId>,
) -> Result<Arc<Measured>, Unavailable> {
    #[derive(Hash, PartialEq, Eq)]
    struct Key(Named, [u64; 4], [Option<Named>; 3]);
    let gaps = [
        length(call, "barrier_gap"),
        length(call, "platform_gap"),
        length(call, "landing_gap"),
        length(call, "climb_distance"),
    ];
    let pick = |key: &str, leave_out: Option<&ObjectId>| {
        selection(context, call, key, leave_out).map_err(crate::selection::property_error)
    };
    // Without `surfaces`, the object alone, which no role kind names.
    let (surfaces, alone) = match (named(call, "surfaces"), object) {
        (Some(surfaces), _) => (surfaces, None),
        (None, Some(object)) => (Named::Object(object.clone()), Some(object)),
        (None, None) => {
            return Err((
                NotEvaluatedReason::InvalidDeclaration,
                "the project's surfaces are named by `surfaces`".to_owned(),
            ));
        }
    };
    let key = Key(
        surfaces,
        gaps.map(f64::to_bits),
        ["barriers", "landings", "climbables"].map(|role| named(call, role)),
    );
    MeasuredMemo::of(context.services, key, || {
        let service = context
            .services
            .get::<GuardServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "guard service is not registered".to_owned(),
                )
            })?;
        let search = guard_search(gaps).ok_or_else(|| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                "horizontal-guard thresholds do not define a usable search".to_owned(),
            )
        })?;
        let surfaces: Vec<ObjectId> = match alone {
            Some(object) => vec![object.clone()],
            None => pick("surfaces", None)?
                .map(|picked| picked.matched.into_iter().collect())
                .unwrap_or_default(),
        };
        let mut search = search.with_surfaces(surfaces);
        // An object a role selection cannot decide might be the rail that
        // guards an edge, or the cupboard that must not.
        let decided = |picked: Option<MeasuredSelection>| match picked {
            Some(picked) if !picked.undecided.is_empty() => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "horizontal-guard: `{}` cannot be decided for {} object(s)",
                    picked.parameter,
                    picked.undecided.len()
                ),
            )),
            picked => Ok(picked.map(|picked| picked.matched.into_iter().collect::<Vec<_>>())),
        };
        if let Some(barriers) = decided(pick("barriers", alone)?)? {
            search = search.with_barrier_candidates(barriers);
        }
        if let Some(landings) = decided(pick("landings", alone)?)? {
            search = search.with_landing_candidates(landings);
        }
        if let Some(climbables) = decided(pick("climbables", alone)?)? {
            search = search.with_climbable_candidates(climbables);
        }
        let evidence = service
            .measure_guard_edges(search.clone())
            .map_err(|error| (unmeasured_reason(error), error.to_string()))?;
        let mut edges: BTreeMap<ObjectId, Vec<GuardEdge>> = BTreeMap::new();
        for edge in evidence.edges() {
            edges
                .entry(edge.surface().clone())
                .or_default()
                .push(admitted(edge, &search));
        }
        Ok(Arc::new(Measured {
            edges,
            evaluated: evidence.evaluated_surfaces(),
            exact: evidence.evidence().exact,
            locator: evidence.evidence().locator.clone(),
        }))
    })
}

/// The least (or greatest) `key` at which the candidates within `gap`
/// cover the whole edge: candidates are taken in order of `key` until
/// their union covers it. `GuardEdge::covered_fraction` leaves out the
/// candidates beyond `gap`, so coverage only grows at a level one within
/// it holds, and only such a level is answered.
fn covering(
    candidates: &[GuardCandidate],
    gap: f64,
    key: impl Fn(&GuardCandidate) -> f64,
    descending: bool,
) -> Option<f64> {
    let mut sorted: Vec<&GuardCandidate> = candidates.iter().collect();
    sorted.sort_by(|a, b| {
        let order = key(a).total_cmp(&key(b));
        if descending { order.reverse() } else { order }
    });
    let mut taken: Vec<GuardCandidate> = Vec::new();
    let mut index = 0;
    while index < sorted.len() {
        let level = key(sorted[index]);
        // Every candidate at this level is taken together.
        while index < sorted.len() && key(sorted[index]).total_cmp(&level).is_eq() {
            taken.push(sorted[index].clone());
            index += 1;
        }
        if GuardEdge::covered_fraction(&taken, gap) >= REQUIRED_COVERAGE {
            return Some(level);
        }
    }
    None
}

fn number(
    value: Option<f64>,
    dimension: Option<QuantityDimension>,
    locator: String,
) -> MemberValue {
    MemberValue::Measured(match value {
        Some(value) => Measurement::Value {
            lower: value,
            upper: value,
            dimension,
            locator,
        },
        None => Measurement::Absent { locator },
    })
}

fn objects(found: Option<&ObjectId>) -> MemberValue {
    MemberValue::Objects {
        objects: found.into_iter().cloned().collect(),
    }
}

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

/// What guards one edge, under the searches and gaps `call` states.
#[allow(clippy::too_many_lines)]
fn edge_member(
    exact: bool,
    call: &MeasuredCall,
    edge: &GuardEdge,
    at: &dyn Fn(&str) -> String,
) -> MeasuredMember {
    // Every filter is the capability's own, under the gaps the list states.
    let (barrier_gap, platform_gap) = (length(call, "barrier_gap"), length(call, "platform_gap"));
    let gap = coverage_gap(barrier_gap, platform_gap);
    let from_curb = call.choice("measure_from") == Some("curb")
        || matches!(
            call.argument("from_curb"),
            Some(MeasuredArgument::Truth(true))
        );
    let height = |barrier: &GuardCandidate| barrier_height(barrier, from_curb);
    let (landing_width, climb_side) = (length(call, "landing_width"), length(call, "climb_side"));
    let climb_distance = length(call, "climb_distance");
    let reaching = reaching_barriers(edge, platform_gap);
    let tallest = tallest_reaching_barrier(edge, platform_gap, from_curb);
    let share = GuardEdge::covered_fraction(&reaching, gap);
    // Barriers within the gap along part of the edge at all.
    let partial = edge
        .barriers()
        .iter()
        .filter(|barrier| {
            let [start, end] = barrier.edge_interval();
            barrier.horizontal_gap_metres() <= gap && end > start
        })
        .map(height)
        .reduce(f64::max);
    let wide: Vec<GuardCandidate> = edge
        .landings()
        .iter()
        .filter(|landing| wide_enough(landing, landing_width))
        .cloned()
        .collect();
    let nearest = nearest_landing(edge);
    let climbing = || {
        edge.climbables()
            .iter()
            .filter(|climbable| may_climb(climbable, climb_distance, climb_side))
    };
    let climbable = climbing()
        .map(axioval_engine::ClimbableCandidate::top_offset_metres)
        .reduce(f64::min);
    // The first no taller than `climb_height`, as the capability names it.
    let defeating = match call.argument("climb_height") {
        Some(MeasuredArgument::Length(most) | MeasuredArgument::Number(most)) => climbing()
            .find(|climbable| climbable.top_offset_metres() <= most + EPSILON_M)
            .map(|climbable| climbable.element().clone()),
        _ => None,
    };
    let guarded = covering(edge.barriers(), gap, height, true);
    let fall = covering(
        &wide,
        length(call, "landing_gap"),
        |landing| -landing.top_offset_metres(),
        false,
    );
    let curb_top = tallest
        .as_ref()
        .filter(|barrier| from_curb && barrier.curb_top_offset_metres().is_some())
        .map(GuardCandidate::top_offset_metres);
    MeasuredMember {
        certain: true,
        exact,
        fields: BTreeMap::from([
            (
                "guarded_height",
                number(guarded, LENGTH, at("guarded_height")),
            ),
            (
                "tallest_barrier",
                number(tallest.as_ref().map(height), LENGTH, at("tallest_barrier")),
            ),
            (
                "tallest",
                objects(tallest.as_ref().map(GuardCandidate::element)),
            ),
            ("tallest_top", number(curb_top, LENGTH, at("tallest_top"))),
            (
                "partial_height",
                number(partial, LENGTH, at("partial_height")),
            ),
            (
                "barrier_share",
                number(Some(share), None, at("barrier_share")),
            ),
            ("landing_fall", number(fall, LENGTH, at("landing_fall"))),
            (
                "nearest_gap",
                number(
                    nearest.as_ref().map(GuardCandidate::horizontal_gap_metres),
                    LENGTH,
                    at("nearest_gap"),
                ),
            ),
            (
                "nearest_fall",
                number(
                    nearest.as_ref().map(|landing| -landing.top_offset_metres()),
                    LENGTH,
                    at("nearest_fall"),
                ),
            ),
            (
                "nearest_width",
                number(
                    nearest.as_ref().map(GuardCandidate::landing_width_metres),
                    LENGTH,
                    at("nearest_width"),
                ),
            ),
            (
                "nearest",
                objects(nearest.as_ref().map(GuardCandidate::element)),
            ),
            (
                "climbable_height",
                number(climbable, LENGTH, at("climbable_height")),
            ),
            ("climbable", objects(defeating.as_ref())),
        ]),
        evidence: Vec::new(),
    }
}

/// The edges of `object` the search measured.
fn edges(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    object: &ObjectId,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let measured = measured(context, call, Some(object))?;
    let edges = measured
        .edges
        .get(object)
        .filter(|edges| !edges.is_empty())
        .ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                "no edge was measured for this walking surface; it has no measurable body"
                    .to_owned(),
            )
        })?;
    let total = edges.len();
    let name = call.name();
    Ok(edges
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            edge_member(measured.exact, call, edge, &|field: &str| {
                format!("{name}:{object}#{}/{total}:{field}", index + 1)
            })
        })
        .collect())
}

impl MeasuredProvider for GuardMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[SURFACES]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[EDGES]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let _ = (call, object, context);
        Err(PropertyResolutionError::InvalidRequest)
    }

    /// How many surfaces the one request measured, of the project.
    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let measured = measured(context, call, None).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of the project: {why}", call.name())))
        })?;
        #[allow(clippy::cast_precision_loss)]
        let count = measured.evaluated as f64;
        Ok((
            interval(
                (count, count),
                None,
                measured.exact,
                measured.locator.clone(),
            ),
            Citation::default(),
        ))
    }

    /// One request per search for the run, whichever value or list reads it.
    fn memoizes(&self) -> bool {
        true
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        edges(context, call, object).map_err(refused(call.name(), object))
    }
}
