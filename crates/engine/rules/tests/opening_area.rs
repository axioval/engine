//! `opening-area`: a wall's gross side area less its net side area equals
//! the area of the openings it hosts, placed in its face.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::contract::ParameterValue;
use axioval_ir::{BODY_SET, NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::OpeningArea;
use common::{Model, findings, kind, property, rule, string, strings, unevaluated};

const ID: &str = "axioval:capability.opening-area";
const QTO: &str = "Qto_WallBaseQuantities";

type Vector = [f64; 3];

fn quantity(value: f64, dimension: QuantityDimension) -> PropertyValue {
    PropertyValue::Quantity { value, dimension }
}

fn length(metres: f64) -> PropertyValue {
    quantity(metres, QuantityDimension::Length)
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

/// Wall `w`: 5 m along x from the origin, 0.2 m thick (y -0.1 to 0.1) and
/// 3 m high, extruded up from its plan outline; gross side area 15 m².
fn wall(net: Option<f64>) -> Model {
    let model = extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        3.0,
        "rectangle",
        &[("XDim", 5.0), ("YDim", 0.2), ("PositionX", 2.5)],
    )
    .value(
        "w",
        QTO,
        "GrossSideArea",
        quantity(15.0, QuantityDimension::Area),
    );
    match net {
        Some(net) => model.value(
            "w",
            QTO,
            "NetSideArea",
            quantity(net, QuantityDimension::Area),
        ),
        None => model,
    }
}

/// An opening of `family` centred at `(x, z)`, entering the wall's front
/// face (y 0.1) and running `depth` into it.
fn opening(
    model: Model,
    local: &str,
    x: f64,
    z: f64,
    depth: f64,
    family: &str,
    dimensions: &[(&str, f64)],
) -> Model {
    extrusion(
        model,
        local,
        "opening",
        [x, 0.1, z],
        [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
        depth,
        family,
        dimensions,
    )
    .edge("voids", "w", local)
}

/// A 1 m x 1.2 m window opening through the wall.
fn window(model: Model, local: &str, x: f64) -> Model {
    opening(
        model,
        local,
        x,
        1.5,
        0.2,
        "rectangle",
        &[("XDim", 1.0), ("YDim", 1.2)],
    )
}

fn check(model: Model) -> CapabilityEvaluation {
    model.evaluate(
        &OpeningArea,
        &rule(
            ID,
            kind("wall"),
            vec![
                ("opening_path", strings(&["voids:forward"])),
                ("length_axis", string("profile-x")),
                ("height_axis", string("extrusion")),
                ("gross_area", property(Some(QTO), "GrossSideArea")),
                ("net_area", property(Some(QTO), "NetSideArea")),
                (
                    "area_tolerance",
                    ParameterValue::Quantity {
                        value: 0.01,
                        unit: "m2".into(),
                    },
                ),
            ],
        ),
    )
}

#[test]
fn a_wall_whose_openings_make_up_gross_less_net_passes() {
    // Two windows of 1.2 m² and a recess stopping short of the middle
    // plane, which takes no side area.
    let model = window(window(wall(Some(12.6)), "o1", 1.0), "o2", 3.0);
    let model = opening(
        model,
        "recess",
        4.2,
        0.5,
        0.05,
        "rectangle",
        &[("XDim", 0.4), ("YDim", 0.4)],
    );
    let evaluation = check(model);
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
}

#[test]
fn a_wall_whose_openings_do_not_make_up_gross_less_net_is_found() {
    let model = window(window(wall(Some(13.0)), "o1", 1.0), "o2", 3.0);
    let evaluation = check(model);
    assert_eq!(
        findings(&evaluation),
        [(
            "w".into(),
            "its openings (o1, o2) cover 2.4 m² of its face, but its gross side area 15 m² \
             less its net side area 13 m² is 2 m²; they must agree within 0.01 m²"
                .into()
        )]
    );
    let related: Vec<&str> = evaluation.findings()[0]
        .related
        .iter()
        .map(|id| id.local_id.as_str())
        .collect();
    assert_eq!(related, ["o1", "o2"]);
    // A round opening counts with its exact area: π × 0.5² ≈ 0.785 m².
    let model = opening(
        wall(Some(15.0 - std::f64::consts::FRAC_PI_4)),
        "round",
        2.5,
        1.5,
        0.2,
        "circle",
        &[("Radius", 0.5)],
    );
    assert!(findings(&check(model)).is_empty());
    // A wall stating an opening area but holding none is found.
    assert_eq!(
        findings(&check(
            wall(Some(14.0))
                .object("slab", "slab")
                .object("elsewhere", "opening")
                .edge("voids", "slab", "elsewhere")
        )),
        [(
            "w".into(),
            "it has no openings, but its gross side area 15 m² less its net side area 14 m² \
             is 1 m²; they must agree within 0.01 m²"
                .into()
        )]
    );
}

#[test]
fn a_wall_with_an_opening_it_cannot_place_is_not_evaluated() {
    let model = window(wall(Some(12.6)), "o1", 1.0)
        .object("free", "opening")
        .value("free", BODY_SET, "Count", PropertyValue::Integer(1))
        .text("free", BODY_SET, "Kind", "extrusion")
        .text("free", BODY_SET, "Profile.Type", "arbitrary-closed")
        .edge("voids", "w", "free");
    let evaluation = check(model);
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("w".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // An opening reaching past the wall's end cannot be placed either.
    let evaluation = check(window(wall(Some(13.8)), "o", 4.8));
    assert_eq!(
        unevaluated(&evaluation),
        [("w".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // Nor can two overlapping openings be summed.
    let evaluation = check(window(window(wall(Some(12.6)), "o1", 1.0), "o2", 1.5));
    assert_eq!(
        unevaluated(&evaluation),
        [("w".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_wall_stating_no_areas_is_not_checked_and_one_stating_one_is_not_evaluated() {
    let unstated = extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        3.0,
        "rectangle",
        &[("XDim", 5.0), ("YDim", 0.2), ("PositionX", 2.5)],
    );
    let evaluation = check(unstated);
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
    let evaluation = check(wall(None));
    assert_eq!(
        unevaluated(&evaluation),
        [("w".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}
