//! Built-in trusted source-neutral rule capabilities.
#![forbid(unsafe_code)]

use axioval_engine::{CapabilityRegistry, EngineError};

mod external_wall_validation;
mod free_floor_circle;
mod free_floor_rectangle;
mod guard_diagnosis;
mod horizontal_guard;
mod property_comparison;
mod property_rules;
mod selection;
mod shelf_capacity;
mod slab_contact;
mod space_validation;

pub use external_wall_validation::ExternalWallValidation;
pub use free_floor_circle::FreeFloorCircle;
pub use free_floor_rectangle::FreeFloorRectangle;
pub use guard_diagnosis::{GuardDefect, GuardDiagnosis};
pub use horizontal_guard::HorizontalGuard;
pub use property_comparison::PropertyComparison;
pub use property_rules::{
    BooleanPropertyEquals, PropertyExists, PropertyPredicate, PropertyRequired,
};
pub use shelf_capacity::ShelfCapacity;
pub use slab_contact::SlabContact;
pub use space_validation::SpaceValidation;

/// Registers all maintained built-in capabilities into a host registry.
///
/// # Errors
///
/// Returns an error if the registry already contains a built-in capability ID.
pub fn register_builtins(registry: CapabilityRegistry) -> Result<CapabilityRegistry, EngineError> {
    registry
        .register(PropertyExists)
        .and_then(|registry| registry.register(PropertyRequired))
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
}
