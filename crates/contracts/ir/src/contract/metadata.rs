#![allow(missing_docs)]
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizedText {
    pub default: String,
    #[serde(default)]
    pub translations: std::collections::BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalName {
    #[serde(rename = "typeSystem")]
    pub type_system: String,
    pub name: String,
}
