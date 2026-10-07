//! `component-visibility` as a template: the measured view from each
//! component's eye (`sight_view`), its targets in view counted from those
//! surely in view to every one that may be, judged by the mode: at least
//! `minimum`, or none.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Condition, Decision, Form, FormCheck, ItemCheck,
    ItemTest, ItemText, ItemUnit, Items, Judge, OnNull, Operand, ParameterDefault, Range, Refusals,
    Requirement, Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.component-visibility";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("targets", ParameterType::Selector),
        ParameterDescriptor::required("blockers", ParameterType::Selector),
        ParameterDescriptor::required("eye_height", ParameterType::Quantity),
        ParameterDescriptor::required("radius", ParameterType::Quantity),
        ParameterDescriptor::required("mode", ParameterType::String),
        ParameterDescriptor::optional("minimum", ParameterType::Integer),
    ]
}

const VIEW: &str =
    "sight_view;targets=@targets;blockers=@blockers;eye_height=@eye_height;radius=@radius";

/// The targets in view within bounds, under the mode `mode`.
fn counted(
    mode: &'static str,
    (at_least, at_most): (Vec<Requirement>, Vec<Requirement>),
    (fail, undecided): (&'static str, &'static str),
    related: &'static str,
) -> ItemCheck {
    ItemCheck::Test(Box::new(ItemTest {
        applies: Applies::default(),
        when: vec![When::Equals {
            parameter: "mode",
            value: mode,
        }],
        judge: Judge::Range(Box::new(Range {
            value: "visible",
            unit: ItemUnit::Count,
            at_least,
            at_most,
            allowance: Allowance::None,
            grade: false,
            null: OnNull::Skip,
            unmeasured: None,
        })),
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some(related),
    }))
}

fn view() -> FormCheck {
    let minimum = Requirement {
        name: "minimum",
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Parameter("minimum")),
        }],
        words: "",
    };
    let none = Requirement {
        name: "none",
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Literal(0.0),
        }],
        words: "",
    };
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: VIEW,
            refused: Some("{why}"),
            checks: vec![
                counted(
                    "at-least",
                    (vec![minimum], Vec::new()),
                    (
                        "{sure:count} target(s) {within} are in view; required at least \
                         {minimum:count}{hidden_note}",
                        "{sure:count} target(s) are in view, {minimum:count} required; \
                         {undecided}",
                    ),
                    "found",
                ),
                counted(
                    "none",
                    (Vec::new(), vec![none]),
                    (
                        "{sure:count} target(s) {within} are in view, none allowed: {seen}",
                        "{undecided}",
                    ),
                    "seen",
                ),
            ],
            together: None,
            passing: None,
            texts: vec![
                ItemText {
                    name: "hidden_note",
                    when: vec![When::Is {
                        field: "hidden",
                        value: 0.0,
                    }],
                    text: "",
                },
                ItemText {
                    name: "hidden_note",
                    when: Vec::new(),
                    text: "; {hidden:count} hidden",
                },
            ],
            merged: false,
            at: None,
            once: false,
            combined: None,
            reason: None,
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

/// `component-visibility`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: super::NAME,
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "minimum",
            from: &[],
            value: ScalarValue::Integer { value: 1 },
        }],
        declaration: vec![
            Check::Kind {
                parameter: "minimum",
            },
            Check::Choice {
                parameter: "mode",
                options: &["at-least", "none"],
            },
            Check::RequiresValue {
                parameter: "minimum",
                with: "mode",
                value: "at-least",
                message: "minimum applies only to mode `at-least`",
            },
            Check::Holds {
                condition: Condition::AtLeast {
                    parameter: "minimum",
                    than: 1.0,
                },
                message: "minimum must be a positive count",
            },
            Check::NonNegativeLength {
                parameter: "eye_height",
                message: "eye_height must be a non-negative length",
            },
            Check::NonNegativeLength {
                parameter: "radius",
                message: "radius must be a non-negative length",
            },
            Check::Required {
                parameter: "targets",
            },
            Check::Required {
                parameter: "blockers",
            },
        ],
        // The view reports a missing service for each component, as the
        // capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the view is judged".into()),
                },
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Within {
                value: "judged",
                minimum: None,
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: Vec::new(),
            related: None,
            checks: vec![view()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
