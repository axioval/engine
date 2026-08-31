//! Built-in trusted source-neutral rule capabilities.
#![forbid(unsafe_code)]

use axioval_engine::{CapabilityRegistry, EngineError};

mod free_floor_circle;
mod free_floor_rectangle;
mod property_comparison;
mod property_rules;
mod selection;

pub use free_floor_circle::FreeFloorCircle;
pub use free_floor_rectangle::FreeFloorRectangle;
pub use property_comparison::PropertyComparison;
pub use property_rules::{BooleanPropertyEquals, PropertyExists, PropertyPredicate};

/// Registers all maintained built-in capabilities into a host registry.
///
/// # Errors
///
/// Returns an error if the registry already contains a built-in capability ID.
pub fn register_builtins(registry: CapabilityRegistry) -> Result<CapabilityRegistry, EngineError> {
    registry
        .register(PropertyExists)
        .and_then(|registry| registry.register(BooleanPropertyEquals))
        .and_then(|registry| registry.register(PropertyPredicate))
        .and_then(|registry| registry.register(PropertyComparison))
        .and_then(|registry| registry.register(FreeFloorCircle))
        .and_then(|registry| registry.register(FreeFloorRectangle))
}
