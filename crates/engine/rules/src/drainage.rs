//! Whether a surface drains to its outlets: `undrained_area`, the plan
//! area of the object's top whose water reaches none of the selected
//! outlets, and `drainage_path`, the longest path water runs to one, as the
//! proximity service traces them (`measure_drainage`).
//!
//! The surface drains where its undrained area is zero. Whatever the
//! service cannot decide (a face level within rounding, a tessellation)
//! widens the undrained area's upper bound, never its drained part. An
//! outlet the selection leaves undecided may take water: the area surely
//! undrained is measured with it, the area surely drained without it, and
//! the path, which it may end, is not evaluated.

use axioval_engine::{
    Citation, DrainageEvidence, DrainagePath, DrainageRequest, MeasuredProvider, Measurement,
    NotEvaluatedReason, PropertyResolutionError, ProximityError, ProximityServiceHandle,
    RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use crate::measured_kinds::{interval, refused, selection};
use crate::support::Unavailable;

const UNDRAINED_AREA: &str = "undrained_area";
const DRAINAGE_PATH: &str = "drainage_path";

/// Measures `undrained_area` and `drainage_path`.
pub(crate) struct DrainageMeasures;

impl MeasuredProvider for DrainageMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[DRAINAGE_PATH, UNDRAINED_AREA]
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
        drainage(call, object, context).map_err(refused(call.name(), object))
    }
}

fn proximity_reason(error: &ProximityError) -> NotEvaluatedReason {
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

/// The drainage of `object` to `outlets`.
fn trace(
    service: &ProximityServiceHandle,
    object: &ObjectId,
    outlets: Vec<ObjectId>,
    fall: f64,
) -> Result<DrainageEvidence, Unavailable> {
    let request = DrainageRequest::try_new(object.clone(), outlets, fall)
        .map_err(|error| (proximity_reason(&error), error.to_string()))?;
    service.measure_drainage(&request).map_err(|error| {
        (
            proximity_reason(&error),
            format!("how water runs off its top could not be traced: {error}"),
        )
    })
}

fn drainage(
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
    let fall = match call.argument("minimum_fall") {
        Some(MeasuredArgument::Number(fall)) => *fall,
        _ => 0.0,
    };
    let selected = selection(context, call, "outlets", Some(object))
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?
        .ok_or_else(|| crate::support::invalid("`outlets` is required"))?;
    let sure: Vec<ObjectId> = selected
        .matched
        .iter()
        .filter(|outlet| *outlet != object)
        .cloned()
        .collect();
    let measured = trace(service, object, sure.clone(), fall)?;
    let mut related = sure;
    let mut evidence = vec![measured.evidence().clone()];
    let mut exact = measured.evidence().exact;
    // An undecided outlet may take water: what surely stays is what stays
    // with it too.
    let possible: Vec<ObjectId> = selected
        .undecided
        .iter()
        .filter(|outlet| *outlet != object)
        .cloned()
        .collect();
    let (mut lower, upper) = measured.undrained_square_metres();
    if !possible.is_empty() {
        let mut every = related.clone();
        every.extend(possible.iter().cloned());
        let widest = trace(service, object, every, fall)?;
        lower = lower.min(widest.undrained_square_metres().0);
        evidence.push(widest.evidence().clone());
        exact = false;
        related.extend(possible.iter().cloned());
        related.sort();
    }
    let locator = format!("{}:{object};{}", call.name(), measured.evidence().locator);
    let citation = Citation {
        related,
        evidence,
        ..Citation::default()
    };
    if call.name() == UNDRAINED_AREA {
        return Ok((
            interval(
                (lower, upper),
                Some(QuantityDimension::Area),
                exact,
                locator,
            ),
            citation,
        ));
    }
    if !possible.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "whether {} takes water is undecided, so where the longest path ends is unknown",
                possible[0]
            ),
        ));
    }
    match measured.path() {
        DrainagePath::Bounded {
            lower_metres,
            upper_metres,
        } => {
            // Bounds that meet are the exact longest path, up to rounding.
            let tight = upper_metres - lower_metres <= 1e-9 * upper_metres.max(1.0);
            Ok((
                interval(
                    (lower_metres, upper_metres),
                    Some(QuantityDimension::Length),
                    exact && tight,
                    locator,
                ),
                citation,
            ))
        }
        DrainagePath::NoOutlet => Ok((Measurement::Absent { locator }, citation)),
        DrainagePath::Unbounded => Err((
            NotEvaluatedReason::IncompleteEvidence,
            "how far water runs on its top cannot be bounded".to_owned(),
        )),
    }
}
