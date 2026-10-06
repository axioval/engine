//! `external-wall-validation`: a model's declared external walls match
//! the building envelope derived from its spaces.
//!
//! ADR 0004: envelope membership is measured by an
//! [`EnvelopeMembershipServiceHandle`](axioval_engine::EnvelopeMembershipServiceHandle),
//! as measured values ([`EnvelopeMeasures`]); whether the model's
//! declaration agrees with the derivation is the template's policy.
//!
//! Which objects bound the envelope is the ruleset's choice, not the host's,
//! and it travels in each request as the measured values' arguments:
//!
//! - `all-spaces` derives around the objects `bounding_selector` selects;
//! - `gross-area-groups` derives around the members of the groups
//!   `gross_area_group_selector` selects, reached from each group along
//!   `gross_area_group_path`.
//!
//! `derivations` lists one or both, and each is reported on its own: every
//! finding and not-evaluated outcome names its derivation. With both, an
//! object on one envelope and not the other is reported too, whatever it
//! declares, and a source in which no selected object is declared external
//! is one finding against the source rather than one per object. An object a
//! selector cannot decide might be a bounding space, so that derivation is not
//! evaluated rather than derived around a guessed region.
//!
//! Applicability is deliberately absent. The source provider inspected a
//! model's industry domain and returned an "irrelevant" flag the rule had to
//! interpret; if a rule should not apply to a model, that belongs to the
//! selector, not behind the evidence seam.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::EnvelopeMeasures;

/// Requires a model's declared external walls to match the derived envelope.
///
/// It runs as a template ([`axioval_engine::template`]): each derivation
/// listed measured once per rule (`envelope_size`), then each source where
/// the rule selects an object judged by how many of them the model
/// declares external (`external_declarations`), and each selected object
/// by its declaration against each envelope (`declared_external`,
/// `on_envelope`) and, with both derivations, by the two envelopes against
/// each other (`bounds_envelope` leaving out what bounds either).
pub struct ExternalWallValidation;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for ExternalWallValidation {
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
