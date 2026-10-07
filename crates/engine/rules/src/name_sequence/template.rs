//! `name-sequence` as a template: the measured `name_sequence` of each
//! anchor, its members in order, each judged on itself: its number stated,
//! whole, not below `first`, and the number the sequence expects.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, ItemCheck, ItemTest,
    ItemText, ItemUnit, Items, Judge, OnNull, Operand, ParameterDefault, Range, Refusals,
    Requirement, Template, TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.name-sequence";

const LIST: &str = "name_sequence;member_selector=@member_selector;name=@name;order=@order;\
                    first=@first;increment=@increment;order_fallback=@order_fallback;\
                    relationship=@relationship;direction=@direction;path=@path;\
                    follow_chain=@follow_chain;\
                    skip_absent_relationship_ends=@skip_absent_relationship_ends";

fn test(judge: Judge, fail: &'static str, then: Option<ItemTest>) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge,
        fail,
        undecided: "",
        effects: Vec::new(),
        then: then.map(Box::new),
        otherwise: None,
        straddled: None,
        related: None,
    }
}

fn bound(name: &'static str, operand: Operand) -> Requirement {
    Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(operand),
        }],
        words: "",
    }
}

fn range(at_least: Vec<Requirement>, at_most: Vec<Requirement>) -> Judge {
    Judge::Range(Box::new(Range {
        value: "value",
        unit: ItemUnit::Count,
        at_least,
        at_most,
        allowance: Allowance::None,
        grade: false,
        null: OnNull::Skip,
        unmeasured: None,
    }))
}

/// Each member: its number stated, then whole, then not below the start,
/// then the one the sequence expects (relating the member below it).
fn sequence() -> ItemCheck {
    let mut expected = test(
        range(
            vec![bound("expected", Operand::Value("expected"))],
            vec![bound("expected", Operand::Value("expected"))],
        ),
        "{broken}",
        None,
    );
    expected.related = Some("below");
    let start = test(
        range(
            vec![bound("first", Operand::Parameter("first"))],
            Vec::new(),
        ),
        "{name} {value:count} is below the start {first:count}",
        Some(expected),
    );
    let whole = test(
        Judge::Truth {
            value: "whole",
            finding: false,
        },
        "{name} {shown} is not a whole number",
        Some(start),
    );
    ItemCheck::Test(Box::new(test(
        Judge::Truth {
            value: "set",
            finding: false,
        },
        "{name} is not set",
        Some(whole),
    )))
}

fn members() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: LIST,
            refused: Some("{why}"),
            checks: vec![sequence()],
            together: None,
            passing: None,
            texts: vec![
                ItemText {
                    name: "broken",
                    when: vec![When::Null { field: "previous" }],
                    text: "{name} of the first member is {value:count}; expected {first:count}",
                },
                ItemText {
                    name: "broken",
                    when: vec![When::Field {
                        field: "above",
                        value: false,
                    }],
                    text: "{name} {value:count} is not above {previous:count}, the member below it",
                },
                ItemText {
                    name: "broken",
                    when: Vec::new(),
                    text: "{name} {value:count} does not follow {previous:count}; expected \
                           {expected:count}",
                },
            ],
            merged: false,
            once: false,
            combined: None,
            reason: None,
            at: Some("member"),
            joined: None,
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

/// `name-sequence`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: super::NAME,
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "first",
            from: &[],
            value: ScalarValue::Integer { value: 1 },
        }],
        // The capability's declaration, in its order and words.
        declaration: vec![Check::Arguments {
            when: &[],
            value: LIST,
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the members are judged".into()),
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
            checks: vec![members()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    super::parameters()
}
