use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparedPropertySpec {
    pub property_set: Option<String>,
    pub property: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelComparisonIdentificationMode {
    Guid,
    GeometryAndPlacement,
    VolumeIntersection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelComparisonPlanSpec {
    pub first_model_index: u32,
    pub second_model_index: u32,
    #[serde(default)]
    pub scope: ElementScopeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_scope: Option<ElementScopeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second_scope: Option<ElementScopeSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identification_modes: Vec<ModelComparisonIdentificationMode>,
    #[serde(default)]
    pub identify_by_guid: bool,
    #[serde(default)]
    pub compare_geometry: bool,
    #[serde(default)]
    pub compare_locations: bool,
    #[serde(default)]
    pub compare_properties: bool,
    #[serde(default)]
    pub compare_quantities: bool,
    #[serde(default)]
    pub compare_coordinate_systems: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compared_properties: Vec<ComparedPropertySpec>,
    #[serde(default)]
    pub compare_property_sets: bool,
}
impl ModelComparisonPlanSpec {
    pub fn verified_default() -> Self {
        Self {
            first_model_index: 0,
            second_model_index: 0,
            scope: ElementScopeSpec::default(),
            first_scope: None,
            second_scope: None,
            identification_modes: vec![
                ModelComparisonIdentificationMode::Guid,
                ModelComparisonIdentificationMode::GeometryAndPlacement,
                ModelComparisonIdentificationMode::VolumeIntersection,
            ],
            identify_by_guid: true,
            compare_geometry: true,
            compare_locations: false,
            compare_properties: false,
            compare_quantities: false,
            compare_coordinate_systems: true,
            compared_properties: Vec::new(),
            compare_property_sets: false,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.compare_geometry
            && !self.compare_locations
            && !self.compare_properties
            && !self.compare_quantities
            && !self.compare_coordinate_systems
            && !self.compare_property_sets
            && self.compared_properties.is_empty()
        {
            return Err("model comparison must enable at least one comparison dimension".into());
        }
        if self
            .compared_properties
            .iter()
            .any(|p| p.property.trim().is_empty())
        {
            return Err("compared property names must be non-empty".into());
        }
        Ok(())
    }
}
