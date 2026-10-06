//! Objects that agree on one property must agree on another.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Requires objects sharing a `key` value to share their `value` too.
///
/// Doors of one type mark must have one fire rating; walls of one
/// construction type one thickness class. Objects are grouped by their key
/// value, compared without regard to case unless `case_sensitive` is `true`,
/// within one source (or the project with `across_sources`), of one kind
/// unless `same_kind` is `false`, and optionally within the objects a
/// declared `relationship` reaches, such as one storey.
///
/// A group whose members disagree raises one finding per member, naming the
/// members that hold another value. An absent value is a value of its own:
/// "some doors of this type state a rating and some do not" is a
/// disagreement. Objects with no key value (absent, null or blank) form one
/// group of their own in each scope, so a missing key is reported only when
/// those objects disagree on the value.
///
/// With `tolerance` (a number, applied to numbers and to quantities in SI
/// units) or `tolerance_quantity` (a quantity, applied to quantities of its
/// dimension), numeric values agree when the group's range, its greatest
/// value less its least, is within the tolerance. A measured interval counts
/// with its full width, and a group whose range may lie on either side of
/// the tolerance is not evaluated. A group beyond it reports each member
/// farther than the tolerance from the group's median, naming the others; a
/// member whose distance straddles the tolerance is not evaluated. A value
/// the tolerance does not apply to (a quantity of another dimension than
/// `tolerance_quantity`, a number under it) leaves its object not evaluated.
/// Numbers against text, or against an absent value, still disagree.
///
/// It runs as a template ([`axioval_engine::template`]): the stated key and
/// value judged by the group decision `Decision::Consistent`.
pub struct ConsistentValue;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for ConsistentValue {
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
