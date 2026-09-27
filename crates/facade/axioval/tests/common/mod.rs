//! Helpers shared by the facade's end-to-end tests.

use axioval::engine::ParameterType;

/// The package spelling of a capability parameter's type, for definitions
/// built from the registry's signature.
pub fn kind(parameter_type: ParameterType) -> &'static str {
    parameter_type.package_kind()
}
