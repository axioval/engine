#![allow(missing_docs)]
use super::{ExternalName, LocalizedText, PackageMetadata, ParameterValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectTypeDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub external_names: Vec<ExternalName>,
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropertySetDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub external_names: Vec<ExternalName>,
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefinitionPackage {
    pub schema_version: String,
    pub package: PackageMetadata,
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
    Reference,
    ObjectTypeReference,
    PropertyReference,
    Selector,
    StringList,
    ReferenceList,
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
    Reference,
    StringList,
    ReferenceList,
}
const fn yes() -> bool {
    true
}
