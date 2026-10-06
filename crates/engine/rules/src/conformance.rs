//! Every selected object must satisfy a declared requirement selector.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Checks each selected object against a `requirement` selector.
///
/// This is the shape of every "agreed list" check. Each allowed combination
/// of values is one `allOf` of property conditions, and the agreed list is
/// their `anyOf`: a space passes when its type, name and number together
/// match at least one agreed row, a door when its construction type is one
/// of the types agreed for doors. An object the requirement cannot be
/// decided for (a property the source cannot resolve) is not evaluated; it
/// is never read as conforming or as violating.
///
/// A failing object is one of two results. When none of the properties the
/// requirement consults has a value (each is absent, null or blank), the
/// object gets its own "no value" finding naming them. Otherwise its values
/// are unknown to the list: objects holding the same combination of values
/// (an empty one shown as absent, null or blank) share one finding, against
/// the first of them, naming the values and relating the others. A
/// requirement that consults no property reports each object on its own.
///
/// The finding cites every property fact the requirement consulted, so the
/// reviewer sees the values that failed. `message` replaces the default
/// text of the unknown-value finding and is followed by the values.
///
/// It runs as a template ([`axioval_engine::template`]): the requirement
/// judged by the group decision `Decision::Conforms`.
pub struct SelectorConformance;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SelectorConformance {
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
