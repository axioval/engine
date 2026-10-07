//! `space-validation` as a template: the model's supports read once per
//! rule where a checked cap names no elements (a refusal leaving every
//! selected space open), then each aspect of each selected space a check of
//! its own at the severity the capability gave it, and, of the project, each
//! region of storey floor belonging to no space and each storey's
//! unallocated share, on the storey, graded by how far they exceed their
//! allowance.

use axioval_engine::template::{
    Applies, Band, Check, Condition, Decision, Derived, Difference, Expect, Form, FormCheck,
    Grading, ItemCheck, ItemTest, ItemUnit, Items, Judge, OnNull, Once, Operand, ParameterDefault,
    Range, Refusals, Requirement, Service, Services, Template, TemplateValue, Term, When,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::Severity;
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.space-validation";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("required_height_metres", ParameterType::Number),
        ParameterDescriptor::required("uncovered_segment_length_metres", ParameterType::Number),
        ParameterDescriptor::required("check_top_cap", ParameterType::Boolean),
        ParameterDescriptor::required("check_bottom_cap", ParameterType::Boolean),
        ParameterDescriptor::required("check_unallocated_area", ParameterType::Boolean),
        ParameterDescriptor::required(
            "maximum_unallocated_area_square_metres",
            ParameterType::Number,
        ),
        ParameterDescriptor::optional("tolerance_metres", ParameterType::Number),
        ParameterDescriptor::optional("maximum_unallocated_share", ParameterType::Number),
        ParameterDescriptor::optional("top_cap_elements", ParameterType::Selector),
        ParameterDescriptor::optional("bottom_cap_elements", ParameterType::Selector),
        ParameterDescriptor::optional("boundary_elements", ParameterType::Selector),
        ParameterDescriptor::optional("intersection_elements", ParameterType::Selector),
    ]
}

/// Every declaration the capability could not realise, in one message.
const UNREALISABLE: &str = "space-validation declaration is missing or not realisable";

/// A stated optional parameter of its kind, or the one message.
macro_rules! of_kind {
    ($name:ident, $parameter:literal) => {
        static $name: Check = Check::Holds {
            condition: Condition::Stated {
                parameter: $parameter,
            },
            message: UNREALISABLE,
        };
    };
}

of_kind!(TOP_KIND, "top_cap_elements");
of_kind!(BOTTOM_KIND, "bottom_cap_elements");
of_kind!(BOUNDARY_KIND, "boundary_elements");
of_kind!(INTERSECTION_KIND, "intersection_elements");

static TOLERANCE: Check = Check::Finite {
    parameters: &["tolerance_metres"],
    above: None,
    at_least: Some(0.0),
    message: UNREALISABLE,
};
static SHARE: Check = Check::Finite {
    parameters: &["maximum_unallocated_share"],
    above: None,
    at_least: Some(0.0),
    message: UNREALISABLE,
};
static WHOLE: Check = Check::AtMost {
    parameters: &["maximum_unallocated_share"],
    value: 1.0,
    message: UNREALISABLE,
};

/// What a rule's parameters must satisfy: every one realisable, or the
/// rule's one message for each selected space.
fn declaration() -> Vec<Check> {
    let stated = |parameter, check| Check::IfStated { parameter, check };
    vec![
        Check::Finite {
            parameters: &[
                "required_height_metres",
                "uncovered_segment_length_metres",
                "maximum_unallocated_area_square_metres",
            ],
            above: None,
            at_least: Some(0.0),
            message: UNREALISABLE,
        },
        Check::Holds {
            condition: Condition::All {
                conditions: &[
                    Condition::Stated {
                        parameter: "check_top_cap",
                    },
                    Condition::Stated {
                        parameter: "check_bottom_cap",
                    },
                    Condition::Stated {
                        parameter: "check_unallocated_area",
                    },
                ],
            },
            message: UNREALISABLE,
        },
        stated("tolerance_metres", &TOLERANCE),
        stated("maximum_unallocated_share", &SHARE),
        stated("maximum_unallocated_share", &WHOLE),
        stated("top_cap_elements", &TOP_KIND),
        stated("bottom_cap_elements", &BOTTOM_KIND),
        stated("boundary_elements", &BOUNDARY_KIND),
        stated("intersection_elements", &INTERSECTION_KIND),
    ]
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

/// A value stated absent where the selection names nothing: the check
/// passes.
fn optional(name: &'static str, property: &'static str) -> TemplateValue {
    TemplateValue {
        expect: Some(Expect::Optional),
        ..measured(name, property)
    }
}

fn literal(name: &'static str, value: f64) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Literal {
            value: ScalarValue::Number { value },
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
}

/// A finding at a fixed severity, or the first of `bands` that holds.
#[allow(clippy::unnecessary_wraps)]
fn severity(bands: Vec<Band>) -> Option<Grading> {
    Some(Grading {
        values: Vec::new(),
        derived: Vec::new(),
        bands,
        undecided: Vec::new(),
    })
}

fn fixed(severity: Severity) -> Vec<Band> {
    vec![Band {
        severity,
        when: None,
    }]
}

/// A check of a selected space, at a fixed severity and stating no
/// deviation, as the capability reported it.
fn check(
    values: Vec<TemplateValue>,
    decision: Decision,
    fail: &'static str,
    related: Option<&'static str>,
    bands: Vec<Band>,
) -> FormCheck {
    FormCheck {
        values,
        derived: Vec::new(),
        decision,
        fail,
        undecided: "{why}",
        related,
        grading: severity(bands),
        applies: None,
        unless: None,
        quiet: false,
        ungraded: true,
    }
}

fn at_most(value: &'static str, bound: Operand) -> Decision {
    Decision::Within {
        value,
        minimum: None,
        maximum: Some(vec![Term::plus(bound)]),
        rounding: Vec::new(),
    }
}

fn at_least(value: &'static str, bound: Operand) -> Decision {
    Decision::Within {
        value,
        minimum: Some(vec![Term::plus(bound)]),
        maximum: None,
        rounding: Vec::new(),
    }
}

/// No other space of the same body.
fn duplicates() -> FormCheck {
    check(
        // Read with the anchor, so its finding relates the duplicates cited.
        vec![measured("duplicates", "space_duplicates;space=@anchor")],
        at_most("duplicates", Operand::Value("zero")),
        "duplicate_space: space body duplicated by {duplicates:least} other space(s)",
        Some("duplicates"),
        fixed(Severity::Error),
    )
}

/// The clear height, raised by the tolerance, at least the requirement.
fn height() -> FormCheck {
    let lowered = TemplateValue {
        name: "lowered",
        expression: Expression::Negate {
            operand: Box::new(Expression::Parameter {
                name: "tolerance_metres".into(),
                label: None,
            }),
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    };
    let mut height = check(
        vec![measured("height", "space_height"), lowered],
        at_least("raised", Operand::Parameter("required_height_metres")),
        "insufficient_height: clear height {height:fixed3} below required \
         {required_height_metres:fixed3}",
        None,
        fixed(Severity::Warning),
    );
    // The height and the tolerance summed as the capability summed them.
    height.derived = vec![Derived::Difference(Difference {
        name: "raised",
        minuend: "height",
        subtrahend: "lowered",
    })];
    height
}

/// No run of the boundary at least the segment uncovered.
fn boundary() -> FormCheck {
    check(
        vec![optional(
            "uncovered",
            "space_uncovered_boundary;segment=@uncovered_segment_length_metres;\
             elements=@boundary_elements",
        )],
        at_most("uncovered", Operand::Value("zero")),
        "uncovered_boundary: {uncovered:fixed3} m of space boundary is uncovered",
        Some("uncovered"),
        fixed(Severity::Warning),
    )
}

/// A finding on each body the space contains, lies in or intersects.
fn overlap(when: &'static str, fail: &'static str) -> ItemCheck {
    ItemCheck::Test(Box::new(ItemTest {
        applies: Applies::default(),
        when: vec![When::Field {
            field: when,
            value: true,
        }],
        judge: Judge::Fails,
        fail,
        undecided: "{why}",
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("other"),
    }))
}

fn overlaps() -> FormCheck {
    let partial = |space: bool, fail: &'static str| {
        ItemCheck::Test(Box::new(ItemTest {
            applies: Applies::default(),
            when: vec![
                When::Field {
                    field: "partial",
                    value: true,
                },
                When::Field {
                    field: "space",
                    value: space,
                },
            ],
            judge: Judge::Fails,
            fail,
            undecided: "{why}",
            effects: Vec::new(),
            then: None,
            otherwise: None,
            straddled: None,
            related: Some("other"),
        }))
    };
    check(
        vec![optional(
            "intersecting",
            "space_intersections;elements=@intersection_elements;tolerance=@tolerance_metres",
        )],
        Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list: "space_overlaps;elements=@intersection_elements;tolerance=@tolerance_metres",
            refused: None,
            checks: vec![
                overlap(
                    "inside",
                    "contained_body: space is contained by another body",
                ),
                overlap("contains", "contained_body: space contains another body"),
                partial(
                    true,
                    "intersecting_space: space intersects another space over {area:fixed4} m2",
                ),
                partial(
                    false,
                    "intersecting_component: space intersects a component over {area:fixed4} m2",
                ),
            ],
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
        "",
        None,
        fixed(Severity::Error),
    )
}

/// A cap covered all but a fiftieth, graded by how little is.
fn cap(value: &'static str, list: &'static str, fail: &'static str) -> FormCheck {
    check(
        vec![optional(value, list)],
        at_least(value, Operand::Value("complete")),
        fail,
        Some(value),
        vec![
            Band {
                severity: Severity::Error,
                when: Some(Condition::Below { value, than: 0.01 }),
            },
            Band {
                severity: Severity::Warning,
                when: Some(Condition::Not {
                    condition: match value {
                        "top" => &Condition::Above {
                            value: "top",
                            than: 0.15,
                        },
                        _ => &Condition::Above {
                            value: "bottom",
                            than: 0.15,
                        },
                    },
                }),
            },
            Band {
                severity: Severity::Info,
                when: None,
            },
        ],
    )
}

/// A requirement bounding an item's number by the rule's parameter.
fn bounded(parameter: &'static str) -> Vec<Requirement> {
    vec![Requirement {
        name: "bound",
        options: vec![axioval_engine::template::Choice {
            when: Vec::new(),
            bound: axioval_engine::template::Bound::Operand(Operand::Parameter(parameter)),
        }],
        words: "",
    }]
}

/// A check of the project's unallocated floor, its items on their storey.
fn unallocated(
    list: &'static str,
    when: &'static [&'static str],
    test: ItemTest,
    refused: Option<&'static str>,
) -> FormCheck {
    FormCheck {
        values: Vec::new(),
        derived: Vec::new(),
        decision: Decision::Items(Box::new(Items {
            applies: Applies::default(),
            list,
            refused,
            checks: vec![ItemCheck::Test(Box::new(test))],
            together: None,
            passing: None,
            texts: Vec::new(),
            merged: false,
            at: Some("storey"),
            once: false,
            combined: None,
            reason: None,
            joined: None,
        })),
        fail: "",
        undecided: "",
        related: None,
        grading: severity(fixed(Severity::Warning)),
        applies: Some(Applies {
            when,
            any: &[],
            condition: None,
        }),
        unless: None,
        quiet: refused.is_none(),
        ungraded: false,
    }
}

/// Each region of storey floor at most the allowance, graded by its
/// excess; each storey's unallocated share at most the maximum.
fn residuals() -> Vec<FormCheck> {
    let range = |value, at_most: Vec<Requirement>, null| {
        Judge::Range(Box::new(Range {
            value,
            unit: ItemUnit::Area,
            at_least: Vec::new(),
            at_most,
            allowance: axioval_engine::template::Allowance::None,
            grade: true,
            null,
            unmeasured: None,
        }))
    };
    let test = |judge, fail, undecided| ItemTest {
        applies: Applies::default(),
        when: Vec::new(),
        judge,
        fail,
        undecided,
        effects: Vec::new(),
        then: None,
        otherwise: None,
        straddled: None,
        related: Some("elements"),
    };
    vec![
        unallocated(
            "unallocated_regions",
            &["check_unallocated_area"],
            test(
                range(
                    "area",
                    bounded("maximum_unallocated_area_square_metres"),
                    OnNull::Skip,
                ),
                "unallocated_area: a region of {area:fixed3} m2 of storey floor belongs to no \
                 space (allowed {maximum_unallocated_area_square_metres:fixed3} m2)",
                "{why}",
            ),
            Some("{why}"),
        ),
        unallocated(
            "unallocated_storeys",
            &["check_unallocated_area", "maximum_unallocated_share"],
            test(
                range(
                    "share",
                    bounded("maximum_unallocated_share"),
                    OnNull::Open(
                        "space-validation: the storey's gross floor area is not measured, so its \
                         unallocated share is undefined",
                    ),
                ),
                "unallocated_area: {share:hundred3}% of the storey's gross floor area \
                 ({area:fixed3} m2 of {gross:fixed3} m2) belongs to no space; required at most \
                 {maximum_unallocated_share:hundred}%",
                "space-validation: the storey's unallocated share ({area:fixed3} m2 of \
                 {gross:fixed3} m2) straddles the maximum of {maximum_unallocated_share:hundred}%",
            ),
            None,
        ),
    ]
}

/// `space-validation`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: true,
        name: "space-validation",
        refusals: Refusals::Objects,
        defaults: vec![ParameterDefault {
            parameter: "tolerance_metres",
            value: ScalarValue::Number { value: 0.005 },
            from: &[],
        }],
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::Space],
            message: "space service is not registered",
            only: None,
            whole: false,
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![literal("zero", 0.0), literal("complete", 0.98)],
            decision: Decision::Within {
                value: "zero",
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
            checks: vec![
                duplicates(),
                height(),
                boundary(),
                overlaps(),
                cap(
                    "top",
                    "space_cap;cap=top;check=@check_top_cap;elements=@top_cap_elements",
                    "uncovered_top_cap: top cap only {top:hundred1}% covered",
                ),
                cap(
                    "bottom",
                    "space_cap;cap=bottom;check=@check_bottom_cap;elements=@bottom_cap_elements",
                    "uncovered_bottom_cap: bottom cap only {bottom:hundred1}% covered",
                ),
            ],
            unless: Vec::new(),
            grading: None,
            once: vec![Once {
                value: measured(
                    "supports",
                    "space_supports;top=@check_top_cap;bottom=@check_bottom_cap;\
                     top_elements=@top_cap_elements;bottom_elements=@bottom_cap_elements",
                ),
                applies: None,
                refused: "{why}",
                required: true,
            }],
            project: residuals(),
            joined: None,
        }],
    }
}
