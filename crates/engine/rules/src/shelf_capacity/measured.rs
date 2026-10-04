//! A space's shelving as values, measured exactly as `shelf-capacity`
//! measures it: the running metres of the declared arrangement and the
//! space's clear height, from one linear-quantity request carrying the
//! doors and openings that reach the space.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CompiledRule, LinearInterval, LinearQuantityServiceHandle, MeasuredProvider, Measurement,
    NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension, RuleId};

use super::{declaration, measure};
use crate::measured_kinds::{interval, objects_of_kinds, refused};
use crate::support::{Parameters, Unavailable};

/// Measures `shelf_length` and `shelf_clear_height`.
pub(crate) struct ShelfMeasures;

const SHELF_LENGTH: &str = "shelf_length";
const SHELF_CLEAR_HEIGHT: &str = "shelf_clear_height";

/// The arrangement's lengths, by measured key and declaration parameter.
const LENGTHS: [(&str, &str); 6] = [
    ("depth", "shelf_depth_metres"),
    ("horizontal", "horizontal_spacing_metres"),
    ("vertical", "vertical_spacing_metres"),
    ("bottom", "bottom_elevation_metres"),
    ("top", "top_elevation_metres"),
    ("clearance", "door_clearance_metres"),
];

/// The `shelf-capacity` declaration `call` stands for.
fn rule(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<CompiledRule, PropertyResolutionError> {
    let mut parameters = BTreeMap::from([(
        "minimum_running_metres".to_owned(),
        ParameterValue::Number { value: 0.0 },
    )]);
    for (key, name) in LENGTHS {
        if let Some(MeasuredArgument::Length(value)) = call.argument(key) {
            parameters.insert(name.to_owned(), ParameterValue::Number { value: *value });
        }
    }
    if let Some(MeasuredArgument::Path(steps)) = call.argument("access") {
        parameters.insert(
            "access_path".to_owned(),
            ParameterValue::StringList {
                value: steps.clone(),
            },
        );
    }
    for (key, name) in [("doors", "door_selector"), ("openings", "opening_selector")] {
        if call.argument(key).is_some() {
            parameters.insert(
                name.to_owned(),
                ParameterValue::Selector {
                    value: Box::new(Selector::Objects {
                        objects: objects_of_kinds(context, call, key, object)?,
                    }),
                },
            );
        }
    }
    Ok(CompiledRule {
        id: RuleId::new("axioval-measured-shelving").expect("a valid rule id"),
        capability: "axioval:capability.shelf-capacity".into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: BTreeSet::from([object.clone()]),
        },
        parameters,
    })
}

fn length(value: LinearInterval, exact: bool, locator: String) -> Measurement {
    interval(
        (value.lower_metres(), value.upper_metres()),
        Some(QuantityDimension::Length),
        exact,
        locator,
    )
}

fn shelving(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    rule: &CompiledRule,
) -> Result<Measurement, Unavailable> {
    let (_, geometry, access) = declaration(rule, &Parameters(rule))?;
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
    if call.name() == SHELF_LENGTH {
        return Ok(length(shelving.measured.measured(), exact, locator));
    }
    match shelving.measured.clear_height() {
        Some(height) => Ok(length(height, exact, locator)),
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
        let rule = rule(call, object, context)?;
        shelving(call, object, context, &rule).map_err(refused(call.name(), object))
    }
}
