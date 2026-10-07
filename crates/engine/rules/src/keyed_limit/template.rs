//! `keyed-limit` as a template: the row an object's keys select (the
//! measured list `limit_row`, no row a finding), then what the row bounds,
//! as the measured list `limited_values` reads it with the row's bounds for
//! the declared `quantity`: one value of
//! the object, the sill height above each floor beside a window (every
//! failing floor one finding), or the step onto each floor a side of a door
//! may step onto (a side failing where a floor it surely steps onto fails,
//! or every floor it may).

use axioval_engine::template::{
    Allowance, Alternatives, Applies, Bound, Check, Choice, Combined, Condition, Decision, Form,
    FormCheck, ItemCheck, ItemTest, ItemUnit, Items, Judge, OnNull, Operand, Range, Refusals,
    Requirement, Template, TemplateValue,
};
use serde_json::json;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.keyed-limit";

/// The row's arguments: the table and the keys.
macro_rules! rows {
    ($name:literal) => {
        concat!(
            $name,
            ";limits=@limits;key_1=@key_1;key_1_path=@key_1_path;key_2=@key_2;\
             key_2_path=@key_2_path;key_3=@key_3;key_3_path=@key_3_path;key_4=@key_4;\
             key_4_path=@key_4_path;pair_key=@pair_key;case_sensitive=@case_sensitive"
        )
    };
}

/// What the row bounds, with every parameter the quantity reads.
const LIMITED: &str = concat!(
    rows!("limited_values"),
    ";quantity=@quantity;quantity_property=@quantity_property;\
     measured_value=@measured_value;floor_path=@floor_path;overall_width=@overall_width;\
     width_deduction=@width_deduction;clear_width_from_leaves=@clear_width_from_leaves;\
     overall_height=@overall_height;lining_thickness=@lining_thickness;\
     threshold_thickness=@threshold_thickness;ramp_selector=@ramp_selector;\
     ramp_reach=@ramp_reach;member_selector=@member_selector;relationship=@relationship;\
     direction=@direction;follow_chain=@follow_chain;path=@path;\
     skip_absent_relationship_ends=@skip_absent_relationship_ends;\
     door_type_defaults=@door_type_defaults"
);

/// The row's bounds on the item's value, each where the row states it.
fn bounded(fail: &'static str, undecided: &'static str) -> ItemTest {
    let bound = |name: &'static str| {
        vec![Requirement {
            name,
            options: vec![Choice {
                when: Vec::new(),
                bound: Bound::Operand(Operand::Value(name)),
            }],
            words: "",
        }]
    };
    ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge: Judge::Range(Box::new(Range {
            value: "value",
            unit: ItemUnit::Area,
            at_least: bound("minimum"),
            at_most: bound("maximum"),
            allowance: Allowance::None,
            grade: true,
            null: OnNull::Judge,
            unmeasured: Some("{why}"),
        })),
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("related"),
    }
}

/// What the row bounds, where `quantity` is one of `quantities`.
fn limited(
    quantities: &'static Condition,
    test: ItemTest,
    combined: Option<Combined>,
) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies {
                when: &[],
                any: &[],
                condition: Some(*quantities),
            },
            list: LIMITED,
            // A quantity that cannot be measured leaves the object open as
            // the capability worded it.
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(test))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: None,
            once: false,
            combined,
            reason: None,
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

const VALUE: Condition = Condition::OneOf {
    parameter: "quantity",
    values: &[
        "plan-area",
        "member-plan-area",
        "property",
        "measured",
        "clear-width",
        "clear-height",
        "glazing-ratio",
    ],
};

const SILL: Condition = Condition::Equals {
    parameter: "quantity",
    value: "sill-height",
};

const STEP: Condition = Condition::Equals {
    parameter: "quantity",
    value: "threshold-step",
};

/// The row as a message describes it.
macro_rules! described {
    () => {
        "(limit row {row:fixed0}: {keys})"
    };
}

/// No row matching the object's keys is a finding; keys that cannot
/// decide the row, or rows tying, leave the object open.
fn listed() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: rows!("limit_row"),
            refused: Some("{why}"),
            checks: vec![ItemCheck::Test(Box::new(ItemTest {
                applies: Applies::default(),
                when: Vec::new(),
                judge: Judge::Truth {
                    value: "listed",
                    finding: false,
                },
                fail: "no limit defined for {keys}",
                undecided: "{why}",
                effects: Vec::new(),
                then: None,
                otherwise: None,
                straddled: None,
                related: Some("related"),
            }))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: None,
            once: false,
            combined: None,
            reason: None,
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

/// Each failing floor one finding with the row, each open one likewise.
fn floors(alternatives: Option<Alternatives>) -> Combined {
    Combined {
        separator: "; ",
        fail: concat!("{findings} ", described!()),
        open: concat!("{opens} ", described!()),
        alternatives,
    }
}

/// `keyed-limit`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> axioval_engine::template::Template {
    Template {
        id: ID,
        parameters: super::parameters(),
        grades: true,
        name: "keyed-limit",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        // The keys, the rows, then the quantity, as the measurement reads
        // them: in the capability's order and words.
        declaration: vec![Check::Arguments {
            when: &[],
            value: LIMITED,
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The object is judged by its checks.
            values: vec![TemplateValue {
                name: "judged",
                expression: serde_json::from_value(
                    json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
                )
                .expect("a literal"),
                expect: None,
                absent: None,
                mismatch: None,
                refused: None,
            }],
            decision: Decision::Holds { value: "judged" },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: vec![
                listed(),
                limited(
                    &VALUE,
                    bounded(
                        concat!(
                            "{what} is {value:area}{unit}; required {bound:plain}{unit} ",
                            described!()
                        ),
                        "{what} is {value:area}{unit}, which straddles the bound \
                         {bound:plain}{unit} (limit row {row:fixed0})",
                    ),
                    None,
                ),
                limited(
                    &SILL,
                    bounded(
                        "{what} is {value:area} m; required {bound:plain} m",
                        "{what} is {value:area} m, which straddles the bound {bound:plain} m",
                    ),
                    Some(floors(None)),
                ),
                limited(
                    &STEP,
                    bounded(
                        "{named}; required {bound:plain} m",
                        "{named}, which straddles the bound {bound:plain} m",
                    ),
                    Some(floors(Some(Alternatives {
                        group: "side",
                        sure: "sure",
                        open: "the floor beside {side} may be any of several, and not every \
                               one fails",
                    }))),
                ),
            ],
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}
