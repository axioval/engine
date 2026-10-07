//! `related-count` as a template: the count of the objects
//! `related_selector` picks (every object where unstated) that each
//! anchor reaches, kept to those sharing its ends with `same_ends`,
//! widened by those whose selection is undecided, judged and graded by the
//! range judge against `minimum` and `maximum`.

use axioval_engine::template::{
    Check, Decision, Form, Members, Operand, Template, TemplateValue, Term, UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{AggregateFunction, Expression};

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.related-count";

/// The selector parameter picking an anchor's members.
const MEMBERS: &str = "related_selector";

/// The one form: the anchor's members counted, the undecided ones
/// widening the count.
fn form() -> Form {
    Form {
        when: &[],
        values: vec![TemplateValue {
            name: "count",
            expression: Expression::Aggregate {
                function: AggregateFunction::Count,
                over: Members::source(MEMBERS),
                filter: None,
                value: None,
                label: Some("related objects".into()),
            },
            expect: None,
            absent: None,
            mismatch: None,
        }],
        decision: Decision::Within {
            value: "count",
            minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
            maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
            rounding: Vec::new(),
        },
        fail: "{count:least} related object(s) {relation}; required {required}",
        undecided: "{count:least} related object(s) {relation} and {undecided} more that may \
                    count; required {required}",
        members: Some(Members {
            selector: MEMBERS,
            undecided: UndecidedMembers::Widen,
            every_when_unstated: true,
            same_ends: Some("same_ends"),
            more: &[],
            checks: Vec::new(),
        }),
        table: None,
        scope: None,
        unless: Vec::new(),
        grading: None,
        derived: Vec::new(),
        related: None,
        checks: Vec::new(),
        once: Vec::new(),
        joined: None,
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
            parameters: &["minimum"],
            message: "minimum is negative",
        },
        Check::NonNegative {
            parameters: &["maximum"],
            message: "maximum is negative",
        },
        Check::Ordered {
            low: "minimum",
            high: "maximum",
            message: "minimum exceeds maximum",
        },
        Check::Kind { parameter: MEMBERS },
        Check::Traversal {
            with: &[],
            message: "",
        },
        Check::Path {
            parameter: "same_ends",
        },
    ]
}

/// `related-count`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::optional(MEMBERS, ParameterType::Selector),
            ParameterDescriptor::optional("minimum", ParameterType::Integer),
            ParameterDescriptor::optional("maximum", ParameterType::Integer),
            ParameterDescriptor::optional("same_ends", ParameterType::StringList),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect(),
        grades: true,
        name: "related-count",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: None,
        texts: Vec::new(),
        forms: vec![form()],
    }
}
