//! The view from a component's eye as a measured member list
//! (`sight_view`), read as `component-visibility` reads it: the targets
//! surely in view, those that may be, and those hidden in range.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};

use super::{Picked, Services, View};
use crate::measured_kinds::{interval, refused};
use crate::support::{Unavailable, invalid};

/// Measures the view from a component's eye.
pub(crate) struct ViewMeasures;

fn length(call: &MeasuredCall, key: &str) -> Result<f64, Unavailable> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Ok(*value),
        _ => Err(invalid(format!("`{key}` is required"))),
    }
}

fn picked(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<Picked, Unavailable> {
    let selection = crate::measured_kinds::selection(context, call, key, None)
        .map_err(crate::selection::property_error)?
        .ok_or_else(|| invalid(format!("`{key}` is required")))?;
    Ok(Picked {
        matched: selection.matched,
        undecided: selection.undecided,
    })
}

/// One item: how many targets are in view (from those surely in view to
/// every one that may be), how many are hidden in range, which, and the
/// words naming the eye and the undecided targets.
fn view(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let services = Services::of(context)?;
    let component = context
        .project
        .object(object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    let (eye_height, radius) = (length(call, "eye_height")?, length(call, "radius")?);
    let targets = picked(context, call, "targets")?;
    let blockers = picked(context, call, "blockers")?;
    let view = View::of(
        &services,
        (eye_height, radius),
        &targets,
        &blockers,
        component,
    )?;
    let exact = view.evidence.iter().all(|evidence| evidence.exact);
    #[allow(clippy::cast_precision_loss)]
    let (sure, most, hidden) = (
        view.visible.len() as f64,
        (view.visible.len() + view.unknown.len()) as f64,
        view.hidden.len() as f64,
    );
    let within = format!("within {radius} m of the eye {eye_height} m above the base of {object}");
    let mut undecided = format!("{} target(s) {within} are undecided:", view.unknown.len());
    for (target, why) in view.unknown.iter().take(3) {
        let _ = write!(undecided, " {target} {why};");
    }
    undecided.pop();
    let at = |field: &str| format!("sight_view:{object}:{field}");
    let count = |(low, high): (f64, f64), field: &str| {
        MemberValue::Measured(interval((low, high), None, exact, at(field)))
    };
    let mut found = view.visible.clone();
    found.extend(view.hidden.iter().cloned());
    Ok(vec![MeasuredMember {
        certain: true,
        exact,
        fields: BTreeMap::from([
            ("visible", count((sure, most), "visible")),
            ("sure", count((sure, sure), "sure")),
            ("hidden", count((hidden, hidden), "hidden")),
            (
                "seen",
                MemberValue::Objects {
                    objects: view.visible.clone(),
                },
            ),
            ("found", MemberValue::Objects { objects: found }),
            ("within", MemberValue::Text { text: within }),
            ("undecided", MemberValue::Text { text: undecided }),
        ]),
        evidence: Vec::new(),
    }])
}

impl MeasuredProvider for ViewMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["sight_view"]
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
        view(call, object, context).map_err(refused(call.name(), object))
    }
}
