use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentVisibilityMode {
    Visible,
    NotVisible,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentVisibilityPlanSpec {
    pub sources: ElementScopeSpec,
    pub blockers: ElementScopeSpec,
    pub targets: ElementScopeSpec,
    pub mode: ComponentVisibilityMode,
    pub radius_metres: f64,
    pub eye_height_metres: f64,
    pub minimum_required_visible_targets: usize,
    pub transparency_threshold: f64,
}

impl ComponentVisibilityPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.radius_metres.is_finite() || self.radius_metres < 0.0 {
            return Err("visibility radius must be finite and non-negative".into());
        }
        if !self.eye_height_metres.is_finite() {
            return Err("visibility eye height must be finite".into());
        }
        if self.minimum_required_visible_targets == 0 {
            return Err("minimum required visible targets must be positive".into());
        }
        if !self.transparency_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.transparency_threshold)
        {
            return Err("transparency threshold must be within [0, 1]".into());
        }
        Ok(())
    }
}
