//! `slab-contact` as a template: the measured `contact_share` of each
//! subject's face at least `minimum_contact_ratio`, a shortfall graded by
//! the gap to the nearest candidate (no contact at all) or by the share's
//! part of the minimum, and a subject on a storey the rule leaves out
//! unjudged.

use axioval_engine::template::{
    Applies, Band, Check, Condition, Decision, Derived, Form, Grading, Operand, Refusals, Service,
    Services, Template, TemplateValue, Term, Text, Unless,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::Severity;
use axioval_ir::contract::Expression;

use crate::support::traversal_parameters;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.slab-contact";

/// The contact the values measure: the rule's counterparts, side and
/// tolerances, each named as the rule names it.
macro_rules! contact {
    ($name:literal) => {
        concat!(
            $name,
            ";with=@counterparts;side=@contact_side;gap=@maximum_gap_metres;\
             intersection=@maximum_intersection_metres;\
             polygon=@minimum_polygon_area_square_metres"
        )
    };
}

/// Whether the subject lies on the end storey `end` names, along the rule's
/// traversal.
macro_rules! storey_end {
    ($end:literal) => {
        concat!(
            "storey_end;end=",
            $end,
            ";storeys=@storey_selector;relationship=@relationship;direction=@direction;\
             path=@path;follow_chain=@follow_chain;\
             skip_absent_relationship_ends=@skip_absent_relationship_ends"
        )
    };
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::required("minimum_contact_ratio", ParameterType::Number),
        ParameterDescriptor::required("contact_side", ParameterType::String),
        ParameterDescriptor::required("maximum_gap_metres", ParameterType::Number),
        ParameterDescriptor::required("maximum_intersection_metres", ParameterType::Number),
        ParameterDescriptor::required("minimum_polygon_area_square_metres", ParameterType::Number),
        ParameterDescriptor::optional("counterparts", ParameterType::Selector),
        ParameterDescriptor::optional("skip_top_storey", ParameterType::Boolean),
        ParameterDescriptor::optional("skip_bottom_storey", ParameterType::Boolean),
        ParameterDescriptor::optional("storey_selector", ParameterType::Selector),
    ]
    .into_iter()
    .chain(traversal_parameters())
    .collect()
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

/// The flags that leave out a storey.
const SKIPPING: &[&str] = &["skip_top_storey", "skip_bottom_storey"];

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    const SHARE: &str = "minimum_contact_ratio must lie in (0, 1]";
    vec![
        Check::Required {
            parameter: "minimum_contact_ratio",
        },
        Check::Finite {
            parameters: &["minimum_contact_ratio"],
            above: Some(0.0),
            at_least: None,
            message: SHARE,
        },
        Check::AtMost {
            parameters: &["minimum_contact_ratio"],
            value: 1.0,
            message: SHARE,
        },
        Check::Required {
            parameter: "contact_side",
        },
        Check::Among {
            parameter: "contact_side",
            options: &["above", "below"],
            message: "contact_side `{value}` is unsupported",
        },
        Check::Required {
            parameter: "maximum_gap_metres",
        },
        Check::Required {
            parameter: "maximum_intersection_metres",
        },
        Check::Required {
            parameter: "minimum_polygon_area_square_metres",
        },
        // The contact service's own refusal of the tolerances.
        Check::Finite {
            parameters: &[
                "maximum_gap_metres",
                "maximum_intersection_metres",
                "minimum_polygon_area_square_metres",
            ],
            above: None,
            at_least: Some(0.0),
            message: "contact areas must be finite, non-negative and contained",
        },
        Check::Kind {
            parameter: "skip_top_storey",
        },
        Check::Kind {
            parameter: "skip_bottom_storey",
        },
        Check::When {
            flags: SKIPPING,
            check: &Check::Kind {
                parameter: "storey_selector",
            },
        },
        Check::When {
            flags: SKIPPING,
            check: &Check::AnyOf {
                parameters: &["storey_selector"],
                message: "skipping a storey needs `storey_selector` to say what a storey is",
            },
        },
        Check::When {
            flags: SKIPPING,
            check: &Check::Traversal {
                with: &[],
                message: "",
            },
        },
        Check::When {
            flags: SKIPPING,
            check: &Check::AnyOf {
                parameters: &["relationship", "path"],
                message: "skipping a storey needs a `relationship` or `path` to each subject's \
                          storey",
            },
        },
        Check::Kind {
            parameter: "counterparts",
        },
    ]
}

/// Nothing touches the face: no part of it is in contact.
const NONE: Condition = Condition::Zero { value: "share" };

/// The severity of a shortfall: no contact by the gap to the nearest
/// candidate (none found the most serious), a partial one by the share's
/// part of the minimum.
fn bands() -> Vec<Band> {
    let band = |severity, when| Band {
        severity,
        when: Some(when),
    };
    vec![
        band(
            Severity::Error,
            Condition::All {
                conditions: &[NONE, Condition::Absent { value: "gap" }],
            },
        ),
        band(
            Severity::Info,
            Condition::All {
                conditions: &[
                    NONE,
                    Condition::Below {
                        value: "gap",
                        than: 0.1,
                    },
                ],
            },
        ),
        band(
            Severity::Error,
            Condition::All {
                conditions: &[
                    NONE,
                    Condition::Above {
                        value: "gap",
                        than: 0.5,
                    },
                ],
            },
        ),
        band(Severity::Warning, NONE),
        band(
            Severity::Info,
            Condition::Above {
                value: "relative",
                than: 0.9,
            },
        ),
        band(
            Severity::Error,
            Condition::Below {
                value: "relative",
                than: 0.3,
            },
        ),
        Band {
            severity: Severity::Warning,
            when: None,
        },
    ]
}

fn unless(flag: &'static [&'static str], name: &'static str, end: &'static str) -> Unless {
    Unless {
        applies: Applies {
            when: flag,
            any: &[],
            condition: None,
        },
        value: measured(name, end),
    }
}

/// `slab-contact`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    let share = measured("share", contact!("contact_share"));
    let minimum = TemplateValue {
        name: "minimum",
        expression: Expression::Parameter {
            name: "minimum_contact_ratio".to_owned(),
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    };
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "slab-contact",
        refusals: Refusals::Prefixed {
            prefix: "slab-contact declaration is invalid",
        },
        defaults: Vec::new(),
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::Contact],
            message: "contact service is not registered",
        }),
        texts: vec![
            Text {
                name: "shortfall",
                when: Some(NONE),
                text: "no contact",
            },
            Text {
                name: "shortfall",
                when: None,
                text: "contact ratio {share:lower4} below required \
                       {minimum_contact_ratio:fixed4}",
            },
        ],
        forms: vec![Form {
            when: &[],
            values: vec![share],
            decision: Decision::Within {
                value: "share",
                minimum: Some(vec![Term::plus(Operand::Parameter(
                    "minimum_contact_ratio",
                ))]),
                maximum: None,
                rounding: Vec::new(),
            },
            fail: "{shortfall}",
            undecided: "contact ratio {share:lower4} is below required \
                        {minimum_contact_ratio:fixed4}, but the counterpart selection is \
                        undecided for {undecided:least} object(s) that could support the face",
            members: None,
            table: None,
            scope: None,
            unless: vec![
                unless(&["skip_top_storey"], "top", storey_end!("top")),
                unless(&["skip_bottom_storey"], "bottom", storey_end!("bottom")),
            ],
            grading: Some(Grading {
                values: vec![measured("gap", contact!("contact_gap")), minimum],
                // The share's part of the minimum.
                derived: vec![Derived::Ratio {
                    name: "relative",
                    numerator: "share",
                    denominator: "minimum",
                    zero: "",
                }],
                bands: bands(),
                // How many counterparts the selection leaves undecided.
                undecided: vec![measured(
                    "undecided",
                    "undecided_count;objects=@counterparts",
                )],
            }),
            derived: Vec::new(),
            // What the face rests on.
            related: Some("share"),
            checks: Vec::new(),
            once: Vec::new(),
        }],
    }
}
