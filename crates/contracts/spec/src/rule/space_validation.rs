use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpaceValidationCategorizationSpec {
    #[default]
    BySpace,
    ByProblem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpaceValidationPlanSpec {
    /// Native `tolerance`, converted from millimetres at the CSET boundary.
    pub tolerance_m: f64,
    /// Native `segmentLength`, converted from millimetres at the CSET boundary.
    pub uncovered_segment_length_m: f64,
    /// Native `requiredHeight`, converted from millimetres at the CSET boundary.
    pub required_height_m: f64,
    pub check_top_surface: bool,
    pub check_bottom_surface: bool,
    pub intersection_component_classes: Vec<String>,
    pub check_unallocated_area: bool,
    /// Native area values are converted from mm² at the CSET boundary.
    pub maximum_unallocated_area_m2: f64,
    pub categorization: SpaceValidationCategorizationSpec,
    pub use_visualization_arrows: bool,
}

impl Default for SpaceValidationPlanSpec {
    fn default() -> Self {
        Self {
            tolerance_m: 0.02,
            uncovered_segment_length_m: 0.02,
            required_height_m: 2.3,
            check_top_surface: false,
            check_bottom_surface: true,
            intersection_component_classes: vec![
                "IFCWALL".into(),
                "IFCCURTAINWALL".into(),
                "IFCCOLUMN".into(),
                "IFCSPACE".into(),
                "IFCSLAB".into(),
                "IFCROOF".into(),
            ],
            check_unallocated_area: true,
            maximum_unallocated_area_m2: 0.836_127_36,
            categorization: SpaceValidationCategorizationSpec::BySpace,
            use_visualization_arrows: true,
        }
    }
}

impl SpaceValidationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("tolerance_m", self.tolerance_m),
            (
                "uncovered_segment_length_m",
                self.uncovered_segment_length_m,
            ),
            ("required_height_m", self.required_height_m),
            (
                "maximum_unallocated_area_m2",
                self.maximum_unallocated_area_m2,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
        }
        if self
            .intersection_component_classes
            .iter()
            .any(|class| class.trim().is_empty())
        {
            return Err("intersection component classes must be non-empty".into());
        }
        Ok(())
    }
}
