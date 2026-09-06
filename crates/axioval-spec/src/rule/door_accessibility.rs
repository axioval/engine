use serde::{Deserialize, Serialize};

use super::assertion::Severity;
use super::spatial::Length;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoorRequirementSpec {
    pub space_classification_from: String,
    pub space_classification_to: String,
    pub minimum_clear_width: Length,
    pub minimum_clear_height: Length,
    pub maximum_threshold_height: Length,
    pub minimum_glazing_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoorAccessibilityPlanSpec {
    pub door_entity_type: String,
    #[serde(default = "default_sliding_door_stop_width")]
    pub sliding_door_stop_width: Length,
    #[serde(default)]
    pub space_classification: String,
    pub rows: Vec<DoorRequirementSpec>,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl DoorAccessibilityPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.door_entity_type.trim().is_empty() {
            return Err("door entity type cannot be blank".into());
        }
        if self.space_classification.trim().is_empty() {
            return Err("space classification cannot be blank".into());
        }
        if self.rows.is_empty() {
            return Err("door requirements cannot be empty".into());
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.space_classification_from.trim().is_empty()
                || row.space_classification_to.trim().is_empty()
            {
                return Err(format!("row {index} space classifications cannot be blank"));
            }
            if !row.minimum_glazing_ratio.is_finite()
                || !(0.0..=1.0).contains(&row.minimum_glazing_ratio)
            {
                return Err(format!("row {index} glazing ratio must be within [0, 1]"));
            }
        }
        Ok(())
    }
}

fn default_sliding_door_stop_width() -> Length {
    Length::millimetres(2.0).expect("the native 2 mm sliding stop default is valid")
}
