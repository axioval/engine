//! Vendor-neutral ramp accessibility contract.
use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandrailRequirement {
    NotRequired,
    AtLeastOneSide,
    BothSides,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RampSelectionSpec {
    pub spaces: ElementScopeSpec,
    pub ramps: ElementScopeSpec,
    pub stairs: ElementScopeSpec,
    pub accessible_surfaces: ElementScopeSpec,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RampGeometrySpec {
    pub minimum_ramp_width_metres: f64,
    pub minimum_clear_width_metres: f64,
    pub minimum_start_space_metres: f64,
    pub minimum_end_space_metres: f64,
    pub minimum_head_clearance_metres: f64,
    pub minimum_landing_length_metres: f64,
    pub minimum_end_landing_width_metres: Option<f64>,
    pub minimum_end_landing_length_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientRequirementSpec {
    pub slope_ratio: f64,
    pub maximum_length_metres: f64,
    pub maximum_rise_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RampPlanSpec {
    pub selection: RampSelectionSpec,
    pub geometry: RampGeometrySpec,
    pub ramp_requirements: Vec<GradientRequirementSpec>,
    pub gradient_requirements: Vec<GradientRequirementSpec>,
    #[serde(default)]
    pub enter_in_gradients: bool,
    pub additional_stair_required: bool,
    pub maximum_distance_to_stair_metres: f64,
    pub check_indoors: bool,
    pub check_outdoors: bool,
    pub require_landing_between_runs: bool,
    pub landing_width_equal_ramp_width: bool,
    pub forbid_doors_on_landings: bool,
    pub enforce_same_gradient: bool,
    pub handrails: Option<HandrailSpec>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandrailSpec {
    pub requirement: HandrailRequirement,
    pub minimum_height_metres: Option<f64>,
    pub maximum_height_metres: Option<f64>,
    pub minimum_extension_metres: Option<f64>,
    pub maximum_extension_metres: Option<f64>,
    pub continuity_tolerance_metres: Option<f64>,
    pub check_obstruction: bool,
}
impl RampPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, s) in [
            ("spaces", &self.selection.spaces),
            ("ramps", &self.selection.ramps),
            ("stairs", &self.selection.stairs),
            ("accessible surfaces", &self.selection.accessible_surfaces),
        ] {
            if s.candidate_types.is_empty() {
                return Err(format!("ramp {name} scope must not be empty"));
            }
        }
        let g = &self.geometry;
        let values = [
            g.minimum_ramp_width_metres,
            g.minimum_clear_width_metres,
            g.minimum_start_space_metres,
            g.minimum_end_space_metres,
            g.minimum_head_clearance_metres,
            g.minimum_landing_length_metres,
            g.minimum_end_landing_length_metres,
            self.maximum_distance_to_stair_metres,
        ];
        if values.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("ramp dimensions must be finite and positive".into());
        }
        if let Some(v) = g.minimum_end_landing_width_metres {
            if !v.is_finite() || v <= 0.0 {
                return Err("end landing width must be positive when enabled".into());
            }
        }
        for row in self
            .ramp_requirements
            .iter()
            .chain(&self.gradient_requirements)
        {
            if !row.slope_ratio.is_finite()
                || row.slope_ratio <= 0.0
                || !row.maximum_length_metres.is_finite()
                || row.maximum_length_metres < 0.0
                || !row.maximum_rise_metres.is_finite()
                || row.maximum_rise_metres < 0.0
            {
                return Err("gradient slopes must be positive and limits non-negative".into());
            }
        }
        if let Some(h) = &self.handrails {
            for v in [
                h.minimum_height_metres,
                h.maximum_height_metres,
                h.minimum_extension_metres,
                h.maximum_extension_metres,
                h.continuity_tolerance_metres,
            ]
            .into_iter()
            .flatten()
            {
                if !v.is_finite() || v < 0.0 {
                    return Err("handrail dimensions must be finite and non-negative".into());
                }
            }
            if matches!((h.minimum_height_metres,h.maximum_height_metres),(Some(a),Some(b)) if a>b)
            {
                return Err("minimum handrail height exceeds maximum".into());
            }
            if matches!((h.minimum_extension_metres,h.maximum_extension_metres),(Some(a),Some(b)) if a>b)
            {
                return Err("minimum handrail extension exceeds maximum".into());
            }
        }
        Ok(())
    }
}
