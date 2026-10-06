//! Exact, source-neutral property-to-property comparison capability.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Compares a property on relationship-selected candidates with a property on each checked object.
///
/// Numbers and quantities may be compared under a declared `tolerance`,
/// `relative_tolerance` or `decimals`, applied between the compared value
/// and the target after its factor: within the tolerance they are equal, and
/// only beyond it greater or less. A tolerance on a text, text list or
/// boolean constant target is an invalid declaration.
///
/// Dates and date-times compare chronologically with the ordered operators,
/// against another property or a `target_date` or `target_date_time`
/// constant: dates by day, date-times as instants whatever their UTC
/// offsets. `precision` `day` reads every date-time as the calendar day it
/// states, so a date-time compares with a date; without it that pair is not
/// evaluated. A factor other than 1 or a tolerance does not apply to them,
/// and `precision` applies to nothing else.
///
/// `between` takes one minimum and one maximum: numbers, quantities, dates
/// or date-times, or properties of the checked object. Two constant bounds
/// must be of one kind. A value outside either bound is outside the range,
/// whatever the other bound leaves undecided; otherwise an undecided order
/// leaves the candidate not evaluated.
///
/// Candidates are the checked object itself (`checked`), the members of a
/// group it shares (`shared`), the objects a relationship or `path` reaches
/// from it (`related`), or the objects in the same space or building as it
/// (`same_space`, `same_building`): those whose nearest `container_selector`
/// object, climbed to along the declared relationship steps, is one of its
/// own.
/// With `container_relationship` `axioval:derived.same-level`, a candidate
/// also shares the checked object's container when one of its containers is
/// on one level with one of the object's in another source (an MEP model's
/// storey and the architecture model's at one elevation); an undecided
/// level leaves the object not evaluated.
///
/// It runs as a template ([`axioval_engine::template`]): the candidates
/// judged by the candidate comparison judge `Decision::Compared`.
pub struct PropertyComparison;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for PropertyComparison {
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
