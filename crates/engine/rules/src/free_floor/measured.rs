//! Whether a shape fits on a space's free floor as a measured member list:
//! the placements the free-space service finds, searched exactly as
//! `free-floor-circle` and `free-floor-rectangle` search them. A found
//! placement is a sure member, a proven absence none, and a search the
//! selections leave open one undecided member, so `count >= 1` is the
//! three-valued fit.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CompiledRule, MeasuredMember, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, RuleCapability, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, RuleId};

use crate::free_floor_circle::FreeFloorCircle;
use crate::free_floor_rectangle::FreeFloorRectangle;

/// Measures `free_placements`.
pub(crate) struct PlacementMeasures;

fn kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<Option<ParameterValue>, PropertyResolutionError> {
    if call.argument(key).is_none() {
        return Ok(None);
    }
    Ok(Some(ParameterValue::Selector {
        value: Box::new(Selector::Objects {
            objects: crate::measured_kinds::objects_of_kinds(context, call, key, object)?,
        }),
    }))
}

/// The free-floor declaration `call` stands for.
fn rule(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<CompiledRule, PropertyResolutionError> {
    let mut parameters = BTreeMap::new();
    let mut put = |name: &str, value: ParameterValue| {
        parameters.insert(name.to_owned(), value);
    };
    let lengths = [
        ("diameter", "diameter_metres"),
        ("width", "width_metres"),
        ("length", "length_metres"),
        ("height", "height_metres"),
        ("band_from", "band_from_metres"),
        ("band_to", "band_to_metres"),
        ("entrance_width", "entrance_path_width"),
    ];
    for (key, name) in lengths {
        if let Some(MeasuredArgument::Length(value)) = call.argument(key) {
            put(name, ParameterValue::Number { value: *value });
        }
    }
    for (key, name) in [("merge", "merge_path"), ("access", "access_path")] {
        if let Some(MeasuredArgument::Path(steps)) = call.argument(key) {
            put(
                name,
                ParameterValue::StringList {
                    value: steps.clone(),
                },
            );
        }
    }
    for (key, name) in [
        ("obstacles", "obstacles"),
        ("swings", "subtract_door_swings"),
        ("doors", "door_selector"),
        ("openings", "opening_selector"),
    ] {
        if let Some(selector) = kinds(context, call, key, object)? {
            put(name, selector);
        }
    }
    if call.choice("shape") == Some("rectangle") {
        put(
            "orientation",
            ParameterValue::String {
                value: "any".into(),
            },
        );
    }
    Ok(CompiledRule {
        id: RuleId::new("axioval-measured-placement").expect("a valid rule id"),
        capability: "axioval:capability.free-floor".into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: BTreeSet::from([object.clone()]),
        },
        parameters,
    })
}

impl MeasuredProvider for PlacementMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["free_placements"]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let rule = rule(call, object, context)?;
        let evaluation = if call.choice("shape") == Some("rectangle") {
            FreeFloorRectangle.evaluate(context, &rule)
        } else {
            FreeFloorCircle.evaluate(context, &rule)
        };
        let member = |certain| MeasuredMember {
            certain,
            fields: BTreeMap::new(),
        };
        if !evaluation.findings().is_empty() {
            return Ok(Vec::new());
        }
        match evaluation.not_evaluated_outcomes().first() {
            None => Ok(vec![member(true)]),
            // The selections leave the fit open: perhaps a placement.
            Some(open) if *open.reason() == NotEvaluatedReason::IncompleteEvidence => {
                Ok(vec![member(false)])
            }
            Some(open) => Err(crate::measured_kinds::resolution_error((
                open.reason().clone(),
                format!("`{}` of {object}: {}", call.name(), open.message()),
            ))),
        }
    }
}
