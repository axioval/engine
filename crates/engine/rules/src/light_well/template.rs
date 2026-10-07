//! `light-well` as a template: the well's measured shared section read
//! first, once its stack is (a well that cannot be measured is open once),
//! then
//! the gaps between its consecutive spaces against the tolerance, and its
//! section, empty or judged by area and width against the row its height
//! selects.

use axioval_engine::template::{
    Allowance, Applies, Bound, Check, Choice, Decision, Form, FormCheck, Group, Guard, ItemCheck,
    ItemTest, ItemUnit, Items, Judge, OnNull, Operand, ParameterDefault, Range, Refusals,
    Requirement, Template, TemplateValue, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

use super::COLUMNS;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.light-well";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("member_path", ParameterType::StringList),
        ParameterDescriptor::required("requirements", ParameterType::Table(COLUMNS)),
        ParameterDescriptor::optional("gap_tolerance_metres", ParameterType::Number),
    ]
}

/// The well's section and the row its height selects: one item.
const REQUIREMENTS: &str = "well_requirements;members=@member_path;requirements=@requirements";

fn measured(name: &'static str, property: &'static str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: property.to_owned(),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
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
        related: Some("members"),
    }
}

/// A number of the item at least the number `required`, where the row
/// states one.
fn at_least(value: &'static str, unit: ItemUnit, required: &'static str) -> Judge {
    Judge::Range(Box::new(Range {
        value,
        unit,
        at_least: vec![Requirement {
            name: required,
            options: vec![Choice {
                when: Vec::new(),
                bound: Bound::Operand(Operand::Value(required)),
            }],
            words: "",
        }],
        at_most: Vec::new(),
        allowance: Allowance::None,
        grade: false,
        null: OnNull::Skip,
        unmeasured: None,
    }))
}

fn items(list: &'static str, checks: Vec<ItemCheck>) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list,
            // The form's values report a well that cannot be measured.
            refused: None,
            checks,
            together: None,
            passing: None,
            texts: Vec::new(),
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

/// No consecutive spaces farther apart than the tolerance.
fn gaps() -> FormCheck {
    let judged = test(
        Judge::Range(Box::new(Range {
            value: "gap",
            unit: ItemUnit::Length,
            at_least: Vec::new(),
            at_most: vec![Requirement {
                name: "tolerance",
                options: vec![Choice {
                    when: Vec::new(),
                    bound: Bound::Operand(Operand::Parameter("gap_tolerance_metres")),
                }],
                words: "",
            }],
            allowance: Allowance::None,
            grade: false,
            null: OnNull::Skip,
            unmeasured: None,
        })),
        "{above} starts {gap:length} above the top of {below}, so the well is not contiguous",
        "the gap between {below} and {above} is {gap:length}",
    );
    items(
        "well_gaps;members=@member_path",
        vec![ItemCheck::Test(Box::new(judged))],
    )
}

/// A shared section, large and wide enough for the row its height selects.
fn section() -> FormCheck {
    let mut empty = test(
        Judge::Fails,
        "the {count:count} stacked spaces share no plan section, so the well is not contiguous",
        "",
    );
    empty.when = vec![When::Null { field: "area" }];
    let area = test(
        at_least("area", ItemUnit::Area, "required_area"),
        "the well's section area is {area:area} m²; row {row:count} requires {bound:plain} m² \
         for a well {height:length} high",
        "the well's section area is {area:area} m²; row {row:count} requires {bound:plain} m², \
         undecided",
    );
    let width = test(
        at_least("width", ItemUnit::Length, "required_width"),
        "the well's width is {width:length}; row {row:count} requires {bound:plain} m for a \
         well {height:length} high",
        "the well's width is {width:length}; row {row:count} requires {bound:plain} m, \
         undecided",
    );
    items(
        REQUIREMENTS,
        vec![
            ItemCheck::Test(Box::new(empty)),
            ItemCheck::Group(Box::new(Group {
                applies: Applies::default(),
                when: Vec::new(),
                // No row, no requirement; an undecided row, the well open.
                guards: vec![Guard {
                    field: "row",
                    when: Vec::new(),
                    undecided: Some(
                        "which row applies to a well {height:length} high is undecided",
                    ),
                    null: OnNull::Skip,
                }],
                checks: vec![
                    ItemCheck::Test(Box::new(area)),
                    ItemCheck::Test(Box::new(width)),
                ],
            })),
        ],
    )
}

/// `light-well`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "light-well",
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "gap_tolerance_metres",
            from: &[],
            value: ScalarValue::Number { value: 0.0 },
        }],
        declaration: vec![
            Check::Required {
                parameter: "member_path",
            },
            Check::Path {
                parameter: "member_path",
            },
            Check::Required {
                parameter: "requirements",
            },
            Check::Arguments {
                when: &["requirements"],
                value: REQUIREMENTS,
            },
            Check::NonNegative {
                parameters: &["gap_tolerance_metres"],
                message: "`gap_tolerance_metres` must not be negative",
            },
        ],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The section, measured once the stack is: a well whose
            // spaces' extents or shared section cannot be measured is open
            // once, for the first one's reason.
            values: vec![measured("area", "well_section_area;members=@member_path")],
            decision: Decision::Within {
                value: "area",
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
            checks: vec![gaps(), section()],
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
