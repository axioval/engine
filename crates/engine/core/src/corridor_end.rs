//! Corridor ends: where the paths through a space's footprint end, which
//! wall each end runs into, and how the requested subjects sit against it.
//!
//! ADR 0004: this seam measures. Whether a window in the end wall of a
//! corridor is allowed is a rule's judgement over these measurements.
//!
//! A footprint's paths are its skeleton, an approximation of its medial axis:
//! the ends, their positions and the direction each path runs in are not
//! certified, so [`CorridorEnds`] always carries approximate evidence. What is
//! certified is stated per part: every end lies inside the footprint, its
//! clearance (distance to the nearest wall) is an interval sure to hold the
//! true value, and a decided end wall is a segment of the footprint's own
//! boundary. A service decides an end's wall only when the approximation
//! cannot change which wall it is; otherwise the wall is
//! [`EndWall::Undecided`] and a rule must not guess it.
//!
//! How a subject sits against a decided wall is measured on the footprints
//! themselves, as [`PlanLength`] intervals exact exactly when a point: the
//! plan gap between the subject's footprint and the wall segment, and the
//! length of the segment the subject's footprint faces (the overlap of its
//! projection onto the wall's line with the segment).

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::plan_span::{PlanLength, PlanSpanError, PlanSpanService};

/// The space whose corridor ends are asked for, and the subjects (such as
/// its windows) to measure against each end wall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorridorEndRequest {
    space: ObjectId,
    subjects: Vec<ObjectId>,
}

impl CorridorEndRequest {
    /// A request for `space`'s corridor ends, measuring `subjects` against
    /// each end wall. Subjects are sorted and deduplicated; the space
    /// itself is refused as a subject.
    pub fn try_new(
        space: ObjectId,
        subjects: impl IntoIterator<Item = ObjectId>,
    ) -> Result<Self, PlanSpanError> {
        let mut subjects: Vec<ObjectId> = subjects.into_iter().collect();
        subjects.sort();
        subjects.dedup();
        if subjects.contains(&space) {
            return Err(PlanSpanError::Unavailable(format!(
                "{space} cannot be measured against its own corridor ends"
            )));
        }
        Ok(Self { space, subjects })
    }

    /// The space.
    #[must_use]
    pub fn space(&self) -> &ObjectId {
        &self.space
    }

    /// The subjects, sorted without repeats.
    #[must_use]
    pub fn subjects(&self) -> &[ObjectId] {
        &self.subjects
    }
}

/// How one subject sits against an end wall.
#[derive(Clone, Debug, PartialEq)]
pub struct WallContact {
    subject: ObjectId,
    gap: PlanLength,
    facing: PlanLength,
}

impl WallContact {
    /// `subject`'s footprint lies `gap` from the wall segment in plan and
    /// faces `facing` of its length.
    #[must_use]
    pub fn new(subject: ObjectId, gap: PlanLength, facing: PlanLength) -> Self {
        Self {
            subject,
            gap,
            facing,
        }
    }

    /// The subject.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// The plan distance between the subject's footprint and the wall
    /// segment: zero where they meet.
    #[must_use]
    pub fn gap(&self) -> &PlanLength {
        &self.gap
    }

    /// The length of the wall segment the subject's footprint faces: the
    /// overlap of its projection onto the wall's line with the segment.
    /// Zero for a subject beside the wall's end, such as a window in a side
    /// wall next to the corner.
    #[must_use]
    pub fn facing(&self) -> &PlanLength {
        &self.facing
    }
}

/// The wall a corridor end runs into.
#[derive(Clone, Debug, PartialEq)]
pub enum EndWall {
    /// A straight run of the footprint's boundary, from `start` to `end` in
    /// canonical metres, and every requested subject measured against it in
    /// the request's order.
    Decided {
        /// The segment's first point.
        start: [f64; 2],
        /// The segment's last point.
        end: [f64; 2],
        /// The requested subjects against this wall.
        contacts: Vec<WallContact>,
    },
    /// The approximation could change which wall the end runs into, or the
    /// path does not stop at one; the reason says which.
    Undecided(String),
}

/// One end of a path through the footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct CorridorEnd {
    point: [f64; 2],
    clearance: (f64, f64),
    wall: EndWall,
}

impl CorridorEnd {
    /// An end at `point` (inside the footprint), whose distance to the
    /// nearest wall lies in `clearance`, running into `wall`.
    pub fn try_new(
        point: [f64; 2],
        clearance: (f64, f64),
        wall: EndWall,
    ) -> Result<Self, PlanSpanError> {
        let (lower, upper) = clearance;
        if !point.iter().all(|value| value.is_finite())
            || !lower.is_finite()
            || !upper.is_finite()
            || lower < 0.0
            || lower > upper
        {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        if let EndWall::Decided { start, end, .. } = &wall {
            #[allow(clippy::float_cmp)]
            let degenerate = start == end;
            if degenerate || !start.iter().chain(end).all(|value| value.is_finite()) {
                return Err(PlanSpanError::InvalidMeasurement);
            }
        }
        Ok(Self {
            point,
            clearance,
            wall,
        })
    }

    /// Where the path ends, in canonical metres: inside the footprint, but
    /// an approximation of where the true axis ends.
    #[must_use]
    pub fn point(&self) -> [f64; 2] {
        self.point
    }

    /// Bounds `(lower, upper)` on the distance from [`Self::point`] to the
    /// nearest wall, sure to hold the true value.
    #[must_use]
    pub fn clearance_metres(&self) -> (f64, f64) {
        self.clearance
    }

    /// The wall the path runs into.
    #[must_use]
    pub fn wall(&self) -> &EndWall {
        &self.wall
    }
}

/// The corridor ends of a space's footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct CorridorEnds {
    space: ObjectId,
    ends: Vec<CorridorEnd>,
    evidence: Evidence,
}

impl CorridorEnds {
    /// The ends of `space`'s footprint. The evidence is always approximate:
    /// the skeleton the ends come from is not certified.
    pub fn try_new(
        space: ObjectId,
        ends: Vec<CorridorEnd>,
        evidence: Evidence,
    ) -> Result<Self, PlanSpanError> {
        if evidence.exact || evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        Ok(Self {
            space,
            ends,
            evidence,
        })
    }

    /// The space whose ends these are.
    #[must_use]
    pub fn space(&self) -> &ObjectId {
        &self.space
    }

    /// The ends, in the service's order.
    #[must_use]
    pub fn ends(&self) -> &[CorridorEnd] {
        &self.ends
    }

    /// Reviewable provenance of the skeleton.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Checks that `ends` answers `request`: the same space, and every decided
/// wall measured against exactly the requested subjects, in order.
pub(crate) fn bound(
    request: &CorridorEndRequest,
    ends: CorridorEnds,
) -> Result<CorridorEnds, PlanSpanError> {
    if ends.space() != request.space() {
        return Err(PlanSpanError::Unavailable(format!(
            "corridor ends of {} were returned for {}",
            ends.space(),
            request.space()
        )));
    }
    for end in ends.ends() {
        if let EndWall::Decided { contacts, .. } = end.wall() {
            let named = contacts.iter().map(WallContact::subject);
            if !named.eq(request.subjects()) {
                return Err(PlanSpanError::Unavailable(format!(
                    "an end wall of {} was not measured against exactly the requested subjects",
                    request.space()
                )));
            }
        }
    }
    Ok(ends)
}

/// Asks `service` for `request`'s corridor ends and binds the answer to it.
pub(crate) fn measure(
    service: &Arc<dyn PlanSpanService>,
    request: &CorridorEndRequest,
) -> Result<CorridorEnds, PlanSpanError> {
    bound(request, service.measure_corridor_ends(request)?)
}

#[cfg(test)]
mod tests {
    use axioval_ir::{Evidence, ObjectId, SourceId};

    use super::{CorridorEnd, CorridorEndRequest, CorridorEnds, EndWall, WallContact, bound};
    use crate::plan_span::{PlanLength, PlanSpanError};

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn approximate() -> Evidence {
        Evidence {
            source: source(),
            locator: "corridor-ends:hall".into(),
            exact: false,
        }
    }

    fn zero() -> PlanLength {
        PlanLength::try_new(0.0, 0.0, Evidence::exact(source(), "gap")).unwrap()
    }

    fn end(contacts: Vec<WallContact>) -> CorridorEnd {
        CorridorEnd::try_new(
            [0.5, 0.5],
            (0.5, 0.5),
            EndWall::Decided {
                start: [0.0, 1.0],
                end: [0.0, 0.0],
                contacts,
            },
        )
        .unwrap()
    }

    #[test]
    fn a_request_sorts_its_subjects_and_refuses_the_space() {
        let request =
            CorridorEndRequest::try_new(id("hall"), [id("w2"), id("w1"), id("w2")]).unwrap();
        assert_eq!(request.subjects(), [id("w1"), id("w2")]);
        assert!(CorridorEndRequest::try_new(id("hall"), [id("hall")]).is_err());
    }

    #[test]
    fn corridor_ends_are_never_exact() {
        let exact = Evidence::exact(source(), "corridor-ends:hall");
        assert_eq!(
            CorridorEnds::try_new(id("hall"), Vec::new(), exact),
            Err(PlanSpanError::InexactEvidence)
        );
        assert!(CorridorEnds::try_new(id("hall"), Vec::new(), approximate()).is_ok());
    }

    #[test]
    fn an_end_refuses_bad_clearances_and_degenerate_walls() {
        let wall = || EndWall::Undecided("unused".into());
        assert!(CorridorEnd::try_new([0.0, 0.0], (0.5, 0.4), wall()).is_err());
        assert!(CorridorEnd::try_new([0.0, 0.0], (-0.1, 0.4), wall()).is_err());
        assert!(CorridorEnd::try_new([f64::NAN, 0.0], (0.1, 0.4), wall()).is_err());
        assert!(
            CorridorEnd::try_new(
                [0.0, 0.0],
                (0.1, 0.1),
                EndWall::Decided {
                    start: [1.0, 1.0],
                    end: [1.0, 1.0],
                    contacts: Vec::new()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn an_answer_is_bound_to_its_space_and_subjects() {
        let request = CorridorEndRequest::try_new(id("hall"), [id("w1")]).unwrap();
        let answer = |space: &str, contacts| {
            CorridorEnds::try_new(id(space), vec![end(contacts)], approximate()).unwrap()
        };
        let contact = |subject: &str| WallContact::new(id(subject), zero(), zero());
        assert!(bound(&request, answer("hall", vec![contact("w1")])).is_ok());
        assert!(bound(&request, answer("room", vec![contact("w1")])).is_err());
        assert!(bound(&request, answer("hall", vec![contact("w2")])).is_err());
        assert!(bound(&request, answer("hall", Vec::new())).is_err());
        assert!(bound(&request, answer("hall", vec![contact("w1"), contact("w1")])).is_err());
    }
}
