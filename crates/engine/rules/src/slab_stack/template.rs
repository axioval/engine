//! `slab-stack-spacing` as a template: each selected slab's measured
//! `stack_distance` to the next slab up in its stack, nothing judged where
//! none stacks above, within each measure's bounds, and against the measured
//! `stack_prevailing` of its stack where the measure must be consistent.

use axioval_engine::template::{
    Applies, Check, Condition, Decision, Expect, Form, FormCheck, Operand, ParameterDefault,
    Reference, Refusals, Service, Services, Template, TemplateValue, Term,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.slab-stack-spacing";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![ParameterDescriptor::required(
        "minimum_overlap_ratio",
        ParameterType::Number,
    )];
    for measure in ["top_to_top", "bottom_to_bottom", "top_to_bottom"] {
        for bound in ["minimum", "maximum"] {
            parameters.push(ParameterDescriptor::optional(
                format!("{measure}_{bound}"),
                ParameterType::Quantity,
            ));
        }
    }
    parameters.push(ParameterDescriptor::optional(
        "consistent",
        ParameterType::StringList,
    ));
    parameters.push(ParameterDescriptor::optional(
        "tolerance",
        ParameterType::Quantity,
    ));
    parameters
}

/// One measure: its name, how messages name it, and the values reading its
/// distance and its stack's prevailing one.
struct Measure {
    name: &'static str,
    distance: &'static str,
    prevailing: &'static str,
    minimum: &'static str,
    maximum: &'static str,
    fail: &'static str,
    undecided: &'static str,
    differs: &'static str,
    unsure: &'static str,
}

macro_rules! measure {
    ($name:literal, $label:literal) => {
        Measure {
            name: $name,
            distance: concat!(
                "stack_distance;measure=",
                $name,
                ";slabs=@selection;ratio=@minimum_overlap_ratio"
            ),
            prevailing: concat!(
                "stack_prevailing;measure=",
                $name,
                ";slabs=@selection;ratio=@minimum_overlap_ratio;tolerance=@tolerance"
            ),
            minimum: concat!($name, "_minimum"),
            maximum: concat!($name, "_maximum"),
            fail: concat!(
                $label,
                " to {distance:cited} is {distance:length}; required {bound}"
            ),
            undecided: concat!(
                $label,
                " to {distance:cited} is {distance:length}, which straddles the bound {bound}"
            ),
            differs: concat!(
                $label,
                " to {distance:cited} is {distance:length}, which differs from the prevailing \
                 {prevailing:length} in this stack"
            ),
            unsure: concat!(
                $label,
                " to {distance:cited} is {distance:length}; whether it equals the prevailing \
                 {prevailing:length} within {tolerance:length} cannot be decided"
            ),
        }
    };
}

const MEASURES: [Measure; 3] = [
    measure!("top_to_top", "top-to-top distance"),
    measure!("bottom_to_bottom", "bottom-to-bottom distance"),
    measure!("top_to_bottom", "clear distance from top to underside"),
];

fn measured(name: &'static str, property: &'static str, expect: Option<Expect>) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: property.to_owned(),
            of: None,
            label: None,
        },
        expect,
        absent: None,
        mismatch: None,
    }
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    const SHARE: &str = "minimum_overlap_ratio must lie in (0, 1]";
    let mut checks = vec![
        Check::Kind {
            parameter: "minimum_overlap_ratio",
        },
        Check::Finite {
            parameters: &["minimum_overlap_ratio"],
            above: Some(0.0),
            at_least: None,
            message: SHARE,
        },
        Check::AtMost {
            parameters: &["minimum_overlap_ratio"],
            value: 1.0,
            message: SHARE,
        },
    ];
    for (measure, ordered) in MEASURES.iter().zip([
        "top_to_top_minimum exceeds top_to_top_maximum",
        "bottom_to_bottom_minimum exceeds bottom_to_bottom_maximum",
        "top_to_bottom_minimum exceeds top_to_bottom_maximum",
    ]) {
        for parameter in [measure.minimum, measure.maximum] {
            checks.push(Check::NonNegativeLength {
                parameter,
                message: match parameter {
                    "top_to_top_minimum" => "top_to_top_minimum must be a non-negative length",
                    "top_to_top_maximum" => "top_to_top_maximum must be a non-negative length",
                    "bottom_to_bottom_minimum" => {
                        "bottom_to_bottom_minimum must be a non-negative length"
                    }
                    "bottom_to_bottom_maximum" => {
                        "bottom_to_bottom_maximum must be a non-negative length"
                    }
                    "top_to_bottom_minimum" => {
                        "top_to_bottom_minimum must be a non-negative length"
                    }
                    _ => "top_to_bottom_maximum must be a non-negative length",
                },
            });
        }
        checks.push(Check::Ordered {
            low: measure.minimum,
            high: measure.maximum,
            message: ordered,
        });
    }
    checks.extend([
        Check::AmongEach {
            parameter: "consistent",
            options: &["top_to_top", "bottom_to_bottom", "top_to_bottom"],
            message: "consistent names `{value}`; expected top_to_top, bottom_to_bottom or \
                      top_to_bottom",
        },
        Check::DeclaresListed {
            parameters: &[
                "top_to_top_minimum",
                "top_to_top_maximum",
                "bottom_to_bottom_minimum",
                "bottom_to_bottom_maximum",
                "top_to_bottom_minimum",
                "top_to_bottom_maximum",
                "consistent",
            ],
            message: "declare a minimum, a maximum or a consistent measure",
        },
        Check::NonNegativeLength {
            parameter: "tolerance",
            message: "tolerance must be a non-negative length",
        },
    ]);
    checks
}

/// The checks of each measure: its bounds, then its consistency within the
/// stack where the rule asks for it.
fn checks() -> Vec<FormCheck> {
    let mut checks = Vec::new();
    for measure in &MEASURES {
        let distance = || measured("distance", measure.distance, None);
        checks.push(FormCheck {
            values: vec![distance()],
            decision: Decision::Within {
                value: "distance",
                minimum: Some(vec![Term::plus(Operand::Parameter(measure.minimum))]),
                maximum: Some(vec![Term::plus(Operand::Parameter(measure.maximum))]),
                rounding: Vec::new(),
            },
            fail: measure.fail,
            undecided: measure.undecided,
            related: Some("distance"),
            grading: None,
            applies: Some(Applies {
                when: &[],
                any: match measure.name {
                    "top_to_top" => &["top_to_top_minimum", "top_to_top_maximum"],
                    "bottom_to_bottom" => &["bottom_to_bottom_minimum", "bottom_to_bottom_maximum"],
                    _ => &["top_to_bottom_minimum", "top_to_bottom_maximum"],
                },
                condition: None,
            }),
        });
        checks.push(FormCheck {
            values: vec![
                distance(),
                measured("prevailing", measure.prevailing, Some(Expect::Optional)),
            ],
            decision: Decision::Near {
                value: "distance",
                reference: Reference::Value("prevailing"),
                tolerance: Operand::Parameter("tolerance"),
            },
            fail: measure.differs,
            undecided: measure.unsure,
            related: Some("distance"),
            grading: None,
            applies: Some(Applies {
                when: &[],
                any: &[],
                condition: Some(Condition::Lists {
                    parameter: "consistent",
                    value: measure.name,
                }),
            }),
        });
    }
    checks
}

/// `slab-stack-spacing`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "slab-stack-spacing",
        refusals: Refusals::ServicesPerObject,
        defaults: vec![ParameterDefault {
            parameter: "tolerance",
            value: ScalarValue::Quantity {
                value: 0.001,
                unit: "m".to_owned(),
            },
        }],
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::VerticalExtent, Service::PlanArea],
            message: "slab-stack-spacing needs the vertical-extent and plan-area services",
        }),
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            // The slab next up, the stack's slabs decided first: nothing is
            // judged where none stacks above.
            values: vec![measured(
                "next",
                MEASURES[0].distance,
                Some(Expect::Optional),
            )],
            decision: Decision::Within {
                value: "next",
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
            checks: checks(),
        }],
    }
}
