//! Facade areas: how much of an object's surface faces the outside.
//!
//! ADR 0004: this seam measures. Whether a storey has enough facade, or too
//! much of it glazed, is a rule's judgement over these measurements.
//!
//! An object's facade area is the area of its steep faces that face away
//! from the interior: neither against another body nor looking into a
//! space. Which objects are spaces is semantic, so the host declares them;
//! whether an object is external is the rule's selection, never this seam's.
//!
//! An area is an interval. A mesh that is the object's exact shape measures
//! exactly; one that approximates curved faces measures within a bound the
//! adapter derives from its declared chord deviation, and a rule must decide
//! from the whole interval, never from its midpoint.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::vertical_extent::FaceNormal;

/// Failure to measure a facade area.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FacadeAreaError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The geometry could not be measured, for example an object without a
    /// body, a mesh that fails its audit, or an unmeasured body that could
    /// cover the object's faces.
    #[error("facade area unavailable: {0}")]
    Unavailable(String),
    /// A measurement is non-finite, negative, its bounds are reversed, or it
    /// names another object.
    #[error("facade area measurement is invalid")]
    InvalidMeasurement,
    /// An interval was reported as exact, or evidence as inexact, inconsistently.
    #[error("facade area evidence does not match its exactness")]
    InexactEvidence,
}

/// The facade area of one object in square metres, with its evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct FacadeArea {
    object: ObjectId,
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl FacadeArea {
    /// The facade area of `object`, known to lie in `[lower, upper]`.
    ///
    /// The evidence is exact exactly when the bounds coincide: an interval
    /// cannot be exact evidence, and a point cannot be approximate.
    pub fn try_new(
        object: ObjectId,
        lower: f64,
        upper: f64,
        evidence: Evidence,
    ) -> Result<Self, FacadeAreaError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(FacadeAreaError::InexactEvidence);
        }
        Ok(Self {
            object,
            lower,
            upper,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// Smallest area the facade can have, in square metres.
    #[must_use]
    pub fn lower_square_metres(&self) -> f64 {
        self.lower
    }

    /// Largest area the facade can have, in square metres.
    #[must_use]
    pub fn upper_square_metres(&self) -> f64 {
        self.upper
    }

    /// Whether the area is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Whether a face looks outside over all of it, or only over part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FacadeFaceSide {
    /// Every point the service probed looks outside.
    Outside,
    /// Some probes look outside and some do not: the face is partly
    /// covered, and how much of it looks outside is unknown.
    Partly,
}

/// One planar part of an object's boundary that looks outside: its
/// outward normal, its area and a point on it.
///
/// The normal points out of the body, as [`FaceNormal`] bounds it; a
/// planar body's part is its triangle, and the box only bounds the
/// rounding of its cross product.
#[derive(Clone, Debug, PartialEq)]
pub struct FacadeFace {
    normal: FaceNormal,
    area: (f64, f64),
    centroid: [f64; 3],
    side: FacadeFaceSide,
}

impl FacadeFace {
    /// A face with the outward `normal`, an area in `[lower, upper]` square
    /// metres and its `centroid`.
    ///
    /// # Errors
    ///
    /// [`FacadeAreaError::InvalidMeasurement`] for an area not finite,
    /// negative or reversed, or a centroid that is not finite.
    pub fn try_new(
        normal: FaceNormal,
        (lower, upper): (f64, f64),
        centroid: [f64; 3],
        side: FacadeFaceSide,
    ) -> Result<Self, FacadeAreaError> {
        if !lower.is_finite()
            || !upper.is_finite()
            || lower < 0.0
            || lower > upper
            || !centroid.iter().all(|value| value.is_finite())
        {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(Self {
            normal,
            area: (lower, upper),
            centroid,
            side,
        })
    }

    /// The outward normal.
    #[must_use]
    pub fn normal(&self) -> &FaceNormal {
        &self.normal
    }

    /// The least and greatest area the face may have, in square metres.
    #[must_use]
    pub fn area_square_metres(&self) -> (f64, f64) {
        self.area
    }

    /// The face's centroid, in canonical metres.
    #[must_use]
    pub fn centroid(&self) -> [f64; 3] {
        self.centroid
    }

    /// Whether all of it looks outside.
    #[must_use]
    pub fn side(&self) -> FacadeFaceSide {
        self.side
    }
}

/// Every face of one object that looks outside, wholly or in part, in the
/// service's stable order, with evidence.
///
/// Unlike [`FacadeAreaService::measure_facade_area`], which counts steep
/// faces only, the list holds faces of every slope: a roof's top and a
/// ground slab's underside look outside too. The evidence is exact only
/// where the service states the faces of the object's exact shape, so
/// every normal box and area holds nothing but rounding; a tessellation is
/// never exact.
#[derive(Clone, Debug, PartialEq)]
pub struct FacadeFaces {
    object: ObjectId,
    faces: Vec<FacadeFace>,
    evidence: Evidence,
}

impl FacadeFaces {
    /// The outward faces of `object`; none where no face looks outside.
    ///
    /// # Errors
    ///
    /// [`FacadeAreaError::InexactEvidence`] for evidence without a locator.
    pub fn try_new(
        object: ObjectId,
        faces: Vec<FacadeFace>,
        evidence: Evidence,
    ) -> Result<Self, FacadeAreaError> {
        if evidence.locator.trim().is_empty() {
            return Err(FacadeAreaError::InexactEvidence);
        }
        Ok(Self {
            object,
            faces,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// Every face looking outside, in the service's order.
    #[must_use]
    pub fn faces(&self) -> &[FacadeFace] {
        &self.faces
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// The volume the outward faces of a set of objects enclose, in cubic
/// metres, with its evidence: the envelope of a building its walls, roofs,
/// slabs, windows and doors make together.
///
/// A service answers only where the faces close, every edge of every face
/// met by faces beyond it, and where every face looks outside wholly; the
/// interval holds the rounding (and a tessellation's chord band), and the
/// evidence is exact only where every face is of its object's exact shape.
#[derive(Clone, Debug, PartialEq)]
pub struct EnclosedVolume {
    objects: Vec<ObjectId>,
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl EnclosedVolume {
    /// The volume `objects` (sorted and deduplicated here) enclose, known
    /// to lie in `[lower, upper]`.
    ///
    /// # Errors
    ///
    /// [`FacadeAreaError::InvalidMeasurement`] for bounds not finite,
    /// negative or reversed, [`FacadeAreaError::InexactEvidence`] for
    /// evidence without a locator.
    pub fn try_new(
        mut objects: Vec<ObjectId>,
        lower: f64,
        upper: f64,
        evidence: Evidence,
    ) -> Result<Self, FacadeAreaError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        if evidence.locator.trim().is_empty() {
            return Err(FacadeAreaError::InexactEvidence);
        }
        objects.sort();
        objects.dedup();
        Ok(Self {
            objects,
            lower,
            upper,
            evidence,
        })
    }

    /// The objects whose faces enclose the volume, sorted.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }

    /// The least and greatest volume, in cubic metres.
    #[must_use]
    pub fn cubic_metres(&self) -> (f64, f64) {
        (self.lower, self.upper)
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures the outward-facing surface area of model objects.
pub trait FacadeAreaService: Send + Sync + 'static {
    /// The area of `object`'s faces that face the outside.
    // gate: measures area
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError>;
    /// The area of `object`'s largest plane face: all of its body's faces
    /// lying in one oriented plane, such as a wall's side less its openings
    /// or a slab's top. The answer is a [`FacadeArea`] of that face.
    ///
    /// The default refuses with [`FacadeAreaError::Unavailable`], so a
    /// service that does not measure faces fails closed.
    // gate: measures area
    fn measure_face_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let _ = object;
        Err(FacadeAreaError::Unavailable(
            "face areas are not measured by this service".into(),
        ))
    }

    /// Every face of `object` that looks outside, of any slope, with its
    /// outward normal, area and centroid, classified as the facade area
    /// classifies its steep faces.
    ///
    /// The default refuses with [`FacadeAreaError::Unavailable`], so a
    /// service that does not list faces fails closed.
    // gate: measures plane_angle, area
    fn measure_facade_faces(&self, object: &ObjectId) -> Result<FacadeFaces, FacadeAreaError> {
        let _ = object;
        Err(FacadeAreaError::Unavailable(
            "outward faces are not listed by this service".into(),
        ))
    }

    /// The volume the faces of `objects` looking outside enclose together,
    /// refused where they do not close or a face only partly looks
    /// outside.
    ///
    /// The default refuses with [`FacadeAreaError::Unavailable`], so a
    /// service that does not measure envelopes fails closed.
    // gate: measures volume
    fn measure_enclosed_volume(
        &self,
        objects: &[ObjectId],
    ) -> Result<EnclosedVolume, FacadeAreaError> {
        let _ = objects;
        Err(FacadeAreaError::Unavailable(
            "enclosed volumes are not measured by this service".into(),
        ))
    }
}

/// Registry handle for a [`FacadeAreaService`].
#[derive(Clone)]
pub struct FacadeAreaServiceHandle(Arc<dyn FacadeAreaService>);

impl FacadeAreaServiceHandle {
    /// Wraps a trusted facade-area service.
    #[must_use]
    pub fn new(service: Arc<dyn FacadeAreaService>) -> Self {
        Self(service)
    }

    /// The facade area of `object`. An area naming another object answers a
    /// different question and is refused.
    pub fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let area = self.0.measure_facade_area(object)?;
        if area.object() != object {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(area)
    }

    /// The area of `object`'s largest plane face. An area naming another
    /// object answers a different question and is refused.
    pub fn measure_face_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let area = self.0.measure_face_area(object)?;
        if area.object() != object {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(area)
    }

    /// Every face of `object` that looks outside. Faces naming another
    /// object answer a different question and are refused.
    pub fn measure_facade_faces(&self, object: &ObjectId) -> Result<FacadeFaces, FacadeAreaError> {
        let faces = self.0.measure_facade_faces(object)?;
        if faces.object() != object {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(faces)
    }

    /// The volume the outward faces of `objects` enclose. A volume of
    /// other objects answers a different question and is refused.
    pub fn measure_enclosed_volume(
        &self,
        objects: &[ObjectId],
    ) -> Result<EnclosedVolume, FacadeAreaError> {
        let volume = self.0.measure_enclosed_volume(objects)?;
        let mut asked = objects.to_vec();
        asked.sort();
        asked.dedup();
        if volume.objects() != asked {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(volume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    fn exact() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "facade-area:a")
    }

    #[test]
    fn exactness_and_bounds_must_agree() {
        assert!(FacadeArea::try_new(id("a"), 2.0, 2.0, exact()).is_ok());
        assert_eq!(
            FacadeArea::try_new(id("a"), 1.0, 2.0, exact()),
            Err(FacadeAreaError::InexactEvidence)
        );
        let mut approximate = exact();
        approximate.exact = false;
        assert!(FacadeArea::try_new(id("a"), 1.0, 2.0, approximate.clone()).is_ok());
        assert_eq!(
            FacadeArea::try_new(id("a"), 2.0, 2.0, approximate),
            Err(FacadeAreaError::InexactEvidence)
        );
    }

    #[test]
    fn reversed_negative_or_non_finite_bounds_are_refused() {
        for (lower, upper) in [
            (2.0, 1.0),
            (-1.0, 1.0),
            (0.0, f64::NAN),
            (0.0, f64::INFINITY),
        ] {
            assert_eq!(
                FacadeArea::try_new(id("a"), lower, upper, exact()),
                Err(FacadeAreaError::InvalidMeasurement),
                "{lower} {upper}"
            );
        }
    }

    #[test]
    fn outward_faces_need_an_area_and_evidence_a_locator() {
        let point = FaceNormal::exact([0.0, -1.0, 0.0]).unwrap();
        let boxed = FaceNormal::try_new([0.0, -1.0, -0.1], [0.0, -1.0, 0.1]).unwrap();
        let face =
            |normal, area| FacadeFace::try_new(normal, area, [0.0; 3], FacadeFaceSide::Outside);
        assert!(face(point, (2.0, 1.0)).is_err());
        assert!(face(point, (-1.0, 1.0)).is_err());
        let exact_face = face(point, (1.0, 1.0)).unwrap();
        assert!(FacadeFaces::try_new(id("a"), vec![exact_face], exact()).is_ok());
        let leaning = face(boxed, (1.0, 1.0)).unwrap();
        let mut unlocated = exact();
        unlocated.locator = " ".into();
        assert_eq!(
            FacadeFaces::try_new(id("a"), vec![leaning.clone()], unlocated),
            Err(FacadeAreaError::InexactEvidence)
        );
        assert!(FacadeFaces::try_new(id("a"), vec![leaning], exact()).is_ok());
        assert!(FacadeFaces::try_new(id("a"), vec![], exact()).is_ok());
    }

    #[test]
    fn an_enclosed_volume_is_of_the_objects_asked_about() {
        struct Fixed;
        impl FacadeAreaService for Fixed {
            fn measure_facade_area(&self, _: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
                Err(FacadeAreaError::InvalidMeasurement)
            }
            fn measure_enclosed_volume(
                &self,
                _: &[ObjectId],
            ) -> Result<EnclosedVolume, FacadeAreaError> {
                EnclosedVolume::try_new(vec![id("b"), id("a")], 1.0, 1.0, exact())
            }
        }
        assert!(EnclosedVolume::try_new(vec![id("a")], 2.0, 1.0, exact()).is_err());
        assert!(EnclosedVolume::try_new(vec![id("a")], -1.0, 1.0, exact()).is_err());
        let handle = FacadeAreaServiceHandle::new(Arc::new(Fixed));
        assert!(
            handle
                .measure_enclosed_volume(&[id("a"), id("b"), id("a")])
                .is_ok()
        );
        assert_eq!(
            handle.measure_enclosed_volume(&[id("a")]),
            Err(FacadeAreaError::InvalidMeasurement)
        );
    }

    struct Other;
    impl FacadeAreaService for Other {
        fn measure_facade_area(&self, _: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
            FacadeArea::try_new(id("b"), 1.0, 1.0, exact())
        }
    }

    #[test]
    fn an_area_of_another_object_is_refused() {
        let handle = FacadeAreaServiceHandle::new(Arc::new(Other));
        assert_eq!(
            handle.measure_facade_area(&id("a")),
            Err(FacadeAreaError::InvalidMeasurement)
        );
    }
}
