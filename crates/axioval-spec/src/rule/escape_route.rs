use serde::{Deserialize, Serialize};

use super::execution::ElementScopeSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteGeneratorSpec {
    Default,
    Metric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteStartPointSpec {
    Door,
    Corner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StairLengthMethodSpec {
    Linear,
    Manhattan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceUseRequirementSpec {
    pub usage: String,
    pub maximum_distance_metres: f64,
    pub area_per_occupant_square_metres: f64,
    pub minimum_route_count: u32,
    pub route_start: RouteStartPointSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<ElementScopeSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassageWidthRequirementSpec {
    pub maximum_occupants: u32,
    pub total_door_width_metres: f64,
    pub total_passage_width_metres: f64,
    pub minimum_door_width_metres: f64,
    pub minimum_passage_width_metres: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExitClassSpec {
    pub name: String,
    pub kind: ExitKindSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKindSpec {
    Primary,
    Secondary,
    NotExit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ExitSelectionSpec {
    pub space_classification: Option<String>,
    pub exit_classification: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<ExitClassSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_filter: Option<ElementScopeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_filter: Option<ElementScopeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_exit_filter: Option<ElementScopeSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VerticalAccessSelectionSpec {
    pub classification: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stair_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<ElementScopeSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FireZoneSpec {
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscapeRoutePlanSpec {
    pub route_generator: RouteGeneratorSpec,
    pub space_uses: Vec<SpaceUseRequirementSpec>,
    pub passage_widths: Vec<PassageWidthRequirementSpec>,
    pub minimum_passage_height_metres: f64,
    pub exits: ExitSelectionSpec,
    pub vertical_access: VerticalAccessSelectionSpec,
    pub check_door_opening_directions: bool,
    pub stair_length_method: StairLengthMethodSpec,
    pub stair_vertical_route_multiplier: f64,
    pub shared_route_multiplier: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fire_zones: Vec<FireZoneSpec>,
}

impl EscapeRoutePlanSpec {
    pub fn verified_default() -> Self {
        Self {
            route_generator: RouteGeneratorSpec::Metric,
            space_uses: vec![SpaceUseRequirementSpec {
                usage: "*".into(),
                maximum_distance_metres: 30.0,
                area_per_occupant_square_metres: 10.0,
                minimum_route_count: 1,
                route_start: RouteStartPointSpec::Door,
                filter: None,
            }],
            passage_widths: vec![PassageWidthRequirementSpec {
                maximum_occupants: 120,
                total_door_width_metres: 1.2,
                total_passage_width_metres: 1.2,
                minimum_door_width_metres: 0.9,
                minimum_passage_width_metres: 0.9,
            }],
            minimum_passage_height_metres: 2.1,
            exits: ExitSelectionSpec {
                classes: vec![ExitClassSpec {
                    name: "*".into(),
                    kind: ExitKindSpec::Primary,
                }],
                ..Default::default()
            },
            vertical_access: VerticalAccessSelectionSpec::default(),
            check_door_opening_directions: true,
            stair_length_method: StairLengthMethodSpec::Linear,
            stair_vertical_route_multiplier: 1.0,
            shared_route_multiplier: 1.0,
            fire_zones: vec![],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        positive("minimum passage height", self.minimum_passage_height_metres)?;
        positive(
            "stair vertical route multiplier",
            self.stair_vertical_route_multiplier,
        )?;
        positive("shared route multiplier", self.shared_route_multiplier)?;
        if self.space_uses.is_empty() {
            return Err("space-use requirements must not be empty".into());
        }
        unique_non_empty(
            self.space_uses.iter().map(|v| v.usage.as_str()),
            "space usage",
        )?;
        for row in &self.space_uses {
            positive("maximum route distance", row.maximum_distance_metres)?;
            non_negative(
                "occupants per square metre",
                row.area_per_occupant_square_metres,
            )?;
            if row.minimum_route_count == 0 {
                return Err("minimum route count must be positive".into());
            }
        }
        if self.passage_widths.is_empty() {
            return Err("passage-width requirements must not be empty".into());
        }
        let mut previous = 0;
        for row in &self.passage_widths {
            if row.maximum_occupants == 0 || row.maximum_occupants <= previous {
                return Err("occupant thresholds must be positive and strictly increasing".into());
            }
            previous = row.maximum_occupants;
            for (name, value) in [
                ("total door width", row.total_door_width_metres),
                ("total passage width", row.total_passage_width_metres),
                ("minimum door width", row.minimum_door_width_metres),
                ("minimum passage width", row.minimum_passage_width_metres),
            ] {
                non_negative(name, value)?;
            }
        }
        unique_non_empty(
            self.exits.classes.iter().map(|v| v.name.as_str()),
            "exit class",
        )?;
        unique_non_empty(
            self.vertical_access.stair_names.iter().map(String::as_str),
            "vertical-access stair",
        )?;
        unique_non_empty(
            self.fire_zones.iter().map(|v| v.usage.as_str()),
            "fire zone",
        )?;
        Ok(())
    }
}

fn positive(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and positive"))
    }
}
fn non_negative(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and non-negative"))
    }
}
fn unique_non_empty<'a>(values: impl Iterator<Item = &'a str>, name: &str) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value) {
            return Err(format!("{name} names must be non-empty and unique"));
        }
    }
    Ok(())
}
