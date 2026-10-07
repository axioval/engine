//! `space-distance` as a template: each applicable row's nearest
//! destination distance (`distance_rows`, the search's bounds from every
//! destination that might qualify to the sure ones) at most the row's
//! `maximum`, then at least its `minimum`, each a graded finding worded as
//! the capability worded it, a distance the bounds leave undecided open
//! once per row.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, ItemCheck, ItemTest,
    ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals, Requirement, Template, TemplateValue,
};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.space-distance";

const ROWS: &str = "distance_rows;distances=@distances;storey_path=@storey_path;\
                    storey_selector=@storey_selector;access_path=@access_path;\
                    door_selector=@door_selector;opening_selector=@opening_selector;\
                    space_selector=@space_selector;walking_radius=@walking_radius;\
                    walking_height=@walking_height;walking_step=@walking_step;\
                    walking_slope=@walking_slope;stair_selector=@stair_selector;\
                    ramp_selector=@ramp_selector;lift_selector=@lift_selector;\
                    stair_length=@stair_length;vertical_factor=@vertical_factor";

/// The row's bound `field`.
fn bound(field: &'static str) -> Vec<Requirement> {
    vec![Requirement {
        name: field,
        options: vec![Choice {
            when: Vec::new(),
            bound: Bound::Operand(Operand::Value(field)),
        }],
        words: "",
    }]
}

/// The nearest distance within `(at_least, at_most)`, graded, worded by
/// `fail` and relating `related`.
fn test(
    bounds: (Vec<Requirement>, Vec<Requirement>),
    (fail, related): (&'static str, &'static str),
    then: Option<Box<ItemTest>>,
) -> ItemTest {
    let (at_least, at_most) = bounds;
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Range(Box::new(Range {
            value: "nearest",
            unit: ItemUnit::Length,
            at_least,
            at_most,
            allowance: Allowance::None,
            grade: true,
            null: OnNull::Skip,
            unmeasured: Some("{why}"),
        })),
        fail,
        undecided: "{open_words}",
        effects: Vec::new(),
        then,
        otherwise: None,
        straddled: None,
        related: Some(related),
    }
}

fn rows() -> FormCheck {
    // The maximum first, as the capability judged it; only where it holds
    // the minimum.
    let minimum = test(
        (bound("minimum"), Vec::new()),
        ("{below_words}", "below_related"),
        None,
    );
    let maximum = test(
        (Vec::new(), bound("maximum")),
        ("{above_words}", "above_related"),
        Some(Box::new(minimum)),
    );
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: ROWS,
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(maximum))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: None,
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

/// `space-distance`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: super::parameters(),
        grades: true,
        name: "space-distance",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Arguments {
            when: &[],
            value: ROWS,
        }],
        // Each row reports the services its measure misses.
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "judged",
                expression: Expression::Literal {
                    value: ScalarValue::Integer { value: 1 },
                    label: Some("the nearest destinations are measured".into()),
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
            checks: vec![rows()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
