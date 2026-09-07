use serde::{Deserialize, Serialize};

use crate::rule::assertion::Severity;
use crate::rule::model::ModelSelectionSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HorizontalGuardPlanSpec {
    pub selection: ModelSelectionSpec,
    pub minimum_barrier_height: f64,
    pub maximum_barrier_gap: f64,
    pub maximum_platform_gap: f64,
    pub maximum_landing_gap: f64,
    pub maximum_fall_height: f64,
    pub minimum_landing_width: f64,
    pub climbable_object_barrier_distance: f64,
    pub maximum_climbable_height: f64,
    pub minimum_climbable_side_length: f64,
    pub measure_barrier_from_curb: bool,
    pub severity: Severity,
    pub unavailable_severity: Severity,
}

impl HorizontalGuardPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            ("minimum_barrier_height", self.minimum_barrier_height),
            ("maximum_barrier_gap", self.maximum_barrier_gap),
            ("maximum_platform_gap", self.maximum_platform_gap),
            ("maximum_landing_gap", self.maximum_landing_gap),
            ("maximum_fall_height", self.maximum_fall_height),
            ("minimum_landing_width", self.minimum_landing_width),
            (
                "climbable_object_barrier_distance",
                self.climbable_object_barrier_distance,
            ),
            ("maximum_climbable_height", self.maximum_climbable_height),
            (
                "minimum_climbable_side_length",
                self.minimum_climbable_side_length,
            ),
        ];
        for (name, value) in values {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        if self.minimum_barrier_height == 0.0 || self.minimum_landing_width == 0.0 {
            return Err("minimum barrier height and landing width must be positive".into());
        }
        Ok(())
    }
}
