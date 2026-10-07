//! `coordinate-consistency` as a template: the federation's reference
//! source read once per rule, then every source of the session judged by
//! the differences its coordinate system shows against the reference's,
//! their words joined into one finding on the source.

use axioval_engine::template::{
    Check, Condition, Decision, Form, Joined, ParameterDefault, Refusals, ScopeMessages,
    ScopeSources, Scopes, Service, Services, Template, TemplateValue, Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ScalarValue};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.coordinate-consistency";

/// The capability's parameter descriptor.
fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("reference", ParameterType::String),
        ParameterDescriptor::optional("length_tolerance", ParameterType::Number),
        ParameterDescriptor::optional("angle_tolerance", ParameterType::Number),
        ParameterDescriptor::optional("scale_tolerance", ParameterType::Number),
        ParameterDescriptor::optional("require_map_conversion", ParameterType::Boolean),
    ]
}

fn default(parameter: &'static str, value: ScalarValue) -> ParameterDefault {
    ParameterDefault {
        parameter,
        value,
        from: &[],
    }
}

/// A tolerance stated is not negative, as the capability worded it.
fn not_negative(parameters: &'static [&'static str], message: &'static str) -> Check {
    Check::NonNegative {
        parameters,
        message,
    }
}

/// The template.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "coordinate-consistency",
        refusals: Refusals::Rule,
        defaults: vec![
            default("length_tolerance", ScalarValue::Number { value: 0.001 }),
            default("angle_tolerance", ScalarValue::Number { value: 0.01 }),
            default("scale_tolerance", ScalarValue::Number { value: 0.0 }),
            default(
                "require_map_conversion",
                ScalarValue::Boolean { value: false },
            ),
        ],
        // In the capability's order: the three tolerances read, then each
        // checked, then the reference and the requirement read.
        declaration: vec![
            Check::Kind {
                parameter: "length_tolerance",
            },
            Check::Kind {
                parameter: "angle_tolerance",
            },
            Check::Kind {
                parameter: "scale_tolerance",
            },
            not_negative(
                &["length_tolerance"],
                "the length tolerance must be finite and not negative",
            ),
            not_negative(
                &["angle_tolerance"],
                "the angle tolerance must be finite and not negative",
            ),
            not_negative(
                &["scale_tolerance"],
                "the scale tolerance must be finite and not negative",
            ),
            Check::Kind {
                parameter: "reference",
            },
            Check::Kind {
                parameter: "require_map_conversion",
            },
        ],
        services: Some(Services {
            needs: vec![Service::CoordinateSystem],
            message: "coordinate-consistency: no coordinate-system service is registered",
        }),
        texts: vec![
            Text {
                name: "verdict",
                when: Some(Condition::Scope { value: "reference" }),
                text: "`{source}`, the reference, {found}",
            },
            Text {
                name: "verdict",
                when: None,
                text: "`{source}` does not share the coordinate system of `{reference:source}`: \
                       {found}",
            },
        ],
        forms: vec![form()],
    }
}

/// Every source judged against the reference.
fn form() -> Form {
    Form {
        when: &[],
        values: Vec::new(),
        decision: Decision::Joined(Joined {
            list: "coordinate_differences;reference=@reference;length=@length_tolerance;\
                   angle=@angle_tolerance;scale=@scale_tolerance;\
                   require_map=@require_map_conversion",
            found: "found",
            words: "finding",
            recorded: Some("recorded"),
            separator: "; ",
            refused: "coordinate-consistency: {why}",
        }),
        fail: "{verdict}",
        undecided: "coordinate-consistency: `{source}` against `{reference:source}`: {open}",
        members: None,
        table: None,
        scope: Some(Scopes {
            // No parameter: each source is a scope.
            across: "",
            disciplines: None,
            sources: ScopeSources::Every,
            needs: None,
            messages: ScopeMessages {
                source: "in source `{source}`",
                project: "in the project",
                no_source: "coordinate-consistency: the run checks no source",
                no_discipline: "coordinate-consistency: no source plays {disciplines}",
                undeclared: "coordinate-consistency: `{source}` declares no discipline",
                undeclared_member: "coordinate-consistency: `{source}` declares no discipline",
                unlisted: "coordinate-consistency: the objects of `{source}` cannot be listed: \
                           {why}",
                no_disciplines: "coordinate-consistency: source disciplines are not available",
            },
        }),
        derived: Vec::new(),
        related: None,
        checks: Vec::new(),
        unless: Vec::new(),
        grading: None,
        once: vec![axioval_engine::template::Once {
            value: TemplateValue {
                name: "reference",
                expression: Expression::Property {
                    property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
                    property: "coordinate_reference;reference=@reference".to_owned(),
                    of: None,
                    label: None,
                },
                expect: None,
                absent: None,
                mismatch: None,
            },
            applies: None,
            refused: "coordinate-consistency: {why}",
            required: true,
        }],
        joined: None,
        project: Vec::new(),
    }
}
