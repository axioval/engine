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
        }
    }
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
