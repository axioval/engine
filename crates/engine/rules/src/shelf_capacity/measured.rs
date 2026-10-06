//! A space's shelving as values, measured exactly as `shelf-capacity`
//! measures it: the running metres of the declared arrangement and the
//! space's clear height, from one linear-quantity request carrying the
//! doors and openings that reach the space. The doors, openings and spaces
//! are named by source kind or bound from the reading rule's selectors
//! (`doors=@door_selector`), and the doors sent are cited as what the
//! value was measured against.

use axioval_engine::{
    ArgumentsKey, Citation, LinearInterval, LinearQuantityServiceHandle, MeasuredMemo,
    MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError, RuleContext,
    ShelfGeometry,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall, MeasuredSelection};
use axioval_ir::{ObjectId, QuantityDimension};

use std::sync::Arc;

use super::{Shelving, measure};
use crate::measured_kinds::{interval, refused, selection};
use crate::space_access::AccessIndex;
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
    fn of(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Self, PropertyResolutionError> {
        Ok(Self {
            doors: selection(context, call, "doors", None)?,
            openings: selection(context, call, "openings", None)?,
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

/// The access index the call's path, elements and spaces declare, built
/// once per run for those arguments.
fn index(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<Arc<AccessIndex>, Unavailable> {
    let key = ArgumentsKey::of_keys(call, &["access", "doors", "openings", "spaces"]);
    MeasuredMemo::of(context.services, key, || {
        let picked = Picked::of(call, context).map_err(crate::selection::property_error)?;
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
        Ok(Arc::new(access.index(context)))
    })
}

/// The shelving of the space `object`, measured once per run for the call's
/// `arguments` (every argument of it, [`ArgumentsKey::of`]): `shelf_length`
/// and `shelf_clear_height` read one request.
fn shelving(
    call: &MeasuredCall,
    arguments: &ArgumentsKey,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Arc<Shelving>, Unavailable> {
    let key = (object.clone(), arguments.clone());
    MeasuredMemo::of(context.services, key, || {
        let geometry = geometry(call)?;
        // The arguments are checked before the service, as the capability
        // checked its declaration first.
        let index = index(call, context)?;
        let service = context
            .services
            .get::<LinearQuantityServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "linear-quantity service is not registered".to_owned(),
                )
            })?;
        measure(service, &index, geometry, object).map(Arc::new)
    })
}

/// What the call measures of the space's shelving, and the doors and
/// openings it was measured with.
fn measured(
    call: &MeasuredCall,
    arguments: &ArgumentsKey,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), PropertyResolutionError> {
    let shelving =
        shelving(call, arguments, object, context).map_err(refused(call.name(), object))?;
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
        None => Err(refused(call.name(), object)((
            NotEvaluatedReason::IncompleteEvidence,
            "the clear height of the space was not measured".into(),
        ))),
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
        measured(call, &ArgumentsKey::of(call), object, context)
    }

    // Each value reads the space's shelving, kept for the run by its
    // arguments.
    fn memoizes(&self) -> bool {
        true
    }
}
