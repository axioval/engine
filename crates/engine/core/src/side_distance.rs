//! Side distances: which footprints lie beside each side of an object's
//! least-area rectangle, and how far from its centre lines.
//!
//! ADR 0004: this seam measures. Which side of a washbasin is its back, or
//! whether a WC's axis lies far enough from the wall beside it, is a rule's
//! judgement over these measurements.
//!
//! A side of the rectangle faces outward along one of its axes. Its *strip*
//! is the region beyond the rectangle's centre line (the line through the
//! centre square to the outward direction), out to the request's reach,
//! whose offset across the outward direction lies strictly within the
//! rectangle's half extent across it, less the request's inset. A candidate
//! is listed on a side when its footprint may meet the strip with positive
//! area, [`SidePresence::Sure`] when it surely does; a candidate not listed
//! on a side surely does not meet its strip. Its distance is the least
//! distance from the centre line of its footprint's part in the strip, as
//! an interval sure to hold the true value (for a possible candidate: should
//! it meet the strip at all).
//!
//! The inset is the rule's allowance for walls that stand flush with a
//! neighbouring side: the wall a component stands against touches the
//! strips of the sides beside it along their edge, and only an inset keeps
//! a measured edge from entering them.
//!
//! Only a rectangle of [`RectangleOrientation::Unique`] orientation has
//! sides of its own; any other refuses.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::plan_span::{
    PlanLength, PlanRectangle, PlanSpanError, PlanSpanService, RectangleOrientation,
};

/// One side of a [`PlanRectangle`], by the axis it faces along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RectangleSide {
    /// Facing along the first axis.
    AlongFirst,
    /// Facing along the second axis.
    AlongSecond,
    /// Facing against the first axis.
    AgainstFirst,
    /// Facing against the second axis.
    AgainstSecond,
}

impl RectangleSide {
    /// Every side, counter-clockwise from the first axis.
    pub const ALL: [Self; 4] = [
        Self::AlongFirst,
        Self::AlongSecond,
        Self::AgainstFirst,
        Self::AgainstSecond,
    ];

    /// The index of the axis the side faces along or against.
    #[must_use]
    pub fn axis(self) -> usize {
        match self {
            Self::AlongFirst | Self::AgainstFirst => 0,
            Self::AlongSecond | Self::AgainstSecond => 1,
        }
    }

    /// `1` along the axis, `-1` against it.
    #[must_use]
    pub fn sign(self) -> f64 {
        match self {
            Self::AlongFirst | Self::AlongSecond => 1.0,
            Self::AgainstFirst | Self::AgainstSecond => -1.0,
        }
    }

    /// The side facing the other way.
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::AlongFirst => Self::AgainstFirst,
            Self::AlongSecond => Self::AgainstSecond,
            Self::AgainstFirst => Self::AlongFirst,
            Self::AgainstSecond => Self::AlongSecond,
        }
    }

    /// The side's outward unit direction in plan.
    #[must_use]
    pub fn outward(self, rectangle: &PlanRectangle) -> [f64; 2] {
        rectangle.axes()[self.axis()].map(|value| self.sign() * value)
    }

    /// The side's stable name, as evidence locators cite it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AlongFirst => "+first",
            Self::AlongSecond => "+second",
            Self::AgainstFirst => "-first",
            Self::AgainstSecond => "-second",
        }
    }
}

/// The object whose rectangle's sides are measured, the candidates to find
/// beside them, how far out to look and how far to narrow each strip.
#[derive(Clone, Debug, PartialEq)]
pub struct SideDistanceRequest {
    object: ObjectId,
    candidates: Vec<ObjectId>,
    reach: f64,
    inset: f64,
}

impl SideDistanceRequest {
    /// A request for the candidates beside `object`'s sides within `reach`
    /// metres of its centre lines, each strip narrowed by `inset` metres on
    /// both edges. Candidates are sorted and deduplicated; the object
    /// itself is refused as one, and so are a reach that is not positive
    /// and finite and an inset that is negative or not finite.
    pub fn try_new(
        object: ObjectId,
        candidates: impl IntoIterator<Item = ObjectId>,
        reach: f64,
        inset: f64,
    ) -> Result<Self, PlanSpanError> {
        let mut candidates: Vec<ObjectId> = candidates.into_iter().collect();
        candidates.sort();
        candidates.dedup();
        if candidates.contains(&object) {
            return Err(PlanSpanError::Unavailable(format!(
                "{object} cannot lie beside itself"
            )));
        }
        if !(reach.is_finite() && reach > 0.0 && inset.is_finite() && inset >= 0.0) {
            return Err(PlanSpanError::Unavailable(
                "a side-distance reach must be positive and its inset non-negative".into(),
            ));
        }
        Ok(Self {
            object,
            candidates,
            reach,
            inset,
        })
    }

    /// The object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The candidates, sorted without repeats.
    #[must_use]
    pub fn candidates(&self) -> &[ObjectId] {
        &self.candidates
    }

    /// How far beyond the centre lines to look, in metres.
    #[must_use]
    pub fn reach_metres(&self) -> f64 {
        self.reach
    }

    /// How far each strip is narrowed on both edges, in metres.
    #[must_use]
    pub fn inset_metres(&self) -> f64 {
        self.inset
    }
}

/// Whether a listed candidate meets a side's strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SidePresence {
    /// Its footprint surely meets the strip with positive area.
    Sure,
    /// It may: the measurement cannot tell.
    Possible,
}

/// One candidate beside one side.
#[derive(Clone, Debug, PartialEq)]
pub struct SideDistance {
    candidate: ObjectId,
    side: RectangleSide,
    presence: SidePresence,
    distance: PlanLength,
}

impl SideDistance {
    /// `candidate` meets `side`'s strip (surely or possibly), the nearest
    /// point of its part there `distance` from the centre line.
    #[must_use]
    pub fn new(
        candidate: ObjectId,
        side: RectangleSide,
        presence: SidePresence,
        distance: PlanLength,
    ) -> Self {
        Self {
            candidate,
            side,
            presence,
            distance,
        }
    }

    /// The candidate.
    #[must_use]
    pub fn candidate(&self) -> &ObjectId {
        &self.candidate
    }

    /// The side it lies beside.
    #[must_use]
    pub fn side(&self) -> RectangleSide {
        self.side
    }

    /// Whether it surely meets the side's strip.
    #[must_use]
    pub fn presence(&self) -> SidePresence {
        self.presence
    }

    /// Its least distance from the side's centre line within the strip.
    #[must_use]
    pub fn distance(&self) -> &PlanLength {
        &self.distance
    }
}

/// The candidates beside an object's sides, with the rectangle they were
/// measured from.
#[derive(Clone, Debug, PartialEq)]
pub struct SideDistances {
    request: SideDistanceRequest,
    rectangle: PlanRectangle,
    distances: Vec<SideDistance>,
    evidence: Evidence,
}

impl SideDistances {
    /// The answer to `request` from `rectangle`.
    ///
    /// The rectangle must be the request object's, with a unique
    /// orientation; each candidate appears at most once per side and only
    /// if requested; the evidence is exact only when the rectangle and
    /// every distance are, and every candidate is surely present.
    pub fn try_new(
        request: SideDistanceRequest,
        rectangle: PlanRectangle,
        mut distances: Vec<SideDistance>,
        evidence: Evidence,
    ) -> Result<Self, PlanSpanError> {
        if rectangle.object() != request.object() {
            return Err(PlanSpanError::Unavailable(format!(
                "side distances from a rectangle of {} were returned for {}",
                rectangle.object(),
                request.object()
            )));
        }
        if rectangle.orientation() != RectangleOrientation::Unique {
            return Err(PlanSpanError::Unavailable(format!(
                "the least-area rectangle of {} is {}, so its sides are not the footprint's own",
                request.object(),
                rectangle.orientation().name()
            )));
        }
        distances.sort_by(|a, b| (a.side, &a.candidate).cmp(&(b.side, &b.candidate)));
        let repeated = distances
            .windows(2)
            .any(|pair| pair[0].side == pair[1].side && pair[0].candidate == pair[1].candidate);
        let unrequested = distances.iter().any(|distance| {
            request
                .candidates
                .binary_search(&distance.candidate)
                .is_err()
        });
        if repeated || unrequested {
            return Err(PlanSpanError::Unavailable(format!(
                "side distances of {} name a candidate twice or one not requested",
                request.object()
            )));
        }
        let exact = rectangle.is_exact()
            && distances.iter().all(|distance| {
                distance.distance.is_exact() && distance.presence == SidePresence::Sure
            });
        if (evidence.exact && !exact) || evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        Ok(Self {
            request,
            rectangle,
            distances,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &SideDistanceRequest {
        &self.request
    }

    /// The object's least-area rectangle.
    #[must_use]
    pub fn rectangle(&self) -> &PlanRectangle {
        &self.rectangle
    }

    /// Every listed candidate, by side and then candidate.
    #[must_use]
    pub fn distances(&self) -> &[SideDistance] {
        &self.distances
    }

    /// The candidates listed beside `side`.
    pub fn beside(&self, side: RectangleSide) -> impl Iterator<Item = &SideDistance> {
        self.distances
            .iter()
            .filter(move |distance| distance.side == side)
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures and checks that the answer is the request's.
pub(crate) fn measure(
    service: &Arc<dyn PlanSpanService>,
    request: &SideDistanceRequest,
) -> Result<SideDistances, PlanSpanError> {
    let answer = service.measure_side_distances(request)?;
    if answer.request() != request {
        return Err(PlanSpanError::Unavailable(format!(
            "side distances for another request were returned for {}",
            request.object()
        )));
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axioval_ir::{Evidence, ObjectId, SourceId};

    use super::{RectangleSide, SideDistance, SideDistanceRequest, SideDistances, SidePresence};
    use crate::plan_span::{
        PlanLength, PlanRectangle, PlanSpanError, PlanSpanService, PlanSpanServiceHandle,
        RectangleOrientation,
    };

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    fn evidence(exact: bool) -> Evidence {
        Evidence {
            source: SourceId::new("cad", "m").unwrap(),
            locator: "test".into(),
            exact,
        }
    }

    fn rectangle(orientation: RectangleOrientation) -> PlanRectangle {
        PlanRectangle::try_new(
            id("wc"),
            [1.2, 0.35],
            0.0,
            [[1.0, 0.0], [0.0, 1.0]],
            0.0,
            [(0.2, 0.2), (0.35, 0.35)],
            orientation,
            evidence(orientation == RectangleOrientation::Unique),
        )
        .unwrap()
    }

    fn beside(candidate: &str) -> SideDistance {
        SideDistance::new(
            id(candidate),
            RectangleSide::AgainstSecond,
            SidePresence::Sure,
            PlanLength::try_new(0.34, 0.36, evidence(false)).unwrap(),
        )
    }

    fn request() -> SideDistanceRequest {
        SideDistanceRequest::try_new(id("wc"), [id("wall")], 1.0, 0.0).unwrap()
    }

    #[test]
    fn requests_refuse_the_object_itself_and_bad_lengths() {
        assert!(SideDistanceRequest::try_new(id("wc"), [id("wc")], 1.0, 0.0).is_err());
        assert!(SideDistanceRequest::try_new(id("wc"), [], 0.0, 0.0).is_err());
        assert!(SideDistanceRequest::try_new(id("wc"), [], 1.0, -0.1).is_err());
    }

    #[test]
    fn answers_need_a_unique_rectangle_and_requested_candidates_once() {
        let unique = rectangle(RectangleOrientation::Unique);
        assert!(
            SideDistances::try_new(
                request(),
                unique.clone(),
                vec![beside("wall")],
                evidence(false)
            )
            .is_ok()
        );
        assert!(
            SideDistances::try_new(
                request(),
                rectangle(RectangleOrientation::Tied),
                vec![beside("wall")],
                evidence(false)
            )
            .is_err()
        );
        assert!(
            SideDistances::try_new(
                request(),
                unique.clone(),
                vec![beside("door")],
                evidence(false)
            )
            .is_err()
        );
        assert!(
            SideDistances::try_new(
                request(),
                unique.clone(),
                vec![beside("wall"), beside("wall")],
                evidence(false)
            )
            .is_err()
        );
        // An interval is never exact evidence.
        assert!(
            SideDistances::try_new(request(), unique, vec![beside("wall")], evidence(true))
                .is_err()
        );
        let out = RectangleSide::AgainstSecond.outward(&rectangle(RectangleOrientation::Unique));
        assert!(
            out[0].abs() < 1e-12 && (out[1] + 1.0).abs() < 1e-12,
            "{out:?}"
        );
    }

    /// Answers every request as if it had asked for a longer reach.
    struct Farther;

    impl PlanSpanService for Farther {
        fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
            Err(PlanSpanError::Unavailable("unused".into()))
        }
        fn measure_span(
            &self,
            _: &ObjectId,
            _: &ObjectId,
            _: crate::plan_span::PlanSpan,
        ) -> Result<PlanLength, PlanSpanError> {
            Err(PlanSpanError::Unavailable("unused".into()))
        }
        fn measure_side_distances(
            &self,
            request: &SideDistanceRequest,
        ) -> Result<SideDistances, PlanSpanError> {
            let farther = SideDistanceRequest::try_new(
                request.object().clone(),
                request.candidates().to_vec(),
                request.reach_metres() + 1.0,
                request.inset_metres(),
            )?;
            SideDistances::try_new(
                farther,
                rectangle(RectangleOrientation::Unique),
                Vec::new(),
                evidence(false),
            )
        }
    }

    #[test]
    fn the_handle_refuses_an_answer_to_another_request() {
        let handle = PlanSpanServiceHandle::new(Arc::new(Farther));
        assert!(matches!(
            handle.measure_side_distances(&request()),
            Err(PlanSpanError::Unavailable(message)) if message.contains("another request")
        ));
    }
}
