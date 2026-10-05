//! Triangle counts: how many triangles the mesh a host produced for an
//! object holds.
//!
//! ADR 0004: this seam counts. Whether an element has too many polygons is a
//! rule's judgement over the count.
//!
//! The count is of the mesh the host registered, not of anything the source
//! states: an extruded box has no triangles of its own, and a curved face
//! has as many as the host's chord budget made of it. Another host, or the
//! same host with another budget, may count differently. The evidence is
//! therefore exact only when the mesh is the object's exact shape (every
//! face planar); a tessellation of curved faces is counted exactly but
//! reported with approximate evidence, since its count depends on the
//! host's tessellation.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to count an object's triangles.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TriangleCountError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The object has a body the host could not mesh, or a mesh that cannot
    /// be read.
    #[error("triangle count unavailable: {0}")]
    Unavailable(String),
    /// The count names another object, or its evidence is not reviewable.
    #[error("triangle count is invalid")]
    InvalidMeasurement,
}

/// The number of triangles in one object's mesh, with evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriangleCount {
    object: ObjectId,
    triangles: u64,
    evidence: Evidence,
}

impl TriangleCount {
    /// `triangles` in the mesh of `object`. The evidence must name where
    /// the count comes from; it is exact when the mesh is the object's
    /// exact shape.
    pub fn try_new(
        object: ObjectId,
        triangles: u64,
        evidence: Evidence,
    ) -> Result<Self, TriangleCountError> {
        if evidence.locator.trim().is_empty() {
            return Err(TriangleCountError::InvalidMeasurement);
        }
        Ok(Self {
            object,
            triangles,
            evidence,
        })
    }

    /// The counted object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The number of triangles in the host's mesh; zero for an object the
    /// host declared bodiless.
    #[must_use]
    pub fn triangles(&self) -> u64 {
        self.triangles
    }

    /// Whether the mesh is the object's exact shape rather than a
    /// tessellation whose count depends on the host.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the count.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Counts the triangles of model objects' meshes.
pub trait TriangleCountService: Send + Sync + 'static {
    /// The number of triangles in `object`'s mesh.
    // gate: measures number
    fn count_triangles(&self, object: &ObjectId) -> Result<TriangleCount, TriangleCountError>;
}

/// Registry handle for a [`TriangleCountService`].
#[derive(Clone)]
pub struct TriangleCountServiceHandle(Arc<dyn TriangleCountService>);

impl TriangleCountServiceHandle {
    /// Wraps a trusted triangle-count service.
    #[must_use]
    pub fn new(service: Arc<dyn TriangleCountService>) -> Self {
        Self(service)
    }

    /// The triangle count of `object`. A count naming another object is
    /// refused.
    pub fn count_triangles(&self, object: &ObjectId) -> Result<TriangleCount, TriangleCountError> {
        let count = self.0.count_triangles(object)?;
        if count.object() != object {
            return Err(TriangleCountError::InvalidMeasurement);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    fn evidence() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "triangle-count:a")
    }

    #[test]
    fn a_count_needs_a_locator() {
        assert!(TriangleCount::try_new(id("a"), 12, evidence()).is_ok());
        let unlocated = Evidence::exact(SourceId::new("cad", "m").unwrap(), " ");
        assert_eq!(
            TriangleCount::try_new(id("a"), 12, unlocated),
            Err(TriangleCountError::InvalidMeasurement)
        );
    }

    struct Other;
    impl TriangleCountService for Other {
        fn count_triangles(&self, _: &ObjectId) -> Result<TriangleCount, TriangleCountError> {
            TriangleCount::try_new(id("b"), 12, evidence())
        }
    }

    #[test]
    fn a_count_of_another_object_is_refused() {
        let handle = TriangleCountServiceHandle::new(Arc::new(Other));
        assert_eq!(
            handle.count_triangles(&id("a")),
            Err(TriangleCountError::InvalidMeasurement)
        );
        assert_eq!(handle.count_triangles(&id("b")).unwrap().triangles(), 12);
    }
}
