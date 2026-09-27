//! Object frames: where an object is placed and, when its source says so,
//! which way it faces.
//!
//! Clearance, placement and orientation checks are measured in an object's
//! own frame: the area in front of a component, beside a fixture, along a
//! bay. This seam supplies that frame as a [`MetricFrame`] in canonical
//! metres, grounded on the requested object, together with the front the
//! source states for it.
//!
//! The frame's axes are the source's placement axes, nothing more. Its
//! `forward` axis is the placement's second axis and says nothing about which
//! side of the object is its front. A front is reported only where the source
//! states one ([`ObjectFront::Stated`]); otherwise it is
//! [`ObjectFront::NotStated`], never guessed from the axes, the shape or the
//! object's type.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId, SourceId};
use thiserror::Error;

use crate::services::reviewable_exact_evidence;
use crate::{MetricDirection, MetricFrame, SnapshotBoundService, SourceSnapshot};

/// Failure to supply an object's frame.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ObjectFrameError {
    /// The service does not cover the object's source.
    #[error("object-frame service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The object is not part of the source.
    #[error("object `{0}` is not in the source")]
    UnknownObject(ObjectId),
    /// The source states no placement for the object, so it has no frame.
    /// Never read as the source's origin.
    #[error("the source states no placement for `{0}`")]
    NotPlaced(ObjectId),
    /// The object is placed by a construct the service cannot resolve
    /// exactly.
    #[error("object placement unsupported: {0}")]
    Unsupported(String),
    /// The placement, or the unit it is stated in, is malformed or cannot be
    /// read exactly.
    #[error("object placement cannot be read exactly: {0}")]
    Unreadable(String),
    /// The frame is not grounded on the object, or its axes are not a
    /// right-handed orthonormal triple.
    #[error("object frame is invalid")]
    InvalidFrame,
    /// The frame's evidence is not exact, not reviewable, or from another
    /// source.
    #[error("object-frame evidence is not exact and reviewable")]
    InexactEvidence,
    /// The service answered for another object.
    #[error("object-frame service answered for another object")]
    ResponseRequestMismatch,
}

/// Which way an object faces, as far as its source states it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObjectFront {
    /// The source states the object's front: the direction, in canonical
    /// coordinates, from the object towards the side it is used from.
    Stated(MetricDirection),
    /// The source states no front. A rule that needs one cannot be decided
    /// from this frame; the placement axes are not a substitute.
    NotStated,
}

/// One object's placement frame and stated front, with provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectFrame {
    object: ObjectId,
    frame: MetricFrame,
    front: ObjectFront,
    evidence: Evidence,
}

impl ObjectFrame {
    /// The frame of `object`.
    ///
    /// The frame's origin must be grounded on `object`, and the evidence must
    /// be exact, reviewable and from the object's source: a placement is
    /// stated, never estimated.
    pub fn try_new(
        object: ObjectId,
        frame: MetricFrame,
        front: ObjectFront,
        evidence: Evidence,
    ) -> Result<Self, ObjectFrameError> {
        if frame.origin().subject() != &object {
            return Err(ObjectFrameError::InvalidFrame);
        }
        if !reviewable_exact_evidence(&evidence) || evidence.source != object.source {
            return Err(ObjectFrameError::InexactEvidence);
        }
        Ok(Self {
            object,
            frame,
            front,
            evidence,
        })
    }

    /// The object this frame belongs to.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The placement frame: origin in canonical metres and right-handed
    /// orthonormal right, forward and up axes.
    #[must_use]
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }

    /// The front the source states, or [`ObjectFront::NotStated`].
    #[must_use]
    pub fn front(&self) -> ObjectFront {
        self.front
    }

    /// Reviewable provenance of the placement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Trusted adapter seam supplying objects' placement frames.
pub trait ObjectFrameService: Send + Sync + 'static {
    /// Exact source snapshots this service answers for.
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The placement frame of `object`, or why it has none.
    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError>;
}

/// Registry handle for an [`ObjectFrameService`].
#[derive(Clone)]
pub struct ObjectFrameServiceHandle(Arc<dyn ObjectFrameService>);

impl ObjectFrameServiceHandle {
    /// Wraps a trusted object-frame service.
    #[must_use]
    pub fn new(service: Arc<dyn ObjectFrameService>) -> Self {
        Self(service)
    }

    /// The frame of `object`. Objects of an uncovered source are refused,
    /// and so is a frame of another object.
    pub fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == object.source)
        {
            return Err(ObjectFrameError::UncoveredSource(object.source.clone()));
        }
        let frame = self.0.object_frame(object)?;
        if frame.object() != object {
            return Err(ObjectFrameError::ResponseRequestMismatch);
        }
        Ok(frame)
    }
}

impl SnapshotBoundService for ObjectFrameServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MetricPoint;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn direction(vector: [f64; 3]) -> MetricDirection {
        MetricDirection::try_new(vector).unwrap()
    }

    fn frame(subject: &str) -> MetricFrame {
        MetricFrame::try_new(
            MetricPoint::try_new(id(subject), [1.0, 2.0, 0.0]).unwrap(),
            direction([1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
        )
        .unwrap()
    }

    fn exact() -> Evidence {
        Evidence::exact(source(), "placement:a")
    }

    #[test]
    fn a_frame_is_grounded_on_its_object() {
        assert!(ObjectFrame::try_new(id("a"), frame("a"), ObjectFront::NotStated, exact()).is_ok());
        assert_eq!(
            ObjectFrame::try_new(id("a"), frame("b"), ObjectFront::NotStated, exact()),
            Err(ObjectFrameError::InvalidFrame)
        );
    }

    #[test]
    fn a_frame_needs_exact_reviewable_evidence_from_its_source() {
        let mut approximate = exact();
        approximate.exact = false;
        let unlocated = Evidence::exact(source(), " ");
        let foreign = Evidence::exact(SourceId::new("cad", "other").unwrap(), "placement:a");
        for evidence in [approximate, unlocated, foreign] {
            assert_eq!(
                ObjectFrame::try_new(id("a"), frame("a"), ObjectFront::NotStated, evidence),
                Err(ObjectFrameError::InexactEvidence)
            );
        }
    }

    #[test]
    fn a_left_handed_frame_cannot_be_formed() {
        let mirrored = MetricFrame::try_new(
            MetricPoint::try_new(id("a"), [0.0; 3]).unwrap(),
            direction([-1.0, 0.0, 0.0]),
            direction([0.0, 1.0, 0.0]),
            direction([0.0, 0.0, 1.0]),
        );
        assert!(mirrored.is_err());
    }

    struct Fixed(Vec<SourceSnapshot>, ObjectFrame);
    impl ObjectFrameService for Fixed {
        fn source_snapshots(&self) -> &[SourceSnapshot] {
            &self.0
        }
        fn object_frame(&self, _: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
            Ok(self.1.clone())
        }
    }

    fn handle() -> ObjectFrameServiceHandle {
        let snapshot = SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap();
        let answer = ObjectFrame::try_new(
            id("b"),
            frame("b"),
            ObjectFront::Stated(direction([0.0, -1.0, 0.0])),
            exact(),
        )
        .unwrap();
        ObjectFrameServiceHandle::new(Arc::new(Fixed(vec![snapshot], answer)))
    }

    #[test]
    fn the_handle_binds_answers_to_the_request() {
        let handle = handle();
        let frame = handle.object_frame(&id("b")).unwrap();
        assert_eq!(
            frame.front(),
            ObjectFront::Stated(direction([0.0, -1.0, 0.0]))
        );
        assert_eq!(
            handle.object_frame(&id("a")),
            Err(ObjectFrameError::ResponseRequestMismatch)
        );
        let foreign = ObjectId::new(SourceId::new("cad", "other").unwrap(), "b").unwrap();
        assert!(matches!(
            handle.object_frame(&foreign),
            Err(ObjectFrameError::UncoveredSource(_))
        ));
    }
}
