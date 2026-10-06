//! `related-count`: how many objects each anchor reaches.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Requires each selected anchor to have a bounded number of related objects.
///
/// Anchors are the rule's selection: a space whose doors are counted, a zone
/// whose member spaces are counted, a building that must contain walls.
/// Related objects are those `related_selector` picks (everything by
/// default) that the declared `relationship` reaches from the anchor; with
/// no relationship, every such object in the anchor's own source counts.
/// `minimum` and `maximum` bound the count, inclusive; at least one is
/// required.
///
/// With `same_ends`, a relationship path like `path`, a related object
/// counts only when that path reaches the same set of objects from it as
/// from the anchor: a revolving door needs a swing door between the same
/// spaces, not any door of one of them. An object whose ends cannot be read
/// counts as unknown; an anchor whose ends cannot be read or reach nothing
/// is not evaluated.
///
/// An object whose membership in `related_selector` cannot be decided is
/// counted as unknown. The anchor is judged when the verdict holds either
/// way, and is not evaluated otherwise.
///
/// It runs as a template ([`axioval_engine::template`]): the count of the
/// anchor's members (`Members`), widened by the undecided ones
/// (`UndecidedMembers::Widen`), judged and graded by the range judge.
pub struct RelatedCount;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for RelatedCount {
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
