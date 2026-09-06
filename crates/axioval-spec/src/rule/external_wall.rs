use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ExternalWallValidationPlanSpec {
    pub check_walls_around_all_spaces: bool,
    pub spaces: ElementScopeSpec,
    pub check_walls_around_space_groups: bool,
    pub space_groups: ElementScopeSpec,
}
impl ExternalWallValidationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}
