//! Areas, shares and coverage as values: the numerators and denominators
//! the area capabilities judge, from the same service requests.
//!
//! Counterparts, covers, sources and blockers are the project's objects of
//! the source kinds a parameter names. Shares are plain numbers from 0 to
//! 1, intervals where the areas are.

use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::{Answer, Measures};
use crate::boundary_coverage::BoundaryCoverageRequest;
use crate::contact::{ContactError, ContactRequest, ContactSide, ContactTolerance};
use crate::coverage::{CoverageRequest, EffectReach, Participant};
use crate::properties::PropertyResolutionError;

/// The names measured here.
pub(super) const NAMES: &[&str] = &[
    "boundary_covered_share",
    "boundary_off_surface_count",
    "boundary_overlap_area",
    "boundary_uncovered_area",
    "contact_area",
    "contact_gap",
    "contact_share",
    "effect_covered_area",
    "effect_covered_share",
    "facade_area",
    "face_area",
    "plan_overlap",
    "uncovered_area",
];

const AREA: Option<QuantityDimension> = Some(QuantityDimension::Area);

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

/// `numerator / denominator` over intervals, rounded outward and kept in
/// `[0, 1]`; none when the denominator may be zero.
fn share((top_low, top_high): (f64, f64), (low, high): (f64, f64)) -> Option<(f64, f64)> {
    (low > 0.0).then(|| {
        (
            (top_low / high).next_down().clamp(0.0, 1.0),
            (top_high / low).next_up().clamp(0.0, 1.0),
        )
    })
}

/// A share as an answer, exact as the areas it divides are: the quotient
/// of two points stays a point only where the division is exact, and
/// rounds outward otherwise.
fn share_answer(
    top: (f64, f64),
    bottom: (f64, f64),
    locator: String,
    exact: bool,
) -> Result<Answer, String> {
    #[allow(clippy::float_cmp)]
    if top.0 == top.1 && bottom.0 == bottom.1 && bottom.0 > 0.0 {
        let quotient = super::span(top.0, top.1)
            .divided_by(super::span(bottom.0, bottom.1))
            .map_err(|_| "the share is not finite")?;
        return Ok(Answer::Value(
            quotient.lower.clamp(0.0, 1.0),
            quotient.upper.clamp(0.0, 1.0),
            None,
            locator,
            exact,
        ));
    }
    let (lower, upper) = share(top, bottom).ok_or("the whole area may be zero")?;
    Ok(Answer::Value(lower, upper, None, locator, exact))
}

impl Measures {
    /// An area or share of `object`, by `call`'s name.
    pub(super) fn area_measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        match name {
            "facade_area" | "face_area" => {
                let service = self
                    .facades
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "facade-area"))?;
                let area = if name == "facade_area" {
                    service.measure_facade_area(object)
                } else {
                    service.measure_face_area(object)
                }
                .map_err(|error| unavailable(error.to_string()))?;
                Ok(Answer::Value(
                    area.lower_square_metres(),
                    area.upper_square_metres(),
                    AREA,
                    area.evidence().locator.clone(),
                    area.evidence().exact,
                ))
            }
            "plan_overlap" | "uncovered_area" => self.plan_measure(call, object),
            "contact_area" | "contact_gap" | "contact_share" => self.contact(call, object),
            "effect_covered_area" | "effect_covered_share" => self.effect(call, object),
            _ => {
                let request =
                    BoundaryCoverageRequest::try_new(object.clone(), length(call, "plane"))
                        .map_err(|error| unavailable(error.to_string()))?;
                let coverage = self
                    .boundaries
                    .as_ref()
                    .ok_or_else(|| Self::missing(name, "boundary-coverage"))?
                    .measure_boundary_coverage(&request)
                    .map_err(|error| unavailable(error.to_string()))?;
                let locator = format!("boundary-coverage:{object}");
                let exact = coverage.evidence().exact;
                Ok(match name {
                    "boundary_covered_share" => {
                        let share = coverage.covered_share();
                        Answer::Value(share.lower(), share.upper(), None, locator, exact)
                    }
                    "boundary_off_surface_count" => {
                        #[allow(clippy::cast_precision_loss)]
                        let count = coverage.off_surface().count() as f64;
                        Answer::Value(count, count, None, locator, exact)
                    }
                    "boundary_uncovered_area" => {
                        let area = coverage.uncovered_area();
                        Answer::Value(
                            area.lower_square_metres(),
                            area.upper_square_metres(),
                            AREA,
                            locator,
                            exact,
                        )
                    }
                    _ => {
                        let area = coverage.overlap_area();
                        Answer::Value(
                            area.lower_square_metres(),
                            area.upper_square_metres(),
                            AREA,
                            locator,
                            exact,
                        )
                    }
                })
            }
        }
    }

    /// A plan overlap with, or the area uncovered by, objects of kinds.
    fn plan_measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let plan = self
            .plan
            .as_ref()
            .ok_or_else(|| Self::missing(name, "plan-area"))?;
        if name == "uncovered_area" {
            let cover = self.of_kinds(call, "by", object)?;
            let area = plan
                .measure_uncovered_area(object, &cover, length(call, "growth"))
                .map_err(|error| unavailable(error.to_string()))?;
            return Ok(Answer::Value(
                area.lower_square_metres(),
                area.upper_square_metres(),
                AREA,
                area.evidence().locator.clone(),
                area.evidence().exact,
            ));
        }
        let others = self.of_kinds(call, "with", object)?;
        let total = call.choice("measure") == Some("total");
        let mut measured = super::span(0.0, 0.0);
        let mut exact = true;
        for other in &others {
            let overlap = plan
                .measure_plan_overlap(object, other)
                .map_err(|error| unavailable(error.to_string()))?;
            exact &= overlap.evidence().exact;
            let each = super::span(overlap.lower_square_metres(), overlap.upper_square_metres());
            measured = if total {
                // Summed outward, so the sum holds the exact one.
                measured
                    .plus(each)
                    .map_err(|_| PropertyResolutionError::InvalidValue)?
            } else {
                measured.max(each)
            };
        }
        Ok(Answer::Value(
            measured.lower,
            measured.upper,
            AREA,
            format!("plan-overlap:{object}:{}", others.len()),
            exact,
        ))
    }

    /// The contact area on a side, its share of the whole face, or the
    /// distance to the nearest candidate.
    fn contact(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let candidates = self.of_kinds(call, "with", object)?;
        let side = if call.choice("side") == Some("above") {
            ContactSide::Above
        } else {
            ContactSide::Below
        };
        let tolerance = ContactTolerance::try_new(
            length(call, "gap"),
            length(call, "intersection"),
            match call.argument("polygon") {
                Some(MeasuredArgument::Length(area)) => *area,
                _ => 0.0,
            },
        )
        .map_err(|error| unavailable(error.to_string()))?;
        let contact = self
            .contacts
            .as_ref()
            .ok_or_else(|| Self::missing(name, "contact"))?
            .measure_contact(&ContactRequest::new(
                object.clone(),
                candidates,
                side,
                tolerance,
            ))
            .map_err(|error| refused_contact(name, object, error))?;
        let locator = contact.evidence().locator.clone();
        let exact = contact.evidence().exact;
        let area = contact.contact_area_square_metres();
        if name == "contact_area" {
            return Ok(Answer::Value(area, area, AREA, locator, exact));
        }
        if name == "contact_gap" {
            return Ok(match contact.nearest_distance_metres() {
                Some(gap) => {
                    Answer::Value(gap, gap, Some(QuantityDimension::Length), locator, exact)
                }
                None => Answer::Absent(format!("{locator}: no candidate is reported near")),
            });
        }
        let whole = contact.whole_area_square_metres();
        share_answer((area, area), (whole, whole), locator, exact).map_err(unavailable)
    }

    /// The part of the footprint the sources' effects cover, or its share.
    fn effect(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
    ) -> Result<Answer, PropertyResolutionError> {
        let name = call.name();
        let unavailable = |error: String| Self::unavailable(name, object, &error);
        let participants = |key: &str| -> Result<Vec<Participant>, PropertyResolutionError> {
            if call.argument(key).is_none() {
                return Ok(Vec::new());
            }
            Ok(self
                .of_kinds(call, key, object)?
                .into_iter()
                .map(|other| Participant::new(other, true))
                .collect())
        };
        let reach = match call.choice("reach") {
            Some("travel") => EffectReach::Travel,
            Some("visible") => EffectReach::Visible,
            _ => EffectReach::Grown,
        };
        let request = CoverageRequest::try_new(
            object.clone(),
            reach,
            length(call, "range"),
            participants("sources")?,
            participants("blockers")?,
        )
        .map_err(|error| unavailable(error.to_string()))?;
        let coverage = self
            .plan
            .as_ref()
            .ok_or_else(|| Self::missing(name, "plan-area"))?
            .measure_coverage(&request)
            .map_err(|error| unavailable(error.to_string()))?;
        let covered = coverage.covered();
        let locator = covered.evidence().locator.clone();
        let covered_exact = covered.evidence().exact;
        let covered = (covered.lower_square_metres(), covered.upper_square_metres());
        if name == "effect_covered_area" {
            return Ok(Answer::Value(
                covered.0,
                covered.1,
                AREA,
                locator,
                covered_exact,
            ));
        }
        let footprint = coverage.footprint();
        share_answer(
            covered,
            (
                footprint.lower_square_metres(),
                footprint.upper_square_metres(),
            ),
            locator,
            covered_exact && footprint.evidence().exact,
        )
        .map_err(unavailable)
    }
}

/// A contact measurement's refusal, for the reason `slab-contact` gives: a
/// body the service could not measure or orient is missing evidence, never
/// a clean face, and an answer it cannot stand behind is invalid evidence.
fn refused_contact(name: &str, object: &ObjectId, error: ContactError) -> PropertyResolutionError {
    let message = format!(
        "`{}` value `{name}` of {object}: {error}",
        axioval_ir::MEASURED_SET
    );
    match error {
        ContactError::Unavailable | ContactError::UncheckableOrientation => {
            PropertyResolutionError::Incomplete(message)
        }
        ContactError::InexactEvidence
        | ContactError::InvalidAreas
        | ContactError::UnrequestedCandidate => PropertyResolutionError::Conflicting(message),
    }
}
