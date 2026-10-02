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
    /// Nested categories headed before each finding's message, outermost
    /// first; see [`CategoryLevel`]. Empty adds none and is omitted when
    /// serialized.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<CategoryLevel>,
    /// Runs the rule only as another rule's outcome allows; see
    /// [`RuleGate`]. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<RuleGate>,
    /// Runs the rule only for the rules that read its outcome through a
    /// gate or a `ruleOutcome` selector: it decides which objects they
    /// check, and its own findings, tables, not-evaluated outcomes and
    /// summary are not reported. What it leaves undecided stays undecided
    /// in the rules that read it. Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auxiliary: bool,
}

/// A rule's gate on another rule of the same ruleset: the rule runs, or
/// selects, only as that rule's outcome allows.
///
/// A gate on a folder applies to every rule in it and its subfolders,
/// together with each rule's own gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleGate {
    /// The id of the rule whose outcome gates.
    pub rule: String,
    pub condition: GateCondition,
}

/// When a gated rule runs, and on what.
///
/// The whole-rule conditions read the other rule's status: `passed` with
/// no finding and nothing left not evaluated, `failed` with any finding.
/// A gate that is not open skips the rule; a status that cannot be decided
/// (no finding, but something not evaluated) leaves the gated rule not
/// evaluated. The object conditions narrow the rule's applicability to the
/// objects the other rule passed or failed, as a `ruleOutcome` selector
/// does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GateCondition {
    /// Every selected object, if the other rule passed.
    AllIfPassed,
    /// Every selected object, if the other rule failed.
    AllIfFailed,
    /// Only the objects the other rule passed.
    PassedObjects,
    /// Only the objects the other rule failed.
    FailedObjects,
}

/// One level of a rule's `categories`: the value of `property` (in
/// `propertySet`, a declared concept or a reserved set) on the finding's
/// subject, or on every object `path` reaches from it (steps as a `related`
/// selector's path).
///
/// Each level heads the message in brackets, `[F90] [Office] ...`; several
/// reached values join in one heading, and no value is `[-]`, so every
/// level keeps its place. A value that cannot be read leaves the finding
/// not evaluated rather than filed under the wrong heading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CategoryLevel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_set: Option<String>,
    pub property: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
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
    /// A gate every rule in the folder and its subfolders takes; see
    /// [`RuleGate`]. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<RuleGate>,
    /// Provenance an importer or authoring tool keeps with the folder, by
    /// namespaced key (`scheme:name`, such as `ids:specification`), as
    /// text. The engine never reads it: an annotation changes no
    /// selection, evidence or outcome. Empty is omitted when serialized.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
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
    /// Classifications the ruleset derives, by id; see
    /// [`ClassificationDefinition`]. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub classifications: BTreeMap<String, ClassificationDefinition>,
    /// Groups the ruleset derives from its members, by id; see
    /// [`GroupingDefinition`]. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groupings: BTreeMap<String, GroupingDefinition>,
}

/// A named set of groups the ruleset derives from its members, such as the
/// flats implied by a flat number on every room.
///
/// Each group is a derived object of kind [`crate::DERIVED_GROUP_KIND`]
/// with the members that share its key; a `derivedGroup` selector selects
/// them, the relationship `axioval:derived.group;by=<id>` runs from each
/// member to its group (`backward` from a group to its members), and the
/// reserved set `axioval:group` states each group's `key` and `members`
/// count. A member without a key is ungrouped; one whose key or membership
/// cannot be read leaves its groups undecided, never ungrouped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupingDefinition {
    pub id: String,
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<LocalizedText>,
    /// The objects grouped, such as the spaces.
    pub members: Selector,
    /// What members are grouped by.
    pub by: GroupingKey,
}

/// What a [`GroupingDefinition`] groups its members by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GroupingKey {
    /// Equal values of one property, read as a property selector reads it:
    /// text as stated, an integer, a boolean or a number. Members of one
    /// source with one value form one group. A derived class is read in the
    /// set `axioval:classification`.
    Property {
        #[serde(
            rename = "propertySet",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        property_set: Option<String>,
        property: String,
    },
    /// Equal codes in one classification system, as the source states
    /// them: the code the member's assignment in `system` carries.
    Classification { system: String },
}

/// A named classification of objects the ruleset derives: ordered rows,
/// each a selector and the class name it assigns.
///
/// Every selector and property reference reads it as the property `id` in
/// the reserved set `axioval:classification`: a first-match
/// classification's value is the class of the first row that matches, once
/// every row before it surely does not; an all-match classification's is
/// the list of every matching row's class, distinct, in row order, once
/// every row is decided. An object no row matches has no value (an exact
/// absence). An object whose deciding rows cannot be decided cannot be
/// read, never unclassified.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassificationDefinition {
    pub id: String,
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<LocalizedText>,
    /// First match (the default, omitted when serialized) or all match.
    #[serde(default, skip_serializing_if = "ClassificationMode::is_first_match")]
    pub mode: ClassificationMode,
    pub rows: Vec<ClassificationRow>,
    /// The classes of a hierarchical classification, a tree by their
    /// `parent`s; see [`ClassDefinition`]. With classes declared, every
    /// row's `class` is a declared class id, a leaf or an inner class.
    /// Empty keeps the classification flat, its classes the names its rows
    /// assign, and is omitted when serialized.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<ClassDefinition>,
}

/// One declared class of a hierarchical [`ClassificationDefinition`].
///
/// A class without a `parent` is a root, at level 1; every other class is
/// one level below its parent. Ids and codes are unique within the
/// classification, every parent is declared, and parents form no cycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassDefinition {
    pub id: String,
    /// The class's code in its classification system, such as `331`.
    /// Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub name: LocalizedText,
    /// The id of the class this one refines. Omitted for a root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// One row of a [`ClassificationDefinition`]: objects `selector` selects
/// take `class`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassificationRow {
    pub selector: Selector,
    pub class: String,
}

/// How a classification's rows assign classes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClassificationMode {
    /// The first matching row's class.
    #[default]
    FirstMatch,
    /// Every matching row's class.
    AllMatch,
}
impl ClassificationMode {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn is_first_match(&self) -> bool {
        matches!(self, Self::FirstMatch)
    }
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
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
fn severity() -> Severity {
    Severity::Error
}
