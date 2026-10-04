//! Exposed edges as measured members: what guards each edge the guard
//! service samples, measured under the searches and gaps the list states,
//! with the thresholds that vary by use (barrier height, fall height, how
//! low a climbable object may be) left to the expression.
//!
//! An edge is guarded by `horizontal-guard` exactly when
//! `guarded_height` reaches the barrier height and no `climbable_height`
//! defeats it, or, with less than half of it reached by barriers
//! (`barrier_share`), when `landing_fall` is within the fall allowed.

use std::collections::BTreeMap;

use axioval_engine::{
    GuardCandidate, GuardEdge, GuardSearch, GuardServiceHandle, MeasuredMember, MeasuredProvider,
    Measurement, MemberValue, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{REQUIRED_COVERAGE, unmeasured_reason};

/// Measures the exposed edges of walking surfaces.
pub(crate) struct GuardMeasures;

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

/// The least (or greatest) `key` at which the candidates `key` admits
/// cover the whole edge: candidates are taken in order of `key` until
/// their union covers it.
fn covering(
    candidates: &[GuardCandidate],
    gap: f64,
    key: impl Fn(&GuardCandidate) -> f64,
    descending: bool,
) -> Option<f64> {
    let mut sorted: Vec<&GuardCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.horizontal_gap_metres() <= gap)
        .collect();
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

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

/// What guards one edge, under the searches and gaps `call` states.
fn edge_member(
    exact: bool,
    call: &MeasuredCall,
    edge: &GuardEdge,
    at: &dyn Fn(&str) -> String,
) -> MeasuredMember {
    let (barrier_gap, platform_gap) = (length(call, "barrier_gap"), length(call, "platform_gap"));
    let from_curb = call.choice("measure_from") == Some("curb");
    let height = |barrier: &GuardCandidate| match (from_curb, barrier.curb_top_offset_metres()) {
        (true, Some(curb)) => barrier.top_offset_metres() - curb,
        _ => barrier.top_offset_metres(),
    };
    let (landing_width, climb_side) = (length(call, "landing_width"), length(call, "climb_side"));
    let climb_distance = length(call, "climb_distance");
    let reaching: Vec<GuardCandidate> = edge
        .barriers()
        .iter()
        .filter(|barrier| barrier.horizontal_gap_metres() <= platform_gap + super::EPSILON_M)
        .cloned()
        .collect();
    let tallest = reaching.iter().map(height).reduce(f64::max);
    let share = GuardEdge::covered_fraction(&reaching, barrier_gap.max(platform_gap));
    let wide: Vec<GuardCandidate> = edge
        .landings()
        .iter()
        .filter(|landing| landing.landing_width_metres() + super::EPSILON_M >= landing_width)
        .cloned()
        .collect();
    let climbable = edge
        .climbables()
        .iter()
        .filter(|climbable| {
            climbable.distance_to_barrier_metres() <= climb_distance + super::EPSILON_M
                && climbable.minimum_side_length_metres() + super::EPSILON_M >= climb_side
        })
        .map(axioval_engine::ClimbableCandidate::top_offset_metres)
        .reduce(f64::min);
    let guarded = covering(edge.barriers(), barrier_gap.max(platform_gap), height, true);
    let fall = covering(
        &wide,
        length(call, "landing_gap"),
        |landing| -landing.top_offset_metres(),
        false,
    );
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
                number(tallest, LENGTH, at("tallest_barrier")),
            ),
            (
                "barrier_share",
                number(Some(share), None, at("barrier_share")),
            ),
            ("landing_fall", number(fall, LENGTH, at("landing_fall"))),
            (
                "climbable_height",
                number(climbable, LENGTH, at("climbable_height")),
            ),
        ]),
    }
}

impl GuardMeasures {
    /// The objects of the kinds `key` names, or `None` when it names none.
    fn role(
        context: &RuleContext<'_>,
        call: &MeasuredCall,
        key: &str,
        object: &ObjectId,
    ) -> Result<Option<Vec<ObjectId>>, PropertyResolutionError> {
        if call.argument(key).is_none() {
            return Ok(None);
        }
        let found = crate::measured_kinds::objects_of_kinds(context, call, key, object)?;
        Ok(Some(found.into_iter().collect()))
    }
}

impl MeasuredProvider for GuardMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["guard_edges"]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let name = call.name();
        let service = context
            .services
            .get::<GuardServiceHandle>()
            .ok_or_else(|| {
                PropertyResolutionError::MissingService("guard service is not registered".into())
            })?;
        let (barrier_gap, platform_gap) =
            (length(call, "barrier_gap"), length(call, "platform_gap"));
        let landing_gap = length(call, "landing_gap");
        let climb_distance = length(call, "climb_distance");
        let radius = barrier_gap
            .max(platform_gap)
            .max(landing_gap)
            .max(climb_distance);
        let spacing = (0.5 * barrier_gap.min(platform_gap)).max(0.1);
        let refused = |why: String| {
            crate::measured_kinds::resolution_error((
                axioval_engine::NotEvaluatedReason::InvalidDeclaration,
                format!("`{name}` of {object}: {why}"),
            ))
        };
        let mut search = GuardSearch::try_new(radius, spacing)
            .map_err(|_| refused("the gaps define no usable search".into()))?
            .with_surfaces(vec![object.clone()]);
        if let Some(barriers) = Self::role(context, call, "barriers", object)? {
            search = search.with_barrier_candidates(barriers);
        }
        if let Some(landings) = Self::role(context, call, "landings", object)? {
            search = search.with_landing_candidates(landings);
        }
        if let Some(climbables) = Self::role(context, call, "climbables", object)? {
            search = search.with_climbable_candidates(climbables);
        }
        let measured = service
            .measure_guard_edges(search.clone())
            .map_err(|error| {
                crate::measured_kinds::resolution_error((
                    unmeasured_reason(error),
                    format!("`{name}` of {object}: {error}"),
                ))
            })?;
        let edges: Vec<GuardEdge> = measured
            .edges()
            .iter()
            .filter(|edge| edge.surface() == object)
            .map(|edge| super::admitted(edge, &search))
            .collect();
        if edges.is_empty() {
            return Err(PropertyResolutionError::Incomplete(format!(
                "`{name}` of {object}: no edge was measured; it has no measurable body"
            )));
        }
        let total = edges.len();
        Ok(edges
            .iter()
            .enumerate()
            .map(|(index, edge)| {
                edge_member(measured.evidence().exact, call, edge, &|field: &str| {
                    format!("{name}:{object}#{}/{total}:{field}", index + 1)
                })
            })
            .collect())
    }
}
