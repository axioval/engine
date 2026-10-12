//! The building envelope as a whole: `envelope_area` (of every face
//! looking outside, or of those looking towards one compass direction),
//! `envelope_volume` and `compactness`, their ratio, each of the project
//! over the elements a selection names.
//!
//! The faces are those the facade-area service lists as looking outside
//! (`measure_facade_faces`), of any slope. A face counts towards the least
//! area only where the selection surely picks its element, all of it looks
//! outside and it surely looks towards the direction asked; a face that may
//! do so adds only to the most. Compass directions are quarters centred on
//! north, east, south and west, read from true north as the source states
//! it (or the plan's y axis); a face tilted less than a nanoradian from
//! level looks in none of them. The volume is the one the faces enclose
//! together (`measure_enclosed_volume`), refused where they do not close.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use axioval_engine::expression::Interval;
use axioval_engine::{
    Citation, FacadeAreaServiceHandle, FacadeFaceSide, FacadeFaces, FaceNormal, MeasuredMemo,
    MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredCall, MeasuredSelection};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, SourceId};

use crate::facade_orientation::{azimuth, facade_unavailable, north_of};
use crate::measured_kinds::{interval, resolution_error, selection};
use crate::support::Unavailable;

const ENVELOPE_AREA: &str = "envelope_area";
const ENVELOPE_VOLUME: &str = "envelope_volume";
const COMPACTNESS: &str = "compactness";

/// A face whose horizontal part is at most this share of its normal looks
/// in no compass direction.
const LEVEL: f64 = 1e-9;

/// Measures `envelope_area`, `envelope_volume` and `compactness`.
pub(crate) struct EnvelopeAreaMeasures;

/// The memo key of one element's outward faces.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FacesKey;

/// A summed quantity: its least and greatest value, whether it is exact,
/// and the evidence it rests on.
struct Sum {
    lower: f64,
    upper: f64,
    exact: bool,
    evidence: Vec<Evidence>,
}

fn missing_service() -> Unavailable {
    (
        NotEvaluatedReason::MissingService,
        "facade-area service is not registered".to_owned(),
    )
}

/// The outward faces of `element`, read once per run.
fn faces(context: &RuleContext<'_>, element: &ObjectId) -> Result<FacadeFaces, Unavailable> {
    MeasuredMemo::of(context.services, (FacesKey, element.clone()), || {
        context
            .services
            .get::<FacadeAreaServiceHandle>()
            .ok_or_else(missing_service)?
            .measure_facade_faces(element)
            .map_err(|error| facade_unavailable(&error))
    })
}

/// The elements `call` names: those surely picked, then those the
/// selection leaves undecided.
fn elements(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
) -> Result<MeasuredSelection, Unavailable> {
    selection(context, call, "elements", None)
        .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?
        .ok_or_else(|| {
            (
                NotEvaluatedReason::InvalidDeclaration,
                "no elements are named".to_owned(),
            )
        })
}

/// Whether the azimuth `angle` lies within the quarter centred on
/// `centre`, from its start up to its end: `Some` where it surely does or
/// surely does not, `None` where it may.
fn within(angle: Interval, centre: f64) -> Option<bool> {
    // Outward rounding of the turn to the quarter's start.
    const PAD: f64 = 1e-12;
    let start = (angle.lower - (centre - FRAC_PI_4)).rem_euclid(TAU);
    let end = start + (angle.upper - angle.lower);
    if start >= PAD && end + PAD < FRAC_PI_2 {
        Some(true)
    } else if start - PAD >= FRAC_PI_2 && end + PAD < TAU {
        Some(false)
    } else {
        None
    }
}

/// Whether a face of outward normal `normal` looks towards the quarter
/// centred on `centre` from `north`.
fn looks_towards(normal: &FaceNormal, centre: f64, north: [f64; 2]) -> Option<bool> {
    let (lower, upper) = (normal.lower(), normal.upper());
    let extreme = |axis: usize| lower[axis].abs().max(upper[axis].abs());
    let least = |axis: usize| {
        if lower[axis] > 0.0 {
            lower[axis]
        } else if upper[axis] < 0.0 {
            -upper[axis]
        } else {
            0.0
        }
    };
    let across = extreme(0).hypot(extreme(1));
    let up = least(2);
    // Surely level: its horizontal part at most a nanoradian's share.
    if across <= LEVEL * up {
        return Some(false);
    }
    let middle: [f64; 3] = std::array::from_fn(|axis| f64::midpoint(lower[axis], upper[axis]));
    let angle = azimuth(std::slice::from_ref(normal), middle, north).ok()?;
    within(angle, centre)
}

/// The summed area of the faces of the selected elements looking outside,
/// towards `toward` where it is stated.
fn area(
    call: &MeasuredCall,
    context: &RuleContext<'_>,
    picked: &MeasuredSelection,
) -> Result<Sum, Unavailable> {
    let centre = match call.choice("toward") {
        None => None,
        Some("north") => Some(0.0),
        Some("east") => Some(FRAC_PI_2),
        Some("south") => Some(PI),
        Some(_) => Some(1.5 * PI),
    };
    let mut norths: Vec<(SourceId, [f64; 2], bool)> = Vec::new();
    let mut sum = Sum {
        lower: 0.0,
        upper: 0.0,
        exact: picked.undecided.is_empty(),
        evidence: Vec::new(),
    };
    let elements = picked
        .matched
        .iter()
        .map(|element| (element, true))
        .chain(picked.undecided.iter().map(|element| (element, false)));
    for (element, sure) in elements {
        let faces = faces(context, element)?;
        sum.exact &= faces.evidence().exact;
        sum.evidence.push(faces.evidence().clone());
        let known = norths
            .iter()
            .find(|(source, ..)| *source == element.source)
            .map(|(_, north, _)| *north);
        let north = match (centre, known) {
            (None, _) => None,
            (Some(_), Some(north)) => Some(north),
            (Some(_), None) => {
                let (north, stated) = north_of(call, context, &element.source)?;
                sum.exact &= stated;
                norths.push((element.source.clone(), north, stated));
                Some(north)
            }
        };
        for face in faces.faces() {
            let towards = match (centre, north) {
                (Some(centre), Some(north)) => looks_towards(face.normal(), centre, north),
                _ => Some(true),
            };
            let (least, most) = face.area_square_metres();
            if sure && face.side() == FacadeFaceSide::Outside && towards == Some(true) {
                sum.lower += least;
            }
            if towards != Some(false) {
                sum.upper += most;
            }
            if towards.is_none() || face.side() == FacadeFaceSide::Partly {
                sum.exact = false;
            }
        }
    }
    // The running sums rounded outward.
    let count = sum.evidence.len().max(1);
    let slack = 8.0 * f64::EPSILON * f64::from(u32::try_from(count).unwrap_or(u32::MAX));
    sum.lower = (sum.lower * (1.0 - slack)).next_down().max(0.0);
    sum.upper = (sum.upper * (1.0 + slack)).next_up();
    Ok(sum)
}

/// The volume the outward faces of the selected elements enclose.
fn volume(context: &RuleContext<'_>, picked: &MeasuredSelection) -> Result<Sum, Unavailable> {
    if let Some(element) = picked.undecided.first() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the selection leaves elements undecided that may bound the envelope, {element} \
                 among them"
            ),
        ));
    }
    let elements: Vec<ObjectId> = picked.matched.iter().cloned().collect();
    let volume = context
        .services
        .get::<FacadeAreaServiceHandle>()
        .ok_or_else(missing_service)?
        .measure_enclosed_volume(&elements)
        .map_err(|error| {
            let (reason, why) = facade_unavailable(&error);
            let reason = if reason == NotEvaluatedReason::BackendUnavailable {
                NotEvaluatedReason::IncompleteEvidence
            } else {
                reason
            };
            (reason, why)
        })?;
    let (lower, upper) = volume.cubic_metres();
    Ok(Sum {
        lower,
        upper,
        exact: volume.evidence().exact,
        evidence: vec![volume.evidence().clone()],
    })
}

impl EnvelopeAreaMeasures {
    fn of_project(
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), Unavailable> {
        let picked = elements(call, context)?;
        let (value, dimension) = match call.name() {
            ENVELOPE_AREA => (area(call, context, &picked)?, QuantityDimension::Area),
            ENVELOPE_VOLUME => (volume(context, &picked)?, QuantityDimension::Volume),
            _ => {
                let (area, volume) = (area(call, context, &picked)?, volume(context, &picked)?);
                if volume.lower <= 0.0 {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        "the envelope may enclose no volume".to_owned(),
                    ));
                }
                let quotient = Interval {
                    lower: area.lower,
                    upper: area.upper,
                }
                .divided_by(Interval {
                    lower: volume.lower,
                    upper: volume.upper,
                })
                .map_err(|_| {
                    (
                        NotEvaluatedReason::IncompleteEvidence,
                        "the area over the volume cannot be divided".to_owned(),
                    )
                })?;
                let mut evidence = area.evidence;
                evidence.extend(volume.evidence);
                let sum = Sum {
                    lower: quotient.lower,
                    upper: quotient.upper,
                    exact: area.exact && volume.exact,
                    evidence,
                };
                let locator = format!("{COMPACTNESS}:elements={}", picked.matched.len());
                return Ok(cited(sum, None, locator, &picked));
            }
        };
        let locator = format!("{}:elements={}", call.name(), picked.matched.len());
        Ok(cited(value, Some(dimension), locator, &picked))
    }
}

/// `sum` as a measurement citing the elements measured.
fn cited(
    sum: Sum,
    dimension: Option<QuantityDimension>,
    locator: String,
    picked: &MeasuredSelection,
) -> (Measurement, Citation) {
    let mut notes = Vec::new();
    if !picked.undecided.is_empty() {
        notes.push(format!(
            "{} elements the selection leaves undecided may add to it",
            picked.undecided.len()
        ));
    }
    (
        interval((sum.lower, sum.upper), dimension, sum.exact, locator),
        Citation {
            related: Vec::new(),
            evidence: sum.evidence,
            notes,
            sources: Vec::new(),
        },
    )
}

/// A refusal of `name` of `subject`, as a property resolution states it.
fn refused(
    name: &str,
    subject: impl std::fmt::Display,
) -> impl Fn(Unavailable) -> PropertyResolutionError {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of {subject}: {why}")))
}

impl MeasuredProvider for EnvelopeAreaMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[ENVELOPE_AREA, ENVELOPE_VOLUME, COMPACTNESS]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Self::of_project(call, context)
            .map(|(measurement, _)| measurement)
            .map_err(refused(call.name(), object))
    }

    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        Self::of_project(call, context).map_err(refused(call.name(), "the project"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(value: f64) -> Interval {
        Interval::point(value)
    }

    #[test]
    fn quarters_hold_what_lies_between_their_ends() {
        assert_eq!(within(point(PI), PI), Some(true));
        assert_eq!(within(point(0.0), 0.0), Some(true));
        assert_eq!(within(point(TAU - 0.1), 0.0), Some(true));
        assert_eq!(within(point(PI), 0.0), Some(false));
        assert_eq!(within(point(FRAC_PI_2), 0.0), Some(false));
        // On a quarter's edge, either may hold it.
        assert_eq!(within(point(FRAC_PI_4), 0.0), None);
        assert_eq!(within(point(FRAC_PI_4), FRAC_PI_2), None);
    }

    #[test]
    fn a_level_face_looks_in_no_compass_direction() {
        let up = FaceNormal::try_new([-1e-17, -1e-17, 1.0], [1e-17, 1e-17, 1.0]).unwrap();
        assert_eq!(looks_towards(&up, 0.0, [0.0, 1.0]), Some(false));
        let south = FaceNormal::exact([0.0, -2.0, 0.0]).unwrap();
        assert_eq!(looks_towards(&south, PI, [0.0, 1.0]), Some(true));
        assert_eq!(looks_towards(&south, 0.0, [0.0, 1.0]), Some(false));
        // True north along +x: the plan's south looks east.
        assert_eq!(looks_towards(&south, FRAC_PI_2, [1.0, 0.0]), Some(true));
    }
}
