//! `opening-area` as a template: where a host states either side area, its
//! openings (`opening_area`) take the stated gross area less the net area,
//! within the tolerance and the rounding, judged by the truth judge.

use axioval_engine::template::{
    Check, Condition, Decision, Expect, Form, ParameterDefault, Refusals, Template, TemplateValue,
    Text,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::ScalarValue;
use serde_json::json;

use super::Openings;
use crate::empty_host::template::{VOIDED, measured, openings_declaration, rounding, value};

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.opening-area";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = Openings::parameters();
    parameters.extend([
        ParameterDescriptor::required("gross_area", ParameterType::PropertyReference),
        ParameterDescriptor::required("net_area", ParameterType::PropertyReference),
        ParameterDescriptor::optional("area_tolerance", ParameterType::Quantity),
    ]);
    parameters
}

/// The side area the rule names `parameter`, as the host states it.
fn stated(parameter: &str) -> serde_json::Value {
    json!({"kind": "property",
        "propertySet": format!("{{{parameter}.set}}"),
        "property": format!("{{{parameter}.name}}")})
}

/// A side area the host states, an area or absent: another kind, and a
/// value that cannot be read, worded as the capability worded them.
fn side(
    name: &'static str,
    parameter: &str,
    (mismatch, refused): (&'static str, &'static str),
) -> TemplateValue {
    TemplateValue {
        expect: Some(Expect::Area),
        mismatch: Some(mismatch),
        refused: Some(refused),
        ..value(name, &stated(parameter))
    }
}

/// The truth that the openings take the stated difference: where either
/// side is stated, `|opening_area − (gross − net)|` at most the tolerance
/// and the rounding. One side stated alone leaves the difference `null`.
fn agrees() -> serde_json::Value {
    let (gross, net) = (stated("gross_area"), stated("net_area"));
    let absolute = |operand: serde_json::Value| json!({"kind": "abs", "operand": operand});
    json!({"kind": "implies",
        "antecedent": {"kind": "or", "operands": [
            {"kind": "isDefined", "operand": gross},
            {"kind": "isDefined", "operand": net}]},
        "consequent": {"kind": "if", "branches": [{
            "when": {"kind": "and", "operands": [
                {"kind": "isDefined", "operand": gross},
                {"kind": "isDefined", "operand": net}]},
            "then": {"kind": "compare", "operator": "lessThanOrEquals",
            "left": absolute(json!({"kind": "subtract",
                "left": measured(VOIDED),
                "right": {"kind": "subtract", "left": gross, "right": net}})),
            "right": {"kind": "add",
                "left": {"kind": "parameter", "name": "area_tolerance"},
                "right": rounding(&[absolute(gross), absolute(net), measured(VOIDED)])}}}],
            // One side alone: the difference is unknown, nothing measured.
            "else": {"kind": "null"}}})
}

/// `opening-area`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    let mut declaration = openings_declaration();
    declaration.extend([
        Check::Required {
            parameter: "gross_area",
        },
        Check::Required {
            parameter: "net_area",
        },
        Check::NonNegativeQuantity {
            parameter: "area_tolerance",
            dimension: QuantityDimension::Area,
            message: "`area_tolerance` is not a non-negative area",
        },
    ]);
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "opening-area",
        refusals: Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "area_tolerance",
            value: ScalarValue::Quantity {
                value: 0.0,
                unit: "m2".to_owned(),
            },
            from: &[],
        }],
        declaration,
        services: None,
        texts: vec![
            Text {
                name: "openings",
                when: Some(Condition::Cites { value: "voided" }),
                text: "its openings ({voided:cited_ids}) cover {voided:m2} of its face",
            },
            Text {
                name: "openings",
                when: None,
                text: "it has no openings",
            },
        ],
        forms: vec![Form {
            when: &[],
            values: vec![
                side(
                    "gross",
                    "gross_area",
                    (
                        "`{gross_area}` is {gross:stated}, not an area",
                        "`{gross_area}`: {why}",
                    ),
                ),
                side(
                    "net",
                    "net_area",
                    (
                        "`{net_area}` is {net:stated}, not an area",
                        "`{net_area}`: {why}",
                    ),
                ),
                value("agrees", &agrees()),
                value("voided", &measured(VOIDED)),
                value(
                    "expected",
                    &json!({"kind": "subtract",
                        "left": stated("gross_area"), "right": stated("net_area")}),
                ),
            ],
            decision: Decision::Holds { value: "agrees" },
            fail: "{openings}, but its gross side area {gross:m2} less its net side area \
                   {net:m2} is {expected:m2}; they must agree within {area_tolerance:m2}",
            undecided: "it states only one of `{gross_area}` and `{net_area}`",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: Some("voided"),
            checks: Vec::new(),
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}
