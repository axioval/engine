//! `property-value` as a template: the property a rule names, or every
//! property matching its name patterns, judged by the facet judge
//! `Decision::Facets`.

use axioval_engine::template::{Decision, FacetParameters, Form, Refusals, Template};
use axioval_engine::{ParameterDescriptor, ParameterType};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.property-value";

/// The parameters naming the property and stating its facets.
const FACETS: FacetParameters = FacetParameters {
    property: "property",
    property_set_pattern: "property_set_pattern",
    property_pattern: "property_pattern",
    data_type: "data_type",
    values: "values",
    patterns: "patterns",
    min_inclusive: "min_inclusive",
    max_inclusive: "max_inclusive",
    min_exclusive: "min_exclusive",
    max_exclusive: "max_exclusive",
    length: "length",
    min_length: "min_length",
    max_length: "max_length",
    total_digits: "total_digits",
    fraction_digits: "fraction_digits",
    optional: "optional",
    precision: "precision",
    quantifier: "quantifier",
    si_units: "si_units",
    target_refusal: "property-value: ",
    constraint_refusal: "property-value parameters are invalid: ",
};

/// `property-value`'s parameters, in the capability's order.
fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![ParameterDescriptor::optional(
        "property",
        ParameterType::PropertyReference,
    )];
    for name in [
        "property_set_pattern",
        "property_pattern",
        "data_type",
        "min_inclusive",
        "max_inclusive",
        "min_exclusive",
        "max_exclusive",
        "precision",
        "quantifier",
    ] {
        parameters.push(ParameterDescriptor::optional(name, ParameterType::String));
    }
    for name in ["values", "patterns"] {
        parameters.push(ParameterDescriptor::optional(
            name,
            ParameterType::StringList,
        ));
    }
    for name in [
        "length",
        "min_length",
        "max_length",
        "total_digits",
        "fraction_digits",
    ] {
        parameters.push(ParameterDescriptor::optional(name, ParameterType::Integer));
    }
    for name in ["optional", "si_units"] {
        parameters.push(ParameterDescriptor::optional(name, ParameterType::Boolean));
    }
    parameters
}

/// `property-value`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "property-value",
        refusals: Refusals::Worded,
        defaults: Vec::new(),
        declaration: Vec::new(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            grading: None,
            unless: Vec::new(),
            when: &[],
            values: Vec::new(),
            decision: Decision::Facets(Box::new(FACETS)),
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            once: Vec::new(),
        }],
    }
}
