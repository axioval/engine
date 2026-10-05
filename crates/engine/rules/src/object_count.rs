//! Existence and cardinality of the rule's selection, per source or project.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Requires the rule's selection to hold a bounded number of objects in each
/// source, or in the whole project with `across_sources`.
///
/// This is the check an object rule cannot make: "the model has a building",
/// "at most one site", "an IDS specification requires at least one wall".
/// An object rule over an empty selection reports nothing, which reads as
/// compliance; this one reports the empty selection itself, against the
/// source or the project rather than an object.
///
/// Every source of the session is counted, including one that holds no
/// objects at all: an empty model does not contain a building, and says so.
///
/// `disciplines` counts only the sources playing one of the listed
/// disciplines: a per-source duct count limited to `mep` judges the MEP
/// models alone, and the architecture model raises no finding. A source that
/// declares no discipline may or may not count: per source it is not
/// evaluated (once, `NotRecorded`), and across sources its matching objects
/// are undecided. No source playing a listed discipline leaves the rule not
/// evaluated rather than passed.
///
/// `minimum` and `maximum` bound the count, inclusive. With neither, the
/// rule is an existence check: at least one object must match. An object
/// whose selection cannot be decided may or may not count; the scope is
/// judged only when those objects cannot change the verdict, and is not
/// evaluated otherwise, together with each undecided object.
///
/// It runs as a template ([`axioval_engine::template`]): the count of the
/// objects selected in each scope (`Scopes`), widened by the undecided
/// ones, judged by the range judge.
pub struct ObjectCount;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for ObjectCount {
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
