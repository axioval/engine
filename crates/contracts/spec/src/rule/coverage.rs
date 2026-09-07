use crate::rule::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageDisplaySpec {
    pub rgb: [f32; 3],
    #[serde(default)]
    pub transparency: f32,
    #[serde(default)]
    pub emission: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageComparisonPlanSpec {
    pub architecture: ElementScopeSpec,
    pub structure: ElementScopeSpec,
    pub tolerance_metres: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_display: Option<CoverageDisplaySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_display: Option<CoverageDisplaySpec>,
}

impl CoverageComparisonPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.tolerance_metres.is_finite() || self.tolerance_metres < 0.0 {
            return Err("coverage tolerance must be finite and non-negative".into());
        }
        for display in [&self.architecture_display, &self.structure_display]
            .into_iter()
            .flatten()
        {
            if display
                .rgb
                .iter()
                .chain([&display.transparency, &display.emission])
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err("coverage display values must be finite and within 0..=1".into());
            }
        }
        Ok(())
    }
}
