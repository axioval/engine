//! `effective-coverage` as a template: the measured `effective_reaching`
//! read first (an element that cannot be measured open once, one stating
//! no area its finding), then the share of its area the sources' effect
//! areas cover at least `minimum_ratio`, and, with a capacity declared, the
//! summed capacity of the sources reaching it at least its area and each
//! surely contributing source's missing value its own finding.

use axioval_engine::template::{
    Applies, Check, Condition, Decision, Derived, Difference, Form, FormCheck, ItemCheck, ItemTest,
    Items, Judge, Operand, Refusals, Template, TemplateValue, Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.effective-coverage";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("sources", ParameterType::Selector),
        ParameterDescriptor::required("mode", ParameterType::String),
        ParameterDescriptor::required("range", ParameterType::Quantity),
        ParameterDescriptor::required("minimum_ratio", ParameterType::Number),
        ParameterDescriptor::optional("blockers", ParameterType::Selector),
        ParameterDescriptor::optional("touch_tolerance", ParameterType::Quantity),
        ParameterDescriptor::optional("capacity_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("capacity_multiplier", ParameterType::Number),
        ParameterDescriptor::optional(
            "capacity_multiplier_property",
            ParameterType::PropertyReference,
        ),
        ParameterDescriptor::optional("area_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("access_path", ParameterType::StringList),
        ParameterDescriptor::optional("door_selector", ParameterType::Selector),
        ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
        ParameterDescriptor::optional("space_selector", ParameterType::Selector),
    ]
}

/// The coverage every value of an element measures: every parameter, named
/// as the rule names it, so the values share one measurement.
macro_rules! effect {
    ($name:literal) => {
        concat!(
            $name,
            ";sources=@sources;blockers=@blockers;mode=@mode;range=@range;\
             touch_tolerance=@touch_tolerance;area_property=@area_property;\
             access_path=@access_path;door_selector=@door_selector;\
             opening_selector=@opening_selector;space_selector=@space_selector;\
             capacity_property=@capacity_property;capacity_multiplier=@capacity_multiplier;\
             capacity_multiplier_property=@capacity_multiplier_property"
        )
    };
}

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

/// A mode that grows the effect, which walls cannot cut.
const GROWN: Condition = Condition::OneOf {
    parameter: "mode",
    values: &["grown", "touching"],
};

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
#[allow(clippy::too_many_lines)]
fn declaration() -> Vec<Check> {
    vec![
        Check::Choice {
            parameter: "mode",
            options: &["grown", "touching", "travel", "visible"],
        },
        Check::NonNegativeLength {
            parameter: "range",
            message: "range must be a non-negative length",
        },
        Check::AnyOf {
            parameters: &["range"],
            message: "range is required",
        },
        Check::Kind {
            parameter: "minimum_ratio",
        },
        Check::Finite {
            parameters: &["minimum_ratio"],
            above: Some(0.0),
            at_least: None,
            message: "minimum_ratio must lie in (0, 1]",
        },
        Check::AtMost {
            parameters: &["minimum_ratio"],
            value: 1.0,
            message: "minimum_ratio must lie in (0, 1]",
        },
        Check::Kind {
            parameter: "blockers",
        },
        Check::Holds {
            condition: Condition::Not {
                condition: &Condition::All {
                    conditions: &[
                        Condition::Stated {
                            parameter: "blockers",
                        },
                        GROWN,
                    ],
                },
            },
            message: "blockers apply only to modes `travel` and `visible`",
        },
        Check::NonNegativeLength {
            parameter: "touch_tolerance",
            message: "touch_tolerance must be a non-negative length",
        },
        Check::RequiresValue {
            parameter: "touch_tolerance",
            with: "mode",
            value: "touching",
            message: "touch_tolerance applies only to mode `touching`",
        },
        // The access declaration, as `space-connection` reads it.
        Check::Arguments {
            when: &[],
            value: effect!("effective_share"),
        },
        Check::Holds {
            condition: Condition::Not {
                condition: &Condition::All {
                    conditions: &[
                        Condition::Stated {
                            parameter: "access_path",
                        },
                        GROWN,
                    ],
                },
            },
            message: "access_path applies only to modes `travel` and `visible`: a grown effect \
                      ignores walls already",
        },
        Check::Required {
            parameter: "sources",
        },
        Check::Kind {
            parameter: "capacity_property",
        },
        Check::Kind {
            parameter: "capacity_multiplier",
        },
        Check::Kind {
            parameter: "capacity_multiplier_property",
        },
        Check::Holds {
            condition: Condition::Not {
                condition: &Condition::All {
                    conditions: &[
                        Condition::Stated {
                            parameter: "capacity_multiplier",
                        },
                        Condition::Not {
                            condition: &Condition::Stated {
                                parameter: "capacity_multiplier_property",
                            },
                        },
                        Condition::Not {
                            condition: &Condition::Positive {
                                parameter: "capacity_multiplier",
                            },
                        },
                    ],
                },
            },
            message: "capacity_multiplier must be a positive number",
        },
        Check::Exclusive {
            one: &["capacity_multiplier"],
            other: &["capacity_multiplier_property"],
            message: "declare capacity_multiplier or capacity_multiplier_property, not both",
        },
        Check::Holds {
            condition: Condition::All {
                conditions: &[
                    // A capacity needs a multiplier,
                    Condition::Not {
                        condition: &Condition::All {
                            conditions: &[
                                Condition::Stated {
                                    parameter: "capacity_property",
                                },
                                Condition::Not {
                                    condition: &Condition::Stated {
                                        parameter: "capacity_multiplier",
                                    },
                                },
                                Condition::Not {
                                    condition: &Condition::Stated {
                                        parameter: "capacity_multiplier_property",
                                    },
                                },
                            ],
                        },
                    },
                    // and a multiplier a capacity.
                    Condition::Not {
                        condition: &Condition::All {
                            conditions: &[
                                Condition::Not {
                                    condition: &Condition::Stated {
                                        parameter: "capacity_property",
                                    },
                                },
                                Condition::Not {
                                    condition: &Condition::All {
                                        conditions: &[
                                            Condition::Not {
                                                condition: &Condition::Stated {
                                                    parameter: "capacity_multiplier",
                                                },
                                            },
                                            Condition::Not {
                                                condition: &Condition::Stated {
                                                    parameter: "capacity_multiplier_property",
                                                },
                                            },
                                        ],
                                    },
                                },
                            ],
                        },
                    },
                ],
            },
            message: "capacity_property is declared together with capacity_multiplier or \
                      capacity_multiplier_property",
        },
        Check::Kind {
            parameter: "area_property",
        },
    ]
}

fn texts() -> Vec<Text> {
    vec![
        Text {
            name: "named",
            when: Some(Condition::Stated {
                parameter: "area_property",
            }),
            text: "the stated area ({area_property})",
        },
        Text {
            name: "named",
            when: None,
            text: "the footprint",
        },
        Text {
            name: "against",
            when: Some(Condition::Stated {
                parameter: "area_property",
            }),
            text: "{named}",
        },
        Text {
            name: "against",
            when: None,
            text: "a footprint",
        },
        Text {
            name: "unreached",
            when: Some(Condition::Below {
                value: "reaching",
                than: 0.5,
            }),
            text: "; no source reaches it",
        },
        Text {
            name: "summed",
            when: Some(Condition::Positive {
                parameter: "capacity_multiplier",
            }),
            text: "{capacity_property} summed over the sources reaching it, times \
                   {capacity_multiplier},",
        },
        Text {
            name: "summed",
            when: None,
            text: "{capacity_property} times {capacity_multiplier_property} summed over the \
                   sources reaching it",
        },
    ]
}

/// Where a capacity is declared.
const CAPACITY: Applies = Applies {
    when: &["capacity_property"],
    any: &[],
    condition: None,
};

/// The share covered at least the minimum.
fn coverage() -> FormCheck {
    FormCheck {
        values: vec![
            measured("area", effect!("effective_area")),
            measured("share", effect!("effective_share")),
            measured("covered", effect!("effective_covered")),
        ],
        derived: Vec::new(),
        decision: Decision::Within {
            value: "share",
            minimum: Some(vec![Term::plus(Operand::Parameter("minimum_ratio"))]),
            maximum: None,
            rounding: Vec::new(),
        },
        fail: "{share:area} of {named} ({covered:area} of {area:area} m²) lies within the \
               sources' effect areas ({mode} by {range:si} m); required {bound:plain}{unreached}",
        undecided: "{share:area} of {named} ({covered:area} of {area:area} m²) lies within the \
                    sources' effect areas ({mode} by {range:si} m), which straddles the bound \
                    {bound:plain}{share:notes3}",
        related: Some("share"),
        grading: None,
        applies: None,
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// The summed capacity at least the area: what it may lack (`spare`) not
/// surely below zero.
fn capacity() -> FormCheck {
    let zero = TemplateValue {
        name: "zero",
        expression: Expression::Literal {
            value: ScalarValue::Number { value: 0.0 },
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    };
    FormCheck {
        values: vec![
            measured("area", effect!("effective_area")),
            measured("summed", effect!("effective_capacity")),
            measured("unread", effect!("effective_unread")),
            zero,
        ],
        derived: vec![
            // Without an upper bound where a contribution cannot be read.
            Derived::Open {
                name: "capacity",
                value: "summed",
                open: "unread",
            },
            Derived::Difference(Difference {
                name: "spare",
                minuend: "capacity",
                subtrahend: "area",
            }),
        ],
        decision: Decision::Within {
            value: "spare",
            minimum: Some(vec![Term::plus(Operand::Value("zero"))]),
            maximum: None,
            rounding: Vec::new(),
        },
        fail: "capacity: {summed} is {capacity:area} m² for {against} of {area:area} m²",
        undecided: "capacity: {summed} is {capacity:area} m² for {against} of {area:area} m², \
                    which cannot be decided{summed:notes3}",
        related: Some("summed"),
        grading: None,
        applies: Some(CAPACITY),
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// Each surely contributing source's missing capacity or multiplier.
fn missing() -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: CAPACITY,
            list: effect!("effective_missing"),
            refused: None,
            checks: vec![ItemCheck::Test(Box::new(ItemTest {
                applies: Applies::default(),
                when: Vec::new(),
                judge: Judge::Truth {
                    value: "missing",
                    finding: true,
                },
                fail: "missing value: {source}'s {property} is not stated",
                undecided: "{why}",
                effects: Vec::new(),
                then: None,
                otherwise: None,
                straddled: None,
                related: Some("source"),
            }))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: None,
        applies: Some(CAPACITY),
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// `effective-coverage`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    let mut reaching = measured("reaching", effect!("effective_reaching"));
    // An element stating no area is checked no further.
    reaching.absent = Some("missing value: its {area_property} is not stated");
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "effective-coverage",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: None,
        texts: texts(),
        forms: vec![Form {
            when: &[],
            values: vec![reaching],
            decision: Decision::Within {
                value: "reaching",
                minimum: None,
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: vec![coverage(), capacity(), missing()],
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
