use serde::{Deserialize, Serialize};

use super::assertion::Severity;
use super::spatial::Length;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceRouteCost {
    DefaultTopology,
    MetricLength,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceSelectorSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
}

impl SpaceSelectorSpec {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.usage, &self.space_type, &self.name, &self.number]
            .into_iter()
            .flatten()
            .any(|value| value.trim().is_empty())
        {
            return Err("space-distance selector values must be non-empty".into());
        }
        if self.usage.is_none()
            && self.space_type.is_none()
            && self.name.is_none()
            && self.number.is_none()
        {
            return Err("space-distance selector has no criteria".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceDistanceRequirementSpec {
    pub direct_access: bool,
    pub linear_measurement: bool,
    pub same_storey: bool,
    pub minimum: Length,
    pub maximum: Length,
}

impl SpaceDistanceRequirementSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.minimum.validate()?;
        self.maximum.validate()?;
        let minimum = self.minimum.as_millimetres();
        let maximum = self.maximum.as_millimetres();
        if minimum > 0.0 && maximum > 0.0 && minimum > maximum {
            return Err("space-distance minimum exceeds maximum".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceDistanceRowSpec {
    pub start: SpaceSelectorSpec,
    pub destination: SpaceSelectorSpec,
    pub requirement: SpaceDistanceRequirementSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceDistancePlanSpec {
    pub classification: String,
    #[serde(default)]
    pub include_space_groups: bool,
    pub route_cost: SpaceRouteCost,
    pub rows: Vec<SpaceDistanceRowSpec>,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl SpaceDistancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.classification.trim().is_empty() {
            return Err("space-distance classification must be non-empty".into());
        }
        if self.rows.is_empty() {
            return Err("space-distance plan has no requirement rows".into());
        }
        for (index, row) in self.rows.iter().enumerate() {
            row.start
                .validate()
                .map_err(|message| format!("space-distance row {index} start: {message}"))?;
            row.destination
                .validate()
                .map_err(|message| format!("space-distance row {index} destination: {message}"))?;
            row.requirement
                .validate()
                .map_err(|message| format!("space-distance row {index}: {message}"))?;
        }
        Ok(())
    }
}
