//! Built-in trusted source-neutral rule capabilities.
#![forbid(unsafe_code)]

use axioval_engine::{CapabilityRegistry, EngineError};

mod clash;
mod comparison;
mod conformance;
mod consistent_value;
mod counts;
mod distance;
mod external_wall_validation;
mod free_floor_circle;
mod free_floor_rectangle;
mod guard_diagnosis;
mod horizontal_guard;
mod manual_issue;
mod name_sequence;
mod pairs;
mod property_comparison;
mod property_predicate;
mod property_rules;
mod property_value;
mod selection;
mod shelf_capacity;
mod slab_contact;
mod space_validation;
mod support;
mod unique_value;
mod xsd_pattern;

pub use clash::Clash;
pub use comparison::{
    AmbiguousIdentity, ComparedObject, ComparedProperty, ComparisonError, ComparisonRequest,
    Difference, ModelComparison, ObjectChange, Side, Unresolved, compare_sessions,
};
pub use conformance::SelectorConformance;
pub use consistent_value::ConsistentValue;
pub use counts::{RelatedCount, RelativeCount};
pub use distance::Distance;
pub use external_wall_validation::ExternalWallValidation;
pub use free_floor_circle::FreeFloorCircle;
pub use free_floor_rectangle::FreeFloorRectangle;
pub use guard_diagnosis::{GuardDefect, GuardDiagnosis};
pub use horizontal_guard::HorizontalGuard;
pub use manual_issue::ManualIssue;
pub use name_sequence::NameSequence;
pub use property_comparison::PropertyComparison;
pub use property_predicate::PropertyPredicate;
pub use property_rules::{
    BooleanPropertyEquals, PropertyDataType, PropertyExists, PropertyRequired,
};
pub use property_value::PropertyValueConstraint;
pub use shelf_capacity::ShelfCapacity;
pub use slab_contact::SlabContact;
pub use space_validation::SpaceValidation;
pub use unique_value::UniqueValue;

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
        .and_then(|registry| registry.register(ShelfCapacity))
        .and_then(|registry| registry.register(SlabContact))
        .and_then(|registry| registry.register(ExternalWallValidation))
        .and_then(|registry| registry.register(SpaceValidation))
        .and_then(|registry| registry.register(HorizontalGuard))
        .and_then(|registry| registry.register(FreeFloorCircle))
        .and_then(|registry| registry.register(FreeFloorRectangle))
        .and_then(|registry| registry.register(Clash))
        .and_then(|registry| registry.register(Distance))
        .and_then(|registry| registry.register(SelectorConformance))
        .and_then(|registry| registry.register(UniqueValue))
        .and_then(|registry| registry.register(ConsistentValue))
        .and_then(|registry| registry.register(RelatedCount))
        .and_then(|registry| registry.register(RelativeCount))
        .and_then(|registry| registry.register(NameSequence))
        .and_then(|registry| registry.register(ManualIssue))
}
