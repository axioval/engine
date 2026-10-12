//! The height of an edge above the surfaces beneath it,
//! `height_above_surface`: the least vertical distance from an object's
//! top or bottom faces down to the nearest selected surface beneath them,
//! over the whole edge, as the proximity service measures it
//! (`measure_height_above`).
//!
//! The surfaces are the selection's objects that may lie beneath the
//! object, by their extents: overlapping its extent in plan and reaching
//! no higher than its top. One the selection cannot decide among them, or
//! whose extent cannot be read, leaves the height undecided, since it may
//! be the nearest surface; none beneath, or a part of the edge with no
//! surface beneath it, leaves it not evaluated, never measured to the
//! surfaces beside it.

use axioval_engine::{
    Citation, HeightAbove, HeightAboveRequest, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, ProximityError, ProximityServiceHandle, RuleContext, SubjectSurface,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{ObjectId, QuantityDimension};

use crate::measured_kinds::{interval, refused, selection};
use crate::support::Unavailable;

/// Measures `height_above_surface`.
pub(crate) struct HeightMeasures;

const HEIGHT_ABOVE_SURFACE: &str = "height_above_surface";

impl MeasuredProvider for HeightMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[HEIGHT_ABOVE_SURFACE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_cited(call, object, context)
            .map(|(measurement, _)| measurement)
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        height(call, object, context).map_err(refused(call.name(), object))
    }
}

fn proximity_reason(error: ProximityError) -> NotEvaluatedReason {
    match error {
        ProximityError::UnsupportedProjection => NotEvaluatedReason::BackendUnavailable,
        ProximityError::InvalidMeasurement
        | ProximityError::EvidenceFidelityMismatch
        | ProximityError::SameObject => NotEvaluatedReason::InvalidEvidence,
        ProximityError::Unavailable | ProximityError::Refused(_) | ProximityError::NoBody => {
            NotEvaluatedReason::IncompleteEvidence
        }
    }
}

fn height(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let service = context
        .services
        .get::<ProximityServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "proximity service is not registered".to_owned(),
            )
        })?;
    let edge = match call.choice("edge") {
        Some("bottom") => SubjectSurface::Bottom,
        _ => SubjectSurface::Top,
    };
    let surfaces = beneath(service, call, object, context)?;
    let request = HeightAboveRequest::try_new(object.clone(), surfaces.clone(), edge)
        .map_err(|error| (proximity_reason(error), error.to_string()))?;
    let measured = service.measure_height_above(&request).map_err(|error| {
        (
            proximity_reason(error),
            format!(
                "the height of its {} edge above the surfaces beneath it could not be measured: \
                 {error}",
                edge.name()
            ),
        )
    })?;
    match measured.height() {
        HeightAbove::Beneath {
            lower_metres,
            upper_metres,
        } => Ok((
            interval(
                (lower_metres, upper_metres),
                Some(QuantityDimension::Length),
                measured.evidence().exact,
                format!(
                    "{HEIGHT_ABOVE_SURFACE}:{object};{}",
                    measured.evidence().locator
                ),
            ),
            Citation {
                related: surfaces,
                evidence: vec![measured.evidence().clone()],
                ..Citation::default()
            },
        )),
        HeightAbove::Uncovered {
            uncovered_square_metres,
            edge_square_metres,
        } => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{uncovered_square_metres:.4} m² of its {} edge's {edge_square_metres:.4} m² in \
                 plan has no selected surface beneath it",
                edge.name()
            ),
        )),
    }
}

/// The selected objects that may lie beneath `object`, sorted: overlapping
/// its extent in plan and reaching no higher than its top.
fn beneath(
    service: &ProximityServiceHandle,
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<ObjectId>, Unavailable> {
    let selected = selection(context, call, "surface", Some(object))
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?
        .ok_or_else(|| crate::support::invalid("`surface` is required"))?;
    let own = service.bounds(object).map_err(|error| {
        (
            proximity_reason(error),
            format!("its extent cannot be read: {error}"),
        )
    })?;
    let (low, high) = (own.enclosing().min(), own.enclosing().max());
    let mut found = Vec::new();
    for candidate in selected.matched.iter().chain(&selected.undecided) {
        if candidate == object {
            continue;
        }
        let bounds = match service.bounds(candidate) {
            Ok(bounds) => bounds,
            Err(ProximityError::NoBody) => continue,
            Err(error) => {
                return Err((
                    proximity_reason(error),
                    format!(
                        "the extent of {candidate}, which may lie beneath it, cannot be read: {error}"
                    ),
                ));
            }
        };
        let (min, max) = (bounds.enclosing().min(), bounds.enclosing().max());
        let overlaps = (0..2).all(|axis| min[axis] <= high[axis] && low[axis] <= max[axis]);
        if !overlaps || min[2] > high[2] {
            continue;
        }
        if selected.undecided.contains(candidate) {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("whether {candidate}, which may lie beneath it, is a surface is undecided"),
            ));
        }
        found.push(candidate.clone());
    }
    if found.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            "no selected surface lies beneath it".to_owned(),
        ));
    }
    Ok(found)
}
