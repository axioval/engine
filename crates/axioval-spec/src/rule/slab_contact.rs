use serde::{Deserialize, Serialize};

use super::execution::ElementScopeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SlabContactLocation {
    #[default]
    Above,
    Below,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlabContactPlanSpec {
    pub components: ElementScopeSpec,
    pub surfaces: ElementScopeSpec,
    pub location: SlabContactLocation,
    pub minimum_contact_ratio: f64,
    pub minimum_contact_polygon_area_m2: f64,
    pub maximum_gap_m: f64,
    pub maximum_intersection_m: f64,
    pub skip_top_storey: bool,
    pub skip_bottom_storey: bool,
}

impl Default for SlabContactPlanSpec {
    fn default() -> Self {
        Self {
            components: ElementScopeSpec {
                candidate_types: vec!["IFCWALL".into()],
                ..Default::default()
            },
            surfaces: ElementScopeSpec {
                candidate_types: vec!["IFCROOT".into()],
                ..Default::default()
            },
            location: SlabContactLocation::Above,
            minimum_contact_ratio: 0.75,
            minimum_contact_polygon_area_m2: 0.0025,
            maximum_gap_m: 0.01,
            maximum_intersection_m: 0.01,
            skip_top_storey: false,
            skip_bottom_storey: false,
        }
    }
}

impl SlabContactPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if !self.minimum_contact_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_contact_ratio)
        {
            return Err("minimum_contact_ratio must be finite and within 0..=1".into());
        }
        for (name, value) in [
            (
                "minimum_contact_polygon_area_m2",
                self.minimum_contact_polygon_area_m2,
            ),
            ("maximum_gap_m", self.maximum_gap_m),
            ("maximum_intersection_m", self.maximum_intersection_m),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        if self.components.candidate_types.is_empty() {
            return Err("components scope must select at least one type".into());
        }
        if self.surfaces.candidate_types.is_empty() {
            return Err("surfaces scope must select at least one type".into());
        }
        Ok(())
    }
}
