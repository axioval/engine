//! Effective coverage: how much of a footprint the union of several sources'
//! effect areas covers.
//!
//! ADR 0004: this seam measures the covered area. Which sources count, how
//! far they reach and what share must be covered are a rule's judgement.
//!
//! Every source has an **effect area** in plan, of one [`EffectReach`] for
//! the whole request: its footprint grown by the range, the points within
//! the range of travel from its centre, or the points it sees from its
//! centre within the range. The covered area is the union of the effect
//! areas clipped to the subject's footprint, reported as an interval: its
//! lower bound comes from inner bounds of the effects of *certain* sources
//! only, its upper bound from outer bounds of every effect. An effect that
//! could not be measured leaves the upper bound at the whole footprint.
//!
//! Travel and sight stay within the subject's **free region**: its footprint
//! less the footprints of the blockers. Blockers the request marks uncertain
//! narrow only the inner bounds, since they may be absent.

use axioval_ir::{Evidence, ObjectId};

use crate::{PlanArea, PlanAreaError};

/// How far one source's effect reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectReach {
    /// The source's footprint grown by the range in every plan direction.
    Grown,
    /// The points of the free region whose shortest path within it from
    /// the source's footprint centre is no longer than the range.
    Travel,
    /// The points of the free region the source's footprint centre sees
    /// within it, no farther than the range.
    Visible,
}

impl EffectReach {
    /// The reach's name, as a rule states it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grown => "grown",
            Self::Travel => "travel",
            Self::Visible => "visible",
        }
    }
}

/// An object taking part in a coverage request, and whether it surely does.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Participant {
    object: ObjectId,
    certain: bool,
}

impl Participant {
    /// `object`, which surely takes part when `certain` and may otherwise.
    #[must_use]
    pub fn new(object: ObjectId, certain: bool) -> Self {
        Self { object, certain }
    }

    /// The participating object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// Whether it surely takes part.
    #[must_use]
    pub fn is_certain(&self) -> bool {
        self.certain
    }
}

/// A question about one subject: how much of its footprint the sources'
/// effect areas cover.
#[derive(Clone, Debug, PartialEq)]
pub struct CoverageRequest {
    subject: ObjectId,
    reach: EffectReach,
    range: f64,
    sources: Vec<Participant>,
    blockers: Vec<Participant>,
}

impl CoverageRequest {
    /// Coverage of `subject` by `sources`, each reaching `range_metres` as
    /// `reach` says, travel and sight avoiding `blockers`.
    ///
    /// Sources and blockers are sorted by object; an object listed twice
    /// keeps its certain listing.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::Unavailable`] for a range that is negative or not
    /// finite, or an object that is both subject, source or blocker.
    pub fn try_new(
        subject: ObjectId,
        reach: EffectReach,
        range_metres: f64,
        sources: Vec<Participant>,
        blockers: Vec<Participant>,
    ) -> Result<Self, PlanAreaError> {
        if !range_metres.is_finite() || range_metres < 0.0 {
            return Err(PlanAreaError::Unavailable(format!(
                "a range of {range_metres} m is not a non-negative length"
            )));
        }
        let sources = merged(sources);
        let blockers = merged(blockers);
        let named = |list: &[Participant], object: &ObjectId| {
            list.iter()
                .any(|participant| participant.object() == object)
        };
        if named(&sources, &subject) || named(&blockers, &subject) {
            return Err(PlanAreaError::Unavailable(format!(
                "{subject} cannot cover or block its own footprint"
            )));
        }
        if let Some(both) = sources
            .iter()
            .find(|source| named(&blockers, source.object()))
        {
            return Err(PlanAreaError::Unavailable(format!(
                "{} is both a source and a blocker",
                both.object()
            )));
        }
        Ok(Self {
            subject,
            reach,
            range: range_metres,
            sources,
            blockers,
        })
    }

    /// The object whose footprint is covered.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// How every source's effect reaches.
    #[must_use]
    pub fn reach(&self) -> EffectReach {
        self.reach
    }

    /// How far, in metres.
    #[must_use]
    pub fn range_metres(&self) -> f64 {
        self.range
    }

    /// The sources, sorted by object.
    #[must_use]
    pub fn sources(&self) -> &[Participant] {
        &self.sources
    }

    /// The blockers travel and sight avoid, sorted by object.
    #[must_use]
    pub fn blockers(&self) -> &[Participant] {
        &self.blockers
    }
}

/// Sorted by object, one entry per object, certain where any listing is.
fn merged(mut list: Vec<Participant>) -> Vec<Participant> {
    list.sort_by(|a, b| a.object.cmp(&b.object).then(b.certain.cmp(&a.certain)));
    list.dedup_by(|later, kept| later.object == kept.object);
    list
}

/// Whether one source's effect area meets the subject's footprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectMeets {
    /// Its inner bound covers part of the footprint.
    Surely,
    /// Its outer bound reaches the footprint, its inner bound does not.
    Possibly,
    /// Its outer bound misses the footprint.
    No,
    /// It could not be measured, for the reason given.
    Unmeasured(String),
}

/// The answer to a [`CoverageRequest`].
#[derive(Clone, Debug, PartialEq)]
pub struct CoverageEvidence {
    subject: ObjectId,
    footprint: PlanArea,
    covered: PlanArea,
    effects: Vec<(ObjectId, EffectMeets)>,
}

impl CoverageEvidence {
    /// The subject's `footprint`, the `covered` part of it and, per source
    /// in request order, whether its effect meets the footprint.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::InvalidMeasurement`] for a covered area that may
    /// exceed the footprint.
    pub fn try_new(
        subject: ObjectId,
        footprint: PlanArea,
        covered: PlanArea,
        effects: Vec<(ObjectId, EffectMeets)>,
    ) -> Result<Self, PlanAreaError> {
        if covered.lower_square_metres() > footprint.upper_square_metres() {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        Ok(Self {
            subject,
            footprint,
            covered,
            effects,
        })
    }

    /// The object whose footprint was covered.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// The subject's footprint area.
    #[must_use]
    pub fn footprint(&self) -> &PlanArea {
        &self.footprint
    }

    /// The covered part of the footprint.
    #[must_use]
    pub fn covered(&self) -> &PlanArea {
        &self.covered
    }

    /// Per source, whether its effect meets the footprint.
    #[must_use]
    pub fn effects(&self) -> &[(ObjectId, EffectMeets)] {
        &self.effects
    }

    /// Reviewable provenance of the footprint and the covered area.
    #[must_use]
    pub fn evidence(&self) -> [&Evidence; 2] {
        [self.footprint.evidence(), self.covered.evidence()]
    }
}

/// Checks an answer against its request: the same subject, one entry per
/// requested source in order, and a covered area no larger than the
/// footprint could be.
pub(crate) fn check_answer(
    request: &CoverageRequest,
    answer: &CoverageEvidence,
) -> Result<(), PlanAreaError> {
    if answer.subject() != request.subject() {
        return Err(PlanAreaError::Unavailable(format!(
            "coverage of {} was returned for {}",
            answer.subject(),
            request.subject()
        )));
    }
    let answered = answer.effects().iter().map(|(object, _)| object);
    let asked = request.sources().iter().map(Participant::object);
    if !answered.eq(asked) {
        return Err(PlanAreaError::Unavailable(
            "the coverage answer does not list the requested sources".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    fn area(value: f64) -> PlanArea {
        PlanArea::try_new(
            value,
            value,
            Evidence::exact(SourceId::new("cad", "m").unwrap(), "area"),
        )
        .unwrap()
    }

    #[test]
    fn a_request_merges_listings_and_refuses_overlaps_and_bad_ranges() {
        let source = |local, certain| Participant::new(id(local), certain);
        let request = CoverageRequest::try_new(
            id("s"),
            EffectReach::Grown,
            1.0,
            vec![source("b", false), source("a", false), source("b", true)],
            vec![],
        )
        .unwrap();
        assert_eq!(request.sources(), [source("a", false), source("b", true)]);
        for range in [-1.0, f64::NAN] {
            assert!(
                CoverageRequest::try_new(id("s"), EffectReach::Grown, range, vec![], vec![])
                    .is_err()
            );
        }
        assert!(
            CoverageRequest::try_new(
                id("s"),
                EffectReach::Travel,
                1.0,
                vec![source("s", true)],
                vec![]
            )
            .is_err()
        );
        assert!(
            CoverageRequest::try_new(
                id("s"),
                EffectReach::Travel,
                1.0,
                vec![source("a", true)],
                vec![source("a", false)]
            )
            .is_err()
        );
    }

    #[test]
    fn an_answer_must_fit_its_request() {
        let request = CoverageRequest::try_new(
            id("s"),
            EffectReach::Grown,
            1.0,
            vec![Participant::new(id("a"), true)],
            vec![],
        )
        .unwrap();
        assert!(CoverageEvidence::try_new(id("s"), area(1.0), area(2.0), vec![]).is_err());
        let wrong = CoverageEvidence::try_new(id("s"), area(1.0), area(0.5), vec![]).unwrap();
        assert!(check_answer(&request, &wrong).is_err());
        let right = CoverageEvidence::try_new(
            id("s"),
            area(1.0),
            area(0.5),
            vec![(id("a"), EffectMeets::Surely)],
        )
        .unwrap();
        assert!(check_answer(&request, &right).is_ok());
    }
}
