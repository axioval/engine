//! Candidates near an object in plan, for measured values that measure one
//! object at a time against a selection (`counterpart-coverage`'s
//! counterparts, `effective-coverage`'s sources and blockers).
//!
//! The candidates' extents are read once per selection ([`Candidates`]);
//! each object then keeps every candidate whose plan box lies within a
//! margin of its own ([`Candidates::near`]): the pairs the engine's plan
//! broad phase keeps for that one object, its boxes flattened and grown by
//! their chord deviation as `projected_candidate_pairs` grows them. A
//! candidate whose extent cannot be read is kept apart (`blind`): it may
//! stand near any object.

use std::collections::BTreeSet;

use axioval_engine::{Bounds3, NotEvaluatedReason, ObjectBounds, ProximityServiceHandle};
use axioval_ir::ObjectId;

use crate::pairs::reason as proximity_reason;
use crate::support::Unavailable;

/// The plan extent of `object` as the proximity service bounds it.
pub(crate) fn bounds(
    proximity: &ProximityServiceHandle,
    object: &ObjectId,
) -> Result<ObjectBounds, Unavailable> {
    match proximity.bounds(object) {
        Ok(extent) if extent.object() == object => Ok(extent),
        Ok(_) => Err((
            NotEvaluatedReason::InvalidEvidence,
            "proximity bounds name a different object".to_owned(),
        )),
        Err(error) => Err((proximity_reason(error), error.to_string())),
    }
}

/// The candidates a selection picks, read once: those surely picked, the
/// plan boxes of those picked or undecided, and those whose extent cannot
/// be read.
pub(crate) struct Candidates {
    /// Candidates the selector picks.
    pub(crate) matched: BTreeSet<ObjectId>,
    /// Each readable candidate's box flattened into the plan, enclosing its
    /// geometry, by identity.
    flat: Vec<(ObjectId, Bounds3)>,
    /// Candidates picked or undecided whose extent cannot be read.
    pub(crate) blind: BTreeSet<ObjectId>,
}

impl Candidates {
    /// The candidates `matched` and `undecided` name, and the extents of
    /// those that can be read.
    pub(crate) fn read(
        proximity: &ProximityServiceHandle,
        matched: BTreeSet<ObjectId>,
        undecided: &BTreeSet<ObjectId>,
    ) -> (Self, Vec<ObjectBounds>) {
        let mut blind = BTreeSet::new();
        let mut extents: Vec<ObjectBounds> = Vec::new();
        for candidate in matched.iter().chain(undecided) {
            match bounds(proximity, candidate) {
                Ok(extent) => extents.push(extent),
                Err(_) => {
                    blind.insert(candidate.clone());
                }
            }
        }
        let mut flat: Vec<(ObjectId, Bounds3)> = extents
            .iter()
            .filter_map(|extent| Some((extent.object().clone(), plan_box(extent)?)))
            .collect();
        flat.sort_by(|left, right| left.0.cmp(&right.0));
        flat.dedup_by(|left, right| left.0 == right.0);
        (
            Self {
                matched,
                flat,
                blind,
            },
            extents,
        )
    }

    /// The candidates whose plan box lies within `margin` of `subject`'s,
    /// by identity, the subject itself left out: every pair the plan broad
    /// phase keeps for one subject.
    pub(crate) fn near(&self, subject: &ObjectBounds, margin: f64) -> Vec<ObjectId> {
        let Some(own) = plan_box(subject) else {
            return Vec::new();
        };
        self.flat
            .iter()
            .filter(|(id, flat)| id != subject.object() && own.gap(flat) <= margin)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// `extent`'s box in the plan, enclosing its geometry as the plan broad
/// phase flattens it (`projected_candidate_pairs`).
fn plan_box(extent: &ObjectBounds) -> Option<Bounds3> {
    let (min, max) = (extent.bounds().min(), extent.bounds().max());
    let flat = Bounds3::try_new([min[0], min[1], 0.0], [max[0], max[1], 0.0]).ok()?;
    ObjectBounds::try_new(extent.object().clone(), flat, extent.fidelity())
        .ok()
        .map(|flat| flat.enclosing())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use axioval_engine::{
        Bounds3, GeometryFidelity, ObjectBounds, ProximityProjection, projected_candidate_pairs,
    };
    use axioval_ir::{ObjectId, SourceId};

    use super::plan_box;

    fn bounds(local: &str, min: [f64; 3], max: [f64; 3], deviation: f64) -> ObjectBounds {
        let fidelity = if deviation > 0.0 {
            GeometryFidelity::tessellated(deviation).unwrap()
        } else {
            GeometryFidelity::Exact
        };
        ObjectBounds::try_new(
            ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap(),
            Bounds3::try_new(min, max).unwrap(),
            fidelity,
        )
        .unwrap()
    }

    /// One subject's near candidates are exactly the pairs the plan broad
    /// phase keeps for it, whatever the margin, heights and deviations.
    #[test]
    fn near_candidates_are_the_broad_phases_pairs_of_one_subject() {
        let mut seed = 7_u64;
        let mut next = |range: f64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[allow(clippy::cast_precision_loss)]
            let unit = (seed >> 11) as f64 / (1_u64 << 53) as f64;
            unit * range
        };
        for _ in 0..200 {
            let mut candidates = Vec::new();
            for index in 0..8 {
                let (x, y, z) = (next(10.0), next(10.0), next(6.0));
                let deviation = if next(1.0) < 0.3 { next(0.05) } else { 0.0 };
                candidates.push(bounds(
                    &format!("c{index}"),
                    [x, y, z],
                    [x + next(3.0), y + next(3.0), z + next(3.0)],
                    deviation,
                ));
            }
            let (x, y) = (next(10.0), next(10.0));
            let subject = bounds("s", [x, y, 0.0], [x + next(4.0), y + 0.2, 3.0], 0.0);
            let margin = next(1.0);
            let expected: BTreeSet<ObjectId> = projected_candidate_pairs(
                std::slice::from_ref(&subject),
                &candidates,
                ProximityProjection::Horizontal,
                margin,
            )
            .unwrap()
            .into_iter()
            .map(|pair| pair.counterpart().clone())
            .collect();
            let own = plan_box(&subject).unwrap();
            let found: BTreeSet<ObjectId> = candidates
                .iter()
                .filter(|candidate| own.gap(&plan_box(candidate).unwrap()) <= margin)
                .map(|candidate| candidate.object().clone())
                .collect();
            assert_eq!(found, expected);
        }
    }
}
