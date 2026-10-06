//! Relative counts: provided objects against required ones, per anchor or
//! per property-value group.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Requires enough provided objects for the required ones at each anchor or
/// in each group.
///
/// **Ratio mode.** With `provided_unit` p and `required_unit` r, the anchor
/// passes when `provided / p` stands in `operator` (`equal`, `not_equal`,
/// `greater`, `at_least`, `less`, `at_most`) to `required / r`: "one
/// washbasin (provided, p = 1) per four workplaces (required, r = 4), at
/// least" is `washbasins * 4 >= workplaces * 1`. Integer arithmetic keeps it
/// exact.
///
/// **Small counts.** With `small_required_below` n and `small_provided` k,
/// both declared together, a required count from 1 up to but excluding n is
/// judged as `provided operator k` instead of by the ratio: "with fewer than
/// four workplaces, at least one washbasin" is n = 4, k = 1; "below ten
/// workplaces nothing is required" is n = 10, k = 0 with `at_least`. A
/// required count of zero is always judged by the ratio.
///
/// **Table mode.** `table` lists rows `R:P`, "from R required objects on, at
/// least P provided". The row with the largest R not above the required
/// count applies. Beyond the last row, each further `additional_required`
/// required objects need `additional_provided` more. Below the first row the
/// table sets no requirement, so the anchor or group is skipped rather than
/// extrapolated; a table of increments alone applies them from zero.
/// Parameters have no table type, so a row is written as text and a
/// malformed one is a declaration error.
///
/// **Anchors.** By default the rule's selection names the anchors, and the
/// relationship works as in `related-count`; with no relationship an
/// anchor's whole source is counted, so selecting the building checks the
/// whole model and selecting storeys checks each storey. An anchor with any
/// undecided member is not evaluated.
///
/// **Groups.** With `group_property`, the rule's selection is instead the set
/// of objects counted, and each is counted in the group of its
/// `group_property` value, within one source unless `across_sources`. Text
/// values are trimmed and compared ignoring case unless `case_sensitive`.
/// Relationship parameters do not apply. A group that has required objects
/// and no provided object is reported as present only in the required set,
/// whatever the mode would say. A group's finding is raised against its
/// lowest required object (its lowest provided object when it has none) and
/// names every member. A counted object with no group value gets a finding
/// of its own; one whose value cannot be read leaves every group of its
/// scope not evaluated, since it could belong to any of them.
///
/// It runs as a template ([`axioval_engine::template`]): the counts of an
/// anchor's two member populations, or of each group, judged by
/// `Decision::Proportion`.
pub struct RelativeCount;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for RelativeCount {
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
