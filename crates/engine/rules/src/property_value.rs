//! Value constraints on an exactly resolved property.
//!
//! Constraints are written as lexical strings, the way XML Schema facets are,
//! and cast to the kind of the value the source resolved:
//!
//! - text compares exactly and case-sensitively, and alone takes `patterns`
//!   and lengths;
//! - a boolean accepts `true`/`1` and `false`/`0`;
//! - an integer takes integer literals, and bounds in any numeric form;
//! - a decimal takes `xs:double` literals and equals within the tolerance
//!   `|x - v| <= |v|·1e-6 + 1e-6`; bounds compare without tolerance;
//! - a date takes `xs:date` literals (`2026-09-27`) and a date-time
//!   `xs:dateTime` literals with a UTC offset, compared chronologically.
//!   With `precision` `day` both read as the calendar day they state, so a
//!   date-time value takes date literals and the reverse.
//!
//! A literal that cannot be cast to the value's kind, or a constraint the kind
//! does not take, makes the object not evaluated (`InvalidDeclaration`): the
//! declaration cannot be applied to this value, which is neither a pass nor a
//! violation. A quantity is compared only when `si_units` states that the
//! literals are in the coherent SI unit of its dimension; otherwise it is not
//! evaluated. A list, bounded value or table is judged by its stated values
//! under the declared `quantifier`, a range also by its open ends.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};
use axioval_ir::{Property, PropertyValue};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// Requires a property's value to meet lexical constraints, cast to its kind.
///
/// Parameters: `property`, or `property_pattern` with an optional
/// `property_set_pattern` (XML Schema patterns over the source's own names,
/// matching them whole); optionally `data_type` (the source-declared type,
/// as in `property-data-type`), `values` (any of), `patterns` (XML Schema
/// regular expressions, any of, whole value), `min_inclusive`,
/// `max_inclusive`, `min_exclusive`, `max_exclusive`, `length`,
/// `min_length`, `max_length`, `total_digits`, `fraction_digits` (for a
/// number only), `optional`, `precision` (`day`, for a date or date-time
/// value only), `quantifier` (`any` or `all`) and `si_units`. All given
/// constraints must hold. A decimal's digits are
/// counted on the shortest decimal that reads back as the same double, the
/// form a model's literal has. Without `optional`, absence, `null`, blank
/// text and an empty list are violations; with it, an absent or `null`
/// property passes and any present value, empty text included, is checked.
///
/// With patterns, every matching property, enumerated exactly through the
/// property service, must meet the constraints, and one must match unless
/// the rule is optional; with `property_set_pattern`, one must match in
/// every set the pattern matches, as IDS requires. Each failing property
/// and each set without a match is its own finding.
///
/// A list, a bounded value or a table is judged by its stated values (see
/// `PropertyValue::stated_values`) under `quantifier`: `any` holds when one
/// of them meets every constraint, `all` when each does, and there is at
/// least one. A range holds every value between its bounds, so under `all`
/// a bounded value open on one side fails every bound on that side. Without
/// a quantifier such a value is not evaluated; a scalar under a quantifier
/// is judged as itself. `si_units` reads numeric literals compared with a
/// quantity in the coherent SI unit of its dimension (metres, square
/// metres, kilograms, ...), the unit the value is stated in; without it a
/// quantity is not evaluated.
///
/// It runs as a template ([`axioval_engine::template`]): the property judged
/// by the facet judge `Decision::Facets`.
pub struct PropertyValueConstraint;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for PropertyValueConstraint {
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

/// The cells of a table value's columns declared `expected`, when the
/// source reports its column types; `None` for any other value.
pub(crate) fn typed_cells<'p>(
    property: &'p Property,
    expected: &str,
) -> Option<Vec<&'p PropertyValue>> {
    let (PropertyValue::Table(rows), Some(types)) = (&property.value, property.column_types())
    else {
        return None;
    };
    let defining = types.defining.eq_ignore_ascii_case(expected);
    let defined = types.defined.eq_ignore_ascii_case(expected);
    Some(
        rows.iter()
            .flat_map(|row| {
                [
                    defining.then_some(&row.defining),
                    defined.then_some(&row.defined),
                ]
            })
            .flatten()
            .collect(),
    )
}
