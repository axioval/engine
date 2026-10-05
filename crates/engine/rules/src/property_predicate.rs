//! `property-predicate`: a declared predicate over one exactly resolved
//! property value.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Checks one property of each selected object against a declared predicate.
///
/// The target is exactly one of `value` (integer), `number`, `quantity`
/// (a value with a unit such as `mm` or `m2`, compared in SI), `text`, `texts`
/// (a list for `one_of`/`none_of`), `boolean`, `date` or `date_time`;
/// `is_defined` and `is_undefined` take none. Text comparisons are
/// case-sensitive unless `case_sensitive` is `false`; `matches` is a regular
/// expression that must match the whole value.
///
/// A `date` or `date_time` target takes the ordered operators and compares
/// chronologically: dates by day, as XML Schema orders them, date-times as
/// instants whatever their UTC offsets. A date stating a time zone equals no
/// date stating none, and within 14 hours of one it is neither before nor
/// after it, so an order there is not evaluated. `precision` `day` reads
/// every date-time and date as the calendar day it states, so a date-time
/// value compares with a `date` target and the reverse; without it that
/// pair is not evaluated. `precision` on any other
/// target is an invalid declaration.
///
/// A comparison presupposes a value: an exactly absent property fails every
/// operator except `is_undefined`, and a value of another type than the
/// target fails too. A quantity is compared only with a `quantity` target of
/// the same dimension, never with a bare number; otherwise the object is not
/// evaluated.
///
/// A numeric target may declare `tolerance`, `relative_tolerance` or
/// `decimals`: a value within the tolerance of the target, or rounding to the
/// same number, is equal to it, and only a value beyond the tolerance is
/// greater or less. A tolerance on a text, text list or boolean target is an
/// invalid declaration.
///
/// It runs as a template ([`axioval_engine::template`]): the stated
/// property, read by the expression evaluator, judged by the generic
/// comparison judge (`Decision::Compare`) through the one comparison every
/// rule uses ([`axioval_engine::comparison`]).
pub struct PropertyPredicate;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

impl RuleCapability for PropertyPredicate {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        crate::templates::run(&TEMPLATE, context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}
