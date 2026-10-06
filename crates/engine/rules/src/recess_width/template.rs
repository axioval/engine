//! `recess-width` as a template: each item of the measured `recesses`
//! list, handed the rule's `requirements`, judged by its width against the
//! width its row requires; a recess no row holds passes, one whose row is
//! undecided is open.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, Group, Guard, ItemCheck,
    ItemTest, ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Service,
    Services, Template, TemplateValue,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

use super::COLUMNS;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.recess-width";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![ParameterDescriptor::required(
        "requirements",
        ParameterType::Table(COLUMNS),
    )]
}

/// The recesses, each with its row and required width.
const RECESSES: &str = "recesses;requirements=@requirements";

/// How a recess is described, as the capability described it.
macro_rules! described {
    ($rest:literal) => {
        concat!(
            "{place} is {width:length} wide and {depth:length} deep; ",
            $rest
        )
    };
}

/// Each recess's width at least the width its row requires.
fn recesses() -> FormCheck {
    let judged = ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Range(Box::new(Range {
            value: "width",
            unit: ItemUnit::Length,
            at_least: vec![Requirement {
                name: "required",
                options: vec![Choice {
                    when: Vec::new(),
                    bound: Bound::Operand(Operand::Value("required")),
                }],
                words: "",
            }],
            at_most: Vec::new(),
            allowance: Allowance::None,
            grade: false,
            null: OnNull::Judge,
            unmeasured: None,
        })),
        fail: described!("row {row:count} requires at least {required}"),
        undecided: described!("row {row:count} requires at least {required}, undecided"),
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: None,
    };
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: RECESSES,
            refused: Some("{why}"),
            checks: vec![ItemCheck::Group(Box::new(Group {
                applies: Applies::default(),
                when: Vec::new(),
                // A recess no row holds has no requirement; one whose row
                // is undecided is open.
                guards: vec![Guard {
                    field: "row",
                    when: Vec::new(),
                    undecided: Some(described!("which row applies is undecided")),
                    null: OnNull::Skip,
                }],
                checks: vec![ItemCheck::Test(Box::new(judged))],
            }))],
            together: None,
            passing: None,
            texts: Vec::new(),
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: None,
        unless: None,
        quiet: false,
    }
}

/// `recess-width`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "recess-width",
        refusals: Refusals::ServicesPerObject,
        defaults: Vec::new(),
        declaration: vec![
            Check::Required {
                parameter: "requirements",
            },
            Check::Arguments {
                when: &["requirements"],
                value: RECESSES,
            },
        ],
        services: Some(Services {
            needs: vec![Service::PlanSpan],
            message: "plan-span service is not registered",
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // Nothing to read but the recesses: the check judges them.
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the recesses are judged".into()),
                },
                expect: None,
                absent: None,
                mismatch: None,
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
            checks: vec![recesses()],
            once: Vec::new(),
        }],
    }
}
