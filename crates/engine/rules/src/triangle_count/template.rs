//! `triangle-count` as a template: the measured `triangle_count` of each
//! object's mesh, judged by the generic range judge against `maximum`,
//! and a finding on a tessellation saying the count depends on it.

use axioval_engine::template::{
    Check, Condition, Decision, Form, Operand, Service, Services, Template, TemplateValue, Term,
    Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::Expression;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.triangle-count";

/// `triangle-count`, rebuilt as a composition with its outside contract
/// kept.
pub(crate) fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![ParameterDescriptor::required(
            "maximum",
            ParameterType::Integer,
        )],
        grades: false,
        name: "triangle-count",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: Vec::new(),
        declaration: vec![Check::Count {
            parameter: "maximum",
        }],
        services: Some(Services {
            needs: vec![Service::TriangleCount],
            message: "triangle-count service is not registered",
        }),
        texts: vec![Text {
            name: "tessellated",
            when: Some(Condition::Inexact { value: "count" }),
            text: "; the mesh tessellates curved faces, so the count depends on the host's \
                   tessellation",
        }],
        forms: vec![Form {
            when: &[],
            values: vec![TemplateValue {
                name: "count",
                expression: Expression::Property {
                    property_set: Some(axioval_ir::MEASURED_SET.to_owned()),
                    property: "triangle_count".to_owned(),
                    of: None,
                    label: None,
                },
                expect: None,
                absent: None,
                mismatch: None,
            }],
            decision: Decision::Within {
                value: "count",
                minimum: None,
                maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
                rounding: Vec::new(),
            },
            fail: "mesh has {count} triangles; at most {maximum} allowed{tessellated}",
            undecided: "mesh has {count} triangles, which straddles at most {maximum}{tessellated}",
            members: None,
            table: None,
            scope: None,
            unless: Vec::new(),
            grading: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            once: Vec::new(),
            joined: None,
            project: Vec::new(),
        }],
    }
}
