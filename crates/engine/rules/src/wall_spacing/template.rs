//! `wall-spacing` as a template: the measured `wall_spacing` of each
//! storey, each pair surely parallel and facing at least `minimum` apart,
//! and each footprint's area outside every band at most `uncovered_above`.

use axioval_engine::ParameterDescriptor;
use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, ItemCheck, ItemTest,
    ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Template, TemplateValue,
    When,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.wall-spacing";

const LIST: &str = "wall_spacing;members=@members;member_path=@member_path;\
                    angle_tolerance=@angle_tolerance;minimum=@minimum;maximum=@maximum;\
                    footprints=@footprints;footprint_path=@footprint_path;\
                    uncovered_above=@uncovered_above";

fn bound(name: &'static str, parameter: &'static str) -> Vec<Requirement> {
    vec![Requirement {
        name,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Parameter(parameter)),
        }],
        words: "",
    }]
}

fn test(
    kind: &'static str,
    value: &'static str,
    (at_least, at_most): (Vec<Requirement>, Vec<Requirement>),
    (fail, undecided): (&'static str, &'static str),
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
            unit: ItemUnit::Length,
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
        related,
    }
}

fn checks() -> Vec<ItemCheck> {
    // A pair surely parallel and facing, at least the minimum apart; one
    // straddling it is among the doubts the list words once.
    let mut close = test(
        "close",
        "distance",
        (bound("least", "minimum"), Vec::new()),
        (
            "{pair:and} are parallel and {apart} m apart in plan; at least {minimum:si} m \
             required",
            "",
        ),
        Some("pair"),
    );
    close.straddled = Some(Box::new(ItemTest {
        judge: Judge::Truth {
            value: "close",
            finding: false,
        },
        related: None,
        ..test(
            "close",
            "distance",
            (Vec::new(), Vec::new()),
            ("", ""),
            None,
        )
    }));
    // Why some pair may stand closer than the minimum.
    let doubt = test("doubt", "spacing", (Vec::new(), Vec::new()), ("", ""), None);
    // A footprint's area outside every band.
    let cover = test(
        "cover",
        "uncovered",
        (Vec::new(), bound("most", "uncovered_above")),
        (
            "{what}; at most {uncovered_above:si} m² allowed",
            "{what}, which straddles {uncovered_above:si} m²{unknown}",
        ),
        Some("related"),
    );
    vec![close, doubt, cover]
        .into_iter()
        .map(|test| ItemCheck::Test(Box::new(test)))
        .collect()
}

fn spacing() -> FormCheck {
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

/// `wall-spacing`, rebuilt as a composition with its outside contract kept.
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
        // The list reports missing services for each storey, as the
        // capability did.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the storey is judged".into()),
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
            checks: vec![spacing()],
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
