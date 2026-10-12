//! Sight distances along alignments: how far ahead along an alignment a
//! target stays in view from an eye at a station.
//!
//! ADR 0004: this seam measures. Whether the available sight distance
//! reaches the stopping or overtaking sight distance a design speed
//! requires is a rule's judgement over this measurement.
//!
//! # What is measured
//!
//! The eye stands in the section at the request's station
//! ([`crate::section`]): at `lateral` metres to the left of the
//! centreline and `up` metres above the gradient line. The target stands
//! in the section at the plan distance `d` further along, at its own
//! lateral offset and height. The **available sight distance** is the
//! greatest `d`, up to the request's `range`, such that for every `d'` in
//! `(0, d]` the straight sight line from the eye to the target at `d'` is
//! clear of
//!
//! - **the gradient line**, the road surface as the developed
//!   longitudinal profile states it: the sight line, drawn over plan
//!   distance against height, stays strictly above the gradient line
//!   between the eye and the target, as the closed-form crest formula
//!   takes it. Cross-fall, cant and the plan's curvature neither raise nor
//!   lower it; and
//! - **every selected body**, in space: no point of the segment from the
//!   eye to the target lies in a selected body.
//!
//! `d` is plan distance along the alignment, never a station difference.
//! Terrain enters only as a selected body: a model whose ground is no
//! body states no terrain, and the measurement then says nothing about
//! it. Its evidence names what it was measured against.
//!
//! # An interval
//!
//! The value is an interval sure to hold the available sight distance:
//! the sight line is proven clear for every target up to its lower bound,
//! and proven blocked at its upper bound, which is `range` when no
//! blocked target was found before it (the measure is capped at `range`
//! by definition). An obstruction that can be neither proven clear of the
//! sight line nor proven to block it leaves the interval open between
//! the two; a sight line is never called clear that was not proven so.

use axioval_ir::{Evidence, ObjectId};

use crate::alignment::{AlignmentError, AlignmentInterval};

/// A question about how far ahead along an alignment a target stays in
/// view (module documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct SightDistanceRequest {
    alignment: ObjectId,
    station: f64,
    eye: [f64; 2],
    target: [f64; 2],
    range: f64,
    step: f64,
    obstacles: Vec<ObjectId>,
}

impl SightDistanceRequest {
    /// From the eye at `station` of `alignment`, at `eye` `(lateral, up)`
    /// in the section, to the target at `target` `(lateral, up)` in the
    /// section further along, looking at most `range` metres of plan
    /// distance ahead, with sampled targets at most `step` apart, past
    /// `obstacles`, which are sorted and deduplicated.
    ///
    /// # Errors
    ///
    /// [`AlignmentError::InvalidMeasurement`] for a value that is not
    /// finite, an eye or target not above the gradient line, a range or
    /// step that is not positive, or the alignment among its own
    /// obstacles.
    pub fn try_new(
        alignment: ObjectId,
        station: f64,
        eye: [f64; 2],
        target: [f64; 2],
        (range, step): (f64, f64),
        mut obstacles: Vec<ObjectId>,
    ) -> Result<Self, AlignmentError> {
        let finite = [station, eye[0], eye[1], target[0], target[1], range, step]
            .iter()
            .all(|value| value.is_finite());
        if !finite || eye[1] <= 0.0 || target[1] <= 0.0 || range <= 0.0 || step <= 0.0 {
            return Err(AlignmentError::InvalidMeasurement);
        }
        obstacles.sort();
        obstacles.dedup();
        if obstacles.binary_search(&alignment).is_ok() {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            alignment,
            station,
            eye,
            target,
            range,
            step,
            obstacles,
        })
    }

    /// The alignment looked along.
    #[must_use]
    pub fn alignment(&self) -> &ObjectId {
        &self.alignment
    }

    /// The station the eye stands at.
    #[must_use]
    pub fn station(&self) -> f64 {
        self.station
    }

    /// The eye in the section, `(lateral, up)` in metres.
    #[must_use]
    pub fn eye(&self) -> [f64; 2] {
        self.eye
    }

    /// The target in the section, `(lateral, up)` in metres.
    #[must_use]
    pub fn target(&self) -> [f64; 2] {
        self.target
    }

    /// How far ahead, in metres of plan distance, the target is looked
    /// for; the measure is capped there.
    #[must_use]
    pub fn range(&self) -> f64 {
        self.range
    }

    /// The greatest plan distance between two sampled targets, in metres.
    #[must_use]
    pub fn step(&self) -> f64 {
        self.step
    }

    /// The bodies that may block the sight line, sorted and unique.
    #[must_use]
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
}

/// What was proven to block the sight line at a sight distance's upper
/// bound.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SightObstruction {
    /// The gradient line rises above the sight line.
    GradientLine,
    /// A selected body.
    Body(ObjectId),
}

/// The answer to a [`SightDistanceRequest`] (module documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct SightDistance {
    request: SightDistanceRequest,
    distance: AlignmentInterval,
    blocked_by: Vec<SightObstruction>,
    evidence: Evidence,
}

impl SightDistance {
    /// The available sight distance, sure to lie in `distance` (metres of
    /// plan distance), with what blocks the sight line at its upper bound
    /// (sorted here).
    ///
    /// # Errors
    ///
    /// [`AlignmentError::InvalidMeasurement`] for a distance outside
    /// `[0, range]`, or an upper bound below the range that names nothing
    /// blocking there; [`AlignmentError::InexactEvidence`] for evidence
    /// that is exact while the distance is no point, or that has no
    /// locator.
    pub fn try_new(
        request: SightDistanceRequest,
        distance: AlignmentInterval,
        mut blocked_by: Vec<SightObstruction>,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        if distance.lower() < 0.0 || distance.upper() > request.range {
            return Err(AlignmentError::InvalidMeasurement);
        }
        blocked_by.sort();
        blocked_by.dedup();
        if distance.upper() < request.range && blocked_by.is_empty() {
            return Err(AlignmentError::InvalidMeasurement);
        }
        if (evidence.exact && !distance.is_exact()) || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            distance,
            blocked_by,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &SightDistanceRequest {
        &self.request
    }

    /// Bounds on the available sight distance, in metres of plan distance.
    #[must_use]
    pub fn distance(&self) -> AlignmentInterval {
        self.distance
    }

    /// What was proven to block the sight line at the upper bound, sorted;
    /// empty where the upper bound is the range.
    #[must_use]
    pub fn blocked_by(&self) -> &[SightObstruction] {
        &self.blocked_by
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "road").unwrap(), local).unwrap()
    }

    fn request(obstacles: Vec<ObjectId>) -> Result<SightDistanceRequest, AlignmentError> {
        SightDistanceRequest::try_new(
            id("axis"),
            1000.0,
            [1.75, 1.0],
            [1.75, 0.15],
            (300.0, 5.0),
            obstacles,
        )
    }

    fn evidence(exact: bool) -> Evidence {
        let mut evidence = Evidence::exact(SourceId::new("cad", "road").unwrap(), "sight");
        evidence.exact = exact;
        evidence
    }

    fn interval(lower: f64, upper: f64) -> AlignmentInterval {
        AlignmentInterval::try_new(lower, upper).unwrap()
    }

    #[test]
    fn a_request_needs_heights_above_the_line_and_a_range_and_step() {
        let made = request(vec![id("b"), id("a"), id("b")]).unwrap();
        assert_eq!(made.obstacles(), [id("a"), id("b")]);
        assert!(request(vec![id("axis")]).is_err());
        for (eye, target, range, step) in [
            ([0.0, 0.0], [0.0, 1.0], 10.0, 1.0),
            ([0.0, 1.0], [0.0, -1.0], 10.0, 1.0),
            ([0.0, 1.0], [0.0, 1.0], 0.0, 1.0),
            ([0.0, 1.0], [0.0, 1.0], 10.0, 0.0),
            ([f64::NAN, 1.0], [0.0, 1.0], 10.0, 1.0),
        ] {
            assert!(
                SightDistanceRequest::try_new(id("axis"), 0.0, eye, target, (range, step), vec![])
                    .is_err()
            );
        }
    }

    #[test]
    fn an_answer_stays_within_the_range_and_names_what_blocks_before_it() {
        let made = request(vec![id("wall")]).unwrap();
        let answer = |distance, blocked_by, exact| {
            SightDistance::try_new(made.clone(), distance, blocked_by, evidence(exact))
        };
        assert!(answer(interval(0.0, 301.0), vec![], false).is_err());
        assert!(answer(interval(90.0, 91.0), vec![], false).is_err());
        assert!(
            answer(
                interval(90.0, 91.0),
                vec![SightObstruction::GradientLine],
                true
            )
            .is_err()
        );
        let found = answer(
            interval(90.0, 91.0),
            vec![
                SightObstruction::Body(id("wall")),
                SightObstruction::GradientLine,
            ],
            false,
        )
        .unwrap();
        assert_eq!(found.blocked_by()[0], SightObstruction::GradientLine);
        assert!(answer(interval(300.0, 300.0), vec![], true).is_ok());
    }

    #[test]
    fn the_handle_refuses_a_body_not_asked_about_and_defaults_refuse() {
        use crate::alignment::AlignmentServiceHandle;
        use crate::alignment::{AlignmentPosition, AlignmentRequest, AlignmentService};
        use std::sync::Arc;

        struct Answering(Option<ObjectId>);
        impl AlignmentService for Answering {
            fn measure_alignment_position(
                &self,
                request: &AlignmentRequest,
            ) -> Result<AlignmentPosition, AlignmentError> {
                Err(AlignmentError::UnknownObject(request.object().clone()))
            }

            fn measure_sight_distance(
                &self,
                request: &SightDistanceRequest,
            ) -> Result<SightDistance, AlignmentError> {
                let Some(body) = &self.0 else {
                    return Err(AlignmentError::Unavailable("none".into()));
                };
                SightDistance::try_new(
                    request.clone(),
                    interval(90.0, 91.0),
                    vec![SightObstruction::Body(body.clone())],
                    evidence(false),
                )
            }
        }
        struct Silent;
        impl AlignmentService for Silent {
            fn measure_alignment_position(
                &self,
                request: &AlignmentRequest,
            ) -> Result<AlignmentPosition, AlignmentError> {
                Err(AlignmentError::UnknownObject(request.object().clone()))
            }
        }
        let made = request(vec![id("wall")]).unwrap();
        let handle = |answer| AlignmentServiceHandle::new(Arc::new(Answering(answer)));
        assert!(
            handle(Some(id("wall")))
                .measure_sight_distance(&made)
                .is_ok()
        );
        assert_eq!(
            handle(Some(id("tree"))).measure_sight_distance(&made),
            Err(AlignmentError::InvalidMeasurement)
        );
        assert!(matches!(
            AlignmentServiceHandle::new(Arc::new(Silent)).measure_sight_distance(&made),
            Err(AlignmentError::Unavailable(_))
        ));
    }
}
