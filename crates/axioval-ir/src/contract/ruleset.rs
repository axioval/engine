#![allow(missing_docs)]
use super::{LocalizedText, PackageMetadata, ParameterValue, Selector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleInstance {
    pub id: String,
    pub definition_id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "severity")]
    pub severity: Severity,
    pub message: Option<LocalizedText>,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParameterValue>,
    #[serde(default)]
    pub applicability: Selector,
    #[serde(default)]
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleFolder {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    #[serde(default)]
    pub rules: Vec<RuleInstance>,
    #[serde(default)]
    pub folders: Vec<RuleFolder>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleSetPackage {
    pub schema_version: String,
    pub package: PackageMetadata,
    pub definition_packages: Vec<String>,
    pub root: RuleFolder,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    #[default]
    Error,
    Warning,
    Info,
}
const fn yes() -> bool {
    true
}
fn severity() -> Severity {
    Severity::Error
}
