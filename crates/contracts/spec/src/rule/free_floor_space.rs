//! Vendor-neutral semantics for accessible free floor space.
use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeFloorSpacePlanSpec {
    pub configuration: FreeFloorSpaceConfigurationSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum FreeFloorSpaceConfigurationSpec {
    ClassificationTable {
        space_classification: String,
        furniture_classification: String,
        requirements: Vec<ClassifiedFloorRequirementSpec>,
    },
    Filters {
        spaces: ElementScopeSpec,
        furniture: ElementScopeSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        free_circle: Option<FreeCircleRequirementSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        free_corridor: Option<FreeCorridorRequirementSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        free_rectangle: Option<FreeRectangleRequirementSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        furniture_side: Option<FurnitureSideRequirementSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        furniture_distance: Option<FurnitureDistanceRequirementSpec>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifiedFloorRequirementSpec {
    pub classification_value: String,
    pub requirements: Vec<FloorSpaceRequirementSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "settings", rename_all = "snake_case")]
pub enum FloorSpaceRequirementSpec {
    FreeCircle(FreeCircleRequirementSpec),
    FreeCorridor(FreeCorridorRequirementSpec),
    FreeRectangle(FreeRectangleRequirementSpec),
    FurnitureSide(FurnitureSideRequirementSpec),
    FurnitureDistance(FurnitureDistanceRequirementSpec),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeCircleRequirementSpec {
    pub diameter_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeCorridorRequirementSpec {
    pub width_metres: f64,
    #[serde(default)]
    pub subtract_door_swings: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeRectangleRequirementSpec {
    pub length_metres: f64,
    pub width_metres: f64,
    #[serde(default = "default_true")]
    pub consider_columns: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FurnitureSideRequirementSpec {
    pub width_metres: f64,
    pub depth_metres: f64,
    #[serde(default)]
    pub free_width: bool,
    #[serde(default)]
    pub double_sided: bool,
    #[serde(default = "default_true")]
    pub front_and_back_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub furniture_classification_value: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FurnitureDistanceRequirementSpec {
    pub minimum_distance_metres: f64,
    pub maximum_distance_metres: f64,
    #[serde(default)]
    pub double_sided: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub furniture_classification_value: Option<String>,
}
fn default_true() -> bool {
    true
}

impl FreeFloorSpacePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        match &self.configuration {
            FreeFloorSpaceConfigurationSpec::ClassificationTable {
                space_classification,
                furniture_classification,
                requirements,
            } => {
                nonempty(space_classification, "space classification")?;
                nonempty(furniture_classification, "furniture classification")?;
                for row in requirements {
                    nonempty(&row.classification_value, "classification value")?;
                    if row.requirements.is_empty() {
                        return Err("classified floor requirement row must not be empty".into());
                    }
                    for requirement in &row.requirements {
                        requirement.validate()?;
                    }
                }
            }
            FreeFloorSpaceConfigurationSpec::Filters {
                spaces,
                furniture,
                free_circle,
                free_corridor,
                free_rectangle,
                furniture_side,
                furniture_distance,
            } => {
                validate_scope(spaces, "space scope")?;
                validate_scope(furniture, "furniture scope")?;
                if [
                    free_circle.is_some(),
                    free_corridor.is_some(),
                    free_rectangle.is_some(),
                    furniture_side.is_some(),
                    furniture_distance.is_some(),
                ]
                .iter()
                .all(|v| !v)
                {
                    return Err(
                        "filter mode must enable at least one floor-space requirement".into(),
                    );
                }
                if let Some(v) = free_circle {
                    v.validate()?;
                }
                if let Some(v) = free_corridor {
                    v.validate()?;
                }
                if let Some(v) = free_rectangle {
                    v.validate()?;
                }
                if let Some(v) = furniture_side {
                    v.validate()?;
                }
                if let Some(v) = furniture_distance {
                    v.validate()?;
                }
            }
        }
        Ok(())
    }
}
impl FloorSpaceRequirementSpec {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::FreeCircle(v) => v.validate(),
            Self::FreeCorridor(v) => v.validate(),
            Self::FreeRectangle(v) => v.validate(),
            Self::FurnitureSide(v) => v.validate(),
            Self::FurnitureDistance(v) => v.validate(),
        }
    }
}
fn validate_scope(scope: &ElementScopeSpec, name: &str) -> Result<(), String> {
    if scope.candidate_types.is_empty() && scope.clauses.is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}
fn positive(v: f64, name: &str) -> Result<(), String> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and positive"))
    }
}
fn nonnegative(v: f64, name: &str) -> Result<(), String> {
    if v.is_finite() && v >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and non-negative"))
    }
}
fn nonempty(v: &str, name: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}
impl FreeCircleRequirementSpec {
    fn validate(&self) -> Result<(), String> {
        positive(self.diameter_metres, "circle diameter")
    }
}
impl FreeCorridorRequirementSpec {
    fn validate(&self) -> Result<(), String> {
        positive(self.width_metres, "corridor width")
    }
}
impl FreeRectangleRequirementSpec {
    fn validate(&self) -> Result<(), String> {
        positive(self.length_metres, "rectangle length")?;
        positive(self.width_metres, "rectangle width")
    }
}
impl FurnitureSideRequirementSpec {
    fn validate(&self) -> Result<(), String> {
        positive(self.width_metres, "furniture-side width")?;
        positive(self.depth_metres, "furniture-side depth")?;
        if let Some(v) = &self.furniture_classification_value {
            nonempty(v, "furniture classification value")?;
        }
        Ok(())
    }
}
impl FurnitureDistanceRequirementSpec {
    fn validate(&self) -> Result<(), String> {
        nonnegative(self.minimum_distance_metres, "minimum furniture distance")?;
        positive(self.maximum_distance_metres, "maximum furniture distance")?;
        if self.minimum_distance_metres > self.maximum_distance_metres {
            return Err("minimum furniture distance must not exceed maximum".into());
        }
        if let Some(v) = &self.furniture_classification_value {
            nonempty(v, "furniture classification value")?;
        }
        Ok(())
    }
}
