use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParametricHostProfile {
    Rectangle,
    L,
    NonUniformL,
    T,
    NonUniformT,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementHolePlanSpec {
    pub profile: ParametricHostProfile,
    pub minimum_distance_to_end_metres: f64,
    pub minimum_distance_to_profile_edge_metres: f64,
}

impl ElementHolePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            (
                "minimum_distance_to_end_metres",
                self.minimum_distance_to_end_metres,
            ),
            (
                "minimum_distance_to_profile_edge_metres",
                self.minimum_distance_to_profile_edge_metres,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        Ok(())
    }
}
