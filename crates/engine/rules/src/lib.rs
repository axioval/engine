//! Built-in trusted source-neutral rule capabilities.
#![forbid(unsafe_code)]

use axioval_engine::{CapabilityRegistry, EngineError};

mod accessible_route;
mod allowed_profile;
mod area_ratio;
mod body_extent;
mod body_facts;
pub mod catalogue;
mod catalogue_texts;
mod centre_line_distance;
mod clash;
mod clash_cases;
mod clash_groups;
mod clash_matrix;
mod clash_severity;
mod classification_requirement;
mod climbing;
mod comparison;
mod component_clearance;
mod component_visibility;
mod conformance;
mod consistent_value;
mod containment;
mod coordinate_consistency;
mod corridor_end_openings;
mod counterpart_coverage;
mod counts;
mod distance;
mod door_swing;
mod door_swing_direction;
mod effective_coverage;
mod empty_host;
mod escape_route;
mod exit_separation;
mod expression_leaves;
mod expression_requirement;
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
mod location;
mod manual_issue;
mod measured_arguments;
mod measured_kinds;
mod measured_openings;
mod measured_plan;
mod name_sequence;
mod near;
mod numbering_consistency;
mod object_count;
mod object_parameters;
mod opening_area;
mod opening_spaces;
mod opening_zone;
mod orientation;
mod pairs;
pub mod parity;
mod parking_bay;
mod passing_spaces;
mod plan_area;
mod plan_coverage;
mod property_comparison;
mod property_predicate;
mod property_requirements;
mod property_rules;
mod property_value;
mod quantity_takeoff;
mod recess_width;
mod refine;
mod related_count;
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
pub mod templates;
mod triangle_count;
mod unclassified;
mod unique_value;
mod wall_sides;
mod wall_spacing;
mod xsd_pattern;

pub use accessible_route::AccessibleRoute;
pub use allowed_profile::AllowedProfile;
pub use area_ratio::AreaRatio;
pub use body_extent::BodyExtent;
pub use centre_line_distance::CentreLineDistance;
pub use clash::Clash;
pub use clash_matrix::ClashMatrix;
pub use classification_requirement::ClassificationRequirement;
pub use comparison::{
    AmbiguousIdentity, CompareModels, ComparedObject, ComparedProperty, ComparisonError,
    ComparisonRequest, ComparisonTolerance, Difference, Facet, GeometryMode, Matcher, Measure,
    Measurement, ModelComparison, ObjectChange, Side, SourceComparison, UndecidedMatch, Unresolved,
    Witness, compare_sessions,
};
pub use component_clearance::ComponentClearance;
pub use component_visibility::ComponentVisibility;
pub use conformance::SelectorConformance;
pub use consistent_value::ConsistentValue;
pub use containment::Containment;
pub use coordinate_consistency::{
    CoordinateAspect, CoordinateConsistency, CoordinateConsistencyCheck, CoordinateTolerance,
    compare_coordinate_systems,
};
pub use corridor_end_openings::CorridorEndOpenings;
pub use counterpart_coverage::CounterpartCoverage;
pub use distance::Distance;
pub use door_swing_direction::DoorSwing;
pub use effective_coverage::EffectiveCoverage;
pub use empty_host::EmptyHost;
pub use escape_route::EscapeRoute;
pub use exit_separation::ExitSeparation;
pub use expression_requirement::ExpressionRequirement;
pub use external_wall_validation::ExternalWallValidation;
pub use free_floor_circle::FreeFloorCircle;
pub use free_floor_rectangle::FreeFloorRectangle;
pub use group_composition::GroupComposition;
pub use guard_diagnosis::GuardDefect;
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
pub use plan_area::PlanAreaRange;
pub use plan_coverage::PlanCoverage;
pub use property_comparison::PropertyComparison;
pub use property_predicate::PropertyPredicate;
pub use property_requirements::PropertyRequirements;
pub use property_rules::{
    BooleanPropertyEquals, PropertyDataType, PropertyExists, PropertyRequired,
};
pub use property_value::PropertyValueConstraint;
pub use quantity_takeoff::{QuantityTakeoff, TAKEOFF_TABLE};
pub use recess_width::RecessWidth;
pub use refine::Refiner;
pub use related_count::RelatedCount;
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
pub use unclassified::UnclassifiedObject;
pub use unique_value::UniqueValue;
pub use wall_spacing::WallSpacing;
/// The implementations of capabilities since rebuilt as templates, kept
/// only as the parity references their templates are held to in tests.
/// Never register one: the capability id resolves to the template.
#[cfg(feature = "parity-reference")]
pub mod reference {
    pub use crate::area_ratio::reference::AreaRatio;
    pub use crate::body_extent::reference::BodyExtent;
    pub use crate::centre_line_distance::reference::CentreLineDistance;
    pub use crate::component_visibility::reference::ComponentVisibility;
    pub use crate::conformance::reference::SelectorConformance;
    pub use crate::consistent_value::reference::ConsistentValue;
    pub use crate::coordinate_consistency::reference::CoordinateConsistencyCheck;
    pub use crate::counterpart_coverage::reference::CounterpartCoverage;
    pub use crate::effective_coverage::reference::EffectiveCoverage;
    pub use crate::external_wall_validation::reference::ExternalWallValidation;
    pub use crate::horizontal_guard::reference::HorizontalGuard;
    pub use crate::level_spacing::reference::LevelSpacing;
    pub use crate::light_well::reference::LightWell;
    pub use crate::object_count::reference::ObjectCount;
    pub use crate::plan_area::reference::PlanAreaRange;
    pub use crate::plan_coverage::reference::PlanCoverage;
    pub use crate::property_comparison::reference::PropertyComparison;
    pub use crate::property_predicate::reference::PropertyPredicate;
    pub use crate::property_requirements::reference::PropertyRequirements;
    pub use crate::property_value::reference::PropertyValueConstraint;
    pub use crate::recess_width::reference::RecessWidth;
    pub use crate::related_count::reference::RelatedCount;
    pub use crate::relative_count::reference::RelativeCount;
    pub use crate::shelf_capacity::reference::ShelfCapacity;
    pub use crate::slab_contact::reference::SlabContact;
    pub use crate::slab_stack::reference::SlabStackSpacing;
    pub use crate::space_boundary_coverage::reference::SpaceBoundaryCoverage;
    pub use crate::stair_geometry::reference::RampGeometryReference as RampGeometry;
    pub use crate::stair_geometry::reference::StairGeometryReference as StairGeometry;
    pub use crate::triangle_count::reference::TriangleCountLimit;
    pub use crate::unique_value::reference::UniqueValue;
}
/// XML Schema patterns (as IDS and `property-value` write them) in `regex`
/// syntax, for a property selector's `matches` operator.
pub use xsd_pattern::translate as translate_xsd_pattern;

/// Registers all maintained built-in capabilities into a host registry, and
/// the [`Refiner`] that applies what rule instances declare about their
/// outcomes.
///
/// # Errors
///
/// Returns an error if the registry already contains a built-in capability ID.
#[allow(clippy::too_many_lines)]
pub fn register_builtins(registry: CapabilityRegistry) -> Result<CapabilityRegistry, EngineError> {
    registry
        .register(PropertyExists)
        .and_then(|registry| registry.register(PropertyRequired))
        .and_then(|registry| registry.register(PropertyDataType))
        .and_then(|registry| registry.register(PropertyValueConstraint))
        .and_then(|registry| registry.register(BooleanPropertyEquals))
        .and_then(|registry| registry.register(PropertyPredicate))
        .and_then(|registry| registry.register(ExpressionRequirement))
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
        .and_then(|registry| registry.register(CompareModels))
        .and_then(|registry| registry.register(CoordinateConsistencyCheck))
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
        .and_then(|registry| registry.register(EmptyHost))
        .and_then(|registry| registry.register(UnclassifiedObject))
        .and_then(|registry| registry.register(QuantityTakeoff))
        .and_then(|registry| registry.register_measured(distance::DistanceMeasures))
        .and_then(|registry| registry.register_measured(measured_openings::OpeningMeasures))
        .and_then(|registry| registry.register_measured(keyed_limit::DoorMeasures))
        .and_then(|registry| registry.register_measured(door_swing_direction::SwingMeasures))
        .and_then(|registry| registry.register_measured(opening_zone::PlacementMeasures))
        .and_then(|registry| registry.register_measured(allowed_profile::ProfileMeasures))
        .and_then(|registry| registry.register_measured(stair_geometry::StairMeasures))
        .and_then(|registry| registry.register_measured(stair_geometry::StairItems))
        .and_then(|registry| registry.register_measured(horizontal_guard::GuardMeasures))
        .and_then(|registry| registry.register_measured(measured_plan::PlanMeasures))
        .and_then(|registry| registry.register_measured(component_visibility::ViewMeasures))
        .and_then(|registry| registry.register_measured(axioval_engine::FacePieceMeasures))
        .and_then(|registry| registry.register_measured(escape_route::TravelMeasures))
        .and_then(|registry| registry.register_measured(free_floor::PlacementMeasures))
        .and_then(|registry| registry.register_measured(parking_bay::BayMeasures))
        .and_then(|registry| registry.register_measured(wall_spacing::SpacingMeasures))
        .and_then(|registry| registry.register_measured(counterpart_coverage::CoverageMeasures))
        .and_then(|registry| registry.register_measured(effective_coverage::EffectMeasures))
        .and_then(|registry| registry.register_measured(space_boundary_coverage::BoundaryMeasures))
        .and_then(|registry| registry.register_measured(triangle_count::TriangleMeasures))
        .and_then(|registry| registry.register_measured(body_extent::ExtentMeasures))
        .and_then(|registry| registry.register_measured(plan_area::AreaMeasures))
        .and_then(|registry| registry.register_measured(area_ratio::RatioMeasures))
        .and_then(|registry| registry.register_measured(plan_coverage::CoverageSearch))
        .and_then(|registry| registry.register_measured(light_area::LightMeasures))
        .and_then(|registry| registry.register_measured(level_spacing::LevelMeasures))
        .and_then(|registry| registry.register_measured(slab_stack::StackMeasures))
        .and_then(|registry| registry.register_measured(shelf_capacity::ShelfMeasures))
        .and_then(|registry| registry.register_measured(slab_contact::StoreyMeasures))
        .and_then(|registry| registry.register_measured(slab_contact::ContactMeasures))
        .and_then(|registry| registry.register_measured(measured_kinds::SelectionMeasures))
        .and_then(|registry| registry.register_measured(coordinate_consistency::CoordinateMeasures))
        .and_then(|registry| registry.register_measured(external_wall_validation::EnvelopeMeasures))
        .map(|registry| registry.with_refiner(Refiner))
}
