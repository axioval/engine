use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallExtrusionDirection {
    NoLimitations,
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallDimensionRequirement {
    pub start: String,
    pub end: String,
    pub direction: String,
    pub distance_metres: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowedWallGeometry {
    pub geometry_type: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallValidationPlanSpec {
    pub walls: ElementScopeSpec,
    #[serde(default)]
    pub dimension_requirements: Vec<WallDimensionRequirement>,
    pub minimum_opening_area_square_metres: f64,
    #[serde(default)]
    pub check_wall_area_consistency: bool,
    pub wall_area_tolerance_square_metres: f64,
    #[serde(default)]
    pub accept_empty_walls: bool,
    #[serde(default)]
    pub allowed_geometry_types: Vec<AllowedWallGeometry>,
    pub extrusion_direction: WallExtrusionDirection,
}

impl WallValidationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        let finite_non_negative = |name: &str, value: f64| {
            if value.is_finite() && value >= 0.0 {
                Ok(())
            } else {
                Err(format!("{name} must be finite and non-negative"))
            }
        };
        finite_non_negative(
            "minimum opening area",
            self.minimum_opening_area_square_metres,
        )?;
        finite_non_negative(
            "wall area tolerance",
            self.wall_area_tolerance_square_metres,
        )?;
        for (index, row) in self.dimension_requirements.iter().enumerate() {
            if row.start.is_empty() || row.end.is_empty() || row.direction.is_empty() {
                return Err(format!("dimension requirement {index} is incomplete"));
            }
            finite_non_negative("dimension distance", row.distance_metres)?;
        }
        if self
            .allowed_geometry_types
            .iter()
            .any(|row| row.geometry_type.is_empty())
        {
            return Err("allowed geometry type must not be empty".into());
        }
        Ok(())
    }
}
