//! A space's shelving as values, measured exactly as `shelf-capacity`
//! measures it: the running metres of the declared arrangement and the
//! space's clear height, from one linear-quantity request carrying the
//! doors and openings that reach the space. The doors, openings and spaces
//! are named by source kind or bound from the reading rule's selectors
//! (`doors=@door_selector`), and the doors sent are cited as what the
//! value was measured against.

use axioval_engine::{
    Citation, LinearInterval, LinearQuantityServiceHandle, MeasuredProvider, Measurement,
    NotEvaluatedReason, PropertyResolutionError, RuleContext, ShelfGeometry,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{ObjectId, QuantityDimension};

use super::measure;
use crate::measured_kinds::{interval, refused, selection};
use crate::space_access::{AccessDeclaration, Pick};
use crate::support::{Unavailable, invalid};

/// Measures `shelf_length` and `shelf_clear_height`.
pub(crate) struct ShelfMeasures;

const SHELF_LENGTH: &str = "shelf_length";
const SHELF_CLEAR_HEIGHT: &str = "shelf_clear_height";

/// The objects of each element and space argument, by key.
struct Picked {
    doors: Option<MeasuredSelection>,
    openings: Option<MeasuredSelection>,
    spaces: Option<MeasuredSelection>,
}

impl Picked {
    fn of(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Self, PropertyResolutionError> {
        Ok(Self {
            doors: selection(context, call, "doors", Some(object))?,
            openings: selection(context, call, "openings", Some(object))?,
            spaces: selection(context, call, "spaces", None)?,
        })
    }
}

fn length(value: LinearInterval, exact: bool, locator: String) -> Measurement {
    interval(
        (value.lower_metres(), value.upper_metres()),
        Some(QuantityDimension::Length),
        exact,
        locator,
    )
}

/// The arrangement the call states, refused as `shelf-capacity` refused an
/// impossible one.
fn geometry(call: &MeasuredCall) -> Result<ShelfGeometry, Unavailable> {
    let metres = |key: &str| match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    };
    let refused = || invalid("shelf geometry parameters are missing or not physically realisable");
    ShelfGeometry::try_new(
        metres("depth").ok_or_else(refused)?,
        metres("horizontal").ok_or_else(refused)?,
        metres("vertical").ok_or_else(refused)?,
        metres("bottom").ok_or_else(refused)?,
        metres("top").ok_or_else(refused)?,
        metres("clearance").ok_or_else(refused)?,
    )
    .map_err(|_| refused())
}

fn shelving(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    picked: &Picked,
) -> Result<(Measurement, Citation), Unavailable> {
    let geometry = geometry(call)?;
    let Some(MeasuredArgument::Path(steps)) = call.argument("access") else {
        return Err(invalid(
            "shelf-capacity: parameter `access_path` is required",
        ));
    };
    let access = AccessDeclaration::of(
        steps,
        picked.doors.as_ref().map(Pick::Selected),
        picked.openings.as_ref().map(Pick::Selected),
        picked.spaces.as_ref().map(Pick::Selected),
    )
    .map_err(|(reason, message)| (reason, format!("shelf-capacity: {message}")))?;
    let service = context
        .services
        .get::<LinearQuantityServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "linear-quantity service is not registered".to_owned(),
            )
        })?;
    let shelving = measure(service, &access.index(context), geometry, object)?;
    let exact = shelving.evidence.iter().all(|evidence| evidence.exact);
    let locator = shelving.measured.evidence().locator.clone();
    // What the length was measured against: the doors and openings sent,
    // with the evidence that reached them.
    let citation = Citation {
        related: shelving.doors.clone(),
        evidence: shelving.evidence[1..].to_vec(),
    };
    if call.name() == SHELF_LENGTH {
        return Ok((
            length(shelving.measured.measured(), exact, locator),
            citation,
        ));
    }
    match shelving.measured.clear_height() {
        Some(height) => Ok((length(height, exact, locator), citation)),
        None => Err((
            NotEvaluatedReason::IncompleteEvidence,
            "the clear height of the space was not measured".into(),
        )),
    }
}

impl MeasuredProvider for ShelfMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[SHELF_CLEAR_HEIGHT, SHELF_LENGTH]
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
        let picked = Picked::of(call, object, context)?;
        shelving(call, object, context, &picked).map_err(refused(call.name(), object))
    }
}
