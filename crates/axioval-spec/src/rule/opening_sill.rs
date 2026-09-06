use serde::{Deserialize, Serialize};

use super::assertion::Severity;
use super::spatial::Length;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowClassificationRequirementSpec {
    pub classification_pattern: String,
    pub maximum_sill_height: Length,
    #[serde(default)]
    pub corridor_end: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpeningSillNativeControlsSpec {
    #[serde(default = "default_true")]
    pub use_filters: bool,
    #[serde(default)]
    pub classification: Option<String>,
    #[serde(default)]
    pub classification_rows: Vec<WindowClassificationRequirementSpec>,
    #[serde(default)]
    pub corridor_end: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for OpeningSillNativeControlsSpec {
    fn default() -> Self {
        Self {
            use_filters: true,
            classification: None,
            classification_rows: Vec::new(),
            corridor_end: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpeningSillSelection {
    pub entity_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpeningSillPlanSpec {
    pub opening: OpeningSillSelection,
    pub adjacent_space: OpeningSillSelection,
    pub maximum: Length,
    #[serde(default)]
    pub native: OpeningSillNativeControlsSpec,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl OpeningSillPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("opening.entity_type", self.opening.entity_type.as_str()),
            (
                "adjacent_space.entity_type",
                self.adjacent_space.entity_type.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(format!("`{name}` must not be blank"));
            }
        }
        if self.maximum.as_millimetres() <= 0.0 && self.native.use_filters {
            return Err("`maximum` sill height must be positive in filter mode".into());
        }
        if !self.native.use_filters {
            if self
                .native
                .classification
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err("classification mode requires a classification".into());
            }
            if self.native.classification_rows.is_empty() {
                return Err("classification mode requires rows".into());
            }
            for (index, row) in self.native.classification_rows.iter().enumerate() {
                if row.classification_pattern.trim().is_empty()
                    || row.maximum_sill_height.as_millimetres() <= 0.0
                {
                    return Err(format!("invalid window classification row {index}"));
                }
            }
        }
        Ok(())
    }
}
