//! `opening-zone`: openings lie within their host and inside the zone the
//! host allows, clear of its ends, edges or flanges and of each other.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::contract::ParameterValue;
use axioval_ir::{BODY_SET, NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::OpeningZone;
use common::{Model, findings, kind, rule, selector, string, strings, unevaluated};

const ID: &str = "axioval:capability.opening-zone";

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
    for (component, axis) in ["X", "Y", "Z"].iter().enumerate().map(|(i, a)| (i, *a)) {
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

/// A 300 x 300 I-beam `b` running 6 m along world x, its profile's Y up;
/// its web lies between z = -0.13 and 0.13.
fn beam() -> Model {
    extrusion(
        Model::default(),
        "b",
        "beam",
        [0.0; 3],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        6.0,
        "i-shape",
        &[
            ("OverallWidth", 0.3),
            ("OverallDepth", 0.3),
            ("WebThickness", 0.01),
            ("FlangeThickness", 0.02),
        ],
    )
}

/// A hole through the web at `(x, z)`: `family` extruded along world y.
fn hole(
    model: Model,
    local: &str,
    x: f64,
    z: f64,
    family: &str,
    dimensions: &[(&str, f64)],
) -> Model {
    extrusion(
        model,
        local,
        "opening",
        [x, -0.2, z],
        [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]],
        0.4,
        family,
        dimensions,
    )
    .edge("voids", "b", local)
}

fn check(model: Model, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("host_path", strings(&["voids:backward"])),
        ("host_selector", selector(kind("beam"))),
        ("length_axis", string("extrusion")),
        ("height_axis", string("profile-y")),
    ];
    parameters.extend(extra);
    model.evaluate(&OpeningZone, &rule(ID, kind("opening"), parameters))
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn sorted(evaluation: &CapabilityEvaluation) -> Vec<(String, String)> {
    let mut found = findings(evaluation);
    found.sort();
    found
}

#[test]
fn a_hole_inside_the_web_zone_passes_and_holes_outside_it_are_found() {
    let circle = |model, local, x, z| hole(model, local, x, z, "circle", &[("Radius", 0.05)]);
    let model = circle(beam(), "inside", 3.0, 0.0);
    let model = circle(model, "near-end", 0.2, 0.0);
    let model = circle(model, "in-flange", 4.5, 0.1);
    let model = circle(model, "outside", 6.0, 0.0);
    let evaluation = check(
        model,
        vec![
            ("end_distance", metres(0.3)),
            ("zone", string("web")),
            ("edge_distance", metres(0.01)),
        ],
    );
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "in-flange".into(),
                "opening reaches 0.02 m into the flanges of its host b; 0.01 m clear \
                 required"
                    .into()
            ),
            (
                "near-end".into(),
                "opening is 0.15 m from an end of its host b; 0.3 m required".into()
            ),
            (
                "outside".into(),
                "opening lies partly outside its host b: along its length it spans \
                 5.95 m to 6.05 m, the host 0 m to 6 m"
                    .into()
            ),
        ]
    );
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        unevaluated(&evaluation)
    );
    // Every finding names the host.
    assert!(
        evaluation
            .findings()
            .iter()
            .all(|finding| finding.related.iter().any(|id| id.local_id == "b"))
    );
}

#[test]
fn openings_too_close_to_each_other_are_found_when_both_are_exact() {
    let rectangle = |model, local, x| {
        hole(
            model,
            local,
            x,
            0.0,
            "rectangle",
            &[("XDim", 0.2), ("YDim", 0.1)],
        )
    };
    let model = rectangle(beam(), "r1", 2.0);
    let model = rectangle(model, "r2", 2.25);
    let model = rectangle(model, "r3", 4.0);
    let evaluation = check(model, vec![("opening_spacing", metres(0.1))]);
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "r1".into(),
                "opening is 0.05 m clear of another opening in its host b; 0.1 m \
                 required"
                    .into()
            ),
            (
                "r2".into(),
                "opening is 0.05 m clear of another opening in its host b; 0.1 m \
                 required"
                    .into()
            ),
        ]
    );
    // Two circles as close are only bounded from below: not evaluated.
    let circle = |model, local, x| hole(model, local, x, 0.0, "circle", &[("Radius", 0.1)]);
    let model = circle(beam(), "c1", 2.0);
    let model = circle(model, "c2", 2.25);
    let evaluation = check(model, vec![("opening_spacing", metres(0.1))]);
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("c1".into(), NotEvaluatedReason::IncompleteEvidence),
            ("c2".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn a_wall_opening_is_placed_in_the_walls_plan_outline_and_height() {
    // A 5 m x 0.2 m wall extruded 3 m up; a 1 m x 1.2 m window opening
    // through it at x = 4.8, reaching past the wall's end.
    let model = extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        3.0,
        "rectangle",
        &[("XDim", 5.0), ("YDim", 0.2), ("PositionX", 2.5)],
    );
    let model = extrusion(
        model,
        "o",
        "opening",
        [4.8, 0.1, 2.2],
        [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
        0.2,
        "rectangle",
        &[("XDim", 1.0), ("YDim", 1.2)],
    )
    .edge("voids", "w", "o");
    let evaluation = model.evaluate(
        &OpeningZone,
        &rule(
            ID,
            kind("opening"),
            vec![
                ("host_path", strings(&["voids:backward"])),
                ("length_axis", string("profile-x")),
                ("height_axis", string("extrusion")),
                ("edge_distance", metres(0.5)),
            ],
        ),
    );
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "o".into(),
                "opening is 0.2 m from an edge of its host w; 0.5 m clear required".into()
            ),
            (
                "o".into(),
                "opening lies partly outside its host w: along its length it spans \
                 1.8 m to 2.8 m, the host -2.5 m to 2.5 m"
                    .into()
            ),
        ]
    );
}

#[test]
fn hosts_and_openings_it_cannot_bound_are_not_evaluated() {
    // An arbitrary outline bounds nothing the body set states.
    let model = extrusion(
        Model::default(),
        "b",
        "beam",
        [0.0; 3],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        6.0,
        "arbitrary-closed",
        &[],
    );
    let model = hole(model, "o", 3.0, 0.0, "circle", &[("Radius", 0.05)]);
    let evaluation = check(model, vec![]);
    assert_eq!(
        unevaluated(&evaluation),
        [("o".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // An opening voiding no checked host (a slab, not a beam) is not checked.
    let model = beam()
        .object("free", "opening")
        .object("slab", "slab")
        .edge("voids", "slab", "free");
    let evaluation = check(model, vec![]);
    assert!(findings(&evaluation).is_empty());
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
}

#[test]
fn a_web_zone_needs_the_height_across_the_profile() {
    let evaluation = check(
        beam(),
        vec![
            ("height_axis", string("profile-x")),
            ("zone", string("web")),
        ],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}
