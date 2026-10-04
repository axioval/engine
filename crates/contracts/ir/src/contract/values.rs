#![allow(missing_docs)]
use super::{ColumnKind, Expression, Selector};
use crate::{Date, DateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One row of a `table` value: cells keyed by column ID.
pub type TableRow = BTreeMap<String, ParameterValue>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// An expression the capability evaluates per object
    /// ([`Expression`]); type checked when the ruleset is compiled.
    Expression {
        value: Box<Expression>,
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
    /// Rows of a `table` parameter read from a data file shipped in the
    /// package: a CSV file, or one named sheet of an xlsx workbook.
    ///
    /// `path` is relative to the package root, `/`-separated, without `.`
    /// or `..` segments; `sha256` is the lowercase hex SHA-256 of the
    /// file's bytes, so the package pins the exact data it was reviewed
    /// with. `columns` declares every column of the file: its header, the
    /// table column it fills and that column's kind. The file is data,
    /// never code: a host loads it before binding
    /// (`axioval_engine::load_table_files`), the binder checks the declared
    /// columns against the parameter's table and binds the rows exactly as
    /// it binds the same rows written inline. A reference that was not
    /// loaded is refused when the package is bound.
    TableFile(Box<TableFileReference>),
}

/// The data file a [`ParameterValue::TableFile`] names, and its rows once
/// loaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableFileReference {
    pub path: String,
    /// The sheet of an xlsx workbook; a CSV file has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    pub sha256: String,
    pub columns: Vec<TableFileColumn>,
    /// The rows read from the file, once loaded; never part of the
    /// package's wire form.
    #[serde(skip)]
    pub rows: Option<Vec<TableRow>>,
}

/// One column of a [`ParameterValue::TableFile`]: the table column `id` it
/// fills, its `kind` (which must be the parameter column's), the file's
/// header naming it (`id` when omitted) and, for a `quantity` column, the
/// unit every cell is stated in.
///
/// A `selector` column cannot be read from a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableFileColumn {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub kind: ColumnKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl TableFileColumn {
    /// The header naming the column in the file.
    #[must_use]
    pub fn header(&self) -> &str {
        self.header.as_deref().unwrap_or(&self.id)
    }
}
const fn yes() -> bool {
    true
}
