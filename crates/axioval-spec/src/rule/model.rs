//! Vendor-neutral selection of members in a federated model set.

use serde::{Deserialize, Serialize};

use crate::rule::assertion::Severity;
use crate::rule::execution::{ElementScopeSpec, ModelDomain};
use crate::rule::relation::RelationResultPolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ModelSelectionSpec {
    Primary,
    All,
    MemberIds {
        ids: Vec<String>,
    },
    Domains {
        domains: Vec<ModelDomain>,
        include_unclassified: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelGrouping {
    Each,
    Combined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowQuantifier {
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiredComponentRowSpec {
    pub selection: ElementScopeSpec,
    pub fallback_type: String,
    pub require_construction_type: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequiredComponentsPlanSpec {
    pub models: ModelSelectionSpec,
    pub model_grouping: ModelGrouping,
    pub row_quantifier: RowQuantifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_scheme: Option<String>,
    pub check_unclassified: bool,
    pub rows: Vec<RequiredComponentRowSpec>,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

/// Derived, vendor-neutral space-group kinds currently exposed by model overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedSpaceGroupTypeSpec {
    GrossArea,
    GrossAreaVerticalProjection,
    FireCompartment,
}

/// Native space identity matching reduced to the classification-free envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedSpaceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_pattern: Option<String>,
    pub type_pattern: String,
    pub name_pattern: String,
    pub number_pattern: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpacesInDerivedGroupsPlanSpec {
    pub models: ModelSelectionSpec,
    pub accepted_group_types: Vec<DerivedSpaceGroupTypeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_classification_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_spaces: Vec<SkippedSpaceSpec>,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceIdentityPatternSpec {
    pub type_pattern: String,
    pub name_pattern: String,
    pub number_pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceQuotaSpec {
    pub space: SpaceIdentityPatternSpec,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceGroupContainmentRowSpec {
    pub group: SpaceIdentityPatternSpec,
    pub requirements: Vec<SpaceQuotaSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceGroupContainmentPlanSpec {
    pub models: ModelSelectionSpec,
    pub rows: Vec<SpaceGroupContainmentRowSpec>,
    pub unavailable_severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FireCompartmentAreaRowSpec {
    pub building_fire_rating: String,
    pub use_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_sprinklered_limit_m2: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprinklered_limit_m2: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FireCompartmentAreaPlanSpec {
    pub models: ModelSelectionSpec,
    pub rows: Vec<FireCompartmentAreaRowSpec>,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

/// Per-building numeric storey-name sequence in ascending elevation order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreyNameSequencePlanSpec {
    pub first_number: i32,
    pub increment: i32,
    pub result: RelationResultPolicy,
}

impl StoreyNameSequencePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.increment <= 0 {
            return Err("storey-name increment must be positive".into());
        }
        self.first_number
            .checked_add(self.increment)
            .ok_or_else(|| "storey-name sequence overflows i32".to_string())?;
        Ok(())
    }
}

impl ModelSelectionSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::MemberIds { ids } => {
                if ids.is_empty() || ids.iter().any(|id| id.trim().is_empty()) {
                    return Err("model member-id selection is empty or contains a blank id".into());
                }
                let unique: std::collections::BTreeSet<_> = ids.iter().collect();
                if unique.len() != ids.len() {
                    return Err("model member-id selection contains duplicates".into());
                }
            }
            Self::Domains { domains, .. } => {
                if domains.is_empty() {
                    return Err("model domain selection is empty".into());
                }
                if domains.contains(&ModelDomain::Any) && domains.len() != 1 {
                    return Err("model domain `any` cannot be combined with named domains".into());
                }
                let unique: std::collections::BTreeSet<_> =
                    domains.iter().map(|d| d.id()).collect();
                if unique.len() != domains.len() {
                    return Err("model domain selection contains duplicates".into());
                }
            }
            Self::Primary | Self::All => {}
        }
        Ok(())
    }
}

impl RequiredComponentsPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.models.validate()?;
        if self.rows.is_empty() {
            return Err("required-components plan has no rows".into());
        }
        if self
            .classification_scheme
            .as_deref()
            .is_some_and(|scheme| scheme.trim().is_empty())
        {
            return Err("required-components classification scheme is blank".into());
        }
        let requires_scheme = self.check_unclassified
            || self
                .rows
                .iter()
                .any(|row| row.classification_pattern.is_some());
        if requires_scheme && self.classification_scheme.is_none() {
            return Err("required-components plan needs a classification scheme".into());
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.fallback_type.trim().is_empty() {
                return Err(format!(
                    "required-components row {index} has a blank fallback type"
                ));
            }
            if row
                .classification_pattern
                .as_deref()
                .is_some_and(|pattern| pattern.trim().is_empty())
            {
                return Err(format!(
                    "required-components row {index} has a blank classification pattern"
                ));
            }
        }
        Ok(())
    }
}

impl SpacesInDerivedGroupsPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.models.validate()?;
        if self.accepted_group_types.is_empty() {
            return Err("at least one derived group type is required".into());
        }
        let scheme_present = self
            .skip_classification_scheme
            .as_deref()
            .is_some_and(|scheme| !scheme.trim().is_empty());
        for (index, row) in self.skipped_spaces.iter().enumerate() {
            let identity_blank = row.type_pattern.trim().is_empty()
                && row.name_pattern.trim().is_empty()
                && row.number_pattern.trim().is_empty();
            let classification_present = row
                .classification_pattern
                .as_deref()
                .is_some_and(|pattern| !pattern.trim().is_empty());
            if identity_blank && !classification_present {
                return Err(format!("skipped-space row {index} is empty"));
            }
            if classification_present && !scheme_present {
                return Err(format!(
                    "skipped-space row {index} names a classification pattern without a configured scheme"
                ));
            }
            for pattern in [&row.type_pattern, &row.name_pattern, &row.number_pattern] {
                if pattern.trim_start().starts_with("rx:") {
                    return Err(format!(
                        "skipped-space row {index} uses an unsupported Java regex `{pattern}`"
                    ));
                }
            }
            if row
                .classification_pattern
                .as_deref()
                .is_some_and(|pattern| pattern.trim_start().starts_with("rx:"))
            {
                return Err(format!(
                    "skipped-space row {index} uses an unsupported Java regex `{}`",
                    row.classification_pattern.as_deref().unwrap_or("")
                ));
            }
        }
        Ok(())
    }
}

impl SpaceGroupContainmentPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.models.validate()?;
        if self.rows.is_empty() {
            return Err("space-group containment plan has no rows".into());
        }
        for (row_index, row) in self.rows.iter().enumerate() {
            validate_identity_pattern(&row.group, "group", row_index, false)?;
            if row.requirements.is_empty() {
                return Err(format!(
                    "space-group containment row {row_index} has no requirements"
                ));
            }
            for (quota_index, quota) in row.requirements.iter().enumerate() {
                validate_identity_pattern(&quota.space, "space", quota_index, true)?;
                if quota.count == 0 {
                    return Err(format!(
                        "space-group containment row {row_index} requirement {quota_index} has zero count"
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_identity_pattern(
    pattern: &SpaceIdentityPatternSpec,
    subject: &str,
    index: usize,
    allow_all_blank: bool,
) -> Result<(), String> {
    let values = [
        &pattern.type_pattern,
        &pattern.name_pattern,
        &pattern.number_pattern,
    ];
    if !allow_all_blank && values.iter().all(|value| value.trim().is_empty()) {
        return Err(format!("{subject} pattern {index} is empty"));
    }
    if let Some(value) = values
        .into_iter()
        .find(|value| value.trim_start().starts_with("rx:"))
    {
        return Err(format!(
            "{subject} pattern {index} uses unsupported Java regex `{value}`"
        ));
    }
    Ok(())
}

impl FireCompartmentAreaPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.models.validate()?;
        if self.rows.is_empty() {
            return Err("fire-compartment area plan has no rows".into());
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.building_fire_rating.trim().is_empty() {
                return Err(format!(
                    "fire-compartment area row {index} has a blank building fire rating"
                ));
            }
            if row.use_class.trim().is_empty() {
                return Err(format!(
                    "fire-compartment area row {index} has a blank use class"
                ));
            }
            if row.not_sprinklered_limit_m2.is_none() && row.sprinklered_limit_m2.is_none() {
                return Err(format!(
                    "fire-compartment area row {index} has no numeric limit"
                ));
            }
            for (name, value) in [
                ("not-sprinklered", row.not_sprinklered_limit_m2),
                ("sprinklered", row.sprinklered_limit_m2),
            ] {
                if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
                    return Err(format!(
                        "fire-compartment area row {index} has an invalid {name} limit"
                    ));
                }
            }
        }
        Ok(())
    }
}
