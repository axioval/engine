//! `clash` and `clash-matrix` as templates: the measured pairs
//! (`clash_pairs`, `clash_matrix_pairs`) each put in the first class that
//! holds against the tolerances listed with it, graded by class, cell or
//! measure, and grouped as the rule declares.
//!
//! The classes are data: a duplicate is a pair whose surfaces lie within
//! the duplicate tolerance, an intersection one penetrating past its
//! tolerance and reaching past the axis and volume tolerances unless a
//! tolerance case excuses it, a clearance shortfall one closer than the
//! clearance. Both capabilities share them.

use axioval_engine::template::{
    Applies, Bound, Check, ClassSeverities, Decision, Form, GradeMeasure, ItemText, Operand,
    PairClass, PairGrades, PairGroups, PairOpen, PairOrder, PairSeverity, PairTest, Pairs,
    ParameterDefault, Refusals, Service, Services, Template, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::ScalarValue;

use crate::clash_cases::case_parameter;
use crate::clash_groups::grouping_parameters;
use crate::clash_severity::severity_parameters;

/// The capabilities' ids.
pub(crate) const CLASH: &str = "axioval:capability.clash";
pub(crate) const CLASH_MATRIX: &str = "axioval:capability.clash-matrix";

/// The descriptor `clash` keeps.
pub(crate) fn clash_parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![ParameterDescriptor::required(
        "counterparts",
        ParameterType::Selector,
    )];
    for (name, number) in super::profile_columns() {
        parameters.push(match (name, number) {
            ("penetration_tolerance_metres", _) => {
                ParameterDescriptor::required(name, ParameterType::Number)
            }
            (_, true) => ParameterDescriptor::optional(name, ParameterType::Number),
            (_, false) => ParameterDescriptor::optional(name, ParameterType::Boolean),
        });
    }
    parameters.extend([
        ParameterDescriptor::optional("exclude_paths", ParameterType::StringList),
        ParameterDescriptor::optional("exclude_target_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("exclude_same_layer", ParameterType::Boolean),
    ]);
    parameters.extend(grouping_parameters());
    parameters.extend(severity_parameters());
    parameters.push(case_parameter());
    parameters
}

/// The parameters both lists read, by the rule's own names.
macro_rules! shared {
    () => {
        "exclude_paths=@exclude_paths;exclude_target_property=@exclude_target_property;\
         exclude_same_layer=@exclude_same_layer;group_by=@group_by;per_storey=@per_storey;\
         storey_path=@storey_path;group_property=@group_property;\
         group_tolerance_metres=@group_tolerance_metres;\
         duplicate_quantities=@duplicate_quantities;tolerance_cases=@tolerance_cases"
    };
}

/// The grading parameters a declaration reads beside the pairs': checked
/// with them, read by the template's grading, never by the list.
macro_rules! graded {
    () => {
        ";severity_by_class=@severity_by_class;grade_by=@grade_by;\
         severity_grades=@severity_grades"
    };
}

/// `clash`'s pairs: the rule's selection against its counterparts, beside
/// the rule's tolerances.
macro_rules! clash_list {
    () => {
        concat!(
            "clash_pairs;subjects=@selection;counterparts=@counterparts;\
     penetration_tolerance_metres=@penetration_tolerance_metres;\
     clearance_metres=@clearance_metres;duplicate_tolerance_metres=@duplicate_tolerance_metres;\
     horizontal_tolerance_metres=@horizontal_tolerance_metres;\
     vertical_tolerance_metres=@vertical_tolerance_metres;\
     volume_tolerance_cubic_metres=@volume_tolerance_cubic_metres;\
     report_duplicates=@report_duplicates;report_containment=@report_containment;\
     report_intersections=@report_intersections;",
            shared!()
        )
    };
}
pub(crate) const CLASH_LIST: &str = clash_list!();
/// `clash`'s declaration, as the capability read it.
const CLASH_CHECK: &str = concat!(clash_list!(), graded!());

/// `clash-matrix`'s pairs: beside the tolerances of the cell covering each.
macro_rules! matrix_list {
    () => {
        concat!(
            "clash_matrix_pairs;subjects=@selection;counterparts=@counterparts;cells=@cells;\
     key_1=@key_1;key_2=@key_2;key_3=@key_3;case_sensitive=@case_sensitive;\
     symmetric=@symmetric;report_unmatched=@report_unmatched;\
     exclude_same_system=@exclude_same_system;system_path=@system_path;",
            shared!()
        )
    };
}
pub(crate) const MATRIX_LIST: &str = matrix_list!();
/// `clash-matrix`'s declaration, as the capability read it.
const MATRIX_CHECK: &str = concat!(matrix_list!(), graded!());

fn compare(value: &'static str, order: PairOrder, bound: Operand, zero: bool) -> PairTest {
    PairTest::Compare {
        value,
        order,
        bound: Bound::Operand(bound),
        zero,
    }
}

/// The tolerances a pair is judged against, by their item fields (a
/// matrix's cells) or the rule's own parameters (`clash`'s, the same for
/// every pair), with how a message shows each.
const TOLERANCES: [(&str, &str, &str); 6] = [
    (
        "penetration_tolerance",
        "penetration_tolerance_metres",
        "fixed4",
    ),
    (
        "duplicate_tolerance",
        "duplicate_tolerance_metres",
        "fixed4",
    ),
    (
        "horizontal_tolerance",
        "horizontal_tolerance_metres",
        "fixed4",
    ),
    ("vertical_tolerance", "vertical_tolerance_metres", "fixed4"),
    (
        "volume_tolerance",
        "volume_tolerance_cubic_metres",
        "fixed6",
    ),
    ("clearance", "clearance_metres", "fixed4"),
];

/// The bound a tolerance names: the item's field in a matrix, the rule's
/// parameter otherwise.
fn tolerance(name: &'static str, matrix: bool) -> Operand {
    let (field, parameter, _) = TOLERANCES
        .iter()
        .find(|(field, ..)| *field == name)
        .copied()
        .unwrap_or((name, name, ""));
    if matrix {
        Operand::Value(field)
    } else {
        Operand::Parameter(parameter)
    }
}

fn class(
    name: &'static str,
    holds: Vec<PairTest>,
    reported: Option<&'static str>,
    fail: &'static str,
    undecided: &'static str,
) -> PairClass {
    PairClass {
        name,
        holds,
        excused: None,
        reported,
        opens: false,
        fail,
        undecided,
    }
}

/// The classes a pair is tried in, in order; with `unmatched`, a pair no
/// matrix cell covers first.
#[allow(clippy::too_many_lines)]
fn classes(matrix: bool) -> Vec<PairClass> {
    let mut classes = Vec::new();
    if matrix {
        classes.push(class(
            "unmatched",
            vec![PairTest::Truth {
                field: "unmatched",
                value: true,
            }],
            None,
            "no clash matrix cell covers {subject_category} against {counterpart_category}",
            "",
        ));
    }
    classes.extend([
        class(
            "duplicate",
            vec![compare(
                "hausdorff",
                PairOrder::AtMost,
                tolerance("duplicate_tolerance", matrix),
                false,
            )],
            Some("report_duplicates"),
            "duplicate of {counterpart}: the surfaces lie within {hausdorff:upper4} m of each \
             other, tolerance {duplicate_tolerance_words} m{note}{copies}",
            "whether {counterpart} is a duplicate cannot be decided: the surfaces lie between \
             {hausdorff:lower4} m and {upper} of each other, tolerance \
             {duplicate_tolerance_words} m{note}",
        ),
        class(
            "containment",
            vec![PairTest::Truth {
                field: "inside",
                value: true,
            }],
            Some("report_containment"),
            "lies wholly inside {counterpart}{note}",
            "",
        ),
        class(
            "containment",
            vec![PairTest::Truth {
                field: "contains",
                value: true,
            }],
            Some("report_containment"),
            "wholly contains {counterpart}{note}",
            "",
        ),
        PairClass {
            excused: Some(PairTest::Truth {
                field: "excused",
                value: true,
            }),
            ..class(
                "intersection",
                vec![
                    compare(
                        "penetration",
                        PairOrder::Above,
                        tolerance("penetration_tolerance", matrix),
                        false,
                    ),
                    compare(
                        "horizontal",
                        PairOrder::Above,
                        tolerance("horizontal_tolerance", matrix),
                        true,
                    ),
                    compare(
                        "vertical",
                        PairOrder::Above,
                        tolerance("vertical_tolerance", matrix),
                        true,
                    ),
                    compare(
                        "shared_volume",
                        PairOrder::Above,
                        tolerance("volume_tolerance", matrix),
                        true,
                    ),
                ],
                Some("report_intersections"),
                "hard clash with {counterpart}: penetration {penetration:fixed4} m exceeds \
                 tolerance {penetration_tolerance_words} m{reach}{note}",
                "whether the intersection with {counterpart} exceeds the horizontal tolerance \
                 {horizontal_tolerance_words} m, the vertical tolerance \
                 {vertical_tolerance_words} m and the volume tolerance \
                 {volume_tolerance_words} m³{excuse} cannot be decided{reach}{note}",
            )
        },
        PairClass {
            opens: true,
            ..class(
                "meeting",
                vec![
                    PairTest::Stated {
                        field: "penetration",
                        stated: false,
                    },
                    PairTest::Compare {
                        value: "separation",
                        order: PairOrder::AtMost,
                        bound: Bound::Literal(0.0),
                        zero: false,
                    },
                ],
                None,
                "surfaces meet {counterpart}, but neither body is a closed solid, so touching \
                 cannot be told from crossing",
                "",
            )
        },
        class(
            "clearance",
            vec![compare(
                "certified",
                PairOrder::Below,
                tolerance("clearance", matrix),
                false,
            )],
            None,
            "clearance clash with {counterpart}: separation certified within \
             [{certified:lower6}, {certified:upper6}] m, below required {clearance_words} \
             m{note}",
            "whether {counterpart} keeps the clearance {clearance_words} m cannot be decided: \
             separation certified within [{certified:lower6}, {certified:upper6}] m{note}",
        ),
        class(
            "clearance",
            vec![
                PairTest::Stated {
                    field: "certified",
                    stated: false,
                },
                compare(
                    "separation",
                    PairOrder::Below,
                    tolerance("clearance", matrix),
                    false,
                ),
            ],
            None,
            "clearance clash with {counterpart}: separation {separation:fixed4} m below \
             required {clearance_words} m{note}",
            "",
        ),
    ]);
    classes
}

fn text(name: &'static str, when: Vec<When>, text: &'static str) -> ItemText {
    ItemText { name, when, text }
}

/// The text naming how a tolerance shows: `{penetration_tolerance_words}`.
fn words(field: &str) -> &'static str {
    match field {
        "penetration_tolerance" => "penetration_tolerance_words",
        "duplicate_tolerance" => "duplicate_tolerance_words",
        "horizontal_tolerance" => "horizontal_tolerance_words",
        "vertical_tolerance" => "vertical_tolerance_words",
        "volume_tolerance" => "volume_tolerance_words",
        _ => "clearance_words",
    }
}

/// A tolerance shown as a message shows it: `{name:fixed4}`.
fn shown(name: &str, format: &str) -> &'static str {
    match (name, format) {
        ("penetration_tolerance", _) => "{penetration_tolerance:fixed4}",
        ("duplicate_tolerance", _) => "{duplicate_tolerance:fixed4}",
        ("horizontal_tolerance", _) => "{horizontal_tolerance:fixed4}",
        ("vertical_tolerance", _) => "{vertical_tolerance:fixed4}",
        ("volume_tolerance", _) => "{volume_tolerance:fixed6}",
        ("clearance", _) => "{clearance:fixed4}",
        ("penetration_tolerance_metres", _) => "{penetration_tolerance_metres:fixed4}",
        ("duplicate_tolerance_metres", _) => "{duplicate_tolerance_metres:fixed4}",
        ("horizontal_tolerance_metres", _) => "{horizontal_tolerance_metres:fixed4}",
        ("vertical_tolerance_metres", _) => "{vertical_tolerance_metres:fixed4}",
        ("volume_tolerance_cubic_metres", _) => "{volume_tolerance_cubic_metres:fixed6}",
        _ => "{clearance_metres:fixed4}",
    }
}

fn grouped_by(value: &'static str) -> When {
    When::Equals {
        parameter: "group_by",
        value,
    }
}

/// The pairs judged, the list's fields named as both lists state them.
#[allow(clippy::too_many_lines)]
fn pairs(list: &'static str, matrix: bool) -> Pairs {
    Pairs {
        list,
        selections: &["counterparts"],
        subject: "subject",
        related: "counterpart",
        open: PairOpen {
            message: "open",
            reason: "reason",
        },
        classes: classes(matrix),
        unless: Some(PairOpen {
            message: "excluded",
            reason: "excluded_reason",
        }),
        severity: PairSeverity {
            stated: matrix.then_some("severity"),
            by_class: Some(ClassSeverities {
                table: "severity_by_class",
                class: "class",
                severity: "severity",
            }),
            grades: Some(Box::new(PairGrades {
                class: "intersection",
                by: "grade_by",
                measures: vec![
                    GradeMeasure {
                        option: "smallest_extent",
                        field: "smallest_extent",
                        words: "smallest extent",
                        digits: 4,
                        unit: "m",
                    },
                    GradeMeasure {
                        option: "volume",
                        field: "shared_volume",
                        words: "shared volume",
                        digits: 6,
                        unit: "m³",
                    },
                ],
                table: "severity_grades",
                above: "above",
                severity: "severity",
                graded: ", graded {severity} by its {measure} of {value}",
                reaches: ", graded {severity}: the most severe grade its {measure} of {value} \
                          may reach",
                unmeasured: ", graded {severity}: its {measure} is unmeasured, so the most \
                             severe grade it may reach",
            })),
        },
        suffix: matrix.then_some("{cell}"),
        texts: vec![
            // The tolerance cases' words, where they were consulted: never
            // for intersections switched off.
            text(
                "excuse",
                vec![When::Field {
                    field: "report_intersections",
                    value: false,
                }],
                "",
            ),
            text("excuse", Vec::new(), "{case}"),
            text(
                "title",
                vec![grouped_by("similar")],
                "{count} similar {class} clashes of {sides}",
            ),
            text(
                "title",
                vec![grouped_by("type_pair")],
                "{count} clashes of {sides}",
            ),
            text("title", Vec::new(), "{count} clashes"),
            text(
                "upper",
                vec![When::Stated {
                    field: "hausdorff_upper",
                }],
                "{hausdorff_upper}",
            ),
            text("upper", Vec::new(), "{hausdorff:upper4} m"),
        ]
        .into_iter()
        .chain(TOLERANCES.iter().map(|(field, parameter, format)| {
            // A matrix's cell, or the rule.
            text(
                words(field),
                Vec::new(),
                if matrix {
                    shown(field, format)
                } else {
                    shown(parameter, format)
                },
            )
        }))
        .collect(),
        groups: Some(PairGroups {
            applies: Applies {
                when: &["group_by"],
                any: &[],
                condition: None,
            },
            key: "group",
            classed: vec![grouped_by("similar")],
            message: "{title}{on}: {parts}",
            part: "[{subject}] {message}",
            alone: " (not grouped: {why})",
        }),
    }
}

fn template(
    id: &'static str,
    name: &'static str,
    parameters: Vec<ParameterDescriptor>,
    (list, check): (&'static str, &'static str),
    matrix: bool,
) -> Template {
    Template {
        id,
        parameters,
        grades: false,
        name,
        // The capability refused each selected object, worded as stated.
        refusals: Refusals::Objects,
        // `clash`'s tolerances, each zero unless stated, as the capability
        // read them; a matrix's cells state their own.
        defaults: if matrix {
            Vec::new()
        } else {
            [
                "duplicate_tolerance_metres",
                "horizontal_tolerance_metres",
                "vertical_tolerance_metres",
                "volume_tolerance_cubic_metres",
            ]
            .into_iter()
            .map(|parameter| ParameterDefault {
                parameter,
                value: ScalarValue::Number { value: 0.0 },
                from: &[],
            })
            .collect()
        },
        // The declaration as the capability read it, in its order and
        // words: the pairs' own argument check.
        declaration: vec![Check::Arguments {
            when: &[],
            value: check,
        }],
        services: Some(Services {
            needs: vec![Service::Proximity],
            message: "proximity service is not registered",
            only: None,
            whole: false,
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: Vec::new(),
            decision: Decision::Pairs(Box::new(pairs(list, matrix))),
            fail: "",
            undecided: "",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}

/// `clash`, rebuilt as a composition with its outside contract kept.
pub(crate) fn clash() -> Template {
    template(
        CLASH,
        "clash",
        clash_parameters(),
        (CLASH_LIST, CLASH_CHECK),
        false,
    )
}

/// `clash-matrix`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn clash_matrix() -> Template {
    template(
        CLASH_MATRIX,
        "clash-matrix",
        crate::clash_matrix::parameters(),
        (MATRIX_LIST, MATRIX_CHECK),
        true,
    )
}
