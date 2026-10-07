//! `space-connection` as a template: each requirement of each row that
//! applies to a space, as the measured list `space_connections` reads it,
//! judged one by one by whether the space is surely linked as the row
//! asks.

use axioval_engine::template::{
    Applies, Check, Decision, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, Refusals,
    Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use serde_json::json;

use super::COLUMNS;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.space-connection";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("connections", ParameterType::Table(COLUMNS)),
        ParameterDescriptor::required("access_path", ParameterType::StringList),
        ParameterDescriptor::optional("door_selector", ParameterType::Selector),
        ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
        ParameterDescriptor::optional("space_selector", ParameterType::Selector),
    ]
}

/// Each requirement of the rows that apply to the space, with whether the
/// space is surely linked as it asks.
const LIST: &str = "space_connections;connections=@connections;access_path=@access_path;\
                    door_selector=@door_selector;opening_selector=@opening_selector;\
                    space_selector=@space_selector";

/// The test of one kind of requirement: `access` or an `exit`, required or
/// forbidden. A required link missing, or a forbidden one present, is a
/// finding; an undecided one leaves the row open.
fn test(access: bool, required: bool, fail: &'static str) -> ItemCheck {
    ItemCheck::Test(Box::new(ItemTest {
        applies: Applies::default(),
        when: vec![
            When::Field {
                field: "access",
                value: access,
            },
            When::Field {
                field: "required",
                value: required,
            },
        ],
        judge: Judge::Truth {
            value: "linked",
            finding: !required,
        },
        fail,
        undecided: "space-connection {row}: {why}",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }))
}

/// The rows' requirements of the space, each its own outcome.
fn connections() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: LIST,
            // A space whose rows cannot be told is open once.
            refused: Some("space-connection: {why}"),
            checks: vec![
                test(
                    true,
                    true,
                    "has no direct access through a {kind} to a space {row} requires (via {via})",
                ),
                test(
                    true,
                    false,
                    "has direct access to {links}, which {row} forbids for a {kind}",
                ),
                test(
                    false,
                    true,
                    "has no {kind} directly to the outside, which {row} requires",
                ),
                test(
                    false,
                    false,
                    "opens directly to the outside through {links}, which {row} forbids for a \
                     {kind}",
                ),
            ],
            together: None,
            passing: None,
            texts: Vec::new(),
            once: false,
            at: None,
            merged: false,
            combined: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// `space-connection`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "space-connection",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        // The access path and selectors, then the rows, as the measurement
        // reads them: in the capability's order and words.
        declaration: vec![Check::Arguments {
            when: &[],
            value: LIST,
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The space is judged by its check.
            values: vec![TemplateValue {
                name: "space",
                expression: serde_json::from_value(
                    json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
                )
                .expect("a literal"),
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Holds { value: "space" },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: vec![connections()],
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}
