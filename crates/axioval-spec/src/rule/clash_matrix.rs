use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClashDimensionSpec {
    Domain,
    IfcEntity,
    Property {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        property_set: Option<String>,
        property: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantity_type: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClashCategorySpec {
    /// Ordered hierarchy values. An empty path is the wildcard category.
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClashMatrixOptionsSpec {
    pub check_duplicates: bool,
    pub check_inside: bool,
    pub check_overlapping: bool,
    pub ignore_same_system: bool,
    pub ignore_same_layer_and_model: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClashToleranceSpec {
    pub name: String,
    pub horizontal_metres: f64,
    pub vertical_metres: f64,
    pub volume_cubic_metres: f64,
    pub use_volume: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClashMatrixCellSpec {
    pub name: Option<String>,
    pub enabled: bool,
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_keys: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClashMatrixPlanSpec {
    #[serde(default)]
    pub scope: ElementScopeSpec,
    pub options: ClashMatrixOptionsSpec,
    pub tolerances: Vec<ClashToleranceSpec>,
    pub row_dimensions: Vec<ClashDimensionSpec>,
    pub column_dimensions: Vec<ClashDimensionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_categories: Vec<ClashCategorySpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_categories: Vec<ClashCategorySpec>,
    pub row_count: usize,
    pub column_count: usize,
    pub cells: Vec<ClashMatrixCellSpec>,
}
impl ClashMatrixPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        let expected = self
            .row_count
            .checked_mul(self.column_count)
            .ok_or_else(|| "clash matrix dimensions overflow".to_string())?;
        if self.row_count == 0 || self.column_count == 0 || self.cells.len() != expected {
            return Err(format!(
                "clash matrix must be rectangular: {}x{} requires {} cells, got {}",
                self.row_count,
                self.column_count,
                expected,
                self.cells.len()
            ));
        }
        for (axis, count, dimensions, categories) in [
            (
                "row",
                self.row_count,
                &self.row_dimensions,
                &self.row_categories,
            ),
            (
                "column",
                self.column_count,
                &self.column_dimensions,
                &self.column_categories,
            ),
        ] {
            if !categories.is_empty() && categories.len() != count {
                return Err(format!(
                    "clash {axis} category count must equal {axis}_count"
                ));
            }
            if categories.iter().any(|category| {
                !category.values.is_empty() && category.values.len() != dimensions.len()
            }) {
                return Err(format!(
                    "clash {axis} category paths must match the hierarchy depth"
                ));
            }
        }
        for dimension in self.row_dimensions.iter().chain(&self.column_dimensions) {
            if let ClashDimensionSpec::Property { property, .. } = dimension {
                if property.trim().is_empty() {
                    return Err("clash hierarchy property names must be non-empty".into());
                }
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for t in &self.tolerances {
            let values = [
                t.horizontal_metres,
                t.vertical_metres,
                t.volume_cubic_metres,
            ];
            if t.name.trim().is_empty()
                || !names.insert(&t.name)
                || values.iter().any(|v| !v.is_finite() || *v < 0.0)
            {
                return Err(
                    "clash tolerances require unique names and finite non-negative values".into(),
                );
            }
        }
        if self
            .cells
            .iter()
            .flat_map(|c| &c.result_keys)
            .any(|k| k.trim().is_empty())
        {
            return Err("clash result keys must be non-empty".into());
        }
        Ok(())
    }
}
