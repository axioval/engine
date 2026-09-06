use serde::{Deserialize, Serialize};

use super::{ElementField, ElementScopeSpec};

/// Which components a similarity check compares against each other. Native
/// `similarIn`: `model` compares across the whole model, `bs` restricts the
/// comparison to components on the same building storey.
///
/// Neither value is federation scoping — both stay inside one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimilarityScope {
    WholeModel,
    BuildingStorey,
}

/// The properties a similarity check compares. Native reads every reference
/// family through `PropertyReference.getStringValue`, so each variant must
/// resolve to a display string through an accessor that is actually grounded;
/// the codec refuses references whose accessor would silently read something
/// other than what the provider compares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsistentPropertyRef {
    /// Native TYPE designation (the wildcard `TypeWildcard` reference).
    TypeDesignation,
    /// `IfcRoot.Name`.
    Name,
    /// Grounded native reference family; native reads all via getStringValue.
    Field(ElementField),
}

/// One `compared -> identical` pair: components agreeing on `compared` must
/// also agree on `identical`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsistentPropertySpec {
    pub compared: ConsistentPropertyRef,
    pub identical: ConsistentPropertyRef,
}

/// One row of a native space-type agreed list. Every column is an optional
/// glob: a blank cell is a wildcard, and a row matches only when ALL populated
/// columns match (native `ConstraintUtils.getMatchingRow`, columns 0/1/2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgreedSpaceRowSpec {
    pub space_type: Option<String>,
    pub space_name: Option<String>,
    pub space_number: Option<String>,
}

/// Native `cpSpaceGroupsCheck` candidate population.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpaceGroupCheckMode {
    /// Ordinary spaces only (`NO_SPACE_GROUPS`).
    #[default]
    NoSpaceGroups,
    /// Ordinary spaces plus provider-derived groups (`ALSO_SPACEGROUPS`).
    AlsoSpaceGroups,
    /// Provider-derived groups only (`ONLY_SPACEGROUPS`).
    OnlySpaceGroups,
}

impl AgreedSpaceRowSpec {
    /// A row with every cell blank matches every space, which would make the
    /// whole check a no-op. Native `getMatchingRow` skips such rows outright.
    pub fn is_blank(&self) -> bool {
        [&self.space_type, &self.space_name, &self.space_number]
            .into_iter()
            .all(|cell| cell.as_ref().is_none_or(|v| v.trim().is_empty()))
    }
}

/// Allowed native checked-property patterns for one exact IFC occurrence class.
/// Rows with the same class and property are folded into `allowed_values` while
/// retaining source order; distinct properties on the same class are evaluated
/// independently against each selected occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreedTypeValueSpec {
    pub applies_to: String,
    pub checked_property: ElementField,
    pub allowed_values: Vec<String>,
}

/// One name-pattern row in a storey aggregate-area specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreyAreaLimitSpec {
    pub storey_name_pattern: String,
    pub min_area_m2: f64,
    pub max_area_m2: f64,
}

/// One native storey-count row. Empty selectors are match-any; non-empty
/// selectors use native case-insensitive literal/wildcard matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreySpaceCountSpec {
    pub storey_name_pattern: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub classification_pattern: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub space_type_pattern: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub space_name_pattern: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub space_number_pattern: String,
    pub required_count: usize,
}

/// One supported individual-space requirement. Tolerance is stored as a
/// fraction (`0.10` = plus/minus 10 percent), matching the native table cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpaceCategorizationSpec {
    #[default]
    SpaceType,
    SpaceName,
    SpaceNumber,
    Property {
        property_set: String,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceTypeSizeCountSpec {
    /// Selected classification item pattern (Java wildcards, case-insensitive).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub classification_pattern: String,
    /// Native `SSpace` Type selector (`IfcSpaceType.Name` for ordinary spaces).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub space_type_pattern: String,
    pub space_name_pattern: String,
    /// Native `SSpace` Number selector (`IfcSpace.Name` for ordinary spaces).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub space_number_pattern: String,
    /// Native `ReqCount == -1`: do not compare count for this row.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub count_disabled: bool,
    pub required_count: usize,
    /// Native `TargetArea == -1`: omit total-area comparison.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub area_disabled: bool,
    pub target_area_m2: f64,
    pub tolerance_fraction: f64,
}

/// A configured requirement row that performs no value lookup or comparison.
///
/// The optional type is retained as semantic provenance. It cannot produce a
/// finding and does not narrow the check's selected/checked component count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonEvaluatingRequirementSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<String>,
    /// Neutralized source state. Runtime ignores every row in this collection;
    /// preserving state distinguishes configured include/exclude/ignore rows.
    #[serde(default, skip_serializing_if = "RequirementState::is_include")]
    pub state: RequirementState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    #[default]
    Include,
    Exclude,
    Ignore,
}

impl RequirementState {
    fn is_include(&self) -> bool {
        *self == Self::Include
    }
}

/// One field predicate in a property-quality check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyPredicateSpec {
    /// Optional IFC type restriction inherited from the native requirement row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<String>,
    pub field: ElementField,
    pub operator: PropertyPredicateOp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<PredicateValue>,
}

/// A format-neutral value comparison. Targets stay typed so numeric filters
/// cannot accidentally degrade into locale-sensitive string comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyPredicateOp {
    Equals,
    NotEquals,
    AtMost,
    AtLeast,
    Greater,
    Smaller,
    OneOf,
    NoneOf,
    Matches,
    MatchesCase,
    Contains,
    IsUndefined,
    IsDefined,
    IsEmpty,
    IsNotEmpty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PredicateValue {
    Text(String),
    Boolean(bool),
    Integer(i64),
    Number(f64),
}

/// One property-existence assertion produced from a configured requirement row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiredPropertySpec {
    /// Runtime finding identity; one source rule may contain multiple rows.
    pub id: String,
    /// Candidate IFC type used by the unfiltered fast path and as the filtered
    /// path's fallback for a universal include.
    pub applies_to: String,
    /// `None` selects the unfiltered per-type runtime path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ElementScopeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_set: Option<String>,
    pub property: String,
}
