//! `centre-line-distance` as a template: each side of the centre line the
//! rule judges, an item of the measured `centre_line_sides`, its distance
//! (from every wall that may be there to the nearest sure one) judged
//! against `minimum` first, then `maximum`.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Effect, Form, FormCheck, ItemCheck,
    ItemTest, ItemText, ItemUnit, Items, Judge, On, OnNull, Operand, Range, Refusals, Requirement,
    Service, Services, Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.centre-line-distance";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("wall_selector", ParameterType::Selector),
        ParameterDescriptor::required("centre_line", ParameterType::String),
        ParameterDescriptor::required("sides", ParameterType::String),
        ParameterDescriptor::optional("minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("maximum", ParameterType::Quantity),
        ParameterDescriptor::required("reach", ParameterType::Quantity),
        ParameterDescriptor::optional("inset", ParameterType::Quantity),
    ]
}

/// The sides the rule judges, each with its nearest walls.
const SIDES: &str = "centre_line_sides;walls=@wall_selector;centre_line=@centre_line;\
                     sides=@sides;reach=@reach;inset=@inset";

/// A bound of the distance: the rule's parameter, where stated.
fn bound(parameter: &'static str) -> Requirement {
    Requirement {
        name: parameter,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Parameter(parameter)),
        }],
        words: "",
    }
}

fn range(at_least: Vec<Requirement>, at_most: Vec<Requirement>) -> Judge {
    Judge::Range(Box::new(Range {
        value: "distance",
        unit: ItemUnit::Length,
        at_least,
        at_most,
        allowance: Allowance::None,
        grade: false,
        null: OnNull::Skip,
        unmeasured: None,
    }))
}

fn test(judge: Judge, fail: &'static str, undecided: &'static str) -> ItemTest {
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("wall"),
    }
}

/// The distance against `maximum`: too far where every wall that may be
/// there lies beyond it, a pass where a sure wall lies within it.
fn too_far() -> ItemTest {
    test(
        range(Vec::new(), vec![bound("maximum")]),
        "{label}: too far: {far}",
        "{label}: {undecided}",
    )
}

/// Each side's distance: none within reach, too close, too far, a pass,
/// or open with what the walls leave undecided.
fn sides() -> FormCheck {
    let mut nothing = test(
        Judge::Fails,
        "{label}: no wall nearby (none within {reach:length})",
        "",
    );
    nothing.when = vec![When::Null { field: "distance" }];
    nothing.related = None;
    // Where the minimum is stated: too close first, then too far; a
    // distance straddling the minimum passes nothing.
    let mut straddled = too_far();
    straddled.effects = vec![Effect {
        when: Vec::new(),
        on: On::Pass,
        message: "{label}: {undecided}",
    }];
    let mut close = test(
        range(vec![bound("minimum")], Vec::new()),
        "{label}: too close: {sure:length} from {wall}, less than the minimum \
         {minimum:length}",
        "{label}: {undecided}",
    );
    close.when = vec![
        When::Stated { field: "distance" },
        When::Declared {
            parameter: "minimum",
        },
    ];
    close.then = Some(Box::new(too_far()));
    close.straddled = Some(Box::new(straddled));
    let mut far = too_far();
    far.when = vec![
        When::Stated { field: "distance" },
        When::Undeclared {
            parameter: "minimum",
        },
    ];
    let sure = || vec![When::Stated { field: "sure" }];
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: SIDES,
            refused: Some("centre line: {why}"),
            checks: vec![
                ItemCheck::Test(Box::new(nothing)),
                ItemCheck::Test(Box::new(close)),
                ItemCheck::Test(Box::new(far)),
            ],
            together: None,
            passing: None,
            texts: vec![
                ItemText {
                    name: "far",
                    when: sure(),
                    text: "{sure:length} from {wall}, more than the maximum {maximum:length}",
                },
                ItemText {
                    name: "far",
                    when: Vec::new(),
                    text: "no wall within the maximum {maximum:length}",
                },
                ItemText {
                    name: "undecided",
                    when: sure(),
                    text: "the nearest wall lies {distance:length} from the centre line ({wall} \
                           {sure:length}), which does not decide the bounds",
                },
                ItemText {
                    name: "undecided",
                    when: Vec::new(),
                    text: "a wall may lie {lower:length} or farther from the centre line, but \
                           none surely does",
                },
            ],
            merged: false,
            at: None,
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

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    const BEYOND: &str =
        "`reach` must be at least `minimum` and `maximum`: a wall beyond it is none";
    vec![
        Check::Choice {
            parameter: "centre_line",
            options: &["long", "short", "against-wall"],
        },
        Check::Choice {
            parameter: "sides",
            options: &["nearest", "both"],
        },
        Check::FiniteLength {
            parameter: "minimum",
        },
        Check::FiniteLength {
            parameter: "maximum",
        },
        Check::FiniteLength { parameter: "reach" },
        Check::Positive {
            parameter: "reach",
            message: Some("`reach` is required and must be positive"),
        },
        Check::AnyOf {
            parameters: &["minimum", "maximum"],
            message: "declare `minimum`, `maximum` or both",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "`minimum` exceeds `maximum`",
        },
        Check::Ordered {
            low: "minimum",
            high: "reach",
            message: BEYOND,
        },
        Check::Ordered {
            low: "maximum",
            high: "reach",
            message: BEYOND,
        },
        Check::FiniteLength { parameter: "inset" },
        Check::Required {
            parameter: "wall_selector",
        },
    ]
}

/// `centre-line-distance`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "centre-line-distance",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::PlanSpan],
            message: "centre-line-distance needs the plan-span service",
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // Nothing to read but the sides: the check judges them.
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the sides are judged".into()),
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
            checks: vec![sides()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
