use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerAgreementRowSpec {
    pub component_type: String,
    pub construction_type_pattern: String,
    pub layer_pattern: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LayerAgreementPlanSpec {
    pub rows: Vec<LayerAgreementRowSpec>,
    pub check_space_groups: bool,
}
impl LayerAgreementPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.rows.is_empty() {
            return Err("layer agreement needs at least one row".into());
        }
        for (i, r) in self.rows.iter().enumerate() {
            if r.component_type.trim().is_empty() || r.layer_pattern.trim().is_empty() {
                return Err(format!(
                    "layer agreement row {i} needs component type and layer"
                ));
            }
        }
        Ok(())
    }
}
