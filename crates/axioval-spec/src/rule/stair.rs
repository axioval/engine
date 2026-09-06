use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StairSelectionSpec {
    pub spaces: ElementScopeSpec,
    pub stairs: ElementScopeSpec,
    pub blast_doors: ElementScopeSpec,
    pub tactile_surfaces: ElementScopeSpec,
    pub accessible_routes: ElementScopeSpec,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeSpec {
    pub minimum: f64,
    pub maximum: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StairGeometrySpec {
    pub minimum_space_beginning_metres: f64,
    pub minimum_space_end_metres: f64,
    pub minimum_total_width_metres: f64,
    pub minimum_clear_width_metres: f64,
    pub minimum_landing_clear_width_metres: f64,
    pub minimum_landing_length_metres: f64,
    pub flight_steps: RangeSpec,
    pub riser_height_metres: RangeSpec,
    pub tread_length_metres: RangeSpec,
    pub tread_riser_sum_metres: RangeSpec,
    pub use_tread_distance: bool,
    pub tread_distance_metres: f64,
    pub minimum_head_clearance_above_metres: f64,
    pub minimum_head_clearance_under_metres: f64,
    pub maximum_nosing_length_metres: f64,
    pub maximum_flight_height_metres: f64,
    pub maximum_stair_height_metres: f64,
    pub allow_open_risers: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StairHandrailSpec {
    pub side: String,
    pub height_metres: RangeSpec,
    pub minimum_extension_metres: f64,
    pub extension_from_nosing: bool,
    pub continuity_tolerance_metres: Option<f64>,
    pub continuity_exemption: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TactileWarningSpec {
    pub depth_metres: f64,
    pub offset_metres: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StairPlanSpec {
    pub selection: StairSelectionSpec,
    pub geometry: StairGeometrySpec,
    pub require_landing_between_flights: bool,
    pub check_slab_connections: bool,
    pub check_uniform_riser_heights: bool,
    pub check_indoors: bool,
    pub check_outdoors: bool,
    pub winder_angle_degrees: Option<RangeSpec>,
    pub handrails: Option<StairHandrailSpec>,
    pub tactile_warning: Option<TactileWarningSpec>,
    /// Native `rpBlastDoorFilter.isEmpty()` activation state. The lowered scope
    /// alone cannot preserve whether an authored component filter was empty.
    #[serde(default)]
    pub blast_door_filter_enabled: bool,
    /// Native `rpAccessibleRouteFilter.isEmpty()` activation state.
    #[serde(default)]
    pub accessible_route_filter_enabled: bool,
}
impl StairPlanSpec {
    pub fn verified_default(selection: StairSelectionSpec) -> Self {
        Self {
            selection,
            geometry: StairGeometrySpec {
                minimum_space_beginning_metres: 1.5,
                minimum_space_end_metres: 1.5,
                minimum_total_width_metres: 1.2,
                minimum_clear_width_metres: 1.0,
                minimum_landing_clear_width_metres: 0.9,
                minimum_landing_length_metres: 1.5,
                flight_steps: RangeSpec {
                    minimum: 0.0,
                    maximum: 12.0,
                },
                riser_height_metres: RangeSpec {
                    minimum: 0.15,
                    maximum: 0.17,
                },
                tread_length_metres: RangeSpec {
                    minimum: 0.26,
                    maximum: 0.30,
                },
                tread_riser_sum_metres: RangeSpec {
                    minimum: 0.60,
                    maximum: 0.66,
                },
                use_tread_distance: true,
                tread_distance_metres: 0.5,
                minimum_head_clearance_above_metres: 2.1,
                minimum_head_clearance_under_metres: 2.1,
                maximum_nosing_length_metres: 0.025,
                maximum_flight_height_metres: 4.0,
                maximum_stair_height_metres: 12.0,
                allow_open_risers: true,
            },
            require_landing_between_flights: true,
            check_slab_connections: true,
            check_uniform_riser_heights: true,
            check_indoors: true,
            check_outdoors: true,
            winder_angle_degrees: None,
            handrails: Some(StairHandrailSpec {
                side: "NOT_REQUIRED".into(),
                height_metres: RangeSpec {
                    minimum: 0.012,
                    maximum: 0.013,
                },
                minimum_extension_metres: 0.015,
                extension_from_nosing: true,
                continuity_tolerance_metres: Some(0.01),
                continuity_exemption: false,
            }),
            tactile_warning: Some(TactileWarningSpec {
                depth_metres: 0.6,
                offset_metres: 0.3,
            }),
            blast_door_filter_enabled: false,
            accessible_route_filter_enabled: false,
        }
    }
}
impl RangeSpec {
    fn valid(&self, zero: bool) -> bool {
        self.minimum.is_finite()
            && self.maximum.is_finite()
            && self.minimum <= self.maximum
            && (zero || self.minimum > 0.0)
    }
}
impl StairPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for s in [&self.selection.spaces, &self.selection.stairs] {
            if s.candidate_types.is_empty() {
                return Err("required stair scope must not be empty".into());
            }
        }
        for (enabled, scope, name) in [
            (
                self.tactile_warning.is_some(),
                &self.selection.tactile_surfaces,
                "tactile surface",
            ),
            (
                self.blast_door_filter_enabled,
                &self.selection.blast_doors,
                "blast door",
            ),
            (
                self.accessible_route_filter_enabled,
                &self.selection.accessible_routes,
                "accessible route",
            ),
        ] {
            if enabled && scope.candidate_types.is_empty() {
                return Err(format!("enabled {name} scope must not be empty"));
            }
        }
        let g = &self.geometry;
        for r in [
            &g.riser_height_metres,
            &g.tread_length_metres,
            &g.tread_riser_sum_metres,
        ] {
            if !r.valid(false) {
                return Err("invalid stair range".into());
            }
        }
        if !g.flight_steps.valid(true)
            || g.flight_steps.minimum.fract() != 0.0
            || g.flight_steps.maximum.fract() != 0.0
        {
            return Err("flight steps must be ordered integers".into());
        }
        let dims = [
            g.minimum_space_beginning_metres,
            g.minimum_space_end_metres,
            g.minimum_total_width_metres,
            g.minimum_clear_width_metres,
            g.minimum_landing_clear_width_metres,
            g.minimum_landing_length_metres,
            g.minimum_head_clearance_above_metres,
            g.minimum_head_clearance_under_metres,
            g.maximum_flight_height_metres,
            g.maximum_stair_height_metres,
        ];
        if dims.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("invalid stair dimension".into());
        }
        if let Some(a) = &self.winder_angle_degrees {
            if !a.valid(true) || a.maximum > 360.0 {
                return Err("invalid winder angle".into());
            }
        }
        Ok(())
    }
}
