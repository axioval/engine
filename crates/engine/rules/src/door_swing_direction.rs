//! `door-swing`: which spaces each door swings into.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ObjectFrameServiceHandle,
    ParameterDescriptor, RuleCapability, RuleContext,
};
use axioval_ir::ObjectId;

use crate::door_swing;
use crate::support::Unavailable;

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::SwingMeasures;

/// Requires each selected door to swing into, or not into, the spaces it
/// opens onto.
///
/// A door's spaces are what `space_path` reaches from it. Its swing comes
/// from its leaves as the object-frame service states them (with IFC, the
/// operation type, the panels and the placement). Which side of the door a
/// space lies on is asked of the free-space service at two small probes per
/// hinged leaf, halfway through its sweep and three quarters of its width
/// out from the hinge: one on the side it opens into, one behind it.
///
/// - `swing_into`: among the reached spaces this selector picks, the door
///   must swing into at least one. It is a finding when every picked space
///   surely lies behind the door and none on its swing side.
/// - `swing_not_into`: the door must swing into none of the reached spaces
///   this selector picks.
///
/// A double-acting leaf swings into the spaces on both sides. A door
/// without a hinged leaf (sliding, rolling up) swings into no space and is
/// not evaluated, like a door whose leaves cannot be read. A space whose
/// selection is undecided, a probe the service cannot answer, and a space
/// neither probe lies in decide only what they cannot change.
///
/// It runs as a template ([`axioval_engine::template`]): the spaces a door
/// opens onto, as the measured list `swing_spaces` probes them, judged one
/// by one against `swing_not_into` and together against `swing_into`.
pub struct DoorSwing;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for DoorSwing {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The leaves of `door`, refused when none is hinged: such a door swings
/// into no space.
pub(crate) fn hinged_leaves(
    frames: &ObjectFrameServiceHandle,
    door: &ObjectId,
) -> Result<axioval_engine::DoorLeaves, Unavailable> {
    let leaves = door_swing::leaves(frames, door)?;
    if leaves.hinged().next().is_none() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the door has no hinged leaf ({}), so it swings into no space",
                leaves.operation()
            ),
        ));
    }
    Ok(leaves)
}
