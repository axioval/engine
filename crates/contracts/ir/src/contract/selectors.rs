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
    Exists,
}
const fn yes() -> bool {
    true
}
