//! `group-composition` as a template: what the maximum matching of each
//! group's members to the requirement rows found (`compositions`, a list
//! of the project), judged where each outcome goes. Entries every maximum
//! matching leaves short hold fewer members than their places, members it
//! leaves without a place outnumber the places they compete for, a group no
//! row matches, a row no group matches and an object no group reaches are
//! findings of their own, and what the matching cannot decide is open.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, ItemCheck, ItemTest,
    ItemText, ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Template,
    TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.group-composition";

const COMPOSITIONS: &str = "compositions;requirements=@requirements;key_1=@key_1;key_2=@key_2;\
                            key_3=@key_3;case_sensitive=@case_sensitive;group_key=@group_key;\
                            group_key_1=@group_key_1;group_key_2=@group_key_2;\
                            group_key_3=@group_key_3;report_absent_groups=@report_absent_groups;\
                            member_selector=@member_selector;\
                            ungrouped_selector=@ungrouped_selector;relationship=@relationship;\
                            direction=@direction;follow_chain=@follow_chain;path=@path;\
                            skip_absent_relationship_ends=@skip_absent_relationship_ends;\
                            selection=@selection";

fn places() -> Vec<Requirement> {
    vec![Requirement {
        name: "places",
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Value("places")),
        }],
        words: "",
    }]
}

fn test(
    when: Vec<When>,
    judge: Judge,
    (fail, undecided): (&'static str, &'static str),
) -> ItemCheck {
    ItemCheck::Test(Box::new(ItemTest {
        applies: Applies::default(),
        when,
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }))
}

fn truth(value: &'static str, fail: &'static str) -> ItemCheck {
    test(
        vec![When::Stated { field: value }],
        Judge::Truth {
            value,
            finding: true,
        },
        (fail, "{why}"),
    )
}

fn counted(
    value: &'static str,
    (at_least, at_most): (Vec<Requirement>, Vec<Requirement>),
) -> Judge {
    Judge::Range(Box::new(Range {
        value,
        unit: ItemUnit::Count,
        at_least,
        at_most,
        allowance: Allowance::None,
        grade: false,
        null: OnNull::Skip,
        unmeasured: None,
    }))
}

fn checks() -> Vec<ItemCheck> {
    vec![
        // A group, member or object left open, for its reason.
        truth("open", ""),
        truth("unmatched", "no requirement row matches the group ({keys})"),
        truth("absent", "not in model: no group matches {name}"),
        truth("ungrouped", "in no group {via}"),
        // Entries holding fewer members than their places.
        test(
            vec![When::Stated { field: "filled" }],
            counted("filled", (places(), Vec::new())),
            (
                "{subject} {filled:count} of {places:count} required member(s) {via}; \
                 {missing:count} missing",
                "",
            ),
        ),
        // Members outnumbering the places they compete for.
        test(
            vec![When::Stated { field: "found" }],
            counted("found", (Vec::new(), places())),
            ("{surplus_words}", ""),
        ),
    ]
}

fn compositions() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: COMPOSITIONS,
            refused: Some("group-composition: {why}"),
            checks: checks(),
            together: None,
            passing: None,
            texts: vec![
                ItemText {
                    name: "surplus_words",
                    when: vec![When::Field {
                        field: "unfit",
                        value: true,
                    }],
                    text: "surplus member {via}: no entry fits it ({keys})",
                },
                ItemText {
                    name: "surplus_words",
                    when: Vec::new(),
                    text: "{entries} {verb} {places:count} member(s), but {found:count} fit \
                           {via}; {surplus:count} surplus",
                },
            ],
            merged: false,
            at: Some("at"),
            once: false,
            combined: None,
            reason: Some("reason"),
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

/// `group-composition`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: super::parameters(),
        grades: false,
        name: "group-composition",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: COMPOSITIONS,
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the groups' members are matched".into()),
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
            checks: Vec::new(),
            once: Vec::new(),
            joined: None,
            project: vec![compositions()],
        }],
    }
}
