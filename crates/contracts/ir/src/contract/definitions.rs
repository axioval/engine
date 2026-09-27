#![allow(missing_docs)]
use super::{Citation, ExternalName, LocalizedText, PackageMetadata, ParameterValue, Source};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectTypeDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub external_names: Vec<ExternalName>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropertyDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub value_kind: PropertyValueKind,
    pub unit_dimension: Option<String>,
    pub external_names: Vec<ExternalName>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropertySetDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub external_names: Vec<ExternalName>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub kind: ParameterKind,
    pub referenced_value_kind: Option<PropertyValueKind>,
    #[serde(default = "yes")]
    pub required: bool,
    pub default_value: Option<ParameterValue>,
    #[serde(default)]
    pub allowed_values: Vec<ParameterValue>,
    pub unit_dimension: Option<String>,
    #[serde(default)]
    pub citations: Vec<Citation>,
    /// The columns of a `table` parameter, in presentation order; empty for
    /// every other kind and then omitted, so existing packages serialize
    /// unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<TableColumnDefinition>,
}
/// One named, typed column of a `table` parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableColumnDefinition {
    pub id: String,
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<LocalizedText>,
    pub kind: ColumnKind,
    /// Whether every row must carry a cell in this column.
    #[serde(default = "yes")]
    pub required: bool,
    /// The dimension of a `quantity` column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_dimension: Option<String>,
}
/// The kind of every cell in one table column.
///
/// Cells reuse the scalar value variants: a `string` or `textPattern` cell is
/// a `string` value, a `selector` cell a `selector` value, and so on. A text
/// pattern is a whole-value wildcard pattern (`*` any run, `?` one character,
/// `\` escapes the next character).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKind {
    String,
    TextPattern,
    Number,
    Quantity,
    Integer,
    Boolean,
    Selector,
    Reference,
    /// A calendar day, written as a `date` value.
    Date,
    /// An instant with its UTC offset, written as a `dateTime` value.
    DateTime,
}
impl ColumnKind {
    /// The kind's name in normalized packages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::TextPattern => "textPattern",
            Self::Number => "number",
            Self::Quantity => "quantity",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Selector => "selector",
            Self::Reference => "reference",
            Self::Date => "date",
            Self::DateTime => "dateTime",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub capability: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParameterDefinition>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefinitionPackage {
    pub schema_version: String,
    pub package: PackageMetadata,
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
    #[serde(default)]
    pub object_types: BTreeMap<String, ObjectTypeDefinition>,
    #[serde(default)]
    pub properties: BTreeMap<String, PropertyDefinition>,
    #[serde(default)]
    pub property_sets: BTreeMap<String, PropertySetDefinition>,
    #[serde(default)]
    pub definitions: BTreeMap<String, RuleDefinition>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterKind {
    String,
    Boolean,
    Integer,
    Number,
    Quantity,
    Enum,
    /// An ISO 8601 calendar date.
    Date,
    /// An ISO 8601 date-time with a UTC offset.
    DateTime,
    Reference,
    ObjectTypeReference,
    PropertyReference,
    Selector,
    StringList,
    ReferenceList,
    /// Rows of typed cells; the definition declares the columns.
    Table,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyValueKind {
    String,
    Boolean,
    Integer,
    Number,
    Quantity,
    Enum,
    /// An ISO 8601 calendar date.
    Date,
    /// An ISO 8601 date-time with a UTC offset.
    DateTime,
    Reference,
    StringList,
    ReferenceList,
}
const fn yes() -> bool {
    true
}
