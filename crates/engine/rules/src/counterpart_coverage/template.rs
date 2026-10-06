//! `counterpart-coverage` as a template: the measured
//! `counterpart_uncovered_share` of each check (plan and height, or the
//! elevation) at most the lowest declared threshold, a finding graded into
//! the most severe band its share may exceed, and an element whose extent
//! or footprint cannot be read open once for both checks.

use axioval_engine::template::{
    Applies, Band, Check, Condition, Decision, End, Expect, Form, FormCheck, Grading, Operand,
    ParameterDefault, Refusals, Template, TemplateValue, Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};
use axioval_ir::{QuantityDimension, Severity};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.counterpart-coverage";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("counterparts", ParameterType::Selector),
        ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
        ParameterDescriptor::optional("horizontal_tolerance", ParameterType::Quantity),
        ParameterDescriptor::optional("vertical_tolerance", ParameterType::Quantity),
        ParameterDescriptor::optional("info_above", ParameterType::Number),
        ParameterDescriptor::optional("warning_above", ParameterType::Number),
        ParameterDescriptor::optional("error_above", ParameterType::Number),
        ParameterDescriptor::optional("axis_tolerance", ParameterType::Quantity),
        ParameterDescriptor::optional("measure", ParameterType::String),
        ParameterDescriptor::optional("infill_counterparts", ParameterType::Selector),
        ParameterDescriptor::optional("infill_above", ParameterType::Number),
    ]
}

/// The bands, least severe first, each with the threshold above which it
/// applies.
const BANDS: [(&str, Severity); 3] = [
    ("info_above", Severity::Info),
    ("warning_above", Severity::Warning),
    ("error_above", Severity::Error),
];

/// The cover every value of an element shares: the rule's counterparts,
/// growths and axis tolerance, each named as the rule names it (the
/// growths as the tolerance the check takes).
macro_rules! cover {
    ($name:literal, $measure:literal) => {
        concat!(
            $name,
            ";by=@counterparts;measure=",
            $measure,
            ";horizontal=@horizontal;vertical=@vertical;axis_tolerance=@axis_tolerance;\
             frame=@infill_counterparts;infill_above=@infill_above"
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
    }
}

/// A threshold in `[0, 1)` where stated.
macro_rules! in_share {
    ($parameter:literal) => {
        Condition::All {
            conditions: &[
                Condition::Not {
                    condition: &Condition::Under {
                        parameter: $parameter,
                        than: 0.0,
                    },
                },
                Condition::Not {
                    condition: &Condition::AtLeast {
                        parameter: $parameter,
                        than: 1.0,
                    },
                },
            ],
        }
    };
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    let length = |parameter: &'static str, message: &'static str| Check::Quantity {
        parameter,
        dimension: QuantityDimension::Length,
        message,
    };
    let mut checks = vec![
        length("tolerance", "tolerance must be a length"),
        length(
            "horizontal_tolerance",
            "horizontal_tolerance must be a length",
        ),
        length("vertical_tolerance", "vertical_tolerance must be a length"),
        Check::AnyOf {
            parameters: &["tolerance", "horizontal_tolerance", "vertical_tolerance"],
            message: "declare `tolerance`, or `horizontal_tolerance` and `vertical_tolerance`",
        },
        Check::Exclusive {
            one: &["tolerance"],
            other: &["horizontal_tolerance", "vertical_tolerance"],
            message: "`tolerance` cannot be combined with `horizontal_tolerance` or \
                      `vertical_tolerance`",
        },
        Check::Together {
            parameters: &["horizontal_tolerance", "vertical_tolerance"],
            message: "`horizontal_tolerance` and `vertical_tolerance` are declared together",
        },
        Check::Holds {
            condition: Condition::Not {
                condition: &Condition::All {
                    conditions: &[
                        Condition::Under {
                            parameter: "horizontal",
                            than: 0.0,
                        },
                        Condition::Under {
                            parameter: "vertical",
                            than: 0.0,
                        },
                    ],
                },
            },
            message: "a negative tolerance switches its check off, and every check is off",
        },
    ];
    checks.extend(thresholds());
    checks.extend(measures());
    checks
}

/// The thresholds' checks: each a share, above the less severe ones, and
/// one stated.
fn thresholds() -> Vec<Check> {
    vec![
        Check::Kind {
            parameter: "info_above",
        },
        Check::Holds {
            condition: in_share!("info_above"),
            message: "info_above must lie in [0, 1)",
        },
        Check::Kind {
            parameter: "warning_above",
        },
        Check::Holds {
            condition: in_share!("warning_above"),
            message: "warning_above must lie in [0, 1)",
        },
        Check::Exceeds {
            parameter: "warning_above",
            earlier: &["info_above"],
            message: "warning_above must exceed the thresholds of less severe bands",
        },
        Check::Kind {
            parameter: "error_above",
        },
        Check::Holds {
            condition: in_share!("error_above"),
            message: "error_above must lie in [0, 1)",
        },
        Check::Exceeds {
            parameter: "error_above",
            earlier: &["info_above", "warning_above"],
            message: "error_above must exceed the thresholds of less severe bands",
        },
        Check::AnyOf {
            parameters: &["info_above", "warning_above", "error_above"],
            message: "declare at least one of `info_above`, `warning_above` and `error_above`",
        },
    ]
}

/// The checks of the measure, the counterparts, the axis and the frame.
fn measures() -> Vec<Check> {
    vec![
        Check::Choice {
            parameter: "measure",
            options: &["plan_and_height", "elevation"],
        },
        Check::Holds {
            condition: Condition::Not {
                condition: &Condition::All {
                    conditions: &[
                        ELEVATION,
                        Condition::Not {
                            condition: &Condition::All {
                                conditions: &[
                                    Condition::AtLeast {
                                        parameter: "horizontal",
                                        than: 0.0,
                                    },
                                    Condition::AtLeast {
                                        parameter: "vertical",
                                        than: 0.0,
                                    },
                                ],
                            },
                        },
                    ],
                },
            },
            message: "the elevation is one check measured with both tolerances; neither may be \
                      negative",
        },
        Check::Required {
            parameter: "counterparts",
        },
        Check::AngleBelow {
            parameter: "axis_tolerance",
            below: 45.0,
            range: "axis_tolerance must lie in [0, 45) degrees",
            angle: "axis_tolerance must be a plane angle",
        },
        Check::Kind {
            parameter: "infill_counterparts",
        },
        Check::Kind {
            parameter: "infill_above",
        },
        Check::Requires {
            parameter: "infill_above",
            with: &["infill_counterparts"],
            message: "`infill_above` needs `infill_counterparts`",
        },
        Check::RequiresValue {
            parameter: "infill_counterparts",
            with: "measure",
            value: "elevation",
            message: "`infill_counterparts` applies only to `measure` `elevation`",
        },
        Check::Holds {
            condition: in_share!("infill_above"),
            message: "infill_above must lie in [0, 1)",
        },
    ]
}

/// The rule measures the elevation.
const ELEVATION: Condition = Condition::Equals {
    parameter: "measure",
    value: "elevation",
};

/// Whether the share surely (`Lower`) or possibly (`Upper`) exceeds the
/// threshold `parameter`.
const fn exceeds(parameter: &'static str, end: End) -> Condition {
    Condition::Exceeds {
        value: "share",
        parameter,
        end,
    }
}

/// The severity of a finding: the most severe band its share may exceed.
fn bands() -> Vec<Band> {
    BANDS
        .iter()
        .rev()
        .map(|(threshold, severity)| Band {
            severity: severity.clone(),
            when: Some(exceeds(threshold, End::Upper)),
        })
        .collect()
}

/// A finding graded more severely by its share's upper end than its lower
/// end reaches says so.
fn texts() -> Vec<Text> {
    const ERROR_UPPER: Condition = exceeds("error_above", End::Upper);
    const ERROR_LOWER: Condition = exceeds("error_above", End::Lower);
    const WARNING_UPPER: Condition = exceeds("warning_above", End::Upper);
    const WARNING_LOWER: Condition = exceeds("warning_above", End::Lower);
    const INFO_LOWER: Condition = exceeds("info_above", End::Lower);
    vec![
        Text {
            name: "graded",
            when: Some(Condition::All {
                conditions: &[
                    ERROR_UPPER,
                    Condition::Not {
                        condition: &ERROR_LOWER,
                    },
                    WARNING_LOWER,
                ],
            }),
            text: "; graded error by its upper bound, at least warning",
        },
        Text {
            name: "graded",
            when: Some(Condition::All {
                conditions: &[
                    ERROR_UPPER,
                    Condition::Not {
                        condition: &ERROR_LOWER,
                    },
                    Condition::Not {
                        condition: &WARNING_LOWER,
                    },
                    INFO_LOWER,
                ],
            }),
            text: "; graded error by its upper bound, at least info",
        },
        Text {
            name: "graded",
            when: Some(Condition::All {
                conditions: &[
                    Condition::Not {
                        condition: &ERROR_UPPER,
                    },
                    WARNING_UPPER,
                    Condition::Not {
                        condition: &WARNING_LOWER,
                    },
                    INFO_LOWER,
                ],
            }),
            text: "; graded warning by its upper bound, at least info",
        },
        // Nothing covers it, nor may.
        Text {
            name: "overlaps",
            when: Some(Condition::Below {
                value: "covering",
                than: 0.5,
            }),
            text: "; no counterpart overlaps it",
        },
        // The frame's infill covers, or may.
        Text {
            name: "infill",
            when: Some(Condition::Above {
                value: "framed",
                than: 0.5,
            }),
            text: " or the infill of the frame of {framed:cited}",
        },
        Text {
            name: "infill",
            when: Some(Condition::Not {
                condition: &Condition::Below {
                    value: "framed",
                    than: 0.5,
                },
            }),
            text: " or, should more than {infill_above} be uncovered (undecided), the infill of \
                   the frame of {framed:cited}",
        },
    ]
}

/// One check: its share, the part uncovered and the whole measured with
/// `measure`, at most the lowest threshold, where `applies` holds.
fn check(
    measure: &'static str,
    framed: bool,
    applies: Condition,
    (fail, undecided): (&'static str, &'static str),
) -> FormCheck {
    let named = |name: &'static str| -> &'static str {
        // Every value of one check names the same measurement.
        match (name, measure) {
            ("share", "plan") => cover!("counterpart_uncovered_share", "plan"),
            ("share", "height") => cover!("counterpart_uncovered_share", "height"),
            ("share", _) => cover!("counterpart_uncovered_share", "elevation"),
            ("uncovered", "plan") => cover!("counterpart_uncovered", "plan"),
            ("uncovered", "height") => cover!("counterpart_uncovered", "height"),
            ("uncovered", _) => cover!("counterpart_uncovered", "elevation"),
            ("whole", "plan") => cover!("counterpart_whole", "plan"),
            ("whole", "height") => cover!("counterpart_whole", "height"),
            ("whole", _) => cover!("counterpart_whole", "elevation"),
            _ => cover!("counterpart_infill", "elevation"),
        }
    };
    let mut values: Vec<TemplateValue> = ["share", "uncovered", "whole"]
        .into_iter()
        .map(|name| measured(name, named(name)))
        .collect();
    if framed {
        values.push(measured("framed", named("framed")));
    }
    // Only the share is cited: the other values word the same measurement.
    for value in &mut values[1..] {
        value.expect = Some(Expect::Words);
    }
    FormCheck {
        derived: Vec::new(),
        values,
        decision: Decision::Within {
            value: "share",
            minimum: None,
            maximum: Some(vec![Term::plus(Operand::Parameter("lowest"))]),
            rounding: Vec::new(),
        },
        fail,
        undecided,
        related: Some("share"),
        grading: Some(Grading {
            values: Vec::new(),
            derived: Vec::new(),
            bands: bands(),
            undecided: Vec::new(),
        }),
        applies: Some(Applies {
            when: &[],
            any: &[],
            condition: Some(applies),
        }),
        unless: None,
        quiet: false,
    }
}

fn default(
    parameter: &'static str,
    value: ScalarValue,
    from: &'static [&'static str],
) -> ParameterDefault {
    ParameterDefault {
        parameter,
        value,
        from,
    }
}

/// `counterpart-coverage`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    let metres = |value: f64| ScalarValue::Quantity {
        value,
        unit: "m".to_owned(),
    };
    Template {
        id: ID,
        parameters: parameters(),
        grades: true,
        name: "counterpart-coverage",
        refusals: Refusals::Rule,
        defaults: vec![
            default(
                "measure",
                ScalarValue::String {
                    value: "plan_and_height".to_owned(),
                },
                &[],
            ),
            default("infill_above", ScalarValue::Number { value: 0.5 }, &[]),
            // Each check's growth: the one tolerance, or its own.
            default(
                "horizontal",
                metres(0.0),
                &["tolerance", "horizontal_tolerance"],
            ),
            default(
                "vertical",
                metres(0.0),
                &["tolerance", "vertical_tolerance"],
            ),
            // The threshold a share must not exceed: the least severe
            // declared (a declaration declares one).
            default(
                "lowest",
                ScalarValue::Number { value: 0.0 },
                &["info_above", "warning_above", "error_above"],
            ),
        ],
        declaration: declaration(),
        services: None,
        texts: texts(),
        forms: vec![Form {
            when: &[],
            // Read first: the services the checks need, and an element whose
            // extent or footprint cannot be read, open once for both checks.
            values: vec![TemplateValue {
                expect: Some(Expect::Words),
                ..measured("covering", cover!("counterpart_covering", "@measure"))
            }],
            decision: Decision::Within {
                value: "covering",
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
            checks: checks(),
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
        }],
    }
}

/// The plan, height and elevation checks, each where the rule's
/// tolerances and measure apply it.
fn checks() -> Vec<FormCheck> {
    vec![
        check(
            "plan",
            false,
            Condition::All {
                conditions: &[
                    Condition::Not {
                        condition: &ELEVATION,
                    },
                    Condition::AtLeast {
                        parameter: "horizontal",
                        than: 0.0,
                    },
                ],
            },
            (
                "plan: {share:area} of the footprint ({uncovered:area} of {whole:area} \
                         m²) lies outside every counterpart grown by {horizontal:si} \
                         m{overlaps}{graded}",
                "plan: {share:area} of the footprint ({uncovered:area} of {whole:area} \
                         m²) lies outside every counterpart grown by {horizontal:si} m, which \
                         straddles the threshold {lowest}{share:notes}",
            ),
        ),
        check(
            "height",
            false,
            Condition::All {
                conditions: &[
                    Condition::Not {
                        condition: &ELEVATION,
                    },
                    Condition::AtLeast {
                        parameter: "vertical",
                        than: 0.0,
                    },
                ],
            },
            (
                "height: {share:area} of the height ({uncovered:area} of {whole:area} m) \
                         lies outside every counterpart overlapping it in plan, grown by \
                         {vertical:si} m{overlaps}{graded}",
                "height: {share:area} of the height ({uncovered:area} of {whole:area} m) \
                         lies outside every counterpart overlapping it in plan, grown by \
                         {vertical:si} m, which straddles the threshold {lowest}{share:notes}",
            ),
        ),
        check(
            "elevation",
            true,
            ELEVATION,
            (
                "elevation: {share:area} of the elevation ({uncovered:area} of \
                         {whole:area} m²) lies outside every counterpart{infill}, grown by \
                         {horizontal:si} m along its axis and {vertical:si} m in \
                         height{overlaps}{graded}",
                "elevation: {share:area} of the elevation ({uncovered:area} of \
                         {whole:area} m²) lies outside every counterpart{infill}, grown by \
                         {horizontal:si} m along its axis and {vertical:si} m in height, which \
                         straddles the threshold {lowest}{share:notes}",
            ),
        ),
    ]
}
