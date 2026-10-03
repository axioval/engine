//! Geometry an application supplies for its model objects.
//!
//! Shared by every service in this crate: the store maps a source-qualified
//! object to its mesh and knows nothing about what will be measured from it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axiolid_brep::ExactBRep;
use axiolid_core::Point3;
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axioval_engine::{GeometryFidelity, ProximityError};
use axioval_ir::ObjectId;

/// Geometry for one object, keyed by the identity the engine uses.
///
/// Holding meshes by `ObjectId` is what keeps this adapter source-neutral:
/// the host decides how its native elements map onto identities.
///
/// Besides a mesh, the host can state two facts about an object. It has **no
/// body** ([`Self::with_no_body`]): a storey or a zone occupies no volume, so
/// it obstructs nothing. Or its body is **unmeasured**
/// ([`Self::with_unmeasured`]): it exists but could not be meshed. The two
/// must not be confused. Treating an unmeasured slab as bodiless would let a
/// wall above it look unsupported, or a room look free where it is not, and
/// still report the result as exact. Services therefore refuse whenever an
/// unmeasured object could have changed their answer.
#[derive(Clone, Debug, Default)]
pub struct AxiolidGeometry {
    meshes: BTreeMap<ObjectId, TriMesh>,
    chord_deviations: BTreeMap<ObjectId, f64>,
    bodiless: BTreeSet<ObjectId>,
    unmeasured: BTreeMap<ObjectId, String>,
    groups: BTreeMap<ObjectId, Result<Vec<ObjectId>, String>>,
    boundaries: BTreeMap<ObjectId, Arc<ExactBRep>>,
}

impl AxiolidGeometry {
    /// Creates an empty geometry set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares that an object occupies no volume, e.g. a storey or a zone.
    ///
    /// Services skip it where they would otherwise need its mesh, such as
    /// when every other object is a candidate obstacle.
    #[must_use]
    pub fn with_no_body(mut self, object: ObjectId) -> Self {
        self.bodiless.insert(object);
        self
    }

    /// Declares that an object has a body this host could not mesh.
    ///
    /// Its extent is unknown, so any measurement it could affect is refused
    /// rather than taken as if the object were not there.
    #[must_use]
    pub fn with_unmeasured(mut self, object: ObjectId, reason: impl Into<String>) -> Self {
        self.unmeasured.insert(object, reason.into());
        self
    }

    /// Declares a bodiless group (a zone, say) and the objects it groups.
    ///
    /// Membership is a semantic fact a mesh cannot show, so the host states
    /// it. A group has no body of its own, so it is also declared bodiless:
    /// it obstructs nothing. Its plan footprint is the union of its members'
    /// footprints; a member may itself be a declared group.
    #[must_use]
    pub fn with_group(
        mut self,
        group: ObjectId,
        members: impl IntoIterator<Item = ObjectId>,
    ) -> Self {
        let mut members: Vec<ObjectId> = members.into_iter().collect();
        members.sort();
        members.dedup();
        self.bodiless.insert(group.clone());
        self.groups.insert(group, Ok(members));
        self
    }

    /// Declares a bodiless group whose membership the host could not decide.
    ///
    /// Measurements that need its members refuse with `reason`, rather than
    /// take the group as empty.
    #[must_use]
    pub fn with_undecided_group(mut self, group: ObjectId, reason: impl Into<String>) -> Self {
        self.bodiless.insert(group.clone());
        self.groups.insert(group, Err(reason.into()));
        self
    }

    /// A declared group's members in identity order, or the reason its
    /// membership is undecided; `None` when the object is no declared group.
    #[must_use]
    pub fn group_members(&self, group: &ObjectId) -> Option<Result<&[ObjectId], &str>> {
        self.groups.get(group).map(|members| match members {
            Ok(members) => Ok(members.as_slice()),
            Err(reason) => Err(reason.as_str()),
        })
    }

    /// Whether the host declared the object bodiless.
    #[must_use]
    pub fn has_no_body(&self, object: &ObjectId) -> bool {
        self.bodiless.contains(object)
    }

    /// Whether the host declared the object's body unmeasured.
    #[must_use]
    pub fn is_unmeasured(&self, object: &ObjectId) -> bool {
        self.unmeasured.contains_key(object)
    }

    /// Every object whose body could not be measured, with the host's reason.
    pub fn unmeasured(&self) -> impl Iterator<Item = (&ObjectId, &str)> {
        self.unmeasured
            .iter()
            .map(|(object, reason)| (object, reason.as_str()))
    }

    /// Registers one object's mesh, asserting every face is planar so the
    /// mesh is the object's exact shape. Curved parts belong in
    /// [`Self::with_tessellated_mesh`].
    #[must_use]
    pub fn with_mesh(mut self, object: ObjectId, mesh: TriMesh) -> Self {
        self.chord_deviations.remove(&object);
        self.meshes.insert(object, mesh);
        self
    }

    /// Registers one object's mesh as a tessellation of curved faces.
    ///
    /// `chord_deviation_metres` bounds how far the true surface may lie from
    /// the mesh. Measurements that honour fidelity report such an object as
    /// approximate, never exact. An invalid deviation is kept and refused when
    /// measured, so it cannot silently become exact.
    #[must_use]
    pub fn with_tessellated_mesh(
        mut self,
        object: ObjectId,
        mesh: TriMesh,
        chord_deviation_metres: f64,
    ) -> Self {
        self.chord_deviations
            .insert(object.clone(), chord_deviation_metres);
        self.meshes.insert(object, mesh);
        self
    }

    /// Registers the exact boundary of an object whose mesh is registered
    /// too, typically a tessellated one.
    ///
    /// The mesh stays the basis of every measurement. The boundary only
    /// narrows proximity between two objects that both have one
    /// ([`crate::AxiolidProximityService`]): the kernel's certified
    /// `boundary_distance` bounds the distance in space, and its plan
    /// measurements (`plan_boundary_distance`, `plan_boundary_clearance`,
    /// `plan_overlap`) the horizontal distance, plan overlap and the
    /// footprint relation of vertical distances, where the chord deviation
    /// would leave them wide or open. The surface distance between two
    /// revisions is measured between their boundaries where both have one
    /// (`one_sided_boundary_hausdorff_with_budget` each way). The host asserts that boundary and
    /// mesh describe one body; a pair whose certified answer contradicts the
    /// mesh's widened one refuses.
    #[must_use]
    pub fn with_exact_boundary(mut self, object: ObjectId, boundary: ExactBRep) -> Self {
        self.boundaries.insert(object, Arc::new(boundary));
        self
    }

    /// The exact boundary registered for an object.
    #[must_use]
    pub fn exact_boundary(&self, object: &ObjectId) -> Option<&ExactBRep> {
        self.boundaries.get(object).map(Arc::as_ref)
    }

    /// The exact boundary registered for an object, shared, to hand out
    /// beside its surface.
    pub(crate) fn shared_exact_boundary(&self, object: &ObjectId) -> Option<Arc<ExactBRep>> {
        self.boundaries.get(object).cloned()
    }

    /// How faithfully an object's mesh represents it.
    ///
    /// # Errors
    ///
    /// Returns an error when the declared chord deviation is negative or
    /// non-finite.
    pub fn fidelity(&self, object: &ObjectId) -> Result<GeometryFidelity, ProximityError> {
        self.chord_deviations
            .get(object)
            .map_or(Ok(GeometryFidelity::Exact), |deviation| {
                GeometryFidelity::tessellated(*deviation)
            })
    }

    /// Returns the mesh registered for an object.
    #[must_use]
    pub fn mesh(&self, object: &ObjectId) -> Option<&TriMesh> {
        self.meshes.get(object)
    }

    /// Whether an object's mesh was registered as a tessellation.
    pub(crate) fn is_tessellated(&self, object: &ObjectId) -> bool {
        self.chord_deviations.contains_key(object)
    }

    /// An object's mesh extent grown by its chord deviation, so it encloses
    /// the true body. `None` without a mesh or with an invalid deviation.
    pub(crate) fn enclosing_extent(&self, object: &ObjectId) -> Option<Extent> {
        let (min, max) = mesh_extent(self.meshes.get(object)?)?;
        let deviation = self.fidelity(object).ok()?.deviation_metres();
        Some((min.map(|v| v - deviation), max.map(|v| v + deviation)))
    }

    /// A tessellated object that could change a measurement taken around
    /// `probe`: its enclosing extent lies within `reach` of it, in plan when
    /// `plan` is set. Objects `skip` accepts are ignored, and an invalid
    /// declared deviation counts as near, since nothing bounds it.
    ///
    /// Services that report exact evidence refuse when this finds anything:
    /// a chord approximation near the measurement makes the result an
    /// estimate, and presenting it as exact would launder it into fact.
    pub(crate) fn tessellated_near(
        &self,
        probe: &Extent,
        reach: f64,
        plan: bool,
        skip: impl Fn(&ObjectId) -> bool,
    ) -> Option<&ObjectId> {
        self.chord_deviations.keys().find(|object| {
            !skip(object)
                && self
                    .enclosing_extent(object)
                    .is_none_or(|extent| extent_gap(probe, &extent, plan) <= reach)
        })
    }

    /// Every registered object and its mesh, in identity order.
    pub(crate) fn objects(&self) -> impl Iterator<Item = (&ObjectId, &TriMesh)> {
        self.meshes.iter()
    }
}

/// An axis-aligned `(min, max)` extent in metres.
pub(crate) type Extent = ([f64; 3], [f64; 3]);

/// The extent of a mesh's positions; `None` for an empty mesh.
pub(crate) fn mesh_extent(mesh: &TriMesh) -> Option<Extent> {
    let mut positions = (0..mesh.position_count()).map(|i| mesh.position(i));
    let first = positions.next()?;
    let (min, max) = positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
    Some((min.to_array(), max.to_array()))
}

/// Euclidean gap between two extents, over x and y only when `plan` is set.
pub(crate) fn extent_gap(a: &Extent, b: &Extent, plan: bool) -> f64 {
    let axes = if plan { 2 } else { 3 };
    (0..axes)
        .map(|axis| {
            let gap = (b.0[axis] - a.1[axis]).max(a.0[axis] - b.1[axis]).max(0.0);
            gap * gap
        })
        .sum::<f64>()
        .sqrt()
}

/// A triangle as three points, the form the geometry primitives consume.
pub(crate) type Triangle = [axiolid_core::Point3; 3];

/// The triangles of a mesh as coordinate triples.
pub(crate) fn triangles(mesh: &TriMesh) -> Vec<Triangle> {
    (0..mesh.triangle_count())
        .map(|index| {
            let [a, b, c] = mesh.triangle(index);
            // Indices come from a foreign mesh, so a value that cannot be a
            // position index is a corrupt mesh, not something to truncate.
            [a, b, c].map(|index| {
                usize::try_from(index)
                    .ok()
                    .filter(|i| *i < mesh.position_count())
                    .map_or(Point3::ZERO, |i| mesh.position(i))
            })
        })
        .collect()
}
