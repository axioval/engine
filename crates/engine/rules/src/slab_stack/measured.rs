//! The distance from a slab to the next slab up in its stack as a value,
//! measured exactly as `slab-stack-spacing` measures it: the slabs of the
//! kinds named stack by the overlap of their footprints, are ordered by
//! their tops, and each is measured against the next one up.

use std::collections::BTreeMap;

use axioval_engine::{
    MeasuredProvider, Measurement, NotEvaluatedReason, PlanAreaServiceHandle,
    PropertyResolutionError, ProximityServiceHandle, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Measure, Member, Stacks, extent_unavailable};
use crate::measured_kinds::{every_object_of_kinds, interval, refused};
use crate::support::Unavailable;

/// Measures `stack_distance`.
pub(crate) struct StackMeasures;

const STACK_DISTANCE: &str = "stack_distance";

fn distance(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, Unavailable> {
    let measure = match call.choice("measure") {
        Some("bottom_to_bottom") => Measure::BottomToBottom,
        Some("top_to_bottom") => Measure::TopToBottom,
        _ => Measure::TopToTop,
    };
    let ratio = match call.argument("ratio") {
        Some(MeasuredArgument::Length(ratio)) if *ratio > 0.0 && *ratio <= 1.0 => *ratio,
        _ => return Err(crate::support::invalid("`ratio` must lie in (0, 1]")),
    };
    let (Some(extents), Some(areas)) = (
        context.services.get::<VerticalExtentServiceHandle>(),
        context.services.get::<PlanAreaServiceHandle>(),
    ) else {
        return Err((
            NotEvaluatedReason::MissingService,
            "slab-stack-spacing needs the vertical-extent and plan-area services".into(),
        ));
    };
    let mut slabs = every_object_of_kinds(context, call, "slabs")
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
    slabs.insert(object.clone());
    let mut stacks = Stacks {
        members: Vec::new(),
        areas,
        boxes: context.services.get::<ProximityServiceHandle>(),
        ratio,
        footprints: BTreeMap::new(),
        partners: BTreeMap::new(),
    };
    let mut unknown = None;
    for slab in slabs {
        match extents.measure_vertical_extent(&slab) {
            Ok(extent) => stacks.members.push(Member {
                id: slab,
                extent,
                selected: true,
            }),
            Err(error) if slab == *object => return Err(extent_unavailable(&error)),
            Err(_) => {
                unknown.get_or_insert(slab);
            }
        }
    }
    if let Some(slab) = unknown {
        // Its elevation is unknown, so it could sit between any two.
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the vertical extent of {slab} could not be measured, so the stacks it may \
                 belong to are unknown"
            ),
        ));
    }
    let index = stacks
        .members
        .iter()
        .position(|member| member.id == *object)
        .expect("the slab measured is a member");
    let locator = format!("{STACK_DISTANCE}:{object}");
    let Some((next, stacked)) = stacks.next_up(index)? else {
        return Ok(Measurement::Absent {
            locator: format!("{locator}: no slab stacks above it"),
        });
    };
    let (lower, upper) = (&stacks.members[index], &stacks.members[next]);
    let between = measure.between(&lower.extent, &upper.extent);
    let exact = stacked
        .iter()
        .chain([lower.extent.evidence(), upper.extent.evidence()])
        .all(|evidence| evidence.exact);
    Ok(interval(
        (between.lower, between.upper),
        Some(QuantityDimension::Length),
        exact,
        format!("{locator}: to {}", upper.id),
    ))
}

impl MeasuredProvider for StackMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[STACK_DISTANCE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        distance(call, object, context).map_err(refused(call.name(), object))
    }
}
