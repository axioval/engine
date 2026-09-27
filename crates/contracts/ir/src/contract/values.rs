#![allow(missing_docs)]
use super::Selector;
use crate::{Date, DateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One row of a `table` value: cells keyed by column ID.
pub type TableRow = BTreeMap<String, ParameterValue>;
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
    /// An ISO 8601 calendar date, `YYYY-MM-DD`; any other text is refused
    /// when the package is read.
    Date {
        value: Date,
    },
    /// An ISO 8601 date-time with an explicit UTC offset,
    /// `YYYY-MM-DDThh:mm:ss[.f](Z|±hh:mm)`; a date-time without an offset is
    /// refused when the package is read.
    DateTime {
        value: DateTime,
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
    Selector {
        value: Box<Selector>,
    },
    StringList {
        value: Vec<String>,
    },
    ReferenceList {
        value: Vec<String>,
    },
    /// Rows of a `table` parameter, in declared order.
    Table {
        value: Vec<TableRow>,
    },
}
const fn yes() -> bool {
    true
}
