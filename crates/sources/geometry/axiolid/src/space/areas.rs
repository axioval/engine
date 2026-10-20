//! The plan areas the space aspects measure of the same bodies again and
//! again, as derivations of the check's store (ADR 0006, #443): each
//! body's footprint ([`PlanFootprints`]) and the area a space's body shares
//! with a candidate's ([`SharedPlanAreas`]).
//!
//! A large model asks for the same footprints for every space beside them
//! and for the same pairs in the overlaps and both caps (#371). Built once
//! per check and kept in the store, they count against the check's memory
//! budget and are released after the last rule that may ask the space
//! service, where the service once kept them for its whole life.

use std::any::TypeId;
use std::sync::Arc;

use axioval_engine::{
    DerivationError, DerivationKind, DerivationVersion, Inputs, Precision, SpaceError,
    SpaceServiceHandle,
};
use axioval_ir::ObjectId;

use super::{PlanArea, SharedArea, plan_area, shared_area};
use crate::derivations::{self, AXIOLID_CORE, AXIOLID_MESH, AXIOLID_OVERLAY, OverGeometry};
use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::walkable::polygons_bytes;

/// The triangles of `object`'s mesh in `geometry`, `Unavailable` without
/// one, as the service reads a body.
fn body(geometry: &AxiolidGeometry, object: &ObjectId) -> Result<Vec<Triangle>, SpaceError> {
    let mesh = geometry.mesh(object).ok_or(SpaceError::Unavailable)?;
    Ok(triangles(mesh))
}

/// The audit tolerance's bits, linear and angular.
fn tolerance_bits(tolerance: axiolid_core::Tolerance) -> [u64; 2] {
    [tolerance.linear().to_bits(), tolerance.angular().to_bits()]
}

/// The tolerance of `bits`.
fn of_bits(bits: [u64; 2]) -> Result<axiolid_core::Tolerance, SpaceError> {
    axiolid_core::Tolerance::new(f64::from_bits(bits[0]), f64::from_bits(bits[1]))
        .map_err(|_| SpaceError::Unavailable)
}

/// Everything a body's plan footprint is built from (#439): the geometry
/// (by identity), the object, the tolerance by its bits and the precision
/// class (#438); the kind's version beside it. The footprint is exact (the
/// overlay computes no fast mode here), so every reader asks Exact.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct FootprintKey {
    pub(crate) geometry: OverGeometry,
    pub(crate) object: ObjectId,
    pub(crate) tolerance: [u64; 2],
    pub(crate) precision: Precision,
}

impl FootprintKey {
    pub(crate) fn new(
        geometry: &AxiolidGeometry,
        object: &ObjectId,
        tolerance: axiolid_core::Tolerance,
    ) -> Self {
        Self {
            geometry: OverGeometry(geometry.clone()),
            object: object.clone(),
            tolerance: tolerance_bits(tolerance),
            precision: Precision::Exact,
        }
    }

    pub(crate) fn measure(&self) -> Result<Arc<PlanArea>, SpaceError> {
        let body = body(&self.geometry.0, &self.object)?;
        plan_area(&body, of_bits(self.tolerance)?).map(Arc::new)
    }
}

/// The derivation kind of bodies' plan footprints.
pub(crate) struct PlanFootprints;

impl DerivationKind for PlanFootprints {
    const NAME: &'static str = "plan footprint";
    /// Bumped whenever the same key could build another footprint: the
    /// bounded footprint, its slivers or the overlay changing what they
    /// return.
    const VERSION: DerivationVersion =
        DerivationVersion::new(1, &[AXIOLID_CORE, AXIOLID_MESH, AXIOLID_OVERLAY]);
    type Key = FootprintKey;
    /// A footprint the overlay refuses is refused alike for every aspect.
    type Value = Result<Arc<PlanArea>, SpaceError>;

    fn build(key: &FootprintKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.measure())
    }

    fn precision(key: &FootprintKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, |area| {
            polygons_bytes(&area.polygons) + std::mem::size_of::<PlanArea>()
        }))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    /// Footprints are read by the space service's aspects alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<SpaceServiceHandle>()])
    }
}

/// Everything the plan area a space's body shares with a candidate's is
/// built from (#439): the geometry (by identity), the space and the
/// candidate in that order, the tolerance by its bits and the precision
/// class; exact, as [`FootprintKey`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SharedKey {
    pub(crate) geometry: OverGeometry,
    pub(crate) space: ObjectId,
    pub(crate) candidate: ObjectId,
    pub(crate) tolerance: [u64; 2],
    pub(crate) precision: Precision,
}

impl SharedKey {
    pub(crate) fn new(
        geometry: &AxiolidGeometry,
        (space, candidate): (&ObjectId, &ObjectId),
        tolerance: axiolid_core::Tolerance,
    ) -> Self {
        Self {
            geometry: OverGeometry(geometry.clone()),
            space: space.clone(),
            candidate: candidate.clone(),
            tolerance: tolerance_bits(tolerance),
            precision: Precision::Exact,
        }
    }

    pub(crate) fn measure(&self) -> Result<SharedArea, SpaceError> {
        let subject = body(&self.geometry.0, &self.space)?;
        let other = body(&self.geometry.0, &self.candidate)?;
        shared_area(&subject, &other, of_bits(self.tolerance)?).map(|area| SharedArea::of(&area))
    }
}

/// The derivation kind of the plan areas two bodies share, as their
/// bounds: the decisions read no more of them (#371).
pub(crate) struct SharedPlanAreas;

impl DerivationKind for SharedPlanAreas {
    const NAME: &'static str = "shared plan area";
    /// Bumped whenever the same key could build another area, as
    /// [`PlanFootprints::VERSION`].
    const VERSION: DerivationVersion =
        DerivationVersion::new(1, &[AXIOLID_CORE, AXIOLID_MESH, AXIOLID_OVERLAY]);
    type Key = SharedKey;
    type Value = Result<SharedArea, SpaceError>;

    fn build(key: &SharedKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.measure())
    }

    fn precision(key: &SharedKey) -> Option<Precision> {
        Some(key.precision)
    }

    fn size_hint(_: &Self::Value) -> Option<usize> {
        Some(std::mem::size_of::<Self::Value>() + std::mem::size_of::<SharedKey>())
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    /// Shared areas are read by the space service's aspects alone.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![TypeId::of::<SpaceServiceHandle>()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::tolerance;
    use axiolid_core::Point3;
    use axiolid_mesh::TriMesh;
    use axioval_engine::KeyCompleteness;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    /// A closed, outward-wound box.
    fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        let positions = vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ];
        TriMesh::new(positions, indices)
    }

    /// A 5 m by 4 m room, a 2 m by 2 m store inside it, and a slab under
    /// half of the room.
    fn geometry(room: [f64; 3]) -> AxiolidGeometry {
        AxiolidGeometry::new()
            .with_mesh(id("room"), cuboid([0.0; 3], room))
            .with_mesh(id("store"), cuboid([0.0; 3], [2.0, 2.0, 3.0]))
            .with_mesh(id("slab"), cuboid([0.0, 0.0, -0.2], [2.5, 4.0, 0.0]))
    }

    fn area(value: &Result<Arc<PlanArea>, SpaceError>) -> String {
        match value {
            Ok(area) => format!("[{:.9}, {:.9}]", area.lower, area.upper),
            Err(error) => format!("refused: {error:?}"),
        }
    }

    fn shared(value: &Result<SharedArea, SpaceError>) -> String {
        match value {
            Ok(area) => format!("[{:.9}, {:.9}]", area.lower, area.upper),
            Err(error) => format!("refused: {error:?}"),
        }
    }

    /// Every input a body's footprint is built from is in its key (#439).
    #[test]
    fn changing_any_input_of_a_plan_footprint_changes_its_key() {
        let (one, other) = (geometry([5.0, 4.0, 3.0]), geometry([6.0, 4.0, 3.0]));
        let tolerance = tolerance().unwrap();
        let base = FootprintKey::new(&one, &id("room"), tolerance);
        KeyCompleteness::<PlanFootprints, _>::new(base.clone(), area)
            .same(
                "the same inputs",
                FootprintKey::new(&one, &id("room"), tolerance),
            )
            .changes_value(
                "the geometry",
                FootprintKey::new(&other, &id("room"), tolerance),
            )
            .changes_value(
                "the object",
                FootprintKey::new(&one, &id("store"), tolerance),
            )
            // The overlay snaps within a tolerance no corner here is near.
            .input(
                "the tolerance",
                FootprintKey::new(
                    &one,
                    &id("room"),
                    axiolid_core::Tolerance::new(1.0e-6, 1.0e-9).unwrap(),
                ),
            )
            .precision(
                "the precision class",
                FootprintKey {
                    precision: Precision::fast(1.0e-3),
                    ..base.clone()
                },
            )
            .check();
        assert_eq!(base.precision, Precision::Exact);
    }

    /// Every input a shared plan area is built from is in its key (#439),
    /// the order of the pair included.
    #[test]
    fn changing_any_input_of_a_shared_plan_area_changes_its_key() {
        let (one, other) = (geometry([5.0, 4.0, 3.0]), geometry([1.0, 4.0, 3.0]));
        let tolerance = tolerance().unwrap();
        let key = |geometry: &AxiolidGeometry, space: &str, candidate: &str| {
            SharedKey::new(geometry, (&id(space), &id(candidate)), tolerance)
        };
        let base = key(&one, "room", "slab");
        KeyCompleteness::<SharedPlanAreas, _>::new(base.clone(), shared)
            .same("the same inputs", key(&one, "room", "slab"))
            .changes_value("the geometry", key(&other, "room", "slab"))
            .changes_value("the space", key(&one, "store", "slab"))
            .changes_value("the candidate", key(&one, "room", "store"))
            // The area two bodies share is the same either way round.
            .input("the order of the pair", key(&one, "slab", "room"))
            .input(
                "the tolerance",
                SharedKey::new(
                    &one,
                    (&id("room"), &id("slab")),
                    axiolid_core::Tolerance::new(1.0e-6, 1.0e-9).unwrap(),
                ),
            )
            .precision(
                "the precision class",
                SharedKey {
                    precision: Precision::fast(1.0e-3),
                    ..base
                },
            )
            .check();
    }
}
