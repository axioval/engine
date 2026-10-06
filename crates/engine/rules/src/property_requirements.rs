//! Tables of property requirements: which properties an object must, may or
//! must not carry, and the values they may hold.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, ParameterDescriptor, RuleCapability,
    RuleContext, TableColumn,
};

#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

/// The columns of the `requirements` table.
pub(crate) const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("applies_to", ColumnKind::Selector),
    TableColumn::optional("property_set", ColumnKind::TextPattern),
    TableColumn::optional("property", ColumnKind::TextPattern),
    TableColumn::optional("property_set_pattern", ColumnKind::String),
    TableColumn::optional("property_pattern", ColumnKind::String),
    TableColumn::optional("requirement", ColumnKind::String),
    TableColumn::optional("state", ColumnKind::String),
    TableColumn::optional("presence", ColumnKind::String),
    TableColumn::optional("value_like", ColumnKind::TextPattern),
    TableColumn::optional("one_of", ColumnKind::String),
    TableColumn::optional("one_of_like", ColumnKind::String),
    TableColumn::optional("contains", ColumnKind::String),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
    TableColumn::optional("unit", ColumnKind::String),
    TableColumn::optional("per", ColumnKind::String),
    TableColumn::optional("decimals", ColumnKind::Integer),
    TableColumn::optional("minimum_exclusive", ColumnKind::Boolean),
    TableColumn::optional("maximum_exclusive", ColumnKind::Boolean),
    TableColumn::optional("minimum_date", ColumnKind::Date),
    TableColumn::optional("maximum_date", ColumnKind::Date),
    TableColumn::optional("minimum_date_time", ColumnKind::DateTime),
    TableColumn::optional("maximum_date_time", ColumnKind::DateTime),
    TableColumn::optional("precision", ColumnKind::String),
];

/// Checks each selected object against every row of a `requirements` table
/// that applies to it.
///
/// A row names a property by `property_set` and `property` and states what
/// must hold of it in one of two forms. A `requirement` row marks the
/// property `required`, `optional` or `forbidden`. A `state` row is a
/// filtered-template row: its statement, a `presence` (`defined`,
/// `undefined`, `empty`, `not-empty`) or value conditions, must hold
/// (`include`) or must not hold (`exclude`), and an `ignore` row is skipped.
///
/// Value conditions are `value_like` (a whole-value wildcard pattern),
/// `one_of` (values separated by `|`, a backslash escaping the next
/// character), `one_of_like` (wildcard patterns separated by `|`),
/// `contains` (a substring of a text value, or an element of a list value)
/// and a numeric range `minimum`/`maximum` in `unit`, optionally `per` the
/// object's `measured-area` (plan footprint), `measured-volume` (certified
/// body volume), `measured-face-area` (its largest plane face, such as a
/// wall's side), `stated-area` (`area_property`) or `stated-volume`
/// (`volume_property`), and optionally rounded to `decimals` in the row's
/// unit before it is bounded. A date range bounds a date or date-time value
/// by `minimum_date`/`maximum_date` or `minimum_date_time`/
/// `maximum_date_time`, compared chronologically (`precision` `day`
/// compares a date-time with a date by its day). `minimum_exclusive` and
/// `maximum_exclusive` make either kind of bound exclude its own value, so
/// `> 0` fails on 0.
/// `applies_to` restricts a row to the objects a selector picks, such as one
/// exact class or a class with its subtypes; a blank cell applies the row to
/// every selected object.
///
/// Each row yields at most one finding per object, whose message begins with
/// its result: `missing property set`, `missing property`, `missing value`,
/// `forbidden property set present`, `forbidden property present`,
/// `forbidden value` or `wrong value`. With `category_property`
/// each finding starts with the object's category in brackets; with
/// `group_by_value` the findings of one row, category and result that found
/// the same value are one finding relating all their objects.
///
/// `property_set` and `property` hold exact names or wildcard patterns;
/// `property_set_pattern` and `property_pattern` hold XML Schema patterns
/// instead, as IDS names sets and properties (`Pset_.*Common`). A pattern
/// matches the whole name the source states, never a concept. A row with a
/// pattern is checked against every matching property, enumerated exactly
/// through the property service: an included statement must hold for each
/// and needs one to match, an excluded one must hold for none, and the
/// first failing property (by set and name) is reported. A row naming its
/// set as well needs a match in every set it names that holds a property,
/// as IDS requires. A row naming a set
/// without a property asks whether the set holds a property. A required
/// property whose named set holds none is a `missing property set`. A
/// source that cannot enumerate leaves a pattern or set row not evaluated,
/// and keeps `missing property` for an exact one.
///
/// It runs as a template ([`axioval_engine::template`]): every applicable
/// row judged by the requirements-table judge `Decision::Requirements`.
pub struct PropertyRequirements;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for PropertyRequirements {
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
