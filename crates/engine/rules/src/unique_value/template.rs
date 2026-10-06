//! `unique-value` as a template: the stated `property` of each selected
//! object compared with those of the other objects of its group (its
//! source, the project with `across_sources`, narrowed by the rule's
//! traversal) by the group decision `Decision::Unique`.

use axioval_engine::template::{
    Check, Decision, Form, ParameterDefault, Template, TemplateValue, Unique,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};
use serde_json::json;

use crate::support::{tolerance_parameters, traversal_parameters};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.unique-value";

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Required {
            parameter: "property",
        },
        Check::Kind { parameter: "trim" },
        Check::Kind {
            parameter: "case_sensitive",
        },
        Check::Kind {
            parameter: "require_value",
        },
        Check::Kind {
            parameter: "across_sources",
        },
        Check::Traversal {
            with: &[],
            message: "",
        },
        Check::Tolerance,
    ]
}

/// A boolean parameter's default.
fn default(parameter: &'static str, value: bool) -> ParameterDefault {
    ParameterDefault {
        parameter,
        value: ScalarValue::Boolean { value },
        from: &[],
    }
}

/// `unique-value`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    let value: Expression = serde_json::from_value(json!({
        "kind": "property",
        "propertySet": "{property.set}",
        "property": "{property.name}",
    }))
    .expect("a built-in template's expression is well formed");
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("trim", ParameterType::Boolean),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("require_value", ParameterType::Boolean),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .chain(tolerance_parameters())
        .collect(),
        grades: false,
        name: "unique-value",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: vec![
            default("trim", true),
            default("case_sensitive", false),
            default("require_value", true),
        ],
        declaration: declaration(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "value",
                expression: value,
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Unique {
                value: "value",
                unique: Unique {
                    across: "across_sources",
                    trim: "trim",
                    case_sensitive: "case_sensitive",
                    require: "require_value",
                    missing: "{property} has no value",
                },
            },
            fail: "{property} {value:stated} is also used by {others} other object(s)\
                   {tolerance:suffix}",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
