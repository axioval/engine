use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GuidUniquenessScopeSpec {
    #[default]
    NotChecked,
    WithinModel,
    AcrossModels,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelArchitecturePlanSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disciplines: Vec<String>,
    pub check_model_hierarchy: bool,
    pub require_direct_relations: bool,
    pub check_empty_storeys: bool,
    pub check_same_storey_elevations: bool,
    pub check_same_storey_names: bool,
    pub check_thicknesses: bool,
    pub require_doors_in_same_storey: bool,
    pub check_polygon_count: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_polygon_count: Option<u32>,
    pub check_space_boundaries: bool,
    pub check_orphan_doors_and_windows: bool,
    pub check_door_opening_directions: bool,
    pub require_single_site: bool,
    pub require_site_geometry: bool,
    pub guid_uniqueness: GuidUniquenessScopeSpec,
}

impl ModelArchitecturePlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.disciplines.iter().any(|value| value.trim().is_empty()) {
            return Err("model disciplines must be non-empty".into());
        }
        let mut normalized = self
            .disciplines
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .collect::<Vec<_>>();
        normalized.sort_unstable();
        if normalized.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("model disciplines must be unique".into());
        }
        match (self.check_polygon_count, self.maximum_polygon_count) {
            (true, None | Some(0)) => Err(
                "maximum polygon count must be positive when polygon checking is enabled".into(),
            ),
            (false, Some(_)) => {
                Err("maximum polygon count must be absent when polygon checking is disabled".into())
            }
            _ => Ok(()),
        }
    }
}
