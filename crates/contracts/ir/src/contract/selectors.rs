#![allow(missing_docs)]
use super::ParameterValue;
use crate::{Discipline, TemporalPrecision};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Selector {
    All,
    EntityType {
        #[serde(rename = "objectType")]
        object_type: String,
        #[serde(rename = "includeSubtypes", default = "yes")]
        include_subtypes: bool,
    },
    Property {
        #[serde(rename = "propertySet")]
        property_set: Option<String>,
        property: String,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
        /// Whether text comparisons respect case; `false` folds both sides.
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        /// Whether a text value is trimmed of surrounding whitespace first.
        #[serde(default, skip_serializing_if = "is_false")]
        trim: bool,
        /// How a list value is compared: `any` element or `all` of them
        /// must satisfy the operator. A scalar value counts as a list of
        /// one. Without it a list value is not evaluated, never compared as
        /// a whole; `exists` takes none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantifier: Option<Quantifier>,
        /// How finely dates and date-times compare. `day` reads a date-time
        /// as the calendar day it states, so it compares with a date; without
        /// it a date-time compared with a date is not evaluated. Applies to
        /// a `date` or `dateTime` value only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        precision: Option<TemporalPrecision>,
    },
    /// Objects by the properties whose set and name match XML Schema
    /// patterns, as IDS names them (`Pset_.*Common`).
    ///
    /// A pattern matches the whole name the source states; it names no
    /// concept and is never bound through the package vocabulary. Without
    /// `propertySetPattern` every property set is searched. The selector
    /// holds when at least one property matches and the comparison holds
    /// for `matched` of them (`any` or `all`); no matching property is no
    /// match. The other fields compare each matched property's value as a
    /// `property` selector does.
    PropertyPattern {
        #[serde(
            rename = "propertySetPattern",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        property_set_pattern: Option<String>,
        #[serde(rename = "propertyPattern")]
        property_pattern: String,
        /// Which matched properties must satisfy the comparison.
        matched: Quantifier,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        trim: bool,
        /// How each matched list, bounded or table value is compared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantifier: Option<Quantifier>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        precision: Option<TemporalPrecision>,
    },
    Classification {
        system: String,
        code: String,
        #[serde(rename = "includeDescendants", default)]
        include_descendants: bool,
    },
    AllOf {
        operands: Vec<Selector>,
    },
    AnyOf {
        operands: Vec<Selector>,
    },
    Not {
        operand: Box<Selector>,
    },
    /// Objects by the objects a relationship `path` reaches from them.
    ///
    /// Each step is `Relationship` or `Relationship:direction` (`forward`,
    /// the default, `backward` or `either`), walked one after another; a step
    /// ending in `+` is taken one or more times, reaching every object along
    /// the relationship's chain. The reached objects are tested against
    /// `selector` under `quantifier`.
    Related {
        path: Vec<String>,
        /// Which reached objects must match; `any` is omitted when
        /// serialized.
        #[serde(default, skip_serializing_if = "RelatedQuantifier::is_any")]
        quantifier: RelatedQuantifier,
        selector: Box<Selector>,
    },
    /// Objects of the sources the host declared to play `value`, such as
    /// `structure`.
    ///
    /// A discipline is source metadata, not an object fact: every object of
    /// a source matches or none does. An object whose source declares no
    /// discipline is not evaluated, never a non-match, so a discipline-scoped
    /// rule cannot pass vacuously over a model nobody classified.
    Discipline {
        value: Discipline,
    },
}
impl Default for Selector {
    fn default() -> Self {
        Self::All
    }
}
impl Selector {
    /// A property selector with case-sensitive, untrimmed text comparison.
    #[must_use]
    pub fn property(
        property_set: Option<String>,
        property: impl Into<String>,
        operator: ComparisonOperator,
        value: Option<ParameterValue>,
    ) -> Self {
        Self::Property {
            property_set,
            property: property.into(),
            operator,
            value,
            case_sensitive: true,
            trim: false,
            quantifier: None,
            precision: None,
        }
    }
}
/// Which elements of a list value a property selector must hold for.
///
/// `all` never holds vacuously: an empty list satisfies neither quantifier,
/// as an absent value satisfies no comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quantifier {
    /// At least one element satisfies the comparison.
    Any,
    /// Every element satisfies it, and there is at least one.
    All,
}
/// Which of the objects a `related` selector reaches must match its selector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelatedQuantifier {
    /// At least one reached object matches.
    #[default]
    Any,
    /// Every reached object matches, and at least one is reached.
    All,
    /// No reached object matches; holds when none is reached.
    None,
}
impl RelatedQuantifier {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }
}
/// How a property selector compares the resolved value with its `value`.
///
/// `matches` is a regular expression and `like` a wildcard pattern (`*` any
/// run, `?` one character, `\` escapes); both must match the whole value.
/// `contains` takes a string, `oneOf` and `noneOf` a string list. The ordered
/// operators also take a `date` or `dateTime`, compared chronologically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComparisonOperator {
    Equals,
    NotEquals,
    LessThan,
    LessThanOrEquals,
    GreaterThan,
    GreaterThanOrEquals,
    Matches,
    Like,
    Contains,
    OneOf,
    NoneOf,
    Exists,
}
const fn yes() -> bool {
    true
}
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_true(value: &bool) -> bool {
    *value
}
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
