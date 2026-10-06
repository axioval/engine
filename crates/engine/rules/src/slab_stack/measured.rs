//! The distance from a slab to the next slab up in its stack as a value,
//! measured exactly as `slab-stack-spacing` measures it, and the distance
//! its stack's consecutive slabs prevailingly share: the slabs named stack
//! by the overlap of their footprints, are ordered by their tops, and each
//! is measured against the next one up. Objects a selection cannot decide
//! may stack, so they may be the next one up, but are never measured from.

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Citation, MeasuredMemo, MeasuredProvider, Measurement, NotEvaluatedReason,
    PlanAreaServiceHandle, PropertyResolutionError, ProximityServiceHandle, RuleContext,
    VerticalExtentServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, SelectionIdentity};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use super::{Measure, Member, Stacks, components, extent_unavailable};
use crate::level_spacing::prevailing;
use crate::measured_kinds::{every_object_of_kinds, interval, refused};
use crate::support::{Unavailable, invalid};

/// Measures `stack_distance` and `stack_prevailing`.
pub(crate) struct StackMeasures;

const STACK_DISTANCE: &str = "stack_distance";
const STACK_PREVAILING: &str = "stack_prevailing";

/// The slabs a call names, as a memo keys the stacks over them.
#[derive(Hash, PartialEq, Eq)]
enum Slabs {
    /// Source kinds as written, the object measured among them.
    Kinds(String, ObjectId),
    /// A rule's selection, by identity.
    Selection(SelectionIdentity),
}

#[derive(Hash, PartialEq, Eq)]
struct StacksKey(Slabs, u64);

/// The next slab up of a slab and the evidence that they stack, none, or
/// why that cannot be decided.
type Next = Result<Option<(usize, Vec<Evidence>)>, Unavailable>;

/// The stacks over the slabs of one call, measured once per run.
struct Run {
    /// Why a slab's own extent cannot be measured.
    own: BTreeMap<ObjectId, Unavailable>,
    /// The first slab whose extent cannot be measured: the stacks every
    /// other slab may belong to are unknown.
    unknown: Option<ObjectId>,
    members: Vec<Member>,
    /// Each member's place among `members`.
    places: BTreeMap<ObjectId, usize>,
    /// Each slab measured from: the next slab up and the evidence that they
    /// stack, none, or why that cannot be decided.
    next: BTreeMap<ObjectId, Next>,
    /// The consecutive pairs `(lower, upper)` of each slab's stack, by slab.
    stacks: BTreeMap<ObjectId, Arc<Vec<(usize, usize)>>>,
}

/// The memo key of the slabs `call` names.
fn key(call: &MeasuredCall, object: &ObjectId) -> Result<Slabs, Unavailable> {
    match call.argument("slabs") {
        Some(MeasuredArgument::Objects(selection)) => {
            Ok(Slabs::Selection(SelectionIdentity(selection.clone())))
        }
        Some(argument) => Ok(Slabs::Kinds(format!("{argument:?}"), object.clone())),
        None => Err(invalid("`slabs` is required")),
    }
}

/// The slabs `call` names, each measured from or not.
fn slabs(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<(ObjectId, bool)>, Unavailable> {
    if let Some(MeasuredArgument::Objects(selection)) = call.argument("slabs") {
        return Ok(selection
            .matched
            .iter()
            .map(|slab| (slab.clone(), true))
            .chain(selection.undecided.iter().map(|slab| (slab.clone(), false)))
            .collect());
    }
    let mut kinds = every_object_of_kinds(context, call, "slabs")
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
    kinds.insert(object.clone());
    Ok(kinds.into_iter().map(|slab| (slab, true)).collect())
}

/// The stacks over `slabs`, measured as `slab-stack-spacing` measures them.
fn measure_stacks(
    context: &RuleContext<'_>,
    slabs: Vec<(ObjectId, bool)>,
    ratio: f64,
) -> Result<Run, Unavailable> {
    let (Some(extents), Some(areas)) = (
        context.services.get::<VerticalExtentServiceHandle>(),
        context.services.get::<PlanAreaServiceHandle>(),
    ) else {
        return Err((
            NotEvaluatedReason::MissingService,
            "slab-stack-spacing needs the vertical-extent and plan-area services".into(),
        ));
    };
    let mut stacks = Stacks {
        members: Vec::new(),
        areas,
        boxes: context.services.get::<ProximityServiceHandle>(),
        ratio,
        footprints: BTreeMap::new(),
        partners: BTreeMap::new(),
    };
    let mut own = BTreeMap::new();
    let mut unknown = None;
    for (slab, selected) in slabs {
        match extents.measure_vertical_extent(&slab) {
            Ok(extent) => stacks.members.push(Member {
                id: slab,
                extent,
                selected,
            }),
            Err(error) => {
                if selected {
                    own.insert(slab.clone(), extent_unavailable(&error));
                }
                unknown.get_or_insert(slab);
            }
        }
    }
    let mut next = BTreeMap::new();
    let mut pairs = Vec::new();
    if unknown.is_none() {
        for slab in 0..stacks.members.len() {
            if !stacks.members[slab].selected {
                continue;
            }
            let found = stacks.next_up(slab);
            if let Ok(Some((upper, evidence))) = &found {
                pairs.push((slab, *upper, evidence.clone()));
            }
            next.insert(stacks.members[slab].id.clone(), found);
        }
    }
    let mut stacked = BTreeMap::new();
    for stack in components(stacks.members.len(), &pairs) {
        let shared: Arc<Vec<(usize, usize)>> = Arc::new(
            stack
                .iter()
                .map(|index| (pairs[*index].0, pairs[*index].1))
                .collect(),
        );
        for index in &stack {
            stacked.insert(stacks.members[pairs[*index].0].id.clone(), shared.clone());
        }
    }
    let places = stacks
        .members
        .iter()
        .enumerate()
        .map(|(place, member)| (member.id.clone(), place))
        .collect();
    Ok(Run {
        own,
        unknown,
        places,
        members: stacks.members,
        next,
        stacks: stacked,
    })
}

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn distance(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let measure = match call.choice("measure") {
        Some("bottom_to_bottom") => Measure::BottomToBottom,
        Some("top_to_bottom") => Measure::TopToBottom,
        _ => Measure::TopToTop,
    };
    let ratio = match length(call, "ratio") {
        Some(ratio) if ratio > 0.0 && ratio <= 1.0 => ratio,
        _ => return Err(invalid("`ratio` must lie in (0, 1]")),
    };
    let run: Result<Arc<Run>, Unavailable> = MeasuredMemo::of(
        context.services,
        StacksKey(key(call, object)?, ratio.to_bits()),
        || measure_stacks(context, slabs(call, object, context)?, ratio).map(Arc::new),
    );
    let run = run?;
    if let Some(error) = run.own.get(object) {
        return Err(error.clone());
    }
    if let Some(slab) = &run.unknown {
        // Its elevation is unknown, so it could sit between any two.
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the vertical extent of {slab} could not be measured, so the stacks it may \
                 belong to are unknown"
            ),
        ));
    }
    let locator = format!("{}:{object}", call.name());
    let lower = *run
        .places
        .get(object)
        .ok_or_else(|| invalid(format!("{object} is no slab of the stacks")))?;
    let Some((upper, stacked)) = run
        .next
        .get(object)
        .cloned()
        .ok_or_else(|| invalid(format!("{object} is not measured from")))??
    else {
        return Ok((
            Measurement::Absent {
                locator: format!("{locator}: no slab stacks above it"),
            },
            Citation::default(),
        ));
    };
    let span = |(lower, upper): (usize, usize)| {
        measure.between(&run.members[lower].extent, &run.members[upper].extent)
    };
    let exact = |(lower, upper): (usize, usize), stacked: &[Evidence]| {
        stacked
            .iter()
            .chain([
                run.members[lower].extent.evidence(),
                run.members[upper].extent.evidence(),
            ])
            .all(|evidence| evidence.exact)
    };
    if call.name() == STACK_DISTANCE {
        let between = span((lower, upper));
        return Ok((
            interval(
                (between.lower, between.upper),
                Some(QuantityDimension::Length),
                exact((lower, upper), &stacked),
                format!("{locator}: to {}", run.members[upper].id),
            ),
            Citation {
                // The next slab up, which a finding relates.
                related: vec![run.members[upper].id.clone()],
                // What showed they stack, and both extents measured.
                evidence: stacked
                    .iter()
                    .chain([
                        run.members[lower].extent.evidence(),
                        run.members[upper].extent.evidence(),
                    ])
                    .cloned()
                    .collect(),
                notes: Vec::new(),
            },
        ));
    }
    // The prevailing distance among the stack's consecutive pairs.
    let tolerance = length(call, "tolerance").unwrap_or(1e-3);
    let stack = run.stacks.get(object).cloned().unwrap_or_default();
    if stack.len() < 2 {
        return Ok((
            Measurement::Absent {
                locator: format!("{locator}: its stack has fewer than two pairs"),
            },
            Citation::default(),
        ));
    }
    let distances: Vec<_> = stack.iter().map(|pair| span(*pair)).collect();
    let midpoints: Vec<f64> = distances
        .iter()
        .map(|distance| distance.midpoint())
        .collect();
    let Some(index) = prevailing(&midpoints, tolerance) else {
        return Ok((
            Measurement::Absent {
                locator: format!("{locator}: no distance prevails in its stack"),
            },
            Citation::default(),
        ));
    };
    let reference = distances[index];
    let (reference_lower, reference_upper) = stack[index];
    let reference_stacked = match run.next.get(&run.members[reference_lower].id) {
        Some(Ok(Some((_, stacked)))) => stacked.clone(),
        _ => Vec::new(),
    };
    Ok((
        interval(
            (reference.lower, reference.upper),
            Some(QuantityDimension::Length),
            exact((reference_lower, reference_upper), &reference_stacked),
            format!(
                "{locator}: from {} to {}",
                run.members[reference_lower].id, run.members[reference_upper].id
            ),
        ),
        Citation::default(),
    ))
}

impl MeasuredProvider for StackMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[STACK_DISTANCE, STACK_PREVAILING]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_cited(call, object, context)
            .map(|(measurement, _)| measurement)
    }

    /// A distance cites the next slab up.
    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        distance(call, object, context).map_err(refused(call.name(), object))
    }

    /// The stacks are measured once per run, whichever value reads them.
    fn memoizes(&self) -> bool {
        true
    }
}
