//! `centre-line-distance`: how far the centre line of a component's
//! footprint lies from the walls beside it, such as a WC's axis from the
//! side wall.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext, SideDistances,
};
use axioval_ir::Evidence;

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

use crate::support::Unavailable;
use crate::wall_sides::Walls;

/// Requires the centre line of each selected component's footprint to lie
/// between `minimum` and `maximum` from the walls beside it.
///
/// The footprint's least-area rectangle gives the centre line: along its
/// long or short axis (`centre_line` `long` or `short`), or, with
/// `against-wall`, from the wall the component stands against to its front
/// (as `component-clearance` derives it). The distance is measured square
/// to the centre line, to the nearest `wall_selector` wall in the strip
/// beside the footprint on either side of it (as long as the footprint,
/// narrowed by `inset` at both ends), within `reach` of the centre line.
/// With `sides` `nearest` the nearer of the two is judged; with `both`
/// each side is judged on its own.
///
/// A distance is an interval: a sure wall surely closer than `minimum` is
/// "too close", every wall that may be there farther than `maximum` "too
/// far", no wall within `reach` "no wall nearby"; a pass needs every wall
/// that may be there no closer than `minimum` and a sure wall no farther
/// than `maximum`. Anything else is not evaluated.
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `centre_line_sides`, one per judged side, each its distance
/// from the walls' lower bound to the nearest sure wall's upper, judged
/// against the bounds.
pub struct CentreLineDistance;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for CentreLineDistance {
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

/// Which centre line of the footprint's least-area rectangle is measured
/// from.
#[derive(Clone, Copy)]
pub(crate) enum Line {
    Long,
    Short,
    AgainstWall,
}

/// The axis a centre line runs along, the front (for `against-wall`) and
/// the evidence deriving them.
pub(crate) type CentreLine = (usize, Option<[f64; 2]>, Vec<Evidence>);

/// The axis the centre line runs along, the front (for `against-wall`)
/// and the evidence deriving them.
pub(crate) fn line(
    centre: Line,
    walls: &Walls,
    measured: &SideDistances,
) -> Result<CentreLine, Unavailable> {
    let rectangle = measured.rectangle();
    match centre {
        Line::Long | Line::Short => {
            let long = rectangle.long_axis().map_err(|reason| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("the footprint has no long axis: {reason}"),
                )
            })?;
            let axis = if matches!(centre, Line::Long) {
                long
            } else {
                1 - long
            };
            Ok((axis, None, Vec::new()))
        }
        Line::AgainstWall => {
            let back = walls.back(measured)?;
            let front = back.side.opposite().outward(rectangle);
            Ok((back.side.axis(), Some(front), back.evidence))
        }
    }
}
