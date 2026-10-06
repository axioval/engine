//! `area-ratio`: the area of one population of an anchor's members over
//! another's, or over the anchor's own.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::RatioMeasures;

/// Requires the plan area of one population to stand in a ratio to another.
///
/// For each anchor, the footprints of the objects `numerator_selector` picks
/// are summed and divided by the summed footprints of those
/// `denominator_selector` picks, or by the anchor's own footprint when that
/// is not declared. Members are reached as in `related-count`: through the
/// declared relationship, or everywhere in the anchor's source. The ratio
/// must lie within `minimum` and `maximum`, inclusive; at least one is
/// required. Footprints are summed, so overlapping members count twice;
/// select members that do not overlap, such as spaces.
///
/// `numerator_property` or `denominator_property` takes that population's
/// areas from an area-quantity property instead of geometry: a window's
/// glazing area is not its plan footprint.
///
/// `measure: facade` measures the outward-facing surface of each object
/// instead of its footprint, through the facade-area service: the
/// window-to-wall ratio of a storey is the facade area of its windows over
/// that of its external walls and windows (walls are measured with their
/// openings cut out, so the windows belong in the denominator too).
/// `numerator_measure` and `denominator_measure` measure each side on its
/// own instead: a storey's external-wall ratio is the facade area of its
/// external walls over its gross footprint.
///
/// With `numerator_derivation` `light-area`, each numerator member's area is
/// its light-transmitting area, taken from the first step of a fallback
/// that produces one: the area `numerator_property` states, else the
/// `light_area` of the most specific `light_area_table` row whose `width`
/// and `height` equal the member's `overall_width` and `overall_height`
/// (within `light_size_tolerance`) and whose `type` pattern matches the
/// `light_type` name (read from the member or, with `light_type_path`, from
/// the objects that path reaches), else the overall width × height less the
/// frame allowance 2·(W+H)·`frame_width`. A step is skipped only when its
/// input is exactly absent; a value of the wrong kind, an unknown type name
/// a row tests, or tied rows stop the chain, and a member the chain cannot
/// give an area leaves its anchor not evaluated. Evidence records the step
/// behind each area. A stated light area larger than the member's overall
/// area is a finding against the member, and its anchor is not evaluated.
///
/// With `empty_numerator_finding`, an anchor that reaches no numerator
/// object (a space with no window) is a finding of its own instead of a
/// ratio of 0.
///
/// `measure: facade` together with `light-area` is an invalid declaration:
/// a light area over facade areas is no defined ratio, and a
/// window-to-wall ratio measures its windows' facade areas instead.
///
/// Areas are intervals, so the ratio is too. An anchor is judged only when
/// the whole interval is on one side of a bound; one straddling it, an
/// undecided member, or a zero denominator is not evaluated.
///
/// Every run reports the table `ratios`, one row per anchor whose ratio was
/// measured, passing or not: `numerator_area`, `denominator_area` and
/// `ratio` (unknown when the denominator may be zero).
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `ratio_area` (or `light_area`) of each member, summed over the numerator
/// and the denominator populations (`Members::more`), their ratio derived
/// (`Derived::Ratio`) and judged by the generic range judge, graded.
pub struct AreaRatio;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for AreaRatio {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn grades_deviation(&self) -> bool {
        TEMPLATE.grades
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
