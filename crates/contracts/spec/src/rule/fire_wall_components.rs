use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FireComponentTypeRowSpec {
    pub wall_type: Option<String>,
    pub door_type: Option<String>,
    pub window_type: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FireWallComponentsPlanSpec {
    pub allowed_type_rows: Vec<FireComponentTypeRowSpec>,
    pub check_non_fire_walls: bool,
}
impl FireWallComponentsPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (index, row) in self.allowed_type_rows.iter().enumerate() {
            if [&row.wall_type, &row.door_type, &row.window_type]
                .iter()
                .all(|value| value.as_ref().is_none_or(|value| value.trim().is_empty()))
            {
                return Err(format!("fire component row {index} is empty"));
            }
        }
        Ok(())
    }
}
