use super::execution::ElementScopeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorwaySeparationMetric {
    ClosestBoundary,
    Centroid,
    FarthestBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BooleanPropertyReferenceSpec {
    pub property_set: String,
    pub property: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExitAccessDoorwayPlanSpec {
    pub spaces: ElementScopeSpec,
    pub doorway_components: ElementScopeSpec,
    pub include_doors_inside_space: bool,
    pub space_sprinkler_property: BooleanPropertyReferenceSpec,
    pub storey_sprinkler_property: BooleanPropertyReferenceSpec,
    pub building_sprinkler_property: BooleanPropertyReferenceSpec,
    pub separation_metric: DoorwaySeparationMetric,
    pub sprinkler_reduces_required_separation: bool,
}

impl ExitAccessDoorwayPlanSpec {
    pub fn validate(&self) -> Result<(), String> {
        for (name, reference) in [
            ("space sprinkler", &self.space_sprinkler_property),
            ("storey sprinkler", &self.storey_sprinkler_property),
            ("building sprinkler", &self.building_sprinkler_property),
        ] {
            if reference.property_set.trim().is_empty() || reference.property.trim().is_empty() {
                return Err(format!("{name} property reference must be complete"));
            }
        }
        Ok(())
    }
}
