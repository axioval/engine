//! Vendor-neutral effective-coverage rule contract.
use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageMode {
    Unoccluded,
    UnoccludedWithinArea,
    DistanceOfTravelWithinArea,
    OccludedWithinArea,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyValueRefSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_set: Option<String>,
    pub property: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveCoveragePlanSpec {
    pub checked_components: ElementScopeSpec,
    pub effect_sources: ElementScopeSpec,
    pub effect_range_metres: f64,
    pub propagate_to_connected_spaces: bool,
    pub minimum_geometric_coverage: f64,
    pub mode: CoverageMode,
    pub required_property_ratio: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_value: Option<PropertyValueRefSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_multiplier: Option<PropertyValueRefSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_value: Option<PropertyValueRefSpec>,
}

impl EffectiveCoveragePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, scope) in [
            ("checked component", &self.checked_components),
            ("effect source", &self.effect_sources),
        ] {
            if scope.candidate_types.is_empty() && scope.clauses.is_empty() {
                return Err(format!("effective coverage {name} scope is empty"));
            }
        }
        if !self.effect_range_metres.is_finite() || self.effect_range_metres < 0.0 {
            return Err("effective coverage range must be finite and non-negative".into());
        }
        for (name, value) in [
            (
                "minimum geometric coverage",
                self.minimum_geometric_coverage,
            ),
            ("required property ratio", self.required_property_ratio),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("effective coverage {name} must be in [0, 1]"));
            }
        }
        for property in [
            &self.source_value,
            &self.source_multiplier,
            &self.area_value,
        ]
        .into_iter()
        .flatten()
        {
            if property.property.trim().is_empty() {
                return Err("effective coverage property name must not be blank".into());
            }
        }
        Ok(())
    }
}
