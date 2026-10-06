//! `empty-host` as a template: a host with an opening on its middle plane
//! (`opening_count`) keeps more of its face there (`middle_face_area`)
//! than its openings take (`opening_area`) and the tolerance, judged by
//! the truth judge.

use axioval_engine::template::{
    Check, Decision, Form, ParameterDefault, Refusals, Template, TemplateValue,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::QuantityDimension;
use axioval_ir::contract::ScalarValue;
use serde_json::json;

use crate::opening_area::Openings;

/// The capability's id.
pub(crate) const ID: &str = "axioval:capability.empty-host";

/// A host's openings as the rule names them, every argument the rule's own
/// parameter.
macro_rules! openings {
    ($name:literal) => {
        concat!(
            $name,
            ";path=@opening_path;length_axis=@length_axis;height_axis=@height_axis;\
             minimum=@minimum_opening_area;openings=@opening_selector"
        )
    };
}

/// The area they take from it.
pub(crate) const VOIDED: &str = openings!("opening_area");
/// The area they take, citing the openings that take it.
const TAKEN: &str = concat!(openings!("opening_area"), ";cites=counted");
/// The host's face there.
const FACE: &str = "middle_face_area;length_axis=@length_axis;height_axis=@height_axis";

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = Openings::parameters();
    parameters.push(ParameterDescriptor::optional(
        "area_tolerance",
        ParameterType::Quantity,
    ));
    parameters
}

/// The declaration checks of a host's openings, in the order
/// `opening-area` and `empty-host` read them.
pub(crate) fn openings_declaration() -> Vec<Check> {
    vec![
        Check::Required {
            parameter: "opening_path",
        },
        Check::Path {
            parameter: "opening_path",
        },
        Check::Kind {
            parameter: "opening_selector",
        },
        Check::Required {
            parameter: "length_axis",
        },
        Check::Required {
            parameter: "height_axis",
        },
        // The face's axes and the minimum area, as the measurement reads
        // them.
        Check::Arguments {
            when: &[],
            value: VOIDED,
        },
    ]
}

/// A measured value read by name.
pub(crate) fn measured(name: &str) -> serde_json::Value {
    json!({"kind": "property", "propertySet": axioval_ir::MEASURED_SET, "property": name})
}

/// A value of the form.
pub(crate) fn value(name: &'static str, expression: &serde_json::Value) -> TemplateValue {
    TemplateValue {
        name,
        expression: serde_json::from_value(expression.clone()).expect("a template expression"),
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
}

/// The rounding of the decimal coordinates an area is composed of, as the
/// opening capabilities allow it: a nanometre, scaled by the areas.
pub(crate) fn rounding(areas: &[serde_json::Value]) -> serde_json::Value {
    let sum = areas.iter().fold(
        json!({"kind": "literal", "value": {"type": "quantity", "value": 1.0, "unit": "m2"}}),
        |sum, area| json!({"kind": "add", "left": sum, "right": area}),
    );
    json!({"kind": "multiply",
        "left": {"kind": "literal", "value": {"type": "number",
            "value": crate::opening_zone::face::ROUNDING}},
        "right": sum})
}

/// The truth that the host is not empty: where an opening takes area from
/// its middle plane, the openings take less than the face less the
/// tolerance and the rounding.
fn kept() -> serde_json::Value {
    json!({"kind": "implies",
        "antecedent": {"kind": "compare", "operator": "greaterThan",
            "left": measured(TAKEN),
            "right": {"kind": "literal", "value": {"type": "quantity", "value": 0.0, "unit": "m2"}}},
        "consequent": {"kind": "compare", "operator": "lessThan",
            "left": measured(TAKEN),
            "right": {"kind": "subtract",
                "left": {"kind": "subtract",
                    "left": measured(FACE),
                    "right": {"kind": "parameter", "name": "area_tolerance"}},
                "right": rounding(&[measured(FACE), measured(TAKEN)])}}})
}

/// `empty-host`, rebuilt as a composition with its outside contract kept.
pub(crate) fn template() -> Template {
    let mut declaration = openings_declaration();
    declaration.push(Check::NonNegativeQuantity {
        parameter: "area_tolerance",
        dimension: QuantityDimension::Area,
        message: "`area_tolerance` is not a non-negative area",
    });
    Template {
        id: ID,
        parameters: parameters(),
        grades: false,
        name: "empty-host",
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
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![
                value("voided", &measured(TAKEN)),
                value("kept", &kept()),
                value("face", &measured(FACE)),
            ],
            decision: Decision::Holds { value: "kept" },
            fail: "host is empty: its openings ({voided:cited_ids}) void {voided:m2} of its \
                   {face:m2} face",
            undecided: "whether its openings void its face cannot be decided from the \
                        measured areas",
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

#[cfg(test)]
mod tests {
    #[test]
    fn its_expressions_are_well_formed() {
        let template = super::template();
        assert_eq!(template.forms[0].values.len(), 3);
        let _: Expression = template.forms[0].requirement();
    }

    use axioval_ir::contract::Expression;
}
