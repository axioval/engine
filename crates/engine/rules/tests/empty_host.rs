//! `empty-host`: a wall whose face its openings void wholly is empty.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::contract::ParameterValue;
use axioval_ir::{BODY_SET, NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::EmptyHost;
use common::{Model, findings, kind, rule, string, strings, unevaluated};

const ID: &str = "axioval:capability.empty-host";

type Vector = [f64; 3];

fn length(metres: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: metres,
        dimension: QuantityDimension::Length,
    }
}

/// A straight extrusion of `family` placed at `origin` with axes `x`, `y`,
/// `z`, extruded along `z` by `depth`.
#[allow(clippy::too_many_arguments)]
fn extrusion(
    mut model: Model,
    local: &str,
    object_kind: &str,
    origin: Vector,
    axes: [Vector; 3],
    depth: f64,
    family: &str,
    dimensions: &[(&str, f64)],
) -> Model {
    model = model
        .object(local, object_kind)
        .value(local, BODY_SET, "Count", PropertyValue::Integer(1))
        .text(local, BODY_SET, "Kind", "extrusion")
        .text(local, BODY_SET, "Profile.Type", family)
        .value(local, BODY_SET, "Extrusion.Depth", length(depth));
    for (component, axis) in ["X", "Y", "Z"].iter().enumerate() {
        model = model
            .value(
                local,
                BODY_SET,
                &format!("Placement.Origin{axis}"),
                length(origin[component]),
            )
            .value(
                local,
                BODY_SET,
                &format!("Extrusion.Direction{axis}"),
                PropertyValue::Decimal(axes[2][component]),
            );
        for (name, vector) in ["XAxis", "YAxis", "ZAxis"].iter().zip(axes) {
            model = model.value(
                local,
                BODY_SET,
                &format!("Placement.{name}{axis}"),
                PropertyValue::Decimal(vector[component]),
            );
        }
    }
    for (parameter, value) in dimensions {
        model = model.value(
            local,
            BODY_SET,
            &format!("Profile.{parameter}"),
            length(*value),
        );
    }
    model
}

/// Wall `w`: 5 m along x from the origin, 0.2 m thick and 3 m high,
/// extruded up from its plan outline.
fn wall() -> Model {
    extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        3.0,
        "rectangle",
        &[("XDim", 5.0), ("YDim", 0.2), ("PositionX", 2.5)],
    )
}

/// A `size` (width, height) rectangular opening through the wall centred
/// at `(x, z)`.
fn opening(model: Model, local: &str, (x, z): (f64, f64), size: (f64, f64)) -> Model {
    extrusion(
        model,
        local,
        "opening",
        [x, 0.1, z],
        [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
        0.2,
        "rectangle",
        &[("XDim", size.0), ("YDim", size.1)],
    )
    .edge("voids", "w", local)
}

fn check(model: Model, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("opening_path", strings(&["voids:forward"])),
        ("length_axis", string("profile-x")),
        ("height_axis", string("extrusion")),
    ];
    parameters.extend(extra);
    model.evaluate(&EmptyHost, &rule(ID, kind("wall"), parameters))
}

fn square_metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m2".into(),
    }
}

#[test]
fn a_wall_fully_covered_by_one_opening_is_empty_and_one_with_a_window_is_not() {
    let whole = opening(wall(), "whole", (2.5, 1.5), (5.0, 3.0));
    let evaluation = check(whole, Vec::new());
    assert_eq!(
        findings(&evaluation),
        [(
            "w".into(),
            "host is empty: its openings (whole) void 15 m² of its 15 m² face".into()
        )]
    );
    // Two openings side by side leave no wall either.
    let model = opening(wall(), "left", (1.0, 1.5), (2.0, 3.0));
    let model = opening(model, "right", (3.5, 1.5), (3.0, 3.0));
    assert_eq!(findings(&check(model, Vec::new())).len(), 1);

    let window = opening(wall(), "window", (2.5, 1.5), (1.0, 1.2));
    let evaluation = check(window, Vec::new());
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_tolerance_small_openings_and_overlaps_are_honoured() {
    // 14.7 m² of 15 m²: empty within 0.5 m², not without.
    let nearly = || opening(wall(), "most", (2.45, 1.5), (4.9, 3.0));
    assert!(findings(&check(nearly(), Vec::new())).is_empty());
    assert_eq!(
        findings(&check(
            nearly(),
            vec![("area_tolerance", square_metres(0.5))]
        ))
        .len(),
        1
    );
    // An opening below the minimum area is ignored.
    let whole = opening(wall(), "whole", (2.5, 1.5), (5.0, 3.0));
    let evaluation = check(whole, vec![("minimum_opening_area", square_metres(20.0))]);
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
    // Openings that may overlap cannot be summed.
    let model = opening(wall(), "a", (2.0, 1.5), (2.0, 3.0));
    let model = opening(model, "b", (2.5, 1.5), (2.0, 3.0));
    let evaluation = check(model, Vec::new());
    assert_eq!(
        unevaluated(&evaluation),
        [("w".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// `empty-host` as an expression: a host with an opening on its middle
/// plane (`opening_count`) whose summed openings (`opening_area`) do not
/// fall short of its middle-plane face (`middle_face_area`) by more than
/// the tolerance is empty. It judges every fixture as the capability does.
#[test]
#[allow(
    clippy::format_push_string,
    clippy::items_after_statements,
    clippy::too_many_lines
)]
fn an_empty_host_as_an_expression_reaches_the_verdicts() {
    use serde_json::{Value, json};
    let measured = |name: &str| json!({"kind": "property", "propertySet": "axioval:measured", "property": name});
    let rounded = |operand: Value| {
        json!({"kind": "round", "operand": operand,
            "step": {"kind": "literal", "value": {"type": "quantity", "value": 1e-6, "unit": "m2"}}})
    };
    let requirement = |tolerance: f64, minimum: Option<f64>| {
        let mut openings =
            "path=voids:forward;length_axis=profile-x;height_axis=extrusion".to_owned();
        if let Some(minimum) = minimum {
            openings.push_str(&format!(";minimum={minimum}"));
        }
        json!({"kind": "implies",
            "antecedent": {"kind": "compare", "operator": "greaterThan",
                "left": measured(&format!("opening_count;{openings}")),
                "right": {"kind": "literal", "value": {"type": "integer", "value": 0}}},
            "consequent": {"kind": "compare", "operator": "lessThan",
                "left": rounded(measured(&format!("opening_area;{openings}"))),
                "right": {"kind": "subtract",
                    "left": rounded(measured(
                        "middle_face_area;length_axis=profile-x;height_axis=extrusion")),
                    "right": {"kind": "literal",
                        "value": {"type": "quantity", "value": tolerance, "unit": "m2"}}}}})
    };
    type Fixture = (fn() -> Model, Option<f64>, Option<f64>);
    let fixtures: [Fixture; 9] = [
        (
            || opening(wall(), "whole", (2.5, 1.5), (5.0, 3.0)),
            None,
            None,
        ),
        (
            || {
                opening(
                    opening(wall(), "left", (1.0, 1.5), (2.0, 3.0)),
                    "right",
                    (3.5, 1.5),
                    (3.0, 3.0),
                )
            },
            None,
            None,
        ),
        (
            || opening(wall(), "window", (2.5, 1.5), (1.0, 1.2)),
            None,
            None,
        ),
        (
            || opening(wall(), "most", (2.45, 1.5), (4.9, 3.0)),
            None,
            None,
        ),
        (
            || opening(wall(), "most", (2.45, 1.5), (4.9, 3.0)),
            Some(0.5),
            None,
        ),
        (
            || opening(wall(), "whole", (2.5, 1.5), (5.0, 3.0)),
            None,
            Some(20.0),
        ),
        (
            || {
                opening(
                    opening(wall(), "a", (2.0, 1.5), (2.0, 3.0)),
                    "b",
                    (2.5, 1.5),
                    (2.0, 3.0),
                )
            },
            None,
            None,
        ),
        (wall, None, None),
        (|| wall().object("bare", "wall"), None, None),
    ];
    let (mut found, mut open) = (0, 0);
    for (index, (model, tolerance, minimum)) in fixtures.into_iter().enumerate() {
        let mut extra = Vec::new();
        if let Some(tolerance) = tolerance {
            extra.push(("area_tolerance", square_metres(tolerance)));
        }
        if let Some(minimum) = minimum {
            extra.push(("minimum_opening_area", square_metres(minimum)));
        }
        let evaluated = check(model(), extra);
        let rewritten = model().evaluate_measured(
            &axioval_rules::ExpressionRequirement,
            &rule(
                "axioval:capability.expression",
                kind("wall"),
                vec![(
                    "requirement",
                    common::expression(requirement(tolerance.unwrap_or(0.0), minimum)),
                )],
            ),
            |_| {},
        );
        let parity = axioval_rules::parity::compare_evaluations(
            (ID, &evaluated),
            ("expression", &rewritten),
        );
        assert!(parity.holds(), "fixture {index}:\n{}", parity.diff());
        found += parity.found;
        open += parity.open;
    }
    // A wall with no recorded voids and one without a body stay open.
    assert_eq!((found, open), (3, 4));
}
