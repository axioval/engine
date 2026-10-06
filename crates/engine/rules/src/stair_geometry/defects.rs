//! How many times a stair or ramp falls foul of a fixed-size search, as a
//! measured value: obstructed end spaces, doors in or over a landing,
//! missing tactile strips, breaks in a handrail across landings and rails
//! reaching over accessible surfaces, from the defects found to those found
//! or left open.
//!
//! Where a search answers per item (`end_spaces`, `landing_doors` and
//! `landing_swings`, a ramp's `rail_continuity` and `rail_obstructions` in
//! `items.rs`), the count sums its items: a found one is a sure defect, an
//! undecided one a possible one. The tactile strips and a whole stair's
//! breaks are still counted by running that one check of `stair-geometry`
//! alone with the sizes and kinds the value states, an inverted wrapper
//! kept until `stair-geometry` is a template too (#280).

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CompiledRule, MeasuredMember, Measurement, PropertyResolutionError, RuleCapability, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{NotEvaluatedReason, ObjectId, RuleId};

use super::{RampGeometryCheck, StairGeometryCheck};

/// The names counted here.
pub(super) const NAMES: &[&str] = &[
    "handrail_breaks",
    "landing_door_conflicts",
    "missing_tactile_strips",
    "obstructed_end_spaces",
    "rails_over_surfaces",
];

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

/// The capability declaration counting `call`'s defects.
fn declaration(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<BTreeMap<String, ParameterValue>, PropertyResolutionError> {
    let mut parameters = BTreeMap::new();
    for (key, name) in [
        ("obstacles", "end_space_obstacles"),
        ("landing", "landing_objects"),
        ("doors", "landing_doors"),
        ("tactiles", "tactile_objects"),
        ("rails", "handrail_objects"),
        ("surfaces", "accessible_surface_selector"),
        ("flights", "stair_flights"),
        ("break_doors", "handrail_break_doors"),
    ] {
        if call.argument(key).is_some() {
            let objects = if key == "flights" {
                crate::measured_kinds::objects_of_kinds_including(context, call, key, object)?
            } else {
                crate::measured_kinds::objects_of_kinds(context, call, key, object)?
            };
            parameters.insert(
                name.to_owned(),
                ParameterValue::Selector {
                    value: Box::new(Selector::Objects { objects }),
                },
            );
        }
    }
    let lengths: &[(&str, &str)] = match call.name() {
        "obstructed_end_spaces" => &[
            ("depth", "end_space_depth"),
            ("width", "end_space_width"),
            ("height", "end_space_height"),
        ],
        "missing_tactile_strips" => &[("offset", "tactile_offset"), ("depth", "tactile_depth")],
        _ => &[
            ("height", "landing_door_height"),
            ("reach_across", "handrail_reach_across"),
            ("reach_above", "handrail_reach_above"),
            ("tolerance", "handrail_continuity_tolerance"),
        ],
    };
    for (key, name) in lengths {
        if let Some(MeasuredArgument::Length(value)) = call.argument(key) {
            parameters.insert((*name).to_owned(), metres(*value));
        }
    }
    if let Some(MeasuredArgument::Path(steps)) = call.argument("stair") {
        parameters.insert(
            "stair_path".to_owned(),
            ParameterValue::StringList {
                value: steps.clone(),
            },
        );
    }
    let mut flag = |name: &str| {
        parameters.insert(name.to_owned(), ParameterValue::Boolean { value: true });
    };
    if call.choice("swing") == Some("yes") {
        flag("landing_door_swing");
    }
    if call.choice("intermediate") == Some("yes") {
        flag("tactile_on_intermediate_landings");
    }
    match call.name() {
        "handrail_breaks" if call.choice("of") == Some("ramp") => {
            flag("check_continuous_handrails");
        }
        "handrail_breaks" => flag("handrail_continuous_across_landings"),
        "rails_over_surfaces" => flag("check_rails_obstruction"),
        _ => {}
    }
    Ok(parameters)
}

/// The objects the check runs on: `object`, or with `within` the whole
/// stairs that path reaches from it, whose check reports about `object`.
fn checked(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<BTreeSet<ObjectId>, PropertyResolutionError> {
    let Some(MeasuredArgument::Path(steps)) = call.argument("within") else {
        return Ok(BTreeSet::from([object.clone()]));
    };
    let everything: Vec<&axioval_ir::Object> = context.project.objects().collect();
    let (stairs, _) = crate::support::Traversal::path(steps)
        .and_then(|path| path.related(context, object, &everything))
        .map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{}` of {object}: {why}", call.name()),
            ))
        })?;
    if stairs.is_empty() {
        return Err(PropertyResolutionError::Incomplete(format!(
            "`{}` of {object}: `within` reaches no stair",
            call.name()
        )));
    }
    Ok(stairs.into_iter().collect())
}

/// The searched items a ramp's or a flight's defects are among, where the
/// item searches answer them: each found one a sure defect, each undecided
/// one a possible defect.
fn searched(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Option<Vec<MeasuredMember>>, PropertyResolutionError> {
    let ramp = call.choice("of") == Some("ramp");
    let name = call.name();
    let searched = matches!(name, "obstructed_end_spaces" | "landing_door_conflicts")
        || (ramp && matches!(name, "handrail_breaks" | "rails_over_surfaces"));
    if !searched {
        return Ok(None);
    }
    checked(call, object, context)?;
    let stairs = super::items::walking(context)?;
    let walked = super::items::walked(call, object, context)?;
    let measured = (&stairs, &walked);
    Ok(Some(match name {
        "obstructed_end_spaces" => super::items::end_spaces(call, object, context, measured)?,
        "landing_door_conflicts" => {
            let mut found = super::items::doors(call, object, context, measured, false)?;
            if call.choice("swing") == Some("yes") {
                found.extend(super::items::doors(call, object, context, measured, true)?);
            }
            found
        }
        "handrail_breaks" => super::items::ramp_rails(call, object, context, measured, true)?,
        _ => super::items::ramp_rails(call, object, context, measured, false)?,
    }))
}

/// The defects `call` counts on `object`: from the items surely found to
/// those found or left open.
pub(super) fn count(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, PropertyResolutionError> {
    if let Some(items) = searched(call, object, context)? {
        let (mut found, mut open, mut exact) = (0_u32, 0_u32, true);
        for item in &items {
            match item.fields.get("found") {
                Some(axioval_engine::MemberValue::Truth { value: true, .. }) => {
                    found += 1;
                    exact &= item.exact;
                }
                Some(axioval_engine::MemberValue::Truth { value: false, .. }) => {}
                _ => open += 1,
            }
        }
        return Ok(Measurement::Cited {
            lower: f64::from(found),
            upper: f64::from(found.saturating_add(open)),
            dimension: None,
            locator: format!("{}:{object}", call.name()),
            exact: exact && open == 0,
        });
    }
    let ramp = call.choice("of") == Some("ramp");
    let rule = CompiledRule {
        id: RuleId::new("axioval-measured-defects").expect("a valid rule id"),
        capability: if ramp {
            "axioval:capability.ramp-geometry"
        } else {
            "axioval:capability.stair-geometry"
        }
        .into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: checked(call, object, context)?,
        },
        parameters: declaration(call, object, context)?,
    };
    let evaluation = if ramp {
        RampGeometryCheck.evaluate(context, &rule)
    } else {
        StairGeometryCheck.evaluate(context, &rule)
    };
    let mut open = 0_u32;
    for outcome in evaluation.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(found) if found == object => open += 1,
            Some(_) => {}
            // The declaration itself is refused: no count.
            None => {
                return Err(crate::measured_kinds::resolution_error((
                    outcome.reason().clone(),
                    format!("`{}` of {object}: {}", call.name(), outcome.message()),
                )));
            }
        }
    }
    let found = evaluation
        .findings()
        .iter()
        .filter(|finding| finding.scope == axioval_ir::Scope::Object(object.clone()))
        .count();
    let found = u32::try_from(found).unwrap_or(u32::MAX);
    if found == 0 && open > 0 {
        // Nothing found, but what is open could be a defect: reasons as the
        // capability's.
        if let Some(first) = evaluation.not_evaluated_outcomes().first()
            && *first.reason() != NotEvaluatedReason::IncompleteEvidence
        {
            return Err(crate::measured_kinds::resolution_error((
                first.reason().clone(),
                format!("`{}` of {object}: {}", call.name(), first.message()),
            )));
        }
    }
    // Cited as exactly as the capability cites what it found.
    let exact = evaluation
        .findings()
        .iter()
        .all(|finding| finding.evidence.iter().all(|evidence| evidence.exact));
    Ok(Measurement::Cited {
        lower: f64::from(found),
        upper: f64::from(found.saturating_add(open)),
        dimension: None,
        locator: format!("{}:{object}", call.name()),
        exact: exact && open == 0,
    })
}
