use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildingStoreyPlanSpec {
    pub check_storey_heights: bool,
    pub height_must_be_consistent: bool,
    pub minimum_floor_height_mm: Option<f64>,
    pub maximum_floor_height_mm: Option<f64>,
    pub ignore_lowest_storey: bool,
    pub ignore_highest_storey: bool,
    pub check_storey_cross_area: bool,
    pub net_area_ratio: Option<f64>,
    pub empty_area_ratio: Option<f64>,
    pub include_high_spaces: bool,
    pub check_external_wall_area: bool,
    pub external_wall_gross_area_ratio: Option<f64>,
    pub check_window_area: bool,
    pub window_area_ratio_storey: Option<f64>,
    pub window_area_ratio: Option<f64>,
    pub use_gross_area_compartment: bool,
    pub use_gross_area_group: bool,
}

impl Default for BuildingStoreyPlanSpec {
    fn default() -> Self {
        Self {
            check_storey_heights: true,
            height_must_be_consistent: true,
            minimum_floor_height_mm: Some(2500.0),
            maximum_floor_height_mm: Some(4000.0),
            ignore_lowest_storey: true,
            ignore_highest_storey: false,
            check_storey_cross_area: true,
            net_area_ratio: Some(0.5),
            empty_area_ratio: Some(0.03),
            include_high_spaces: false,
            check_external_wall_area: true,
            external_wall_gross_area_ratio: Some(0.7),
            check_window_area: true,
            window_area_ratio_storey: Some(0.15),
            window_area_ratio: Some(0.5),
            use_gross_area_compartment: true,
            use_gross_area_group: false,
        }
    }
}

impl BuildingStoreyPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.minimum_floor_height_mm,
            self.maximum_floor_height_mm,
            self.net_area_ratio,
            self.empty_area_ratio,
            self.external_wall_gross_area_ratio,
            self.window_area_ratio_storey,
            self.window_area_ratio,
        ];
        if values
            .into_iter()
            .flatten()
            .any(|v| !v.is_finite() || v < 0.0)
        {
            return Err("measurements and ratios must be finite and non-negative".into());
        }
        let ratios = [
            self.net_area_ratio,
            self.empty_area_ratio,
            self.external_wall_gross_area_ratio,
            self.window_area_ratio_storey,
            self.window_area_ratio,
        ];
        if ratios.into_iter().flatten().any(|v| v > 1.0) {
            return Err("area ratios must not exceed one".into());
        }
        if matches!((self.minimum_floor_height_mm, self.maximum_floor_height_mm), (Some(a), Some(b)) if a > b)
        {
            return Err("minimum floor height must not exceed maximum".into());
        }
        Ok(())
    }
}
