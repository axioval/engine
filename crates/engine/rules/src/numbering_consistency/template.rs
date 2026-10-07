//! `numbering-consistency` as a template: the measured `numbering` of each
//! selected object, its number read, its prefix's lead over the other
//! prefixes of its scope at least one, and its number at most one above the
//! next lower number of its scope.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Effect, Form, FormCheck, ItemCheck,
    ItemTest, ItemUnit, Items, Judge, On, OnNull, Range, Refusals, Requirement, Template,
    TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.numbering-consistency";

const LIST: &str = "numbering;property=@property;pattern=@pattern;\
                    prefix_length=@prefix_length;gap_free=@gap_free;\
                    across_sources=@across_sources;relationship=@relationship;\
                    direction=@direction;path=@path;follow_chain=@follow_chain;\
                    skip_absent_relationship_ends=@skip_absent_relationship_ends;\
                    selection=@selection";

fn literal(name: &'static str, value: f64) -> Requirement {
    Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Literal(value),
        }],
        words: "",
    }
}

fn test(
    kind: &'static str,
    value: &'static str,
    (at_least, at_most): (Vec<Requirement>, Vec<Requirement>),
    fail: &'static str,
    related: Option<&'static str>,
) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: vec![When::Field {
            field: kind,
            value: true,
        }],
        judge: Judge::Range(Box::new(Range {
            value,
            unit: ItemUnit::Count,
            at_least,
            at_most,
            allowance: Allowance::None,
            grade: false,
            null: OnNull::Skip,
            unmeasured: None,
        })),
        fail,
        undecided: "",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related,
    }
}

fn checks() -> Vec<ItemCheck> {
    // A number that cannot be read: open for its reason.
    let unread = test("unread", "number", (Vec::new(), Vec::new()), "", None);
    // The prefix most objects of the scope share, leading every other.
    let prefix = test(
        "prefixed",
        "lead",
        (vec![literal("lead", 1.0)], Vec::new()),
        "{departs}",
        Some("related"),
    );
    // The number at most one above the next lower one.
    let mut gap = test(
        "stepped",
        "step",
        (Vec::new(), vec![literal("step", 1.0)]),
        "{property} {shown} follows {below}; {missing}",
        Some("related"),
    );
    gap.effects = vec![Effect {
        when: vec![When::Field {
            field: "fillable",
            value: true,
        }],
        on: On::Fail,
        message: "{property} {shown} follows {below}, but an object whose number could not \
                  be read may fill the gap",
    }];
    vec![unread, prefix, gap]
        .into_iter()
        .map(|test| ItemCheck::Test(Box::new(test)))
        .collect()
}

fn numbers() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: LIST,
            refused: Some("{why}"),
            checks: checks(),
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            once: false,
            combined: None,
            reason: Some("reason"),
            at: None,
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

/// `numbering-consistency`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: super::NAME,
        refusals: Refusals::Rule,
        defaults: Vec::new(),
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
                    label: Some("the numbers are judged".into()),
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
            checks: vec![numbers()],
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
