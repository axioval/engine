//! The distance from an object to the nearest roof feature line of a class
//! (`distance_to_feature`): a skylight's distance to a valley, a chimney's
//! to the ridge, in space or in plan.
//!
//! The roofs are the objects `roofs` names; their lines are classified as
//! `roof_features` classifies them ([`crate::roof_features::lines`]). The
//! proximity service measures the object's body to each line that may be
//! of the class ([`ProximityServiceHandle::measure_line_distance`]). The
//! nearest is taken over the lines that surely are: its interval runs from
//! the least lower bound to the least upper bound, so it holds the true
//! least distance. A line that may be of the class but is not surely one
//! (or a line of a roof that may be selected) and may lie nearer than that
//! leaves the distance not evaluated; with no line surely of the class, a
//! possible one leaves it not evaluated too, and none at all is none.

use axioval_engine::{
    Citation, LineDistanceRequest, LineProjection, MeasuredProvider, Measurement,
    NotEvaluatedReason, PropertyResolutionError, ProximityError, ProximityServiceHandle,
    RuleContext,
};
use axioval_ir::measured::MeasuredCall;
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use crate::measured_kinds::{interval, refused, selection};
use crate::roof_features::{Feature, chosen, lines, tolerance};
use crate::support::{Unavailable, invalid};

/// Measures `distance_to_feature`.
pub(crate) struct RoofDistanceMeasures;

const NAME: &str = "distance_to_feature";

/// Why a proximity measurement failed, in a report's words.
fn proximity_error(error: &ProximityError) -> Unavailable {
    let reason = match error {
        ProximityError::NoBody
        | ProximityError::Unavailable
        | ProximityError::Refused(_)
        | ProximityError::UnsupportedProjection => NotEvaluatedReason::BackendUnavailable,
        _ => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, format!("proximity: {error}"))
}

/// A line's distance from the object, and what it was measured from.
struct Measured {
    roof: ObjectId,
    lower: f64,
    upper: f64,
    evidence: Vec<Evidence>,
    place: String,
}

/// The lines that surely are of the class, each measured from `object`,
/// and those that may be, each with why it is not surely one.
type MeasuredLines = (Vec<Measured>, Vec<(Measured, String)>);

/// Every line of the roofs `call` names that may be of `feature`, measured
/// from `object` in `projection`.
fn measured_lines(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    feature: Feature,
    projection: LineProjection,
) -> Result<MeasuredLines, Unavailable> {
    let tolerance = tolerance(call)?;
    let proximity = context
        .services
        .get::<ProximityServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "the proximity service is not registered".to_owned(),
            )
        })?;
    let roofs = selection(context, call, "roofs", Some(object))
        .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?
        .ok_or_else(|| invalid("`roofs` is required"))?;
    let mut sure: Vec<Measured> = Vec::new();
    let mut possible: Vec<(Measured, String)> = Vec::new();
    let candidates = roofs
        .matched
        .iter()
        .map(|roof| (roof, true))
        .chain(roofs.undecided.iter().map(|roof| (roof, false)));
    for (roof, selected) in candidates {
        let (lines, edges) = lines(context, roof, &tolerance)?;
        for line in lines.iter().filter(|line| line.may_be(feature)) {
            let request = LineDistanceRequest::new(
                object.clone(),
                line.edge.start(),
                line.edge.end(),
                projection,
            );
            let distance = proximity
                .measure_line_distance(&request)
                .map_err(|error| proximity_error(&error))?;
            let measured = Measured {
                roof: roof.clone(),
                lower: distance.lower_metres(),
                upper: distance.upper_metres(),
                evidence: vec![distance.evidence().clone(), edges.clone()],
                place: line.place(),
            };
            if selected && line.decided() == Some(feature) {
                sure.push(measured);
            } else {
                let why = if selected {
                    line.why(roof)
                } else {
                    format!("whether {roof} is one of `roofs` is undecided")
                };
                possible.push((measured, why));
            }
        }
    }
    Ok((sure, possible))
}

/// The nearest line of the class from `object`, with its citation.
fn nearest(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), Unavailable> {
    let feature = chosen(call)?.ok_or_else(|| invalid("`feature` is required"))?;
    let projection = match call.choice("projection") {
        Some("plan") => LineProjection::Plan,
        _ => LineProjection::Space,
    };
    let (sure, possible) = measured_lines(call, object, context, feature, projection)?;
    let name = feature.name();
    let locator = format!("{NAME}:{name}:{}:{object}", projection.name());
    let Some(least_upper) = sure.iter().map(|line| line.upper).reduce(f64::min) else {
        return match possible.first() {
            None => Ok((Measurement::Absent { locator }, Citation::default())),
            Some((line, why)) => Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "no line of the roofs is surely a {name}, and the line {} may be one: {why}",
                    line.place
                ),
            )),
        };
    };
    if let Some((line, why)) = possible.iter().find(|(line, _)| line.lower < least_upper) {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the line {} may be a {name} nearer than the nearest that surely is: {why}",
                line.place
            ),
        ));
    }
    let least_lower = sure
        .iter()
        .map(|line| line.lower)
        .fold(f64::INFINITY, f64::min);
    // Every line that may be the nearest is cited and its roof related.
    let nearest: Vec<&Measured> = sure
        .iter()
        .filter(|line| line.lower <= least_upper)
        .collect();
    let exact = nearest
        .iter()
        .all(|line| line.evidence.iter().all(|evidence| evidence.exact));
    let mut related: Vec<ObjectId> = nearest.iter().map(|line| line.roof.clone()).collect();
    related.sort();
    related.dedup();
    let citation = Citation {
        related,
        evidence: nearest
            .iter()
            .flat_map(|line| line.evidence.iter().cloned())
            .collect(),
        ..Citation::default()
    };
    Ok((
        interval(
            (least_lower, least_upper),
            Some(QuantityDimension::Length),
            exact,
            locator,
        ),
        citation,
    ))
}

impl MeasuredProvider for RoofDistanceMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[NAME]
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        nearest(call, object, context).map_err(refused(call.name(), object))
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
}
