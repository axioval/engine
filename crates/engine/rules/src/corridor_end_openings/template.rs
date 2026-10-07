//! `corridor-end-openings` as a template: the openings a corridor reaches,
//! searched against its end walls (`corridor_end_openings`), each judged on
//! the opening by whether it sits in one and whether it is selected.

use axioval_engine::template::{
    Applies, Check, Decision, Effect, Form, FormCheck, ItemCheck, ItemTest, Items, Judge, On,
    Refusals, Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use serde_json::json;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.corridor-end-openings";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("opening_path", ParameterType::StringList),
        ParameterDescriptor::required("opening_selector", ParameterType::Selector),
        ParameterDescriptor::optional("wall_depth", ParameterType::Number),
        ParameterDescriptor::optional("facing", ParameterType::Number),
    ]
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Required {
            parameter: "opening_path",
        },
        Check::Path {
            parameter: "opening_path",
        },
        Check::Required {
            parameter: "opening_selector",
        },
        Check::NonNegative {
            parameters: &["wall_depth"],
            message: "`wall_depth` must be a non-negative length in metres",
        },
        Check::NonNegative {
            parameters: &["facing"],
            message: "`facing` must be a non-negative length in metres",
        },
    ]
}

fn test(when: Vec<When>, judge: Judge, fail: &'static str, undecided: &'static str) -> ItemTest {
    ItemTest {
        applies: Applies {
            when: &[],
            any: &[],
            condition: None,
        },
        when,
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: None,
    }
}

/// Each opening sitting in an end wall: a finding on the opening where it
/// is selected, open where the selection or the end walls cannot decide
/// it; nothing where neither is decided.
fn openings() -> FormCheck {
    let mut selected = test(
        vec![When::Field {
            field: "picked",
            value: true,
        }],
        Judge::Truth {
            value: "sits",
            finding: true,
        },
        "sits in the end wall of corridor {corridor}: {walls}",
        "whether it sits in an end wall of corridor {corridor} is undecided: {why}",
    );
    selected.related = Some("corridor");
    let mut possible = test(
        vec![
            When::Unknown { field: "picked" },
            When::Field {
                field: "sits",
                value: true,
            },
        ],
        Judge::Fails,
        "",
        "",
    );
    possible.effects = vec![Effect {
        when: Vec::new(),
        on: On::Fail,
        message: "sits in the end wall of corridor {corridor}, but whether it is selected is \
                  undecided: {unpicked}",
    }];
    FormCheck {
        values: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies {
                when: &[],
                any: &[],
                condition: None,
            },
            list: "corridor_end_openings;path=@opening_path;openings=@opening_selector;\
                   depth=@wall_depth;facing=@facing",
            refused: Some("{why}"),
            checks: vec![
                ItemCheck::Test(Box::new(selected)),
                ItemCheck::Test(Box::new(possible)),
            ],
            together: None,
            passing: None,
            texts: Vec::new(),
            once: false,
            at: Some("opening"),
            merged: false,
            combined: None,
            reason: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        derived: Vec::new(),
        quiet: false,
        unless: None,
        ungraded: false,
    }
}

/// `corridor-end-openings`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "corridor-end-openings",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The corridor is judged by its openings.
            values: vec![TemplateValue {
                name: "corridor",
                expression: serde_json::from_value(
                    json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
                )
                .expect("a literal"),
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Holds { value: "corridor" },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: vec![openings()],
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}
