//! Helpers shared by the facade's end-to-end tests.

use axioval::engine::ParameterType;

/// The package spelling of a capability parameter's type, for definitions
/// built from the registry's signature.
pub fn kind(parameter_type: ParameterType) -> &'static str {
    match parameter_type {
        ParameterType::Boolean => "boolean",
        ParameterType::Integer => "integer",
        ParameterType::Number => "number",
        ParameterType::String => "string",
        ParameterType::Quantity => "quantity",
        ParameterType::Enum => "enum",
        ParameterType::Date => "date",
        ParameterType::DateTime => "dateTime",
        ParameterType::Reference => "reference",
        ParameterType::ObjectTypeReference => "objectTypeReference",
        ParameterType::PropertyReference => "propertyReference",
        ParameterType::Selector => "selector",
        ParameterType::StringList => "stringList",
        ParameterType::ReferenceList => "referenceList",
        ParameterType::Table(_) => "table",
    }
}
