//! `plan-coverage` as a template: the measured `plan_coverage` of each
//! subject (the share of its footprint within the candidate covering most of
//! it, searched along the rule's traversal) at least `minimum_ratio`.

use axioval_engine::template::{
    Check, Condition, Decision, Form, Operand, Refusals, Template, TemplateValue, Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::Expression;

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.plan-coverage";

/// The search, every rule parameter it reads named as the rule names it.
macro_rules! search {
    () => {
        "plan_coverage;candidates=@candidate_selector;minimum=@minimum_ratio;\
         relationship=@relationship;direction=@direction;path=@path;\
         follow_chain=@follow_chain;\
         skip_absent_relationship_ends=@skip_absent_relationship_ends"
    };
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("candidate_selector", ParameterType::Selector),
        ParameterDescriptor::required("minimum_ratio", ParameterType::Number),
    ]
    .into_iter()
    .chain(traversal_parameters())
    .collect()
}

fn value(name: &'static str, property: &'static str) -> TemplateValue {
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

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    const SHARE: &str = "minimum_ratio must lie in (0, 1]";
    vec![
        Check::Kind {
            parameter: "minimum_ratio",
        },
        Check::Finite {
            parameters: &["minimum_ratio"],
            above: Some(0.0),
            at_least: None,
            message: SHARE,
        },
        Check::AtMost {
            parameters: &["minimum_ratio"],
            value: 1.0,
            message: SHARE,
        },
        Check::Required {
            parameter: "candidate_selector",
        },
        Check::Traversal {
            with: &[],
            message: "",
        },
    ]
}

/// `plan-coverage`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "plan-coverage",
        refusals: Refusals::Rule,
        defaults: Vec::new(),
        declaration: declaration(),
        services: None,
        texts: vec![
            Text {
                name: "candidate",
                when: Some(Condition::Not {
                    condition: &Condition::Cites { value: "share" },
                }),
                text: "candidate (there are none)",
            },
            Text {
                name: "candidate",
                when: None,
                text: "candidate",
            },
        ],
        forms: vec![Form {
            when: &[],
            values: vec![value("share", search!())],
            decision: Decision::Within {
                value: "share",
                minimum: Some(vec![Term::plus(Operand::Parameter("minimum_ratio"))]),
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "at most {share:share} of the footprint lies within any {candidate}; \
                   required {minimum_ratio}",
            undecided: "coverage of {minimum_ratio} cannot be decided from the measured areas",
            members: None,
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: Vec::new(),
            related: Some("share"),
            checks: Vec::new(),
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
