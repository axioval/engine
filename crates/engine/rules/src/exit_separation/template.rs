//! `exit-separation` as a template: the measured `exit_separation` of each
//! space, its exits counted against `minimum_exits` and the separation of
//! their pairs (the greatest for `pairs` `any`, the least for `all`)
//! against the share of the longest plan diagonal required, an interval
//! over both shares where the flag is unknown.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Effect, Form, FormCheck, ItemCheck,
    ItemTest, ItemUnit, Items, Judge, On, OnNull, Operand, Range, Refusals, Requirement, Template,
    TemplateValue, When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.exit-separation";

const LIST: &str = "exit_separation;exit_path=@exit_path;exit_selector=@exit_selector;\
                    fraction=@fraction;flag=@flag;flag_path=@flag_path;\
                    flagged_fraction=@flagged_fraction;flag_sources=@flag_sources;\
                    flag_default=@flag_default;separation=@separation;pairs=@pairs;\
                    minimum_exits=@minimum_exits";

/// Where the separation is open: why the pairs are, and the requirement.
const OPEN: &str = "{open} ({requirement})";

fn requirement(name: &'static str, operand: Operand) -> Requirement {
    Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(operand),
        }],
        words: "",
    }
}

fn test(
    when: Vec<When>,
    judge: Judge,
    (fail, undecided): (&'static str, &'static str),
) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
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

fn range(value: &'static str, unit: ItemUnit, at_least: Requirement, null: OnNull) -> Judge {
    Judge::Range(Box::new(Range {
        value,
        unit,
        at_least: vec![at_least],
        at_most: Vec::new(),
        allowance: Allowance::None,
        grade: false,
        null,
        unmeasured: None,
    }))
}

/// A space surely short of exits, whose finding stands: what the
/// separation leaves open passes instead.
fn standing() -> Judge {
    Judge::Truth {
        value: "short",
        finding: false,
    }
}

/// The exits counted against `minimum_exits`.
fn counted() -> ItemCheck {
    let mut counted = test(
        vec![When::Field {
            field: "counted",
            value: true,
        }],
        range(
            "exits",
            ItemUnit::Count,
            requirement("minimum_exits", Operand::Parameter("minimum_exits")),
            OnNull::Skip,
        ),
        (
            "has {possible:count} exit(s) via {relation}; at least {minimum_exits:count} required",
            "{sure:count} certain exit(s), at least {minimum_exits:count} required: {undecided}",
        ),
    );
    counted.related = Some("named");
    ItemCheck::Test(Box::new(counted))
}

/// The separation of the pairs against the separation required.
fn separated() -> ItemCheck {
    let mut separated = test(
        vec![When::Field {
            field: "separated",
            value: true,
        }],
        range(
            "separation",
            ItemUnit::Length,
            requirement("required", Operand::Value("required")),
            OnNull::Skip,
        ),
        ("{failed}; {requirement}", OPEN),
    );
    let open = |when: Vec<When>, on: On| Effect {
        when,
        on,
        message: OPEN,
    };
    let not_short = When::Field {
        field: "short",
        value: false,
    };
    separated.effects = vec![
        // No pair measured.
        open(vec![not_short], On::Null),
        // Every pair measured far enough apart, but pairs or exits remain.
        open(
            vec![
                When::Field {
                    field: "undecided_pass",
                    value: true,
                },
                not_short,
            ],
            On::Pass,
        ),
    ];
    // Every pair measured too close, but pairs or exits remain.
    let mut unsettled = test(
        vec![When::Field {
            field: "undecided_fail",
            value: true,
        }],
        standing(),
        (OPEN, OPEN),
    );
    unsettled.effects = vec![open(Vec::new(), On::Fail)];
    separated.otherwise = Some(Box::new(unsettled));
    separated.straddled = Some(Box::new(test(
        vec![When::Field {
            field: "short",
            value: true,
        }],
        standing(),
        ("", ""),
    )));
    separated.related = Some("related");
    ItemCheck::Test(Box::new(separated))
}

fn exits() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: LIST,
            refused: Some("{why}"),
            checks: vec![counted(), separated()],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            once: false,
            combined: None,
            reason: None,
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

/// `exit-separation`, rebuilt as a composition with its outside contract
/// kept.
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
        // The list reports a missing service for each space, as the
        // capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the exits are judged".into()),
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
            checks: vec![exits()],
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
