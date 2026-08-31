#![allow(missing_docs)]
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ParameterValue {
    String {
        value: String,
    },
    Boolean {
        value: bool,
    },
    Integer {
        value: i64,
    },
    Number {
        value: f64,
    },
    Quantity {
        value: f64,
        unit: String,
    },
    Enum {
        value: String,
    },
    Reference {
        value: String,
    },
    ObjectTypeReference {
        #[serde(rename = "objectType")]
        object_type: String,
        #[serde(rename = "includeSubtypes", default = "yes")]
        include_subtypes: bool,
    },
    PropertyReference {
        property: String,
        #[serde(rename = "propertySet")]
        property_set: Option<String>,
    },
    StringList {
        value: Vec<String>,
    },
    ReferenceList {
        value: Vec<String>,
    },
}
const fn yes() -> bool {
    true
}
