//! How many times a stair or ramp falls foul of a fixed-size search, as a
//! measured value: obstructed end spaces, doors in or over a landing,
//! missing tactile strips, breaks in a handrail across landings and rails
//! reaching over accessible surfaces, from the defects found to those found
//! or left open.
//!
//! Each count sums the items of the search's list (`items.rs`): a found
//! item is a sure defect, an undecided one a possible one, exactly as the
//! templates of `stair-geometry` and `ramp-geometry` judge the same items.

use std::collections::BTreeSet;

use axioval_engine::{
    MeasuredMember, Measurement, MemberValue, PropertyResolutionError, RuleContext,
};
use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};

use super::items;

/// The names counted here.
pub(super) const NAMES: &[&str] = &[
    "handrail_breaks",
    "landing_door_conflicts",
    "missing_tactile_strips",
    "obstructed_end_spaces",
    "rails_over_surfaces",
];

/// The stairs `within` reaches from `object`, the first of which a
/// flight's ends are judged in; `object` itself without it.
fn checked(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Option<BTreeSet<ObjectId>>, PropertyResolutionError> {
    let Some(MeasuredArgument::Path(steps)) = call.argument("within") else {
        return Ok(None);
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
    Ok(Some(stairs.into_iter().collect()))
}

/// The searched items `call` counts the defects of on `object`.
fn searched(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let stairs = items::walking(context)?;
    let within = checked(call, object, context)?;
    if call.name() == "handrail_breaks" && call.choice("of") != Some("ramp") {
        // A whole stair's breaks are found on the stair.
        if within.is_some() {
            return Ok(Vec::new());
        }
        return items::stair_continuity(call, object, context, (&stairs, ("stair", "break_doors")));
    }
    let walked = items::walked(call, object, context)?;
    let measured = (&stairs, &walked);
    Ok(match call.name() {
        "obstructed_end_spaces" => items::end_spaces(call, object, context, measured)?,
        "landing_door_conflicts" => {
            let mut found = items::doors(call, object, context, measured, false)?;
            if call.choice("swing") == Some("yes") {
                found.extend(items::doors(call, object, context, measured, true)?);
            }
            found
        }
        "missing_tactile_strips" => {
            // An end on a landing between two of the stair's flights only
            // where `intermediate` asks for it.
            let ends = match within.as_ref().and_then(|stairs| stairs.iter().next()) {
                Some(stair) => items::intermediate_in(
                    &items::stair_of(call, (stair, "stair"), context)?,
                    object,
                ),
                None => (false, false),
            };
            let asked = call.choice("intermediate") == Some("yes");
            items::tactile_strips(call, object, context, measured, ends)?
                .into_iter()
                .filter(|item| {
                    asked
                        || !matches!(
                            item.fields.get("intermediate"),
                            Some(MemberValue::Truth { value: true, .. })
                        )
                })
                .collect()
        }
        "handrail_breaks" => items::ramp_rails(call, object, context, measured, true)?,
        _ => items::ramp_rails(call, object, context, measured, false)?,
    })
}

/// The defects `call` counts on `object`: from the items surely found to
/// those found or left open.
pub(super) fn count(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, PropertyResolutionError> {
    let (mut found, mut open, mut exact) = (0_u32, 0_u32, true);
    for item in &searched(call, object, context)? {
        match item.fields.get("found") {
            Some(MemberValue::Truth { value: true, .. }) => {
                found += 1;
                exact &= item.exact;
            }
            Some(MemberValue::Truth { value: false, .. }) => {}
            _ => open += 1,
        }
    }
    Ok(Measurement::Cited {
        lower: f64::from(found),
        upper: f64::from(found.saturating_add(open)),
        dimension: None,
        locator: format!("{}:{object}", call.name()),
        exact: exact && open == 0,
    })
}
