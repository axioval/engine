//! `property-requirements` as a template: every applicable row of a
//! requirements table per object, judged by the requirements-table judge
//! `Decision::Requirements`.

use axioval_engine::template::{Decision, Form, Refusals, RequirementParameters, Template};
use axioval_engine::{ParameterDescriptor, ParameterType};

use super::COLUMNS;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.property-requirements";

/// The parameters stating the table and how it is judged.
const REQUIREMENTS: RequirementParameters = RequirementParameters {
    requirements: "requirements",
    case_sensitive: "case_sensitive",
    area_property: "area_property",
    volume_property: "volume_property",
    group_by_value: "group_by_value",
    category_property: "category_property",
};

/// `property-requirements`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("requirements", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("area_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("volume_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("group_by_value", ParameterType::Boolean),
            ParameterDescriptor::optional("category_property", ParameterType::PropertyReference),
        ],
        grades: false,
        name: "property-requirements",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: Vec::new(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            grading: None,
            unless: Vec::new(),
            when: &[],
            values: Vec::new(),
            decision: Decision::Requirements(Box::new(REQUIREMENTS)),
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
