use crate::rule::assertion::Severity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeCountGrouping {
    WholeModel,
    BuildingStorey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegerComparison {
    Equal,
    Greater,
    Less,
    AtLeast,
    AtMost,
    NotEqual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelativeCountRowSpec {
    pub provided: u32,
    pub required: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RelativeCountMode {
    Ratio {
        provided_unit: u32,
        required_unit: u32,
        comparison: IntegerComparison,
    },
    Table {
        rows: Vec<RelativeCountRowSpec>,
        additional_provided: u32,
        additional_required: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelativeCountPlanSpec {
    pub provided_entity_type: String,
    pub required_entity_type: String,
    pub grouping: RelativeCountGrouping,
    pub mode: RelativeCountMode,
    pub severity: Severity,
}

impl RelativeCountPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.provided_entity_type.trim().is_empty()
            || self.required_entity_type.trim().is_empty()
        {
            return Err("relative-count entity types must not be empty".into());
        }
        match &self.mode {
            RelativeCountMode::Ratio {
                provided_unit,
                required_unit,
                ..
            } if *provided_unit == 0 || *required_unit == 0 => {
                Err("relative-count ratio units must be positive".into())
            }
            RelativeCountMode::Table {
                rows,
                additional_provided,
                additional_required,
            } => {
                if rows.is_empty() && (*additional_provided == 0 || *additional_required == 0) {
                    return Err(
                        "relative-count table needs rows or positive extrapolation increments"
                            .into(),
                    );
                }
                if (*additional_provided == 0) != (*additional_required == 0) {
                    return Err(
                        "relative-count extrapolation increments must both be zero or positive"
                            .into(),
                    );
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
