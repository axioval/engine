//! `consistent-value` as a template: the stated `key` and `value` of each
//! selected object, the objects sharing a key within their group (their
//! source, the project with `across_sources`, of one kind unless
//! `same_kind` is false, narrowed by the rule's traversal) required to share
//! their value by the group decision `Decision::Consistent`.

use axioval_engine::template::{
    Check, Consistent, ConsistentMessages, Decision, Form, ParameterDefault, Template,
    TemplateValue,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};
use serde_json::json;

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.consistent-value";

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Kind {
            parameter: "tolerance",
        },
        Check::Kind {
            parameter: "tolerance_quantity",
        },
        Check::Exclusive {
            one: &["tolerance"],
            other: &["tolerance_quantity"],
            message: "declare either `tolerance` or `tolerance_quantity`, not both",
        },
        Check::NonNegative {
            parameters: &["tolerance", "tolerance_quantity"],
            message: "the tolerance is negative",
        },
        Check::Required { parameter: "key" },
        Check::Required { parameter: "value" },
        Check::Kind {
            parameter: "case_sensitive",
        },
        Check::Kind {
            parameter: "same_kind",
        },
        Check::Kind {
            parameter: "across_sources",
        },
        Check::Traversal {
            with: &[],
            message: "",
        },
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

/// A stated property the rule names by the reference parameter `name`.
fn stated(name: &'static str) -> TemplateValue {
    let expression: Expression = serde_json::from_value(json!({
        "kind": "property",
        "propertySet": format!("{{{name}.set}}"),
        "property": format!("{{{name}.name}}"),
    }))
    .expect("a built-in template's expression is well formed");
    TemplateValue {
        name,
        expression,
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
}

/// `consistent-value`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("key", ParameterType::PropertyReference),
            ParameterDescriptor::required("value", ParameterType::PropertyReference),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("same_kind", ParameterType::Boolean),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
            ParameterDescriptor::optional("tolerance", ParameterType::Number),
            ParameterDescriptor::optional("tolerance_quantity", ParameterType::Quantity),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect(),
        grades: false,
        name: "consistent-value",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: vec![default("case_sensitive", false), default("same_kind", true)],
        declaration: declaration(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            grading: None,
            unless: Vec::new(),
            when: &[],
            values: vec![stated("key"), stated("value")],
            decision: Decision::Consistent {
                key: "key",
                value: "value",
                consistent: Consistent {
                    across: "across_sources",
                    case_sensitive: "case_sensitive",
                    same_kind: "same_kind",
                    tolerance: "tolerance",
                    tolerance_quantity: "tolerance_quantity",
                    messages: ConsistentMessages {
                        differs: "{value} is {value:stated} where other objects with {key} \
                                  {key:stated} have {others}",
                        differs_unkeyed: "{key} has no value, and {value} is {value:stated} \
                                          where other objects without {key} have {others}",
                        keyed: "objects with {key} {key:stated}",
                        unkeyed: "objects without {key}",
                        straddles: "consistent-value: the range of {value} over the {objects} \
                                    may lie on either side of the tolerance {spread}",
                        beyond: "{value} is {value:stated}, farther than the tolerance {spread} \
                                 from the median {median} of the {objects}",
                        at_end: "{value} is {value:stated}, at an end of the range {range} of \
                                 {value} over the {objects}, which exceeds the tolerance {spread}",
                        undecided: "consistent-value: {value} {value:stated} may lie within the \
                                    tolerance {spread} of the median {median} of the {objects} \
                                    or beyond it",
                        inapplicable: "consistent-value: the tolerance {spread} does not apply \
                                       to {value:stated}",
                        not_finite: "consistent-value: {value:stated} is not a finite value",
                        inexact: "consistent-value: integer cannot be represented exactly as a \
                                  decimal",
                    },
                },
            },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
