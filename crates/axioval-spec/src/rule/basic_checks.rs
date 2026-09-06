//! Format-neutral plans for small reusable model checks.
//!
//! These plans describe executable capabilities. They deliberately contain no
//! the provider class names or source-format vocabulary; codecs own that mapping.

use serde::{Deserialize, Serialize};

use super::assertion::Severity;

pub const CONDITIONAL_PRESENCE_ID: &str = "model.conditional_presence";
pub const ELEMENT_VALIDATION_ID: &str = "geometry.element_validation";
pub const ELEMENT_DIMENSION_ID: &str = "geometry.element_dimension";
pub const TYPE_GROUP_SIZE_OUTLIER_ID: &str = "geometry.type_group_size_consistency";
pub const WINDOW_FLOOR_RATIO_ID: &str = "geometry.window_floor_ratio";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConditionalPresencePlanSpec {
    pub trigger_entity_type: String,
    pub required_entity_type: String,
    pub severity: Severity,
}

impl ConditionalPresencePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        nonempty(&self.trigger_entity_type, "trigger_entity_type")?;
        nonempty(&self.required_entity_type, "required_entity_type")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ElementValidationPlanSpec {
    pub entity_type: String,
    pub minimum_extent_mm: f64,
    pub severity: Severity,
}

impl ElementValidationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        nonempty(&self.entity_type, "entity_type")?;
        finite_nonnegative(self.minimum_extent_mm, "minimum_extent_mm")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ElementDimensionPlanSpec {
    pub entity_type: String,
    pub attribute_slot: Option<usize>,
    pub property_set: Option<String>,
    pub property: Option<String>,
    pub minimum_metres: Option<f64>,
    pub maximum_metres: Option<f64>,
    pub dimension_label: String,
    pub severity: Severity,
}

impl ElementDimensionPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        nonempty(&self.entity_type, "entity_type")?;
        nonempty(&self.dimension_label, "dimension_label")?;
        if self.attribute_slot.is_none() && self.property.is_none() {
            return Err("element dimension requires an attribute slot or property".into());
        }
        if self.minimum_metres.is_none() && self.maximum_metres.is_none() {
            return Err("element dimension requires at least one bound".into());
        }
        if let Some(value) = self.minimum_metres {
            finite_nonnegative(value, "minimum_metres")?;
        }
        if let Some(value) = self.maximum_metres {
            finite_nonnegative(value, "maximum_metres")?;
        }
        if let (Some(minimum), Some(maximum)) = (self.minimum_metres, self.maximum_metres) {
            if minimum > maximum {
                return Err("element dimension minimum exceeds maximum".into());
            }
        }
        if self.property.is_none() && self.property_set.is_some() {
            return Err("property_set cannot be supplied without property".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeGroupSizeOutlierPlanSpec {
    pub entity_type: String,
    pub relative_tolerance: f64,
    pub severity: Severity,
}

impl TypeGroupSizeOutlierPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        nonempty(&self.entity_type, "entity_type")?;
        finite_nonnegative(self.relative_tolerance, "relative_tolerance")
    }
}

fn nonempty(value: &str, name: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowFloorRatioPlanSpec {
    pub minimum_ratio: f64,
    #[serde(default = "default_warning")]
    pub severity: Severity,
}

impl WindowFloorRatioPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        finite_nonnegative(self.minimum_ratio, "window-floor minimum ratio")
    }
}

fn default_warning() -> Severity {
    Severity::Warning
}

fn finite_nonnegative(value: f64, name: &str) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and non-negative"))
    }
}
