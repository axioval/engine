use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSurface {
    Top,
    Side,
    Bottom,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceSide {
    Inside,
    Outside,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionBandSpec {
    pub surface: HostSurface,
    pub side: SurfaceSide,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_metres: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_metres: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentContainmentPlanSpec {
    pub outer_components: ElementScopeSpec,
    pub inner_components: ElementScopeSpec,
    #[serde(default)]
    pub combine_outer_components: bool,
    #[serde(default)]
    pub dimensions: Vec<DimensionBandSpec>,
    #[serde(default)]
    pub check_component_counts: bool,
    #[serde(default = "default_one")]
    pub minimum_count: usize,
    #[serde(default = "default_one")]
    pub maximum_count: usize,
    #[serde(default)]
    pub check_inner_components: bool,
    #[serde(default)]
    pub check_outer_components: bool,
    #[serde(default)]
    pub forbid_orphans: bool,
}
fn default_one() -> usize {
    1
}

impl ComponentContainmentPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.check_component_counts && self.minimum_count > self.maximum_count {
            return Err("minimum_count exceeds maximum_count".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for band in &self.dimensions {
            let key = (band.surface as u8, band.side as u8);
            if !seen.insert(key) {
                return Err("duplicate surface/side requirement".into());
            }
            for value in [band.minimum_metres, band.maximum_metres]
                .into_iter()
                .flatten()
            {
                if !value.is_finite() || value < 0.0 {
                    return Err("distance must be finite and non-negative".into());
                }
            }
            if let (Some(min), Some(max)) = (band.minimum_metres, band.maximum_metres) {
                if min > max {
                    return Err("minimum distance exceeds maximum distance".into());
                }
            }
        }
        Ok(())
    }
}
