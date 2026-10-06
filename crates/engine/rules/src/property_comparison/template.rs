//! `property-comparison` as a template: the candidates each checked object
//! reaches compared with its target by the candidate comparison judge
//! `Decision::Compared`.

use axioval_engine::template::{ComparedParameters, Decision, Form, Refusals, Template};
use axioval_engine::{ParameterDescriptor, ParameterType};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.property-comparison";

/// The parameters stating the comparison.
const COMPARED: ComparedParameters = ComparedParameters {
    compared_selector: "compared_selector",
    compared_property: "compared_property",
    operator: "operator",
    factor: "factor",
    quantifier: "quantifier",
    component_mode: "component_mode",
    container_selector: "container_selector",
    container_relationship: "container_relationship",
    level_property: "level_property",
    case_sensitive: "case_sensitive",
    category_property: "category_property",
    target_property: "target_property",
    target_number: "target_number",
    target_quantity: "target_quantity",
    target_text: "target_text",
    target_texts: "target_texts",
    target_boolean: "target_boolean",
    target_date: "target_date",
    target_date_time: "target_date_time",
    minimum: "minimum",
    maximum: "maximum",
    refusal: "property-comparison parameters are invalid: ",
};

/// `property-comparison`'s parameters, in the capability's order.
fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("compared_selector", ParameterType::Selector),
        // Not read by `count`, which compares the number of candidates.
        ParameterDescriptor::optional("compared_property", ParameterType::PropertyReference),
        // Exactly one target: a property of the checked object, a
        // constant, a text list or a range.
        ParameterDescriptor::optional("target_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("target_number", ParameterType::Number),
        ParameterDescriptor::optional("target_quantity", ParameterType::Quantity),
        ParameterDescriptor::optional("target_text", ParameterType::String),
        ParameterDescriptor::optional("target_texts", ParameterType::StringList),
        ParameterDescriptor::optional("target_boolean", ParameterType::Boolean),
        ParameterDescriptor::optional("target_date", ParameterType::Date),
        ParameterDescriptor::optional("target_date_time", ParameterType::DateTime),
        ParameterDescriptor::optional("precision", ParameterType::String),
        ParameterDescriptor::optional("minimum_number", ParameterType::Number),
        ParameterDescriptor::optional("maximum_number", ParameterType::Number),
        ParameterDescriptor::optional("minimum_quantity", ParameterType::Quantity),
        ParameterDescriptor::optional("maximum_quantity", ParameterType::Quantity),
        ParameterDescriptor::optional("minimum_date", ParameterType::Date),
        ParameterDescriptor::optional("maximum_date", ParameterType::Date),
        ParameterDescriptor::optional("minimum_date_time", ParameterType::DateTime),
        ParameterDescriptor::optional("maximum_date_time", ParameterType::DateTime),
        ParameterDescriptor::optional("minimum_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("maximum_property", ParameterType::PropertyReference),
        ParameterDescriptor::required("operator", ParameterType::String),
        ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ParameterDescriptor::required("factor", ParameterType::Number),
        ParameterDescriptor::required("component_mode", ParameterType::String),
        ParameterDescriptor::optional("container_selector", ParameterType::Selector),
        ParameterDescriptor::optional("container_relationship", ParameterType::String),
        ParameterDescriptor::optional("level_property", ParameterType::PropertyReference),
        ParameterDescriptor::required("quantifier", ParameterType::String),
        ParameterDescriptor::optional("category_property", ParameterType::PropertyReference),
    ]
    .into_iter()
    .chain(crate::support::traversal_parameters())
    .chain(crate::support::tolerance_parameters())
    .collect()
}

/// `property-comparison`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "property-comparison",
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
            decision: Decision::Compared(Box::new(COMPARED)),
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
        }],
    }
}
