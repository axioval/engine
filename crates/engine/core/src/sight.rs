//! Lines of sight: whether any part of a target is in view from an eye
//! point, past a stated set of blockers.
//!
//! ADR 0004: this seam proves what can be seen. How many targets must be in
//! view, from which eye and within what radius, is a rule's judgement.
//!
//! Each answer is one of three, and only two of them are proofs. **Visible**
//! names a witness: a point of the target the straight segment from the eye
//! reaches before it meets any blocker. **Hidden** names the occluders that
//! together cover every ray from the eye to the target. **Undecided** is
//! neither, never a guess: a target only grazed, or covered only where
//! several blockers meet.
//!
//! Blockers are the request's (the rule's selection), never the host's: an
//! answer naming an occluder the request did not send is refused.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to assess a line of sight.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SightError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The request is malformed: a non-finite eye, a negative range, or the
    /// target among its own blockers.
    #[error("invalid sight request: {0}")]
    InvalidRequest(String),
    /// The geometry could not be assessed, for example a blocker whose body
    /// was not measured or a tessellated target.
    #[error("line of sight unavailable: {0}")]
    Unavailable(String),
    /// The answer does not fit the request: another target, an occluder the
    /// request did not send, a witness that is not finite, or a distance
    /// whose bounds are reversed.
    #[error("line-of-sight evidence is invalid: {0}")]
    InvalidEvidence(String),
}

/// A question about one target: is any part of it in view from `eye`?
#[derive(Clone, Debug, PartialEq)]
pub struct SightRequest {
    eye: [f64; 3],
    target: ObjectId,
    blockers: Vec<ObjectId>,
    within: Option<f64>,
}

impl SightRequest {
    /// A line of sight from `eye` (canonical metres, the source's coordinate
    /// system) to `target`, past `blockers`, which are sorted and
    /// deduplicated.
    ///
    /// With `within`, the service measures the distance first and does not
    /// look at a target surely farther than that from the eye.
    ///
    /// # Errors
    ///
    /// [`SightError::InvalidRequest`] for a non-finite eye, a range that is
    /// negative or not finite, or a target among its blockers.
    pub fn try_new(
        eye: [f64; 3],
        target: ObjectId,
        mut blockers: Vec<ObjectId>,
        within: Option<f64>,
    ) -> Result<Self, SightError> {
        if !eye.iter().all(|value| value.is_finite()) {
            return Err(SightError::InvalidRequest(
                "the eye is not a finite point".into(),
            ));
        }
        if within.is_some_and(|range| !range.is_finite() || range < 0.0) {
            return Err(SightError::InvalidRequest(
                "the range is not a non-negative length".into(),
            ));
        }
        blockers.sort();
        blockers.dedup();
        if blockers.binary_search(&target).is_ok() {
            return Err(SightError::InvalidRequest(format!(
                "{target} cannot block the view of itself"
            )));
        }
        Ok(Self {
            eye,
            target,
            blockers,
            within,
        })
    }

    /// The eye point, in canonical metres.
    #[must_use]
    pub fn eye(&self) -> [f64; 3] {
        self.eye
    }

    /// The object looked at.
    #[must_use]
    pub fn target(&self) -> &ObjectId {
        &self.target
    }

    /// The objects that may block the view, sorted and unique.
    #[must_use]
    pub fn blockers(&self) -> &[ObjectId] {
        &self.blockers
    }

    /// The range beyond which the target is not looked at.
    #[must_use]
    pub fn within_metres(&self) -> Option<f64> {
        self.within
    }
}

/// What was proven about the view of one target.
#[derive(Clone, Debug, PartialEq)]
pub enum SightOutcome {
    /// The segment from the eye to `through`, a point of the target, meets
    /// no blocker before it reaches the target.
    Visible {
        /// The witness point on the target, in canonical metres.
        through: [f64; 3],
    },
    /// Every ray from the eye to the target meets one of `occluders` first.
    Hidden {
        /// The blockers the proof used, sorted and unique, never empty.
        occluders: Vec<ObjectId>,
    },
    /// Neither could be proven.
    Undecided,
}

/// The answer to a [`SightRequest`].
#[derive(Clone, Debug, PartialEq)]
pub struct SightEvidence {
    target: ObjectId,
    distance: (f64, f64),
    outcome: Option<SightOutcome>,
    evidence: Evidence,
}

impl SightEvidence {
    /// The view of `target`: the distance from the eye to its nearest point,
    /// known to lie in `distance` (metres, inclusive), and the outcome, which
    /// is `None` only when the target lies surely beyond the request's range.
    ///
    /// # Errors
    ///
    /// [`SightError::InvalidEvidence`] for reversed, negative or non-finite
    /// distance bounds, a witness that is not finite, a hidden target with no
    /// occluders, or evidence without a locator.
    pub fn try_new(
        target: ObjectId,
        distance: (f64, f64),
        mut outcome: Option<SightOutcome>,
        evidence: Evidence,
    ) -> Result<Self, SightError> {
        let (lower, upper) = distance;
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(SightError::InvalidEvidence(
                "the distance bounds are not an interval of lengths".into(),
            ));
        }
        match &mut outcome {
            Some(SightOutcome::Visible { through }) if !through.iter().all(|v| v.is_finite()) => {
                return Err(SightError::InvalidEvidence(
                    "the witness is not a finite point".into(),
                ));
            }
            Some(SightOutcome::Hidden { occluders }) => {
                occluders.sort();
                occluders.dedup();
                if occluders.is_empty() {
                    return Err(SightError::InvalidEvidence(
                        "a hidden target names no occluder".into(),
                    ));
                }
            }
            _ => {}
        }
        if evidence.locator.trim().is_empty() {
            return Err(SightError::InvalidEvidence(
                "the evidence has no locator".into(),
            ));
        }
        Ok(Self {
            target,
            distance,
            outcome,
            evidence,
        })
    }

    /// The object looked at.
    #[must_use]
    pub fn target(&self) -> &ObjectId {
        &self.target
    }

    /// Bounds on the distance from the eye to the target's nearest point, in
    /// metres.
    #[must_use]
    pub fn distance_metres(&self) -> (f64, f64) {
        self.distance
    }

    /// The outcome, or `None` for a target surely beyond the range.
    #[must_use]
    pub fn outcome(&self) -> Option<&SightOutcome> {
        self.outcome.as_ref()
    }

    /// Reviewable provenance of the assessment.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Assesses lines of sight between an eye point and model objects.
pub trait SightService: Send + Sync + 'static {
    /// Whether any part of the request's target is in view from its eye.
    fn assess_sight(&self, request: &SightRequest) -> Result<SightEvidence, SightError>;
}

/// Registry handle for a [`SightService`].
#[derive(Clone)]
pub struct SightServiceHandle(Arc<dyn SightService>);

impl SightServiceHandle {
    /// Wraps a trusted line-of-sight service.
    #[must_use]
    pub fn new(service: Arc<dyn SightService>) -> Self {
        Self(service)
    }

    /// The view of the request's target.
    ///
    /// An answer about another target, naming an occluder the request did
    /// not send, or skipping a target that may lie within the range, is
    /// refused.
    pub fn assess_sight(&self, request: &SightRequest) -> Result<SightEvidence, SightError> {
        let answer = self.0.assess_sight(request)?;
        if answer.target() != request.target() {
            return Err(SightError::InvalidEvidence(format!(
                "an answer about {} was returned for {}",
                answer.target(),
                request.target()
            )));
        }
        if let Some(SightOutcome::Hidden { occluders }) = answer.outcome() {
            if let Some(stranger) = occluders
                .iter()
                .find(|occluder| request.blockers().binary_search(occluder).is_err())
            {
                return Err(SightError::InvalidEvidence(format!(
                    "{stranger} was not among the requested blockers"
                )));
            }
        }
        if answer.outcome().is_none()
            && !request
                .within_metres()
                .is_some_and(|range| answer.distance_metres().0 > range)
        {
            return Err(SightError::InvalidEvidence(format!(
                "{} may lie within range but was not looked at",
                request.target()
            )));
        }
        Ok(answer)
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
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "sight:a")
    }

    struct Answer(Option<SightOutcome>, (f64, f64), ObjectId);

    impl SightService for Answer {
        fn assess_sight(&self, _: &SightRequest) -> Result<SightEvidence, SightError> {
            SightEvidence::try_new(self.2.clone(), self.1, self.0.clone(), evidence())
        }
    }

    fn handle(
        outcome: Option<SightOutcome>,
        distance: (f64, f64),
        target: &str,
    ) -> SightServiceHandle {
        SightServiceHandle::new(Arc::new(Answer(outcome, distance, id(target))))
    }

    #[test]
    fn a_request_refuses_a_bad_eye_range_or_a_self_blocking_target() {
        assert!(SightRequest::try_new([0.0, f64::NAN, 0.0], id("t"), vec![], None).is_err());
        assert!(SightRequest::try_new([0.0; 3], id("t"), vec![], Some(-1.0)).is_err());
        assert!(SightRequest::try_new([0.0; 3], id("t"), vec![id("t")], None).is_err());
        let request = SightRequest::try_new(
            [0.0; 3],
            id("t"),
            vec![id("b"), id("a"), id("b")],
            Some(2.0),
        )
        .unwrap();
        assert_eq!(request.blockers(), [id("a"), id("b")]);
    }

    #[test]
    fn evidence_needs_an_interval_a_finite_witness_and_occluders() {
        let hidden = |occluders| Some(SightOutcome::Hidden { occluders });
        assert!(SightEvidence::try_new(id("t"), (2.0, 1.0), None, evidence()).is_err());
        assert!(SightEvidence::try_new(id("t"), (1.0, 1.0), hidden(vec![]), evidence()).is_err());
        let witness = Some(SightOutcome::Visible {
            through: [f64::INFINITY, 0.0, 0.0],
        });
        assert!(SightEvidence::try_new(id("t"), (1.0, 1.0), witness, evidence()).is_err());
        let sorted = SightEvidence::try_new(
            id("t"),
            (1.0, 1.0),
            hidden(vec![id("b"), id("a")]),
            evidence(),
        )
        .unwrap();
        assert_eq!(
            sorted.outcome(),
            Some(&SightOutcome::Hidden {
                occluders: vec![id("a"), id("b")]
            })
        );
    }

    #[test]
    fn the_handle_refuses_strangers_other_targets_and_unlooked_targets_in_range() {
        let request = SightRequest::try_new([0.0; 3], id("t"), vec![id("a")], Some(2.0)).unwrap();
        let stranger = Some(SightOutcome::Hidden {
            occluders: vec![id("z")],
        });
        assert!(
            handle(stranger, (1.0, 1.0), "t")
                .assess_sight(&request)
                .is_err()
        );
        assert!(
            handle(Some(SightOutcome::Undecided), (1.0, 1.0), "u")
                .assess_sight(&request)
                .is_err()
        );
        assert!(
            handle(None, (1.5, 2.5), "t")
                .assess_sight(&request)
                .is_err()
        );
        assert!(handle(None, (2.5, 3.0), "t").assess_sight(&request).is_ok());
        let known = Some(SightOutcome::Hidden {
            occluders: vec![id("a")],
        });
        assert!(
            handle(known, (1.0, 1.0), "t")
                .assess_sight(&request)
                .is_ok()
        );
    }
}
