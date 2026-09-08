//! Geometry an application supplies for its model objects.
//!
//! Shared by every service in this crate: the store maps a source-qualified
//! object to its mesh and knows nothing about what will be measured from it.

use std::collections::BTreeMap;

use axiolid_core::Point3;
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axioval_ir::ObjectId;

/// Geometry for one object, keyed by the identity the engine uses.
///
/// Holding meshes by `ObjectId` is what keeps this adapter source-neutral:
/// the host decides how its native elements map onto identities.
#[derive(Clone, Debug, Default)]
pub struct AxiolidGeometry {
    meshes: BTreeMap<ObjectId, TriMesh>,
    doorways: BTreeMap<ObjectId, usize>,
}

impl AxiolidGeometry {
    /// Creates an empty geometry set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            meshes: BTreeMap::new(),
            doorways: BTreeMap::new(),
        }
    }

    /// Registers one object's mesh.
    #[must_use]
    pub fn with_mesh(mut self, object: ObjectId, mesh: TriMesh) -> Self {
        self.meshes.insert(object, mesh);
        self
    }

    /// Returns the mesh registered for an object.
    #[must_use]
    pub fn mesh(&self, object: &ObjectId) -> Option<&TriMesh> {
        self.meshes.get(object)
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
