//! Vendor-neutral directional clearance and protrusion semantics around components.
use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceSide {
    Front,
    Back,
    Side,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideExtentMode {
    Relative,
    Absolute,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceWidthReference {
    Edge,
    Midline,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElevationReference {
    ComponentTop,
    ComponentFootprint,
    Floor,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentClearancePlanSpec {
    pub targets: ElementScopeSpec,
    pub obstacles: ElementScopeSpec,
    pub spaces: ElementScopeSpec,
    #[serde(default)]
    pub merge_spaces: bool,
    #[serde(default)]
    pub require_containing_space: bool,
    pub side: ClearanceSide,
    #[serde(default)]
    pub check_both_sides: bool,
    pub side_extent_mode: SideExtentMode,
    pub front_offset_metres: f64,
    pub back_offset_metres: f64,
    pub side_clearance_length_metres: f64,
    pub side_clearance_width_metres: f64,
    pub clearance_length_metres: f64,
    pub side_range_metres: f64,
    pub width_reference: ClearanceWidthReference,
    pub top_reference: ElevationReference,
    pub bottom_reference: ElevationReference,
    pub top_offset_metres: f64,
    pub bottom_offset_metres: f64,
    pub minimum_protrusion_metres: f64,
    pub additional_allowed_protrusion_metres: f64,
    #[serde(default)]
    pub check_protrusion: bool,
    pub touching_tolerance_metres: f64,
}
impl ComponentClearancePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        scope(&self.targets, "targets")?;
        scope(&self.obstacles, "obstacles")?;
        if self.require_containing_space {
            scope(&self.spaces, "spaces")?;
        }
        for (n, v) in [
            ("front offset", self.front_offset_metres),
            ("back offset", self.back_offset_metres),
            ("side clearance length", self.side_clearance_length_metres),
            ("side clearance width", self.side_clearance_width_metres),
            ("clearance length", self.clearance_length_metres),
            ("side range", self.side_range_metres),
            ("top offset", self.top_offset_metres),
            ("bottom offset", self.bottom_offset_metres),
            ("minimum protrusion", self.minimum_protrusion_metres),
            (
                "additional protrusion",
                self.additional_allowed_protrusion_metres,
            ),
        ] {
            nonnegative(v, n)?;
        }
        if self.clearance_length_metres <= 0.0 {
            return Err("clearance length must be positive".into());
        }
        if self.side_range_metres <= 0.0 {
            return Err("side range must be positive".into());
        }
        if !self.touching_tolerance_metres.is_finite() || self.touching_tolerance_metres < 0.0001 {
            return Err("touching tolerance must be finite and at least 0.0001 m".into());
        }
        Ok(())
    }
}
fn scope(v: &ElementScopeSpec, n: &str) -> Result<(), String> {
    if v.candidate_types.is_empty() && v.clauses.is_empty() {
        Err(format!("{n} must not be empty"))
    } else {
        Ok(())
    }
}
fn nonnegative(v: f64, n: &str) -> Result<(), String> {
    if v.is_finite() && v >= 0.0 {
        Ok(())
    } else {
        Err(format!("{n} must be finite and non-negative"))
    }
}
