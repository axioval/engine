use serde::{Deserialize, Serialize};

use super::assertion::Severity;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedSpaceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FireCompartmentMembershipPlanSpec {
    pub storey_entity_type: String,
    pub space_entity_type: String,
    pub fire_compartment_entity_type: String,
    pub classification: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_spaces: Vec<SkippedSpaceSpec>,
    pub minimum_intersection_ratio: f64,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl FireCompartmentMembershipPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("storey entity type", &self.storey_entity_type),
            ("space entity type", &self.space_entity_type),
            (
                "fire compartment entity type",
                &self.fire_compartment_entity_type,
            ),
            ("classification", &self.classification),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{name} must not be blank"));
            }
        }
        if !self.minimum_intersection_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_intersection_ratio)
        {
            return Err("intersection ratio must be finite and between 0 and 1".into());
        }
        if self.skipped_spaces.iter().any(|row| {
            [&row.classification, &row.space_type, &row.name, &row.number]
                .into_iter()
                .all(|value| value.as_deref().is_none_or(str::is_empty))
        }) {
            return Err("skipped-space rows must contain at least one selector".into());
        }
        Ok(())
    }
}
