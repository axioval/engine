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
    /// Severities graded by how far a measured value misses its bound, for
    /// capabilities that report the deviation; see [`SeverityBand`]. Empty
    /// keeps the rule's severity and is omitted when serialized.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub severity_bands: Vec<SeverityBand>,
    /// Severities chosen by the objects a finding involves; see
    /// [`SeverityOverride`]. Empty keeps the severity and is omitted when
    /// serialized.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub severity_overrides: Vec<SeverityOverride>,
}

/// One entry of a rule's `severityOverrides`: a finding whose subject or any
/// related object `selector` selects takes `severity`.
///
/// Entries are tried in order and the first that holds decides, over the
/// severity the rule (and its bands) gave. An entry that cannot be decided
/// for an involved object leaves the finding's severity undecided whenever
/// it could change it: the finding is then reported not evaluated, never
/// given a default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeverityOverride {
    pub selector: Selector,
    pub severity: Severity,
}

/// One band of a rule's `severityBands`: a finding whose relative deviation
/// from its bound lies below `below` (and at or above the previous band's
/// `below`, or zero for the first) takes `severity`.
///
/// Bands are ascending; a deviation at or beyond the last band keeps the
/// rule's severity. The deviation is `|value - bound| / |bound|` for the
/// bound the value misses. A capability measures it as an interval; one that
/// straddles bands takes the most severe band it may reach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeverityBand {
    pub below: f64,
    pub severity: Severity,
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
