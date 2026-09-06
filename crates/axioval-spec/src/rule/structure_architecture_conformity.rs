use crate::rule::coverage::CoverageDisplaySpec;
use crate::rule::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructureArchitectureConformityPlanSpec {
    pub structure: ElementScopeSpec,
    pub architecture: ElementScopeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal_tolerance_metres: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_tolerance_metres: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_display: Option<CoverageDisplaySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_display: Option<CoverageDisplaySpec>,
}
impl StructureArchitectureConformityPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.structure.candidate_types.is_empty() || self.architecture.candidate_types.is_empty()
        {
            return Err("structure and architecture scopes must not be empty".into());
        }
        if self.horizontal_tolerance_metres.is_none() && self.vertical_tolerance_metres.is_none() {
            return Err("at least one conformity axis must be enabled".into());
        }
        for (name, value) in [
            ("horizontal", self.horizontal_tolerance_metres),
            ("vertical", self.vertical_tolerance_metres),
        ] {
            if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
                return Err(format!("{name} tolerance must be finite and non-negative"));
            }
        }
        for display in [&self.structure_display, &self.architecture_display]
            .into_iter()
            .flatten()
        {
            if display
                .rgb
                .iter()
                .chain([&display.transparency, &display.emission])
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err("display values must be finite and within 0..=1".into());
            }
        }
        Ok(())
    }
}
