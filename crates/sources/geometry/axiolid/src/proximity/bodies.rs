//! Bodies indexed for measurement, as a derivation of the check's store
//! (ADR 0006, #408).
//!
//! Every pair a clash, clearance or distance rule measures reads both
//! bodies' triangles through a bounding-volume hierarchy. A body is in many
//! pairs, of one rule and of several rules over the same objects, so its
//! index is built once per check ([`IndexedBodies`]) and shared by every
//! pair, rule and thread reading it (single flight), within the bound the
//! adapter's kinds keep ([`crate::derivations::keep_bytes`]).

use std::any::TypeId;
use std::sync::Arc;

use axiolid_core::Tolerance;
use axiolid_mesh::{TriMesh, audit_mesh};
use axioval_engine::{
    Bounds3, DerivationError, DerivationKind, DerivationStore, DerivationVersion,
    DerivedRelationshipServiceHandle, Inputs, ProximityError, ProximityServiceHandle,
};
use axioval_ir::ObjectId;

use super::zero_area::Surface;
use super::{ANGULAR_TOLERANCE, Indexed, LINEAR_TOLERANCE, triangle_box};
use crate::derivations::{
    self, AXIOLID_CORE, AXIOLID_MEASURE, AXIOLID_MESH, AXIOLID_SPATIAL, Over,
};
use crate::geometry::{AxiolidGeometry, Triangle, triangles};

/// A body's triangles with their boxes and a hierarchy over them, whether
/// it is closed, and its surface without zero-area triangles where it has
/// any.
pub(crate) struct IndexedBody {
    pub(crate) soup: Indexed<Triangle>,
    /// Closed and consistently wound, zero-area triangles counted, so it
    /// has an inside.
    pub(crate) solid: bool,
    /// The mesh without its zero-area triangles, when it holds any.
    pub(crate) surface: Option<Surface>,
}

/// Everything a body's index is built from (#439): the geometry (by
/// identity: its meshes in its one frame, model coordinates), the object,
/// whether its void is measured in place of it (only for an object without
/// a body of its own, so the material and void views share every other
/// body) and the audit tolerance by its bits; the kind's version
/// ([`IndexedBodies::VERSION`]) beside it. The index is exact: it holds the
/// mesh's own coordinates.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct BodyKey {
    pub(crate) geometry: Over<AxiolidGeometry>,
    pub(crate) object: ObjectId,
    pub(crate) void: bool,
    pub(crate) tolerance: [u64; 2],
}

impl BodyKey {
    /// The key of `object`'s body in `geometry`, its void's where `voids`
    /// measures it so.
    pub(crate) fn new(geometry: &Arc<AxiolidGeometry>, object: &ObjectId, voids: bool) -> Self {
        Self {
            geometry: Over(Arc::clone(geometry)),
            object: object.clone(),
            void: voids && geometry.has_no_body(object),
            tolerance: [LINEAR_TOLERANCE.to_bits(), ANGULAR_TOLERANCE.to_bits()],
        }
    }

    /// The mesh measured for the key: the object's void in the view
    /// measuring voids, never material; its body otherwise.
    pub(crate) fn mesh<'a>(
        &self,
        geometry: &'a AxiolidGeometry,
    ) -> Result<&'a TriMesh, ProximityError> {
        if geometry.has_no_body(&self.object) {
            match geometry.void(&self.object).filter(|_| self.void) {
                Some(Ok(mesh)) => Ok(mesh),
                Some(Err(_)) => Err(ProximityError::Unavailable),
                None => Err(ProximityError::NoBody),
            }
        } else {
            geometry
                .mesh(&self.object)
                .ok_or(ProximityError::Unavailable)
        }
    }

    fn index(&self) -> Result<Arc<IndexedBody>, ProximityError> {
        let mesh = self.mesh(&self.geometry.0)?;
        let triangles = triangles(mesh);
        let [linear, angular] = self.tolerance.map(f64::from_bits);
        let tolerance = Tolerance::new(linear, angular).map_err(|_| ProximityError::Unavailable)?;
        let health = audit_mesh(mesh, tolerance);
        // Every coordinate feeds a measurement reported as evidence; a mesh
        // with bad indices or non-finite positions cannot be measured at all.
        if health.invalid_indices == 0
            && health.non_finite_positions == 0
            && health.usable_triangles == 0
            && health.degenerate_triangles > 0
        {
            return Err(ProximityError::Refused(
                "every triangle of the body's mesh has zero area, so it bounds no surface",
            ));
        }
        if triangles.is_empty() || !health.is_surface_usable() {
            return Err(ProximityError::Unavailable);
        }
        let boxes: Vec<Bounds3> = triangles.iter().map(triangle_box).collect();
        // Zero-area triangles add no surface. A mesh closed only with them
        // (a T-junction closed by a sliver) still bounds a solid, whose
        // winding number is taken without them.
        let surface = Surface::of(mesh, health.degenerate_triangles);
        let solid = match &surface {
            None => health.is_closed_two_manifold(),
            Some(surface) => surface.closed,
        };
        Ok(Arc::new(IndexedBody {
            soup: Indexed::build(triangles, boxes)?,
            solid,
            surface,
        }))
    }
}

/// What an indexed triangle takes: its corners, its box and its share of
/// the hierarchy.
const TRIANGLE_BYTES: usize = 72 + 48 + 64;

/// The derivation kind of indexed bodies.
pub(crate) struct IndexedBodies;

impl DerivationKind for IndexedBodies {
    const NAME: &'static str = "indexed body";
    /// Bumped whenever the same key could index another body: the audit,
    /// the zero-area surface or the hierarchy changing what they return.
    const VERSION: DerivationVersion = DerivationVersion::new(
        1,
        &[AXIOLID_CORE, AXIOLID_MEASURE, AXIOLID_MESH, AXIOLID_SPATIAL],
    );
    type Key = BodyKey;
    /// A body that cannot be measured is refused alike for every pair.
    type Value = Result<Arc<IndexedBody>, ProximityError>;

    fn build(key: &BodyKey, _: &Inputs<'_>) -> Result<Self::Value, DerivationError> {
        Ok(key.index())
    }

    fn size_hint(value: &Self::Value) -> Option<usize> {
        Some(value.as_ref().map_or(0, |body| {
            body.soup.items.len() * TRIANGLE_BYTES + body.surface.as_ref().map_or(0, Surface::bytes)
        }))
    }

    fn keep_bytes() -> Option<u64> {
        Some(derivations::keep_bytes())
    }

    /// Bodies are indexed for the proximity service's pairs, and for the
    /// relationships derived from geometry, which measure through it.
    fn read_through() -> Option<Vec<TypeId>> {
        Some(vec![
            TypeId::of::<ProximityServiceHandle>(),
            TypeId::of::<DerivedRelationshipServiceHandle>(),
        ])
    }
}

/// The index of `key`'s body: the check's, built once for every pair,
/// rule and thread reading it; outside a check, built for this read alone.
pub(crate) fn indexed(key: &BodyKey) -> Result<Arc<IndexedBody>, ProximityError> {
    DerivationStore::derive_current::<IndexedBodies>(key)
        .map_err(|_| ProximityError::Unavailable)?
        .as_ref()
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiolid_core::Point3;
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

    /// What of an index a test compares: its triangles, their bounds,
    /// whether it is solid and has a surface of its own, or why it was
    /// refused.
    fn fingerprint(value: &Result<Arc<IndexedBody>, ProximityError>) -> String {
        match value {
            Ok(body) => format!(
                "{} triangles in {:?}, solid {}, surface {}",
                body.soup.items.len(),
                body.soup.bounds,
                body.solid,
                body.surface.is_some()
            ),
            Err(error) => format!("refused: {error:?}"),
        }
    }

    /// Every input a body's index is built from is in its key (#439):
    /// changing any one of them changes the key, and the index where it
    /// indexes another body.
    #[test]
    fn changing_any_input_of_an_indexed_body_changes_its_key() {
        let geometry = Arc::new(
            AxiolidGeometry::new()
                .with_no_body(id("opening"))
                .with_void(id("opening"), cuboid([0.0; 3], [1.0, 0.2, 2.0]))
                .with_mesh(id("wall"), cuboid([0.0; 3], [4.0, 0.2, 3.0])),
        );
        let other = Arc::new(
            AxiolidGeometry::new()
                .with_no_body(id("opening"))
                .with_void(id("opening"), cuboid([0.0; 3], [0.9, 0.2, 2.1])),
        );
        let base = BodyKey::new(&geometry, &id("opening"), true);
        KeyCompleteness::<IndexedBodies, _>::new(base.clone(), fingerprint)
            .same(
                "the same inputs",
                BodyKey::new(&geometry, &id("opening"), true),
            )
            .changes_value("the geometry", BodyKey::new(&other, &id("opening"), true))
            .changes_value("the object", BodyKey::new(&geometry, &id("wall"), true))
            .changes_value(
                "measuring its void",
                BodyKey::new(&geometry, &id("opening"), false),
            )
            .changes_value(
                "the linear tolerance",
                BodyKey {
                    // Every triangle of the void is degenerate at 2 m.
                    tolerance: [2.0_f64.to_bits(), base.tolerance[1]],
                    ..base.clone()
                },
            )
            // The audit reads no angle.
            .input(
                "the angular tolerance",
                BodyKey {
                    tolerance: [base.tolerance[0], (ANGULAR_TOLERANCE * 2.0).to_bits()],
                    ..base.clone()
                },
            )
            .check();
        // A body of its own is the same body in either view.
        assert!(
            BodyKey::new(&geometry, &id("wall"), true)
                == BodyKey::new(&geometry, &id("wall"), false)
        );
    }
}
