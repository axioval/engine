#![allow(missing_docs)]
use super::ParameterValue;
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
/// How a property selector compares the resolved value with its `value`.
///
/// `matches` is a regular expression and `like` a wildcard pattern (`*` any
/// run, `?` one character, `\` escapes); both must match the whole value.
/// `contains` takes a string, `oneOf` and `noneOf` a string list.
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
