use serde::{Deserialize, Serialize};

use super::assertion::Severity;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfCapacityRequirementSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub minimum_running_metres: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfCapacityPlanSpec {
    pub classification: String,
    pub requirements: Vec<ShelfCapacityRequirementSpec>,
    pub door_clearance: f64,
    pub horizontal_spacing: f64,
    pub vertical_spacing: f64,
    pub shelf_depth: f64,
    pub bottom_elevation: f64,
    pub top_elevation: f64,
    pub severity: Severity,
}

impl ShelfCapacityPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.classification.trim().is_empty() {
            return Err("shelf classification must be nonblank".into());
        }
        if self.requirements.is_empty() {
            return Err("shelf requirements must not be empty".into());
        }
        positive("vertical spacing", self.vertical_spacing)?;
        positive("shelf depth", self.shelf_depth)?;
        nonnegative("door clearance", self.door_clearance)?;
        nonnegative("horizontal spacing", self.horizontal_spacing)?;
        nonnegative("bottom elevation", self.bottom_elevation)?;
        nonnegative("top elevation", self.top_elevation)?;
        for (index, row) in self.requirements.iter().enumerate() {
            if [&row.usage, &row.space_type, &row.name, &row.number]
                .into_iter()
                .flatten()
                .all(|value| value.trim().is_empty())
            {
                return Err(format!("shelf requirement row {index} has no selector"));
            }
            positive("minimum running metres", row.minimum_running_metres)?;
        }
        Ok(())
    }
}

fn positive(name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be finite and positive"));
    }
    Ok(())
}

fn nonnegative(name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} must be finite and non-negative"));
    }
    Ok(())
}
