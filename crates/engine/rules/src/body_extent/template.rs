//! `body-extent` as a template: the measured `body_extent` along the
//! rule's axis, judged by the generic range judge against a stated length
//! within a tolerance or against a range, its bounds widened by the binary
//! rounding of the body's coordinates (`body_position`), the stated length
//! and the tolerance, exactly as the capability always allowed.

use axioval_engine::template::{
    Check, Decision, End, Expect, Form, Magnitude, Operand, ParameterDefault, Service, Services,
    Template, TemplateValue, Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};
use serde_json::json;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.body-extent";

fn expression(value: serde_json::Value) -> Expression {
    serde_json::from_value(value).expect("a built-in template's expression is well formed")
}

/// A measured value read with the rule's axis.
fn measured(name: &str) -> Expression {
    expression(json!({
        "kind": "property",
        "propertySet": axioval_ir::MEASURED_SET,
        "property": name,
    }))
}

/// The extent along the axis and the positions of its two ends.
fn extent_values() -> Vec<TemplateValue> {
    let value = |name, call: &str| TemplateValue {
        name,
        expression: measured(call),
        expect: None,
        absent: None,
        mismatch: None,
    };
    vec![
        value("extent", "body_extent;axis={axis}"),
        value("low", "body_position;axis={axis};end=low"),
        value("high", "body_position;axis={axis};end=high"),
    ]
}

/// The rounding allowance's magnitudes beside the bounds' own: the lowest
/// and highest coordinate of the body along the axis.
fn positions() -> [Magnitude; 2] {
    [
        Magnitude {
            end: End::Lower,
            operand: Operand::Value("low"),
        },
        Magnitude {
            end: End::Upper,
            operand: Operand::Value("high"),
        },
    ]
}

/// The form against a stated length within a tolerance.
fn stated_form() -> Form {
    Form {
        when: &["target_property"],
        values: {
            let mut values = extent_values();
            values.push(TemplateValue {
                name: "target",
                expression: expression(json!({
                    "kind": "property",
                    "propertySet": "{target_property.set}",
                    "property": "{target_property.name}",
                })),
                expect: Some(Expect::Length),
                absent: Some("{measured}; `{target_property}` is absent"),
                mismatch: Some("`{target_property}` is {target:stated}, not a length"),
            });
            values
        },
        decision: Decision::Within {
            value: "extent",
            minimum: Some(vec![
                Term::plus(Operand::Value("target")),
                Term::minus(Operand::Parameter("tolerance")),
            ]),
            maximum: Some(vec![
                Term::plus(Operand::Value("target")),
                Term::plus(Operand::Parameter("tolerance")),
            ]),
            rounding: {
                let mut rounding = positions().to_vec();
                rounding.push(Magnitude {
                    end: End::Lower,
                    operand: Operand::Value("target"),
                });
                rounding.push(Magnitude {
                    end: End::Lower,
                    operand: Operand::Parameter("tolerance"),
                });
                rounding
            },
        },
        fail: "{measured}{stated}",
        undecided: "{measured}, which straddles the stated length{stated}",
        members: None,
        table: None,
        scope: None,
        derived: Vec::new(),
        related: None,
        checks: Vec::new(),
    }
}

/// The form against a range.
fn range_form() -> Form {
    Form {
        when: &[],
        values: extent_values(),
        decision: Decision::Within {
            value: "extent",
            minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
            maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
            rounding: {
                let mut rounding = positions().to_vec();
                rounding.push(Magnitude {
                    end: End::Upper,
                    operand: Operand::Value("extent"),
                });
                rounding
            },
        },
        fail: "{measured}; required {bound}",
        undecided: "{measured}, which straddles {bound}",
        members: None,
        table: None,
        scope: None,
        derived: Vec::new(),
        related: None,
        checks: Vec::new(),
    }
}

/// What a rule's parameters must satisfy, as the capability checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Choice {
            parameter: "axis",
            options: &["right", "forward", "up"],
        },
        Check::Length {
            parameter: "tolerance",
        },
        Check::Length {
            parameter: "minimum",
        },
        Check::Length {
            parameter: "maximum",
        },
        Check::Exclusive {
            one: &["target_property"],
            other: &["minimum", "maximum"],
            message: "declare either `target_property` or `minimum`/`maximum`, not both",
        },
        Check::AnyOf {
            parameters: &["target_property", "minimum", "maximum"],
            message: "`target_property`, `minimum` or `maximum` is required",
        },
        Check::Requires {
            parameter: "tolerance",
            with: &["target_property"],
            message: "`tolerance` applies to `target_property` only",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "`minimum` exceeds `maximum`",
        },
    ]
}

/// The message parts.
fn texts() -> Vec<Text> {
    vec![
        Text {
            name: "measured",
            when: None,
            text: "body extent along `{axis}` is {extent:length}",
        },
        Text {
            name: "within",
            when: Some(axioval_engine::template::Condition::Positive {
                parameter: "tolerance",
            }),
            text: " within {tolerance:length}",
        },
        Text {
            name: "stated",
            when: None,
            text: "; `{target_property}` states {target:length}{within}",
        },
    ]
}

/// `body-extent`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("axis", ParameterType::String),
            ParameterDescriptor::optional("target_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum", ParameterType::Quantity),
        ],
        grades: false,
        name: "body-extent",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "tolerance",
            value: ScalarValue::Quantity {
                value: 0.0,
                unit: "m".into(),
            },
        }],
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::ObjectFrame, Service::VerticalExtent],
            message: "body-extent needs the object-frame and vertical-extent services",
        }),
        texts: texts(),
        forms: vec![stated_form(), range_form()],
    }
}
