//! The travel distance to the nearest exit as a measured value, walked
//! exactly as `escape-route` walks it: from the space's farthest point or
//! each of its doors, along a walking profile, to the representative points
//! of its exits.

use std::collections::BTreeMap;

use axioval_engine::{
    CompiledRule, MeasuredProvider, Measurement, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity, TableRow};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension, RuleId};

use super::{Judge, Start, declaration};

/// Measures `travel_distance`.
pub(crate) struct TravelMeasures;

fn path(call: &MeasuredCall, key: &str) -> Option<ParameterValue> {
    match call.argument(key) {
        Some(MeasuredArgument::Path(steps)) => Some(ParameterValue::StringList {
            value: steps.clone(),
        }),
        _ => None,
    }
}

fn number(call: &MeasuredCall, key: &str) -> ParameterValue {
    let value = match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    };
    ParameterValue::Number { value }
}

/// The objects of the kinds `key` names, as a selector.
fn kinds(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<ParameterValue, PropertyResolutionError> {
    Ok(ParameterValue::Selector {
        value: Box::new(Selector::Objects {
            objects: crate::measured_kinds::objects_of_kinds(context, call, key, object)?,
        }),
    })
}

/// The `escape-route` declaration `call` stands for.
fn rule(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<CompiledRule, PropertyResolutionError> {
    let start = call.choice("start").unwrap_or("farthest-point");
    let row: TableRow = BTreeMap::from([
        (
            "spaces".to_owned(),
            ParameterValue::Selector {
                value: Box::new(Selector::All),
            },
        ),
        (
            "maximum_travel".to_owned(),
            ParameterValue::Number { value: 0.0 },
        ),
        (
            "route_start".to_owned(),
            ParameterValue::String {
                value: start.to_owned(),
            },
        ),
    ]);
    let mut parameters = BTreeMap::from([
        (
            "uses".to_owned(),
            ParameterValue::Table { value: vec![row] },
        ),
        (
            "exit_path".to_owned(),
            path(call, "exits").ok_or(PropertyResolutionError::InvalidRequest)?,
        ),
        (
            "exit_selector".to_owned(),
            kinds(context, call, "kinds", object)?,
        ),
        ("walking_height".to_owned(), number(call, "walking_height")),
        ("walking_step".to_owned(), number(call, "walking_step")),
    ]);
    if let Some(doors) = path(call, "doors") {
        parameters.insert("door_path".to_owned(), doors);
        parameters.insert(
            "door_selector".to_owned(),
            kinds(context, call, "door_kinds", object)?,
        );
    }
    Ok(CompiledRule {
        id: RuleId::new("axioval-measured-travel").expect("a valid rule id"),
        capability: "axioval:capability.escape-route".into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: std::collections::BTreeSet::from([object.clone()]),
        },
        parameters,
    })
}

impl MeasuredProvider for TravelMeasures {
    fn names(&self) -> &'static [&'static str] {
        &["travel_distance"]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let name = call.name();
        let refused = |(reason, why): crate::support::Unavailable| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{name}` of {object}: {why}"),
            ))
        };
        let rule = rule(call, object, context)?;
        let declared = declaration(&rule).map_err(refused)?;
        let judge = Judge::new(context, &rule, &declared);
        let start = if call.choice("start") == Some("door") {
            Start::Door
        } else {
            Start::FarthestPoint
        };
        let locator = format!("{name}:{object}");
        Ok(match judge.plain_travel(object, start).map_err(refused)? {
            // Walked with exact evidence, the interval holds only the
            // walk's rounding, and the capability cites it as exact.
            Some((lower, upper, true)) => Measurement::Rounded {
                lower,
                upper,
                dimension: Some(QuantityDimension::Length),
                locator,
            },
            Some((lower, upper, false)) => Measurement::Value {
                lower,
                upper,
                dimension: Some(QuantityDimension::Length),
                locator,
            },
            None => Measurement::Absent {
                locator: format!("{locator}: no exit is reached"),
            },
        })
    }
}
