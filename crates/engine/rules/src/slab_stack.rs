//! The spacing of slabs stacked above one another, from their surfaces.
//!
//! `level-spacing` reads storey elevations; this reads the measured bottom
//! and top of each slab. Two slabs belong to one stack when their plan
//! footprints overlap by at least a declared share, and each slab is checked
//! against the next one up in its stack.
//!
//! Every measurement is an interval: a tessellated slab's elevations and a
//! tessellated footprint are approximate. A verdict needs the whole interval
//! on one side of a bound, and anything the intervals leave open -- whether
//! two slabs stack, which of two slabs is the next one up, whether a distance
//! meets a bound -- is not evaluated, never passed.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ElevationInterval, NotEvaluatedReason, ParameterDescriptor,
    PlanArea, PlanAreaError, PlanAreaServiceHandle, ProximityServiceHandle, RuleCapability,
    RuleContext, VerticalExtent, VerticalExtentError,
};
use axioval_ir::{Evidence, ObjectId};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::StackMeasures;

use crate::support::Unavailable;

/// Checks the distances between consecutive slabs of each stack.
///
/// Two selected slabs stack when the overlap of their footprints is at least
/// `minimum_overlap_ratio` of the smaller footprint. Stacked slabs are
/// ordered by the elevation of their tops, and each slab is checked against
/// the next slab up that it stacks with:
///
/// - top to top: the rise from this slab's top to the next one's top;
/// - bottom to bottom: the rise from bottom to bottom;
/// - top to bottom: the clear gap from this slab's top to the next one's
///   underside, negative when they overlap vertically.
///
/// Each distance may be bounded by `<measure>_minimum` and
/// `<measure>_maximum`, inclusive. `consistent` names the measures that must
/// be equal, within `tolerance` (1 mm by default), across every pair of one
/// stack: slabs connected through consecutive pairs. The reference is the
/// prevailing distance, as in `level-spacing`.
///
/// A slab with no stack partner above it, including the top of a stack and
/// a slab standing alone, has nothing to check.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `stack_distance` of each slab to the next one up in its stack within
/// each measure's bounds, and against `stack_prevailing`, the distance its
/// stack's consecutive slabs prevailingly share, where the measure must be
/// consistent.
pub struct SlabStackSpacing;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SlabStackSpacing {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Measure {
    TopToTop,
    BottomToBottom,
    TopToBottom,
}

impl Measure {
    /// The distance from `lower` up to `upper`, as an interval.
    fn between(self, lower: &VerticalExtent, upper: &VerticalExtent) -> Interval {
        let (from, to) = match self {
            Self::TopToTop => (lower.top(), upper.top()),
            Self::BottomToBottom => (lower.bottom(), upper.bottom()),
            Self::TopToBottom => (lower.top(), upper.bottom()),
        };
        Interval {
            lower: to.lower_metres() - from.upper_metres(),
            upper: to.upper_metres() - from.lower_metres(),
        }
    }
}

#[derive(Clone, Copy)]
struct Interval {
    lower: f64,
    upper: f64,
}

impl Interval {
    fn midpoint(self) -> f64 {
        f64::midpoint(self.lower, self.upper)
    }
}

fn extent_unavailable(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

fn area_unavailable(error: &PlanAreaError) -> Unavailable {
    let reason = match error {
        PlanAreaError::UnknownObject(_) | PlanAreaError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        PlanAreaError::InvalidMeasurement | PlanAreaError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

struct Member {
    id: ObjectId,
    extent: VerticalExtent,
    /// `false` for an object whose selection is undecided: it may be a slab,
    /// so it may be the next one up, but it is never checked itself.
    selected: bool,
}

/// Whether two objects stack, as far as their footprints tell.
#[derive(Clone)]
enum Partnership {
    Stacked(Vec<Evidence>),
    Apart,
    Unknown(Unavailable),
}

/// Whether `upper` surely comes after `lower` in stack order: its top is
/// above `lower`'s top whatever the intervals hold, or both tops are the
/// same exact elevation and identity breaks the tie.
fn above(lower: &Member, upper: &Member) -> bool {
    let (from, to) = (lower.extent.top(), upper.extent.top());
    #[allow(clippy::float_cmp)]
    let tied = from.is_exact() && to.is_exact() && from.lower_metres() == to.lower_metres();
    to.lower_metres() > from.upper_metres() || (tied && upper.id > lower.id)
}

fn midpoint(elevation: ElevationInterval) -> f64 {
    f64::midpoint(elevation.lower_metres(), elevation.upper_metres())
}

struct Stacks<'a> {
    members: Vec<Member>,
    areas: &'a PlanAreaServiceHandle,
    /// Optional broad phase: enclosing boxes prove disjoint footprints apart
    /// without measuring their overlap.
    boxes: Option<&'a ProximityServiceHandle>,
    ratio: f64,
    footprints: BTreeMap<usize, Result<PlanArea, Unavailable>>,
    partners: BTreeMap<(usize, usize), Partnership>,
}

impl Stacks<'_> {
    fn footprint(&mut self, index: usize) -> Result<PlanArea, Unavailable> {
        let areas = self.areas;
        let members = &self.members;
        self.footprints
            .entry(index)
            .or_insert_with(|| {
                areas
                    .measure_footprint(&members[index].id)
                    .map_err(|error| area_unavailable(&error))
            })
            .clone()
    }

    /// Whether the enclosing plan boxes of two objects share no area.
    fn plan_disjoint(&self, first: usize, second: usize) -> bool {
        let Some(boxes) = self.boxes else {
            return false;
        };
        let plan = |index: usize| {
            let bounds = boxes.bounds(&self.members[index].id).ok()?;
            (bounds.object() == &self.members[index].id).then(|| bounds.enclosing())
        };
        let (Some(a), Some(b)) = (plan(first), plan(second)) else {
            return false;
        };
        (0..2).any(|axis| a.max()[axis].min(b.max()[axis]) <= a.min()[axis].max(b.min()[axis]))
    }

    fn partnership(&mut self, first: usize, second: usize) -> Partnership {
        let key = (first.min(second), first.max(second));
        if let Some(known) = self.partners.get(&key) {
            return known.clone();
        }
        let found = self.measure(key.0, key.1);
        self.partners.insert(key, found.clone());
        found
    }

    fn measure(&mut self, first: usize, second: usize) -> Partnership {
        if self.plan_disjoint(first, second) {
            return Partnership::Apart;
        }
        let measured = (|| {
            let a = self.footprint(first)?;
            let b = self.footprint(second)?;
            let overlap = self
                .areas
                .measure_plan_overlap(&self.members[first].id, &self.members[second].id)
                .map_err(|error| area_unavailable(&error))?;
            let smaller_upper = a.upper_square_metres().min(b.upper_square_metres());
            if smaller_upper <= 0.0 {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} or {} has no plan footprint to stack by",
                        self.members[first].id, self.members[second].id
                    ),
                ));
            }
            let smaller_lower = a.lower_square_metres().min(b.lower_square_metres());
            let lower = overlap.lower_square_metres() / smaller_upper;
            let upper = if smaller_lower > 0.0 {
                overlap.upper_square_metres() / smaller_lower
            } else {
                f64::INFINITY
            };
            let evidence = vec![
                a.evidence().clone(),
                b.evidence().clone(),
                overlap.evidence().clone(),
            ];
            Ok((lower, upper, evidence))
        })();
        let (lower, upper, evidence) = match measured {
            Ok(measured) => measured,
            Err(unavailable) => return Partnership::Unknown(unavailable),
        };
        let (a, b) = (&self.members[first], &self.members[second]);
        if upper < self.ratio {
            Partnership::Apart
        } else if lower < self.ratio {
            Partnership::Unknown((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether {} and {} stack cannot be decided: their plan overlap ratio \
                     straddles {}",
                    a.id, b.id, self.ratio
                ),
            ))
        } else if !(a.selected && b.selected) {
            let undecided = if a.selected { &b.id } else { &a.id };
            Partnership::Unknown((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{undecided} stacks here but whether it is selected is undecided"),
            ))
        } else {
            Partnership::Stacked(evidence)
        }
    }

    fn next_up(&mut self, slab: usize) -> Result<Option<(usize, Vec<Evidence>)>, Unavailable> {
        // Candidates that may stack with `slab` and are not surely below it.
        let mut higher = Vec::new();
        for other in 0..self.members.len() {
            if other == slab || above(&self.members[other], &self.members[slab]) {
                continue;
            }
            let partnership = self.partnership(slab, other);
            if matches!(partnership, Partnership::Apart) {
                continue;
            }
            if !above(&self.members[slab], &self.members[other]) {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} may stack with {} but which of their tops is higher cannot be \
                         decided",
                        self.members[slab].id, self.members[other].id
                    ),
                ));
            }
            higher.push((other, partnership));
        }
        let members = &self.members;
        let Some((position, _)) = higher.iter().enumerate().min_by(|(_, left), (_, right)| {
            let (left, right) = (&members[left.0], &members[right.0]);
            midpoint(left.extent.top())
                .total_cmp(&midpoint(right.extent.top()))
                .then_with(|| left.id.cmp(&right.id))
        }) else {
            return Ok(None);
        };
        let (next, partnership) = higher.swap_remove(position);
        if let Some((other, _)) = higher
            .iter()
            .find(|(other, _)| !above(&members[next], &members[*other]))
        {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the next slab up may be {} or {}: their tops cannot be ordered",
                    members[next].id, members[*other].id
                ),
            ));
        }
        match partnership {
            Partnership::Stacked(evidence) => Ok(Some((next, evidence))),
            Partnership::Unknown((reason, message)) => Err((
                reason,
                format!("the next slab up cannot be identified: {message}"),
            )),
            Partnership::Apart => unreachable!("slabs apart were skipped"),
        }
    }
}

/// A consecutive pair and what was measured between it.
fn root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Pair indices grouped by stack: slabs connected through consecutive pairs.
fn components(count: usize, pairs: &[(usize, usize, Vec<Evidence>)]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..count).collect();
    for (lower, upper, _) in pairs {
        let (a, b) = (root(&mut parent, *lower), root(&mut parent, *upper));
        parent[a.max(b)] = a.min(b);
    }
    let mut stacks: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (lower, _, _)) in pairs.iter().enumerate() {
        stacks
            .entry(root(&mut parent, *lower))
            .or_default()
            .push(index);
    }
    stacks.into_values().collect()
}
