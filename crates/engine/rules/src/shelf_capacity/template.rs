//! `shelf-capacity` as a template: the measured `shelf_length` and
//! `shelf_clear_height` of each space, both measured with the rule's own
//! arrangement and with the doors and openings its selectors pick
//! (`doors=@door_selector`), the clear height judged against the
//! shelving's top and the running metres against the minimum, each its own
//! finding.

use axioval_engine::template::{
    Check, Decision, Form, FormCheck, Operand, Refusals, Template, TemplateValue, Term,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.shelf-capacity";

/// How every measured value of the template names the rule's arrangement
/// and the selectors picking the doors, openings and spaces: one request
/// per space, as the capability measured it.
const SHELVING: &str = "depth=@shelf_depth_metres;horizontal=@horizontal_spacing_metres;\
                        vertical=@vertical_spacing_metres;bottom=@bottom_elevation_metres;\
                        top=@top_elevation_metres;clearance=@door_clearance_metres;\
                        access=@access_path;doors=@door_selector;openings=@opening_selector;\
                        spaces=@space_selector";

const GEOMETRY: &str = "shelf geometry parameters are missing or not physically realisable";
const NEED_PATH: &str =
    "shelf-capacity: `door_selector`, `opening_selector` and `space_selector` need `access_path`";

/// The measured value `name` in metres as a plain number, as the
/// capability's parameters state lengths.
fn metres(name: &'static str, measured: &str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Divide {
            left: Box::new(Expression::Property {
                property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
                property: format!("{measured};{SHELVING}"),
                of: None,
                label: None,
            }),
            right: Box::new(Expression::Literal {
                value: ScalarValue::Quantity {
                    value: 1.0,
                    unit: "m".into(),
                },
                label: None,
            }),
            label: Some("in metres".into()),
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// `value` at least the parameter `minimum`, as declared: no rounding
/// allowance, as the capability compared.
fn at_least(value: &'static str, minimum: &'static str) -> Decision {
    Decision::Within {
        value,
        minimum: Some(vec![Term::plus(Operand::Parameter(minimum))]),
        maximum: None,
        rounding: Vec::new(),
    }
}

/// `shelf-capacity`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("minimum_running_metres", ParameterType::Number),
            ParameterDescriptor::required("shelf_depth_metres", ParameterType::Number),
            ParameterDescriptor::required("horizontal_spacing_metres", ParameterType::Number),
            ParameterDescriptor::required("vertical_spacing_metres", ParameterType::Number),
            ParameterDescriptor::required("bottom_elevation_metres", ParameterType::Number),
            ParameterDescriptor::required("top_elevation_metres", ParameterType::Number),
            ParameterDescriptor::required("door_clearance_metres", ParameterType::Number),
            ParameterDescriptor::required("access_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
        ],
        grades: false,
        name: "shelf-capacity",
        // The capability judged its declaration for each selected space.
        refusals: Refusals::Objects,
        defaults: Vec::new(),
        declaration: declaration(),
        // The linear-quantity service is the measured values'; without it
        // each space is left open as the capability left it.
        services: None,
        texts: Vec::new(),
        forms: vec![form()],
    }
}

/// The capability's declaration checks, in its order and words.
fn declaration() -> Vec<Check> {
    vec![
        Check::Finite {
            parameters: &["minimum_running_metres"],
            above: None,
            at_least: Some(0.0),
            message: "shelf capacity minimum must be a finite, non-negative number",
        },
        Check::Finite {
            parameters: &[
                "shelf_depth_metres",
                "horizontal_spacing_metres",
                "vertical_spacing_metres",
            ],
            above: Some(0.0),
            at_least: None,
            message: GEOMETRY,
        },
        Check::Finite {
            parameters: &["bottom_elevation_metres", "door_clearance_metres"],
            above: None,
            at_least: Some(0.0),
            message: GEOMETRY,
        },
        Check::Finite {
            parameters: &["top_elevation_metres"],
            above: None,
            at_least: None,
            message: GEOMETRY,
        },
        Check::Increasing {
            low: "bottom_elevation_metres",
            high: "top_elevation_metres",
            message: GEOMETRY,
        },
        Check::Requires {
            parameter: "door_selector",
            with: &["access_path"],
            message: NEED_PATH,
        },
        Check::Requires {
            parameter: "opening_selector",
            with: &["access_path"],
            message: NEED_PATH,
        },
        Check::Requires {
            parameter: "space_selector",
            with: &["access_path"],
            message: NEED_PATH,
        },
        Check::AnyOf {
            parameters: &["access_path"],
            message: "shelf-capacity: parameter `access_path` is required",
        },
        Check::AnyOf {
            parameters: &["door_selector", "opening_selector"],
            message: "shelf-capacity: `access_path` needs `door_selector`, \
                          `opening_selector` or both",
        },
    ]
}

/// The running metres against the minimum, after the clear height against
/// the shelving's top.
fn form() -> Form {
    Form {
        when: &[],
        values: vec![metres("length", "shelf_length")],
        decision: at_least("length", "minimum_running_metres"),
        fail: "shelf running metres {length:upper3} below required \
                   {minimum_running_metres:fixed3}",
        undecided: "measured shelf length {length:length} spans the required minimum \
                        {minimum_running_metres:fixed3}",
        members: None,
        table: None,
        scope: None,
        unless: Vec::new(),
        grading: None,
        derived: Vec::new(),
        // The doors and openings whose clearances were kept free.
        related: Some("length"),
        checks: vec![FormCheck {
            derived: Vec::new(),
            applies: None,
            values: vec![metres("height", "shelf_clear_height")],
            decision: at_least("height", "top_elevation_metres"),
            fail: "space too low for the shelving: clear height {height:length} below the \
                       shelving's top elevation {top_elevation_metres:length}",
            undecided: "clear height {height:length} may or may not reach the shelving's \
                            top elevation {top_elevation_metres:length}",
            related: None,
            grading: None,
            unless: None,
            quiet: false,
            ungraded: false,
        }],
        once: Vec::new(),
        joined: None,
        project: Vec::new(),
    }
}
