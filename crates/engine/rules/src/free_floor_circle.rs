//! Exact source-neutral free-floor-circle capability, a template over the
//! free-floor search (`free_floor/template.rs`).

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

/// Exact free-floor circle placement using a trusted free-space service.
pub struct FreeFloorCircle;

static TEMPLATE: LazyLock<Template> = LazyLock::new(crate::free_floor::template::circle);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for FreeFloorCircle {
    fn id(&self) -> &'static str {
        TEMPLATE.id
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
