use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecessRequirementSpec {
    pub maximum_depth_metres: f64,
    pub minimum_width_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AirwellRequirementSpec {
    pub maximum_height_metres: f64,
    pub minimum_area_square_metres: f64,
    pub minimum_width_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BuildingEnvelopePlanSpec {
    #[serde(default)]
    pub recess_components: ElementScopeSpec,
    #[serde(default)]
    pub check_recesses: bool,
    #[serde(default)]
    pub recess_requirements: Vec<RecessRequirementSpec>,
    #[serde(default)]
    pub airwell_spaces: ElementScopeSpec,
    #[serde(default)]
    pub check_airwells: bool,
    #[serde(default)]
    pub airwell_requirements: Vec<AirwellRequirementSpec>,
}
impl BuildingEnvelopePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.check_recesses && self.recess_requirements.is_empty() {
            return Err("enabled recess checking requires requirement rows".into());
        }
        if self.check_airwells && self.airwell_requirements.is_empty() {
            return Err("enabled airwell checking requires requirement rows".into());
        }
        for r in &self.recess_requirements {
            finite_nonnegative("recess maximum depth", r.maximum_depth_metres)?;
            finite_nonnegative("recess minimum width", r.minimum_width_metres)?;
        }
        for r in &self.airwell_requirements {
            finite_nonnegative("airwell maximum height", r.maximum_height_metres)?;
            finite_nonnegative("airwell minimum area", r.minimum_area_square_metres)?;
            finite_nonnegative("airwell minimum width", r.minimum_width_metres)?;
        }
        Ok(())
    }
}
fn finite_nonnegative(name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{name} must be finite"));
    }
    if value < 0.0 {
        return Err(format!("{name} must be non-negative"));
    }
    Ok(())
}
