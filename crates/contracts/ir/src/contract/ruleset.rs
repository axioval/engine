#![allow(missing_docs)]
use super::{Citation, LocalizedText, PackageMetadata, ParameterValue, Selector, Source};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One named population participating in a rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetGroup {
    pub id: String,
    pub name: LocalizedText,
    pub description: Option<LocalizedText>,
    pub selector: Selector,
}

/// Rich applicability: several independently selected, named populations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupApplicability {
    pub groups: BTreeMap<String, TargetGroup>,
}

/// Which objects a rule applies to.
///
/// MCS allows either one selector or named target groups. A selector always
/// carries a `kind` discriminator and groups never do, so the untagged form is
/// unambiguous.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuleApplicability {
    Selector(Selector),
    Groups(GroupApplicability),
}
impl Default for RuleApplicability {
    fn default() -> Self {
        Self::Selector(Selector::All)
    }
}
impl From<Selector> for RuleApplicability {
    fn from(selector: Selector) -> Self {
        Self::Selector(selector)
    }
}

/// A human-readable expected state bound to named target groups.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub statement: LocalizedText,
    pub description: Option<LocalizedText>,
    pub target_groups: Vec<String>,
    #[serde(default)]
    pub citations: Vec<Citation>,
}

/// Applies one citation to bound rule parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterCitation {
    pub parameter_ids: Vec<String>,
    pub citation: Citation,
}

/// A package-contained explanatory image. The engine never reads the file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanatoryImage {
    pub id: String,
    pub path: String,
    pub media_type: String,
    pub alternative_text: LocalizedText,
    pub caption: Option<LocalizedText>,
}

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
    pub applicability: RuleApplicability,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub parameter_citations: Vec<ParameterCitation>,
    #[serde(default)]
    pub explanatory_images: Vec<ExplanatoryImage>,
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
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
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
