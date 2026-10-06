//! Identifiers that must not repeat within a scope.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Requires a property to be unique among the selected objects of one scope.
///
/// The typical use is a space number. Values are compared after trimming
/// (unless `trim` is `false`) and without regard to case (unless
/// `case_sensitive` is `true`). The scope is one source, since two models of
/// a federation number independently; `across_sources` widens it to the whole
/// project, and a declared `relationship` narrows it to the objects that
/// reach the same related objects, such as the spaces of one storey.
///
/// Numbers and quantities may be compared under a declared tolerance (see
/// `tolerance`, `relative_tolerance` and `decimals`), quantities only with
/// quantities of the same dimension. Rounding to `decimals` sorts values into
/// classes: objects whose values round alike are duplicates of each other.
/// A tolerance is not transitive, so it is judged pair by pair: an object is
/// a duplicate of every other object whose value lies within the tolerance
/// of its own, and its finding names exactly those; `1.0` and `1.2` under a
/// tolerance of `0.1` are both near `1.1` but not near each other.
///
/// Every object sharing a value gets one finding naming the others. An
/// object without a value (absent, null or blank) gets a finding of its own
/// unless `require_value` is `false`, in which case it is not compared.
///
/// It runs as a template ([`axioval_engine::template`]): the stated
/// property judged by the group decision `Decision::Unique`.
pub struct UniqueValue;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for UniqueValue {
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
