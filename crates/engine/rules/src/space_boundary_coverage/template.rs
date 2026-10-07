//! `space-boundary-coverage` as a template: the measured
//! `boundary_coverage_off` read first (a space the service cannot measure
//! open once, for its reason), a boundary off the body's surface always a
//! finding relating the elements it bounds against, then each declared
//! check (the share covered at least `minimum_covered_share`, the area left
//! uncovered at most `maximum_uncovered_area`, the area covered twice at
//! most `maximum_overlap_area`), a straddling one open, and whatever one
//! space leaves open one outcome.

use axioval_engine::template::{
    Applies, Check, Condition, Decision, Expect, Form, FormCheck, Operand, ParameterDefault,
    Refusals, Service, Services, Template, TemplateValue, Term, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.space-boundary-coverage";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("minimum_covered_share", ParameterType::Number),
        ParameterDescriptor::optional("maximum_uncovered_area", ParameterType::Quantity),
        ParameterDescriptor::optional("maximum_overlap_area", ParameterType::Quantity),
        ParameterDescriptor::optional("plane_tolerance", ParameterType::Quantity),
    ]
}

/// A measured value of the space's coverage under the rule's plane
/// tolerance.
fn measured(name: &'static str, value: &'static str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Property {
            property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
            property: format!("{value};plane=@plane_tolerance"),
            of: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
}

/// A value read only to word an outcome.
fn words(name: &'static str, value: &'static str) -> TemplateValue {
    TemplateValue {
        expect: Some(Expect::Words),
        ..measured(name, value)
    }
}

const SHARE_OUTSIDE: &str = "`minimum_covered_share` lies outside 0 to 1";

/// What a rule's parameters must satisfy, in the order the capability
/// checked them.
fn declaration() -> Vec<Check> {
    vec![
        Check::NonNegative {
            parameters: &["minimum_covered_share"],
            message: SHARE_OUTSIDE,
        },
        Check::AtMost {
            parameters: &["minimum_covered_share"],
            value: 1.0,
            message: SHARE_OUTSIDE,
        },
        // An area of at least zero: of another dimension first.
        Check::Quantity {
            parameter: "maximum_uncovered_area",
            dimension: QuantityDimension::Area,
            message: "`maximum_uncovered_area` is not an area",
        },
        Check::NonNegative {
            parameters: &["maximum_uncovered_area"],
            message: "`maximum_uncovered_area` is negative",
        },
        Check::Quantity {
            parameter: "maximum_overlap_area",
            dimension: QuantityDimension::Area,
            message: "`maximum_overlap_area` is not an area",
        },
        Check::NonNegative {
            parameters: &["maximum_overlap_area"],
            message: "`maximum_overlap_area` is negative",
        },
        Check::AnyOf {
            parameters: &[
                "minimum_covered_share",
                "maximum_uncovered_area",
                "maximum_overlap_area",
            ],
            message: "`minimum_covered_share`, `maximum_uncovered_area` or \
                      `maximum_overlap_area` is required",
        },
        Check::Length {
            parameter: "plane_tolerance",
        },
    ]
}

/// Where the parameter is stated.
const fn stated(parameter: &'static [&'static str]) -> Applies {
    Applies {
        when: parameter,
        any: &[],
        condition: None,
    }
}

/// The share covered at least the minimum.
fn share() -> FormCheck {
    FormCheck {
        values: vec![
            measured("share", "boundary_coverage_share"),
            words("surface", "boundary_coverage_surface"),
            words("uncovered", "boundary_coverage_uncovered"),
        ],
        derived: Vec::new(),
        decision: Decision::Within {
            value: "share",
            minimum: Some(vec![Term::plus(Operand::Parameter(
                "minimum_covered_share",
            ))]),
            maximum: None,
            rounding: Vec::new(),
        },
        fail: "declared boundaries cover {share:percent} of the {surface:m2} surface, leaving \
               {uncovered:m2} uncovered; at least {minimum_covered_share:percent} required",
        undecided: "declared boundaries cover {share:percent} of the {surface:m2} surface, \
                    leaving {uncovered:m2} uncovered, which straddles the required \
                    {minimum_covered_share:percent}",
        related: None,
        grading: None,
        applies: Some(stated(&["minimum_covered_share"])),
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// The area left uncovered at most the maximum.
fn uncovered() -> FormCheck {
    FormCheck {
        values: vec![
            measured("uncovered", "boundary_coverage_uncovered"),
            words("surface", "boundary_coverage_surface"),
        ],
        derived: Vec::new(),
        decision: Decision::Within {
            value: "uncovered",
            minimum: None,
            maximum: Some(vec![Term::plus(Operand::Parameter(
                "maximum_uncovered_area",
            ))]),
            rounding: Vec::new(),
        },
        fail: "declared boundaries leave {uncovered:m2} of the {surface:m2} surface uncovered; \
               at most {maximum_uncovered_area:m2} allowed",
        undecided: "declared boundaries leave {uncovered:m2} of the {surface:m2} surface \
                    uncovered, which straddles the allowed {maximum_uncovered_area:m2}",
        related: None,
        grading: None,
        applies: Some(stated(&["maximum_uncovered_area"])),
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// The area covered twice at most the maximum, relating the elements of
/// the boundaries that surely overlap.
fn overlap() -> FormCheck {
    FormCheck {
        values: vec![measured("overlap", "boundary_coverage_overlap")],
        derived: Vec::new(),
        decision: Decision::Within {
            value: "overlap",
            minimum: None,
            maximum: Some(vec![Term::plus(Operand::Parameter("maximum_overlap_area"))]),
            rounding: Vec::new(),
        },
        fail: "declared boundaries overlap over {overlap:m2} of the surface{between}; at most \
               {maximum_overlap_area:m2} allowed",
        undecided: "declared boundaries overlap over {overlap:m2} of the surface{between}, \
                    which straddles the allowed {maximum_overlap_area:m2}",
        related: Some("overlap"),
        grading: None,
        applies: Some(stated(&["maximum_overlap_area"])),
        unless: None,
        quiet: false,
        ungraded: false,
    }
}

/// `space-boundary-coverage`, rebuilt as a composition with its outside
/// contract kept.
pub(crate) fn template() -> Template {
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
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "space-boundary-coverage",
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "plane_tolerance",
            value: ScalarValue::Quantity {
                value: 0.0,
                unit: "m".into(),
            },
            from: &[],
        }],
        declaration: declaration(),
        services: Some(Services {
            needs: vec![Service::BoundaryCoverage],
            message: "space-boundary coverage service is not registered",
            only: None,
            whole: false,
        }),
        texts: vec![Text {
            name: "between",
            when: Some(Condition::Noted { value: "overlap" }),
            text: " (boundaries {overlap:noted})",
        }],
        forms: vec![Form {
            when: &[],
            // A boundary off the surface covers nothing, whatever the
            // checks declared.
            values: vec![measured("off", "boundary_coverage_off"), zero],
            decision: Decision::Within {
                value: "off",
                minimum: None,
                maximum: Some(vec![Term::plus(Operand::Value("zero"))]),
                rounding: Vec::new(),
            },
            fail: "space boundary {off:noted} lies on no face of the space's body, so it covers \
                   nothing",
            undecided: "space boundary {off:noted} may lie on no face of the space's body",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: Some("off"),
            checks: vec![share(), uncovered(), overlap()],
            unless: Vec::new(),
            grading: None,
            joined: Some("; "),
            once: Vec::new(),
            project: Vec::new(),
        }],
    }
}
