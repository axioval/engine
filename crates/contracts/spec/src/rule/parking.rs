//! Vendor-neutral parking-space geometry contract.
use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObstructionCount {
    None,
    One,
    Both,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParkingOrientation {
    Unclear,
    Parallel,
    Perpendicular,
    Angled,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionBoundsSpec {
    pub minimum_metres: Option<f64>,
    pub maximum_metres: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParkingPlanSpec {
    pub parking_spaces: ElementScopeSpec,
    pub specify_aisle: bool,
    pub aisles: ElementScopeSpec,
    pub obstructions: ElementScopeSpec,
    pub width: DimensionBoundsSpec,
    pub length: DimensionBoundsSpec,
    pub height: DimensionBoundsSpec,
    pub allowed_end_obstructions: BTreeSet<ObstructionCount>,
    pub allowed_side_obstructions: BTreeSet<ObstructionCount>,
    pub allowed_orientations: BTreeSet<ParkingOrientation>,
    pub obstruction_free_zone_metres: Option<f64>,
}
impl DimensionBoundsSpec {
    fn validate(&self, name: &str) -> Result<(), String> {
        for value in [self.minimum_metres, self.maximum_metres]
            .into_iter()
            .flatten()
        {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("parking {name} bounds must be finite and positive"));
            }
        }
        if let (Some(min), Some(max)) = (self.minimum_metres, self.maximum_metres) {
            if min > max {
                return Err(format!("parking {name} minimum exceeds maximum"));
            }
        }
        Ok(())
    }
}
impl ParkingPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, scope) in [
            ("spaces", &self.parking_spaces),
            ("aisles", &self.aisles),
            ("obstructions", &self.obstructions),
        ] {
            if scope.candidate_types.is_empty() && scope.clauses.is_empty() {
                return Err(format!("parking {name} scope is empty"));
            }
        }
        self.width.validate("width")?;
        self.length.validate("length")?;
        self.height.validate("height")?;
        if self.allowed_end_obstructions.is_empty()
            || self.allowed_side_obstructions.is_empty()
            || self.allowed_orientations.is_empty()
        {
            return Err("parking allowed-state sets must not be empty".into());
        }
        if let Some(v) = self.obstruction_free_zone_metres {
            if !v.is_finite() || v <= 0.0 {
                return Err("parking obstruction-free zone must be finite and positive".into());
            }
        }
        Ok(())
    }
}
