use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

/// Vendor-neutral directional clearance envelope in front of selected components.
/// All distances are SI metres; CSET millimetres are an adapter concern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrontClearancePlanSpec {
    pub checked_components: ElementScopeSpec,
    pub allowed_components: ElementScopeSpec,
    pub width_min_metres: f64,
    pub width_max_metres: f64,
    pub height_min_metres: f64,
    pub height_max_metres: f64,
    pub depth_min_metres: f64,
    pub depth_max_metres: f64,
    pub width_tolerance_metres: f64,
    pub height_tolerance_metres: f64,
    pub depth_tolerance_metres: f64,
    #[serde(default)]
    pub check_both_sides: bool,
    #[serde(default)]
    pub allow_floating_to_side: bool,
    pub maximum_floating_distance_metres: f64,
    #[serde(default)]
    pub allow_floating_covering_front: bool,
}

impl FrontClearancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            ("width_min_metres", self.width_min_metres),
            ("width_max_metres", self.width_max_metres),
            ("height_min_metres", self.height_min_metres),
            ("height_max_metres", self.height_max_metres),
            ("depth_min_metres", self.depth_min_metres),
            ("depth_max_metres", self.depth_max_metres),
            ("width_tolerance_metres", self.width_tolerance_metres),
            ("height_tolerance_metres", self.height_tolerance_metres),
            ("depth_tolerance_metres", self.depth_tolerance_metres),
            (
                "maximum_floating_distance_metres",
                self.maximum_floating_distance_metres,
            ),
        ];
        for (name, value) in values {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        if self.checked_components.candidate_types.is_empty()
            && self.checked_components.clauses.is_empty()
        {
            return Err("front-clearance checked-component scope is empty".into());
        }
        Ok(())
    }
}
