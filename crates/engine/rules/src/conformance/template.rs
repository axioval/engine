//! `selector-conformance` as a template: each selected object judged by
//! the `requirement` selector, the objects it rejects grouped by the values
//! the selector consults, by the group decision `Decision::Conforms`.

use axioval_engine::template::{
    Check, Conformance, Decision, Form, ParameterDefault, Refusals, Template,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::ScalarValue;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.selector-conformance";

/// `selector-conformance`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("requirement", ParameterType::Selector),
            ParameterDescriptor::optional("message", ParameterType::String),
        ],
        grades: false,
        name: "selector-conformance",
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "message",
            value: ScalarValue::String {
                value: "does not match any agreed combination of values".into(),
            },
            from: &[],
        }],
        declaration: vec![
            Check::Required {
                parameter: "requirement",
            },
            Check::Kind {
                parameter: "message",
            },
        ],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            grading: None,
            unless: Vec::new(),
            when: &[],
            values: Vec::new(),
            decision: Decision::Conforms(Conformance {
                requirement: "requirement",
                alone: "{message}",
                no_value: "{properties} has no value to compare with the agreed list",
                unknown: "{message}: {values}",
            }),
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
        }],
    }
}
