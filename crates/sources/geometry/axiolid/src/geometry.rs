//! Geometry an application supplies for its model objects.
//!
//! Shared by every service in this crate: the store maps a source-qualified
//! object to its mesh and knows nothing about what will be measured from it.

use std::collections::BTreeMap;

use axiolid_core::Point3;
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axioval_engine::{GeometryFidelity, ProximityError};
use axioval_ir::ObjectId;

/// Geometry for one object, keyed by the identity the engine uses.
///
/// Holding meshes by `ObjectId` is what keeps this adapter source-neutral:
/// the host decides how its native elements map onto identities.
#[derive(Clone, Debug, Default)]
pub struct AxiolidGeometry {
    meshes: BTreeMap<ObjectId, TriMesh>,
    doorways: BTreeMap<ObjectId, usize>,
    chord_deviations: BTreeMap<ObjectId, f64>,
}

impl AxiolidGeometry {
    /// Creates an empty geometry set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            meshes: BTreeMap::new(),
            doorways: BTreeMap::new(),
            chord_deviations: BTreeMap::new(),
        }
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

    /// Records how many doorways interrupt an object's perimeter.
    ///
    /// Openings are a semantic fact: a mesh of a room does not say which of
    /// its wall segments are doors. The host supplies the count rather than
    /// this adapter guessing at it from geometry alone.
    #[must_use]
    pub fn with_doorways(mut self, object: ObjectId, count: usize) -> Self {
        self.doorways.insert(object, count);
        self
    }

    /// Doorways recorded for an object; absent means none were declared.
    #[must_use]
    pub fn doorway_count(&self, object: &ObjectId) -> usize {
        self.doorways.get(object).copied().unwrap_or(0)
    }

    /// Every registered object and its mesh, in identity order.
    pub(crate) fn objects(&self) -> impl Iterator<Item = (&ObjectId, &TriMesh)> {
        self.meshes.iter()
    }

    /// Every registered object other than `subject`, in identity order.
    pub(crate) fn counterparts(
        &self,
        subject: &ObjectId,
    ) -> impl Iterator<Item = (&ObjectId, &TriMesh)> {
        self.meshes.iter().filter(move |(id, _)| *id != subject)
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
