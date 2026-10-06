//! `plan-area` as a template: the measured `plan_area` of each selected
//! object, or summed over the members an anchor reaches, judged by the
//! range judge against `minimum` and `maximum`, graded, and reported in
//! the table `areas`.

use axioval_engine::template::{
    Check, Column, Condition, Decision, Form, Members, Operand, ParameterDefault, Table, Template,
    TemplateValue, Term, Text, UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{AggregateFunction, Expression, ScalarValue};

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.plan-area";

/// The selector parameter picking an anchor's members.
const MEMBERS: &str = "member_selector";

/// The area of the object in scope, measured as the rule's `measure` says.
fn area() -> Expression {
    Expression::Property {
        property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
        property: "plan_area;measure={measure}".to_owned(),
        of: None,
        label: None,
    }
}

/// An area in square metres as a plain number, as the capability's bounds
/// are stated.
fn value(area: Expression) -> Vec<TemplateValue> {
    vec![TemplateValue {
        name: "area",
        expression: Expression::Divide {
            left: Box::new(area),
            right: Box::new(Expression::Literal {
                value: ScalarValue::Quantity {
                    value: 1.0,
                    unit: "m2".into(),
                },
                label: None,
            }),
            label: Some("in square metres".into()),
        },
        expect: None,
        absent: None,
        mismatch: None,
    }]
}

/// The area within `minimum` and `maximum`, as declared: no rounding
/// allowance, as the capability judged.
fn range() -> Decision {
    Decision::Within {
        value: "area",
        minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
        maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
        rounding: Vec::new(),
    }
}

/// The table `areas`, one row per object whose area was measured.
fn areas() -> Table {
    Table {
        name: "areas",
        columns: vec![Column {
            id: "{column}",
            value: "area",
            dimension: QuantityDimension::Area,
        }],
    }
}

/// The summed areas of the members an anchor reaches.
fn members_form() -> Form {
    Form {
        when: &[MEMBERS],
        values: value(Expression::Aggregate {
            function: AggregateFunction::Sum,
            over: Members::source(MEMBERS),
            filter: None,
            value: Some(Box::new(area())),
            label: None,
        }),
        decision: range(),
        fail: "summed {noun} of the members is {area:area} m²; required {bound:plain} m²",
        undecided: "summed {noun} of the members is {area:area} m², which straddles the bound \
                    {bound:plain} m²",
        members: Some(Members {
            selector: MEMBERS,
            undecided: UndecidedMembers::OnlyExcess {
                message: "{undecided} member(s) {relation} cannot be assigned",
            },
            every_when_unstated: false,
            same_ends: None,
            more: &[],
        }),
        table: Some(areas()),
        scope: None,
        derived: Vec::new(),
    }
}

/// The object's own area.
fn own_form() -> Form {
    Form {
        when: &[],
        values: value(area()),
        decision: range(),
        fail: "{noun} is {area:area} m²; required {bound:plain} m²",
        undecided: "{noun} is {area:area} m², which straddles the bound {bound:plain} m²",
        members: None,
        table: Some(areas()),
        scope: None,
        derived: Vec::new(),
    }
}

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::Kind {
            parameter: "minimum",
        },
        Check::Kind {
            parameter: "maximum",
        },
        Check::AnyOf {
            parameters: &["minimum", "maximum"],
            message: "minimum or maximum is required",
        },
        Check::NonNegative {
            parameters: &["minimum", "maximum"],
            message: "an area bound is negative",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "minimum exceeds maximum",
        },
        Check::Kind { parameter: MEMBERS },
        Check::Traversal {
            with: &[MEMBERS],
            message: "a relationship reaches members only with `member_selector`",
        },
        Check::Choice {
            parameter: "measure",
            options: &["footprint", "facade"],
        },
    ]
}

/// The message parts: what is measured, and the table column holding it.
fn texts() -> Vec<Text> {
    let facade = Some(Condition::Equals {
        parameter: "measure",
        value: "facade",
    });
    vec![
        Text {
            name: "noun",
            when: facade,
            text: "facade area",
        },
        Text {
            name: "noun",
            when: None,
            text: "plan area",
        },
        Text {
            name: "column",
            when: facade,
            text: "facade_area",
        },
        Text {
            name: "column",
            when: None,
            text: "plan_area",
        },
    ]
}

/// `plan-area`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::optional("minimum", ParameterType::Number),
            ParameterDescriptor::optional("maximum", ParameterType::Number),
            ParameterDescriptor::optional(MEMBERS, ParameterType::Selector),
            ParameterDescriptor::optional("measure", ParameterType::String),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect(),
        grades: true,
        name: "plan-area",
        defaults: vec![ParameterDefault {
            parameter: "measure",
            value: ScalarValue::String {
                value: "footprint".into(),
            },
        }],
        declaration: declaration(),
        services: None,
        texts: texts(),
        forms: vec![members_form(), own_form()],
    }
}
