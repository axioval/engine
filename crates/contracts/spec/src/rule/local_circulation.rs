//! Vendor-neutral local accessible-circulation contract.
use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentConnectionMode {
    OneComponentConnectedToPath,
    TwoComponentsConnectedByPath,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndClearanceSpec {
    pub exception_minimum_width_metres: f64,
    pub exclude_ends_shorter_than_metres: f64,
    pub free_space_width_metres: f64,
    pub free_space_length_metres: f64,
    pub exclude_near_components: Option<ElementScopeSpec>,
    pub maximum_distance_metres: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentConnectionSpec {
    pub components_a: ElementScopeSpec,
    pub components_b: ElementScopeSpec,
    pub mode: ComponentConnectionMode,
    pub tolerance_distance_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalCirculationPlanSpec {
    pub spaces: ElementScopeSpec,
    pub merge_spaces: bool,
    pub entrances: ElementScopeSpec,
    pub obstacles: ElementScopeSpec,
    pub minimum_width_metres: f64,
    pub elevation_band_metres: Option<(f64, f64)>,
    pub subtract_door_swings: bool,
    pub can_walk_through_doors: bool,
    pub require_entrances: bool,
    pub check_entrance_width: bool,
    pub end_clearance: Option<EndClearanceSpec>,
    pub component_connection: Option<ComponentConnectionSpec>,
}
impl LocalCirculationPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, s) in [
            ("spaces", &self.spaces),
            ("entrances", &self.entrances),
            ("obstacles", &self.obstacles),
        ] {
            if s.candidate_types.is_empty() && s.clauses.is_empty() {
                return Err(format!("local circulation {name} scope is empty"));
            }
        }
        if !self.minimum_width_metres.is_finite() || self.minimum_width_metres <= 0.0 {
            return Err("local circulation width must be finite and positive".into());
        }
        if let Some((lo, hi)) = self.elevation_band_metres {
            if !lo.is_finite() || !hi.is_finite() || lo < 0.0 || hi <= lo {
                return Err("local circulation elevation band is invalid".into());
            }
        }
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if let Some(e) = &self.end_clearance {
            if ![
                e.exception_minimum_width_metres,
                e.exclude_ends_shorter_than_metres,
                e.free_space_width_metres,
                e.free_space_length_metres,
            ]
            .into_iter()
            .all(positive)
            {
                return Err(
                    "local circulation end-clearance dimensions must be finite and positive".into(),
                );
            }
            if e.maximum_distance_metres.is_some_and(|v| !positive(v)) {
                return Err(
                    "local circulation maximum distance must be finite and positive".into(),
                );
            }
            if e.exclude_near_components
                .as_ref()
                .is_some_and(|s| s.candidate_types.is_empty() && s.clauses.is_empty())
            {
                return Err("local circulation near-component scope is empty".into());
            }
        }
        if let Some(c) = &self.component_connection {
            if !positive(c.tolerance_distance_metres)
                || [&c.components_a, &c.components_b]
                    .into_iter()
                    .any(|s| s.candidate_types.is_empty() && s.clauses.is_empty())
            {
                return Err("local circulation component-connection plan is invalid".into());
            }
        }
        Ok(())
    }
}
