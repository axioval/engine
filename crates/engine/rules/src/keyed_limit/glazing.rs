//! Each leaf's glazing, measured from the door's body: the measured member
//! list `leaf_glazing`, and what `keyed-limit` reads for a glazing ratio
//! per leaf (`glazing_ratio_per` `leaf`).
//!
//! The leaves are the ones the source states (the object-frame service);
//! the leaf-glazing service measures each one's glazed area and area from
//! the body. A leaf the service cannot decide is undecided, never zero. A
//! door stating no leaves, or without a body, states nothing to measure:
//! the capability may then take its type's default, the list refuses.

use std::collections::BTreeMap;

use axioval_engine::{
    DoorLeavesError, LeafGlazing, LeafGlazingError, LeafGlazingServiceHandle, MeasuredMember,
    MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason, ObjectFrameServiceHandle,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use crate::door_swing;
use crate::measured_kinds::{interval, refused};
use crate::support::Unavailable;

const LEAF_GLAZING: &str = "leaf_glazing";

/// Measures each leaf's glazing.
pub(crate) struct GlazingMeasures;

/// One leaf's glazing, as measured.
pub(super) struct Leaf {
    /// The leaf as a message names it: `leaf 1 of 2`.
    pub(super) name: String,
    pub(super) glazing: LeafGlazing,
}

impl Leaf {
    /// The glazed share of the leaf, between 0 and 1; why it is undecided
    /// otherwise.
    pub(super) fn ratio(&self) -> Result<(f64, f64), String> {
        match &self.glazing {
            LeafGlazing::Undecided(why) => {
                Err(format!("the glazing of {} is undecided: {why}", self.name))
            }
            measured @ LeafGlazing::Measured { .. } => {
                let (lower, upper) = measured
                    .ratio()
                    .ok_or_else(|| format!("the glazing of {} is undecided", self.name))?;
                Ok((lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0)))
            }
        }
    }
}

/// Every leaf's glazing, with the evidence of the leaves and the
/// measurement.
pub(super) struct Leaves {
    pub(super) each: Vec<Leaf>,
    pub(super) exact: bool,
    pub(super) evidence: Vec<Evidence>,
}

/// The glazing of `door`'s leaves; `Ok(Err(why))` where the door states
/// nothing to measure (no leaves, or no body), so a default may stand in.
/// A missing service, leaves or a body that cannot be read, refuse.
pub(super) fn measured(
    context: &RuleContext<'_>,
    door: &ObjectId,
) -> Result<Result<Leaves, String>, Unavailable> {
    let Some(frames) = context.services.get::<ObjectFrameServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "the object-frame service is not registered, so the door's leaves are unknown".into(),
        ));
    };
    let Some(glazing) = context.services.get::<LeafGlazingServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "the leaf-glazing service is not registered, so the leaves' glazing cannot be \
             measured"
                .into(),
        ));
    };
    let leaves = match frames.leaves(door) {
        Ok(leaves) => leaves,
        Err(DoorLeavesError::NotStated(why)) => {
            return Ok(Err(format!("the door states no leaves ({why})")));
        }
        Err(error) => {
            return Err((
                door_swing::reason(&error),
                format!("the door's leaves are unknown: {error}"),
            ));
        }
    };
    let measured = match glazing.measure_leaf_glazing(&leaves) {
        Ok(measured) => measured,
        Err(LeafGlazingError::NoBody(_)) => {
            return Ok(Err(
                "the door has no body to measure its glazing from".into()
            ));
        }
        Err(error) => {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the leaves' glazing cannot be measured: {error}"),
            ));
        }
    };
    let count = leaves.leaves().len();
    Ok(Ok(Leaves {
        each: measured
            .leaves()
            .iter()
            .enumerate()
            .map(|(index, glazing)| Leaf {
                name: format!("leaf {} of {count}", index + 1),
                glazing: glazing.clone(),
            })
            .collect(),
        exact: measured.is_exact() && leaves.evidence().exact,
        evidence: vec![leaves.evidence().clone(), measured.evidence().clone()],
    }))
}

/// One item per leaf.
fn members(
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
    let leaves =
        measured(context, object)?.map_err(|why| (NotEvaluatedReason::IncompleteEvidence, why))?;
    let exact = leaves.exact;
    let items = leaves
        .each
        .iter()
        .enumerate()
        .map(|(index, leaf)| {
            let at = |field: &str| format!("{LEAF_GLAZING}:{object}:{}:{field}", index + 1);
            let area = |bounds: (f64, f64), field: &str| {
                MemberValue::Measured(interval(
                    bounds,
                    Some(QuantityDimension::Area),
                    exact,
                    at(field),
                ))
            };
            let (glazed, area_of_leaf, ratio) = match (&leaf.glazing, leaf.ratio()) {
                (LeafGlazing::Measured { glazed, leaf, .. }, Ok(ratio)) => (
                    area(*glazed, "glazed_area"),
                    area(*leaf, "leaf_area"),
                    MemberValue::Measured(interval(ratio, None, exact, at("ratio"))),
                ),
                (_, ratio) => {
                    let why = ratio
                        .err()
                        .unwrap_or_else(|| format!("the glazing of {} is undecided", leaf.name));
                    let undecided = || MemberValue::Undecided { why: why.clone() };
                    (undecided(), undecided(), undecided())
                }
            };
            MeasuredMember {
                certain: true,
                exact,
                fields: BTreeMap::from([
                    (
                        "leaf",
                        MemberValue::Text {
                            text: leaf.name.clone(),
                        },
                    ),
                    ("glazed_area", glazed),
                    ("leaf_area", area_of_leaf),
                    ("ratio", ratio),
                ]),
                evidence: Vec::new(),
            }
        })
        .collect();
    Ok((items, leaves.evidence))
}

impl MeasuredProvider for GlazingMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LEAF_GLAZING]
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
        self.members_cited(call, object, context)
            .map(|(members, _)| members)
    }

    /// The items cite the leaves and the measurement.
    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        members(object, context).map_err(refused(call.name(), object))
    }
}
