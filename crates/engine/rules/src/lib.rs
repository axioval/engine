//! Built-in trusted source-neutral rule capabilities.
#![forbid(unsafe_code)]

use axioval_engine::{CapabilityRegistry, EngineError};

mod accessible_route;
mod allowed_profile;
mod body_extent;
mod body_facts;
mod centre_line_distance;
mod clash;
mod clash_cases;
mod clash_groups;
mod clash_matrix;
mod clash_severity;
mod classification_requirement;
mod comparison;
mod component_clearance;
mod component_visibility;
mod conformance;
mod consistent_value;
mod containment;
mod corridor_end_openings;
mod counterpart_coverage;
mod counts;
mod distance;
mod door_swing;
mod door_swing_direction;
mod effective_coverage;
mod escape_route;
mod exit_separation;
mod external_wall_validation;
mod free_floor;
mod free_floor_circle;
mod free_floor_rectangle;
mod group_composition;
mod guard_diagnosis;
mod horizontal_guard;
mod keyed_limit;
mod level_spacing;
mod levels;
mod light_area;
mod light_well;
mod local_circulation;
mod manual_issue;
mod name_sequence;
mod numbering_consistency;
mod object_count;
mod opening_area;
mod opening_spaces;
mod opening_zone;
mod orientation;
mod pairs;
mod parking_bay;
mod passing_spaces;
mod plan_area;
mod property_comparison;
mod property_predicate;
mod property_requirements;
mod property_rules;
mod property_value;
mod recess_width;
mod relative_count;
mod same_container;
mod selection;
mod shelf_capacity;
mod slab_contact;
mod slab_stack;
mod space_access;
mod space_boundary_coverage;
mod space_connection;
mod space_distance;
mod space_validation;
mod stair_geometry;
mod support;
mod table_allocation;
mod triangle_count;
mod unique_value;
mod wall_sides;
mod wall_spacing;
mod xsd_pattern;

pub use accessible_route::AccessibleRoute;
pub use allowed_profile::AllowedProfile;
pub use body_extent::BodyExtent;
pub use centre_line_distance::CentreLineDistance;
pub use clash::Clash;
pub use clash_matrix::ClashMatrix;
pub use classification_requirement::ClassificationRequirement;
pub use comparison::{
    AmbiguousIdentity, ComparedObject, ComparedProperty, ComparisonError, ComparisonRequest,
    ComparisonTolerance, Difference, Facet, Measure, Measurement, ModelComparison, ObjectChange,
    Side, SourceComparison, Unresolved, compare_sessions,
};
pub use component_clearance::ComponentClearance;
pub use component_visibility::ComponentVisibility;
pub use conformance::SelectorConformance;
pub use consistent_value::ConsistentValue;
pub use containment::Containment;
pub use corridor_end_openings::CorridorEndOpenings;
pub use counterpart_coverage::CounterpartCoverage;
pub use counts::RelatedCount;
pub use distance::Distance;
pub use door_swing_direction::DoorSwing;
pub use effective_coverage::EffectiveCoverage;
pub use escape_route::EscapeRoute;
pub use exit_separation::ExitSeparation;
pub use external_wall_validation::ExternalWallValidation;
pub use free_floor_circle::FreeFloorCircle;
pub use free_floor_rectangle::FreeFloorRectangle;
pub use group_composition::GroupComposition;
pub use guard_diagnosis::{GuardDefect, GuardDiagnosis};
pub use horizontal_guard::HorizontalGuard;
pub use keyed_limit::KeyedLimit;
pub use level_spacing::LevelSpacing;
pub use light_well::LightWell;
pub use local_circulation::LocalCirculation;
pub use manual_issue::ManualIssue;
pub use name_sequence::NameSequence;
pub use numbering_consistency::NumberingConsistency;
pub use object_count::ObjectCount;
pub use opening_area::OpeningArea;
pub use opening_spaces::OpeningSpaces;
pub use opening_zone::OpeningZone;
pub use parking_bay::ParkingBay;
pub use plan_area::{AreaRatio, PlanAreaRange, PlanCoverage};
pub use property_comparison::PropertyComparison;
pub use property_predicate::PropertyPredicate;
pub use property_requirements::PropertyRequirements;
pub use property_rules::{
    BooleanPropertyEquals, PropertyDataType, PropertyExists, PropertyRequired,
};
pub use property_value::PropertyValueConstraint;
pub use recess_width::RecessWidth;
pub use relative_count::RelativeCount;
pub use same_container::SameContainer;
pub use shelf_capacity::ShelfCapacity;
pub use slab_contact::SlabContact;
pub use slab_stack::SlabStackSpacing;
pub use space_boundary_coverage::SpaceBoundaryCoverage;
pub use space_connection::SpaceConnection;
pub use space_distance::SpaceDistance;
pub use space_validation::{SpaceCategory, SpaceValidation};
pub use stair_geometry::{RampGeometryCheck, StairGeometryCheck};
pub use table_allocation::TableAllocation;
pub use triangle_count::TriangleCountLimit;
pub use unique_value::UniqueValue;
pub use wall_spacing::WallSpacing;
/// XML Schema patterns (as IDS and `property-value` write them) in `regex`
/// syntax, for a property selector's `matches` operator.
pub use xsd_pattern::translate as translate_xsd_pattern;

/// Registers all maintained built-in capabilities into a host registry.
///
/// # Errors
///
/// Returns an error if the registry already contains a built-in capability ID.
pub fn register_builtins(registry: CapabilityRegistry) -> Result<CapabilityRegistry, EngineError> {
    registry
        .register(PropertyExists)
        .and_then(|registry| registry.register(PropertyRequired))
        .and_then(|registry| registry.register(PropertyDataType))
        .and_then(|registry| registry.register(PropertyValueConstraint))
        .and_then(|registry| registry.register(BooleanPropertyEquals))
        .and_then(|registry| registry.register(PropertyPredicate))
        .and_then(|registry| registry.register(PropertyComparison))
        .and_then(|registry| registry.register(PropertyRequirements))
        .and_then(|registry| registry.register(ShelfCapacity))
        .and_then(|registry| registry.register(RecessWidth))
        .and_then(|registry| registry.register(LightWell))
        .and_then(|registry| registry.register(SlabContact))
        .and_then(|registry| registry.register(ExternalWallValidation))
        .and_then(|registry| registry.register(SpaceValidation))
        .and_then(|registry| registry.register(HorizontalGuard))
        .and_then(|registry| registry.register(FreeFloorCircle))
        .and_then(|registry| registry.register(FreeFloorRectangle))
        .and_then(|registry| registry.register(Clash))
        .and_then(|registry| registry.register(ClashMatrix))
        .and_then(|registry| registry.register(Containment))
        .and_then(|registry| registry.register(Distance))
        .and_then(|registry| registry.register(DoorSwing))
        .and_then(|registry| registry.register(SelectorConformance))
        .and_then(|registry| registry.register(ClassificationRequirement))
        .and_then(|registry| registry.register(UniqueValue))
        .and_then(|registry| registry.register(ConsistentValue))
        .and_then(|registry| registry.register(RelatedCount))
        .and_then(|registry| registry.register(ObjectCount))
        .and_then(|registry| registry.register(RelativeCount))
        .and_then(|registry| registry.register(NameSequence))
        .and_then(|registry| registry.register(NumberingConsistency))
        .and_then(|registry| registry.register(ManualIssue))
        .and_then(|registry| registry.register(LevelSpacing))
        .and_then(|registry| registry.register(AreaRatio))
        .and_then(|registry| registry.register(PlanCoverage))
        .and_then(|registry| registry.register(PlanAreaRange))
        .and_then(|registry| registry.register(KeyedLimit))
        .and_then(|registry| registry.register(SlabStackSpacing))
        .and_then(|registry| registry.register(TableAllocation))
        .and_then(|registry| registry.register(OpeningSpaces))
        .and_then(|registry| registry.register(GroupComposition))
        .and_then(|registry| registry.register(ExitSeparation))
        .and_then(|registry| registry.register(EscapeRoute))
        .and_then(|registry| registry.register(CounterpartCoverage))
        .and_then(|registry| registry.register(ParkingBay))
        .and_then(|registry| registry.register(WallSpacing))
        .and_then(|registry| registry.register(BodyExtent))
        .and_then(|registry| registry.register(TriangleCountLimit))
        .and_then(|registry| registry.register(SameContainer))
        .and_then(|registry| registry.register(StairGeometryCheck))
        .and_then(|registry| registry.register(RampGeometryCheck))
        .and_then(|registry| registry.register(AccessibleRoute))
        .and_then(|registry| registry.register(SpaceConnection))
        .and_then(|registry| registry.register(SpaceDistance))
        .and_then(|registry| registry.register(ComponentClearance))
        .and_then(|registry| registry.register(CentreLineDistance))
        .and_then(|registry| registry.register(ComponentVisibility))
        .and_then(|registry| registry.register(EffectiveCoverage))
        .and_then(|registry| registry.register(LocalCirculation))
        .and_then(|registry| registry.register(CorridorEndOpenings))
        .and_then(|registry| registry.register(SpaceBoundaryCoverage))
        .and_then(|registry| registry.register(AllowedProfile))
        .and_then(|registry| registry.register(OpeningZone))
        .and_then(|registry| registry.register(OpeningArea))
}
