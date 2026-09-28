//! `opening-zone`: openings lie within their host and inside the zone the
//! host allows, clear of its ends, edges or flanges and of each other.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, GeometryFidelity, ObjectBounds, ProjectedDistanceEvidence,
    ProximityError, ProximityEvidence, ProximityRequest, ProximityService, ProximityServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{
    BODY_SET, Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension,
};
use axioval_rules::OpeningZone;
use common::{Model, findings, kind, rule, selector, source, string, strings, unevaluated};

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
    // An arbitrary profile whose outline the body set does not state (a
    // curved one) bounds nothing.
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

/// A column under the beam `b` at `x`: `family` extruded up from 3 m below
/// to the beam's underside, its section's X axis along `x_axis`.
fn column(
    model: Model,
    local: &str,
    x: f64,
    x_axis: Vector,
    family: &str,
    dimensions: &[(&str, f64)],
) -> Model {
    let y_axis = [-x_axis[1], x_axis[0], 0.0];
    extrusion(
        model,
        local,
        "column",
        [x, 0.0, -3.0],
        [x_axis, y_axis, [0.0, 0.0, 1.0]],
        2.85,
        family,
        dimensions,
    )
}

fn square(model: Model, local: &str, x: f64) -> Model {
    column(
        model,
        local,
        x,
        [1.0, 0.0, 0.0],
        "rectangle",
        &[("XDim", 0.3), ("YDim", 0.3)],
    )
}

fn heb(model: Model, local: &str, x: f64, x_axis: Vector) -> Model {
    column(
        model,
        local,
        x,
        x_axis,
        "i-shape",
        &[
            ("OverallWidth", 0.3),
            ("OverallDepth", 0.3),
            ("WebThickness", 0.01),
            ("FlangeThickness", 0.02),
        ],
    )
}

fn circle(model: Model, local: &str, x: f64) -> Model {
    hole(model, local, x, 0.0, "circle", &[("Radius", 0.05)])
}

fn supported(model: Model, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("support_path", strings(&["connects:either"])),
        ("support_selector", selector(kind("column"))),
        ("support_distance", metres(0.5)),
    ];
    parameters.extend(extra);
    check(model, parameters)
}

#[test]
fn a_hole_inside_a_support_zone_is_found_and_one_outside_passes() {
    // A square column under the start (x 0.35 to 0.65) and an HEB column,
    // its flanges' width along the beam (x 5.35 to 5.65), under the end.
    let model = square(beam(), "c1", 0.5)
        .edge("connects", "b", "c1")
        .edge("connects", "c2", "b");
    let model = heb(model, "c2", 5.5, [1.0, 0.0, 0.0]);
    let model = circle(model, "near-start", 0.9);
    let model = circle(model, "mid-span", 3.0);
    let model = circle(model, "near-end", 4.9);
    let evaluation = supported(model, vec![]);
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "near-end".into(),
                "opening is 0.4 m from support c2 along its host b; 0.5 m required".into()
            ),
            (
                "near-start".into(),
                "opening is 0.2 m from support c1 along its host b; 0.5 m required".into()
            ),
        ]
    );
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    // The finding relates the support and the host.
    let related: Vec<&str> = evaluation
        .findings()
        .iter()
        .find(|finding| finding.message.contains("c1"))
        .unwrap()
        .related
        .iter()
        .map(|id| id.local_id.as_str())
        .collect();
    assert_eq!(related, ["b", "c1"]);
}

#[test]
fn a_column_unconnected_or_unselected_is_no_support() {
    // c1 is not connected to the beam; the connected slab is not a column.
    let model = square(beam(), "c1", 0.5)
        .object("slab", "slab")
        .edge("connects", "b", "slab");
    let model = circle(model, "near-start", 0.9);
    let evaluation = supported(model, vec![]);
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_support_known_only_by_its_box_finds_near_holes_and_leaves_straddling_ones() {
    // An HEB column turned 45°: along the beam it reaches at least its
    // centre (x 3) and at most 0.212 m either side.
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let model = heb(beam(), "c", 3.0, [diagonal, diagonal, 0.0]).edge("connects", "b", "c");
    let model = circle(model, "near", 3.35);
    let model = circle(model, "between", 3.7);
    let model = circle(model, "far", 4.0);
    let evaluation = supported(model, vec![]);
    assert_eq!(
        sorted(&evaluation),
        [(
            "near".into(),
            "opening is at most 0.3 m from support c along its host b; 0.5 m required".into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("between".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_support_whose_body_cannot_be_read_leaves_the_holes_not_evaluated() {
    let model = beam()
        .object("c", "column")
        .value("c", BODY_SET, "Count", PropertyValue::Integer(1))
        .text("c", BODY_SET, "Kind", "brep")
        .edge("connects", "b", "c");
    let model = circle(model, "o", 3.0);
    let evaluation = supported(model, vec![]);
    assert!(findings(&evaluation).is_empty());
    let outcomes = evaluation.not_evaluated_outcomes();
    assert_eq!(outcomes.len(), 1);
    assert!(
        outcomes[0]
            .message()
            .contains("(the support's body is a `brep`"),
        "{}",
        outcomes[0].message()
    );
}

#[test]
fn a_hole_overlapping_a_connecting_beams_footprint_is_found() {
    // A secondary beam of a 0.1 x 0.2 m rectangle frames into the web at
    // x 2 (x 1.95 to 2.05, z -0.1 to 0.1), extruded along y.
    let model = extrusion(
        beam(),
        "s",
        "beam",
        [2.0, 0.005, 0.0],
        [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        3.0,
        "rectangle",
        &[("XDim", 0.1), ("YDim", 0.2)],
    )
    .edge("connects", "s", "b");
    let rectangle = |model, local, x| {
        hole(
            model,
            local,
            x,
            0.0,
            "rectangle",
            &[("XDim", 0.1), ("YDim", 0.1)],
        )
    };
    let model = rectangle(model, "overlapping", 2.02);
    let model = rectangle(model, "beside", 2.5);
    // The column under the start is connected too; its footprint lies
    // below the web.
    let model = square(model, "c1", 0.5).edge("connects", "b", "c1");
    let model = rectangle(model, "above-column", 0.5);
    let evaluation = check(
        model,
        vec![
            ("support_path", strings(&["connects:either"])),
            (
                "support_selector",
                selector(Selector::AnyOf {
                    operands: vec![kind("column"), kind("beam")],
                }),
            ),
            ("support_clearance", metres(0.0)),
        ],
    );
    assert_eq!(
        sorted(&evaluation),
        [(
            "overlapping".into(),
            "opening overlaps connecting member s by 0.08 m in the face of its host b".into()
        )]
    );
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
}

#[test]
fn supports_need_a_way_to_be_found_and_a_requirement() {
    for parameters in [
        vec![("support_distance", metres(0.5))],
        vec![("support_path", strings(&["connects:either"]))],
        vec![("support_selector", selector(kind("column")))],
    ] {
        assert_eq!(
            unevaluated(&check(beam(), parameters)),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Answers the distance between the beam and each column from a table;
/// a column not in it is far away.
struct Touching(BTreeMap<String, (f64, f64)>);

impl ProximityService for Touching {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("contact is measured through measure_distance")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let counterpart = &request.counterpart().local_id;
        let (lower, upper) = self.0.get(counterpart).copied().unwrap_or((5.0, 5.0));
        let fidelity = if lower < upper {
            GeometryFidelity::tessellated(upper - lower)?
        } else {
            GeometryFidelity::Exact
        };
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            fidelity,
            Evidence {
                source: source(),
                locator: format!("contact:{counterpart}"),
                exact: fidelity.is_exact(),
            },
        )
    }
}

#[test]
fn supports_are_found_by_contact_and_an_undecided_contact_is_not_ignored() {
    // c1 touches the beam; c3 is close along it but apart (another frame);
    // whether c4 touches is undecided.
    let model = square(beam(), "c1", 0.5);
    let model = square(model, "c3", 1.2);
    let model = square(model, "c4", 3.3);
    let model = circle(model, "near-start", 0.9);
    let model = circle(model, "near-c4", 3.0);
    let model = circle(model, "clear", 2.2);
    let touching = Touching(BTreeMap::from([
        ("c1".to_owned(), (0.0, 0.0)),
        ("c3".to_owned(), (1.0, 1.0)),
        ("c4".to_owned(), (0.0, 0.01)),
    ]));
    let rule = rule(
        ID,
        kind("opening"),
        vec![
            ("host_path", strings(&["voids:backward"])),
            ("host_selector", selector(kind("beam"))),
            ("length_axis", string("extrusion")),
            ("height_axis", string("profile-y")),
            ("support_selector", selector(kind("column"))),
            ("support_gap", metres(0.001)),
            ("support_distance", metres(0.5)),
        ],
    );
    let evaluation = model.evaluate_with(&OpeningZone, &rule, |services| {
        services
            .register(ProximityServiceHandle::new(Arc::new(touching)))
            .unwrap();
    });
    assert_eq!(
        sorted(&evaluation),
        [(
            "near-start".into(),
            "opening is 0.2 m from support c1 along its host b; 0.5 m required".into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("near-c4".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // The contact is cited.
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "contact:c1")
    );
    // Without a proximity service contact cannot be measured.
    let model = circle(square(beam(), "c1", 0.5), "o", 3.0);
    assert_eq!(
        unevaluated(&model.evaluate(&OpeningZone, &rule)),
        [("o".into(), NotEvaluatedReason::MissingService)]
    );
}

fn lengths(values: impl IntoIterator<Item = f64>) -> PropertyValue {
    PropertyValue::List(values.into_iter().map(length).collect())
}

/// States `ring` as the outline of `local`'s profile.
fn outlined(model: Model, local: &str, ring: &[[f64; 2]]) -> Model {
    model
        .value(
            local,
            BODY_SET,
            "Profile.OutlineX",
            lengths(ring.iter().map(|vertex| vertex[0])),
        )
        .value(
            local,
            BODY_SET,
            "Profile.OutlineY",
            lengths(ring.iter().map(|vertex| vertex[1])),
        )
}

const UPRIGHT: [Vector; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Wall `w`, 0.2 m thick (y 0 to 0.2), extruded 3 m up from a plan
/// outline mitred at its far end: 5 m long on its face y = 0, 5.2 m on its
/// face y = 0.2.
fn mitred_wall() -> Model {
    let model = extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        UPRIGHT,
        3.0,
        "arbitrary-closed",
        &[],
    );
    outlined(
        model,
        "w",
        &[[0.0, 0.0], [5.0, 0.0], [5.2, 0.2], [0.0, 0.2]],
    )
}

/// An opening in wall `w` extruded straight through it along -y (from
/// y 0.3 to -0.1), its section's X along the wall and its Y up.
fn through_wall(
    model: Model,
    local: &str,
    origin: Vector,
    family: &str,
    dimensions: &[(&str, f64)],
) -> Model {
    extrusion(
        model,
        local,
        "opening",
        origin,
        [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
        0.4,
        family,
        dimensions,
    )
    .edge("voids", "w", local)
}

/// A 1 m x 1.2 m window through wall `w`, centred at `x` and 1.5 m up.
fn wall_window(model: Model, local: &str, x: f64) -> Model {
    through_wall(
        model,
        local,
        [x, 0.3, 1.5],
        "rectangle",
        &[("XDim", 1.0), ("YDim", 1.2)],
    )
}

fn wall_check(model: Model, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("host_path", strings(&["voids:backward"])),
        ("host_selector", selector(kind("wall"))),
        ("length_axis", string("profile-x")),
        ("height_axis", string("extrusion")),
    ];
    parameters.extend(extra);
    model.evaluate(&OpeningZone, &rule(ID, kind("opening"), parameters))
}

#[test]
fn openings_near_a_mitred_wall_end_are_bounded_by_its_outline() {
    let model = wall_window(mitred_wall(), "middle", 2.5);
    // 0.5 m from the end of the short face, though 0.7 m from the long one.
    let model = wall_window(model, "near-mitre", 4.0);
    // Inside the box around the outline, but through the mitre.
    let model = wall_window(model, "across-mitre", 4.6);
    let model = wall_window(model, "past-end", 5.5);
    // Extruded along the wall rather than through it (x 4.3 to 4.8, y 0.05
    // to 0.15): only its box is known in the plan, which comes within
    // 0.25 m of the mitre.
    let model = extrusion(
        model,
        "along",
        "opening",
        [4.3, 0.1, 1.5],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        0.5,
        "rectangle",
        &[("XDim", 0.1), ("YDim", 1.0)],
    )
    .edge("voids", "w", "along");
    let evaluation = wall_check(
        model,
        vec![
            ("end_distance", metres(0.6)),
            ("edge_distance", metres(0.2)),
        ],
    );
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "across-mitre".into(),
                "opening lies partly outside its host w: it crosses the edge of the host's \
                 outline"
                    .into()
            ),
            (
                "near-mitre".into(),
                "opening is 0.5 m from an end of its host w; 0.6 m required".into()
            ),
            (
                "past-end".into(),
                "opening lies partly outside its host w: along its length it spans 5 m to \
                 6 m, the host 0 m to 5.2 m"
                    .into()
            ),
        ]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("along".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // Without an end distance the opening along the wall lies inside it.
    let evaluation = wall_check(wall_window(mitred_wall(), "w1", 2.5), vec![]);
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_mitred_wall_end_reached_only_part_way_through_is_further_away() {
    // A recess 0.1 m deep into the wall's long face (y 0.1 to 0.2) at x
    // 4.4 to 4.6 meets the mitre at x 5.1 at the earliest: 0.5 m away.
    let model = || {
        extrusion(
            mitred_wall(),
            "recess",
            "opening",
            [4.5, 0.2, 1.5],
            [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
            0.1,
            "rectangle",
            &[("XDim", 0.2), ("YDim", 0.5)],
        )
        .edge("voids", "w", "recess")
    };
    let evaluation = wall_check(model(), vec![("end_distance", metres(0.5))]);
    assert!(
        findings(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(unevaluated(&evaluation).is_empty());
    assert_eq!(
        findings(&wall_check(model(), vec![("end_distance", metres(0.55))])),
        [(
            "recess".into(),
            "opening is 0.5 m from an end of its host w; 0.55 m required".into()
        )]
    );
}

#[test]
fn an_l_shaped_opening_reaches_as_far_as_its_outline() {
    // An L: a 1 m x 0.5 m foot with a 0.5 m x 1 m leg on its left, placed
    // at x 3.8 and 0.5 m up, so it spans x 3.8 to 4.8 and z 0.5 to 2.
    let model = through_wall(mitred_wall(), "l", [3.8, 0.3, 0.5], "arbitrary-closed", &[]);
    let model = outlined(
        model,
        "l",
        &[
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.5],
            [0.5, 0.5],
            [0.5, 1.5],
            [0.0, 1.5],
        ],
    );
    let evaluation = wall_check(
        model,
        vec![
            ("end_distance", metres(0.3)),
            ("edge_distance", metres(0.6)),
        ],
    );
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "l".into(),
                "opening is 0.2 m from an end of its host w; 0.3 m required".into()
            ),
            (
                "l".into(),
                "opening is 0.5 m from an edge of its host w; 0.6 m clear required".into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_free_outline_that_is_not_one_region_or_is_curved_is_not_evaluated() {
    // The outline crosses itself.
    let model = outlined(
        extrusion(
            Model::default(),
            "w",
            "wall",
            [0.0; 3],
            UPRIGHT,
            3.0,
            "arbitrary-closed",
            &[],
        ),
        "w",
        &[[0.0, 0.0], [5.0, 0.0], [5.0, 0.2], [3.0, -0.1], [0.0, 0.2]],
    );
    let evaluation = wall_check(wall_window(model, "o", 2.5), vec![]);
    assert_eq!(
        unevaluated(&evaluation),
        [("o".into(), NotEvaluatedReason::InvalidEvidence)]
    );
    // An outline the source states no vertices for (a curved one) bounds
    // nothing.
    let model = extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        UPRIGHT,
        3.0,
        "arbitrary-closed",
        &[],
    );
    let evaluation = wall_check(wall_window(model, "o", 2.5), vec![]);
    assert_eq!(
        unevaluated(&evaluation),
        [("o".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_shaft_through_a_notched_slab_keeps_clear_of_the_notch() {
    // Slab `s`, 0.2 m thick, of a 4 m x 3 m outline with a 2 m x 1 m
    // notch out of its corner at x 2 to 4, y 2 to 3.
    let model = outlined(
        extrusion(
            Model::default(),
            "s",
            "slab",
            [0.0; 3],
            UPRIGHT,
            0.2,
            "arbitrary-closed",
            &[],
        ),
        "s",
        &[
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 2.0],
            [2.0, 2.0],
            [2.0, 3.0],
            [0.0, 3.0],
        ],
    );
    let shaft = |model: Model, local: &str, x: f64, y: f64| {
        extrusion(
            model,
            local,
            "opening",
            [x, y, -0.1],
            UPRIGHT,
            0.4,
            "rectangle",
            &[("XDim", 0.5), ("YDim", 0.5)],
        )
        .edge("voids", "s", local)
    };
    // 0.15 m below the notch, though 1.15 m from the slab's far edge.
    let model = shaft(model, "below-notch", 3.0, 1.6);
    let model = shaft(model, "in-notch", 2.2, 2.2);
    let model = shaft(model, "clear", 1.0, 1.0);
    let evaluation = model.evaluate(
        &OpeningZone,
        &rule(
            ID,
            kind("opening"),
            vec![
                ("host_path", strings(&["voids:backward"])),
                ("host_selector", selector(kind("slab"))),
                ("length_axis", string("profile-x")),
                ("height_axis", string("profile-y")),
                ("edge_distance", metres(0.3)),
            ],
        ),
    );
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "below-notch".into(),
                "opening is 0.15 m from an edge of its host s; 0.3 m clear required".into()
            ),
            (
                "in-notch".into(),
                "opening lies partly outside its host s: it crosses the edge of the host's \
                 outline"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

/// A 5 m x 0.2 m wall 3 m high holding 1 m x 1.2 m windows centred at
/// `(x, z)`: `(1, 1.8)` has its head 0.6 m below the wall top, `(3.5, 2)`
/// 0.4 m.
fn windows_below_the_top() -> Model {
    let model = extrusion(
        Model::default(),
        "w",
        "wall",
        [0.0; 3],
        UPRIGHT,
        3.0,
        "rectangle",
        &[("XDim", 5.0), ("YDim", 0.2), ("PositionX", 2.5)],
    );
    let window = |model: Model, local: &str, x: f64, z: f64| {
        extrusion(
            model,
            local,
            "opening",
            [x, 0.1, z],
            [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
            0.2,
            "rectangle",
            &[("XDim", 1.0), ("YDim", 1.2)],
        )
        .edge("voids", "w", local)
    };
    let model = window(model, "low", 1.0, 1.8);
    window(model, "high", 3.5, 2.0)
}

fn wall_edges(extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("host_path", strings(&["voids:backward"])),
        ("length_axis", string("profile-x")),
        ("height_axis", string("extrusion")),
    ];
    parameters.extend(extra);
    windows_below_the_top().evaluate(&OpeningZone, &rule(ID, kind("opening"), parameters))
}

/// `edge_distance_maximum` bounds the distance to the edges `maximum_edges`
/// names: a head 0.6 m below the wall top fails a 0.5 m maximum, one 0.4 m
/// below passes, and the sills far above the bottom are not judged.
#[test]
fn a_window_head_too_far_below_the_wall_top_is_found() {
    let evaluation = wall_edges(vec![
        ("edge_distance_maximum", metres(0.5)),
        ("maximum_edges", string("top")),
    ]);
    assert_eq!(
        sorted(&evaluation),
        [(
            "low".into(),
            "opening is 0.6 m from the top edge of its host w; at most 0.5 m allowed".into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());

    // Both edges by default: every sill is far above the bottom too.
    let evaluation = wall_edges(vec![("edge_distance_maximum", metres(0.5))]);
    assert_eq!(sorted(&evaluation).len(), 3);

    // An unknown edge, or edges without a maximum, are invalid declarations.
    for extra in [
        vec![
            ("edge_distance_maximum", metres(0.5)),
            ("maximum_edges", string("left")),
        ],
        vec![("maximum_edges", string("top"))],
    ] {
        let evaluation = wall_edges(extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

const INTERSECTS: &str = "axioval:derived.intersects";

/// A 0.1 m square duct running along world y from -1 m to 1 m at `(x, z)`,
/// with no opening modelled: it reaches each beam it passes through by the
/// derived relationship.
fn duct(model: Model, local: &str, x: f64, z: f64, beams: &[&str]) -> Model {
    let mut model = extrusion(
        model,
        local,
        "duct",
        [x, -1.0, z],
        [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]],
        2.0,
        "rectangle",
        &[("XDim", 0.1), ("YDim", 0.1)],
    );
    for beam in beams {
        model = model.edge(INTERSECTS, local, beam);
    }
    model
}

fn penetrations(model: Model) -> CapabilityEvaluation {
    model.evaluate(
        &OpeningZone,
        &rule(
            ID,
            kind("duct"),
            vec![
                ("host_path", strings(&[INTERSECTS])),
                ("host_selector", selector(kind("beam"))),
                ("length_axis", string("extrusion")),
                ("height_axis", string("profile-y")),
                ("end_distance", metres(0.3)),
                ("zone", string("web")),
            ],
        ),
    )
}

/// A duct crossing a beam with no void modelled is judged in every beam it
/// passes through: inside the web zone it passes, near an end or in a
/// flange it is found, once per beam.
#[test]
fn a_duct_through_beams_without_a_void_is_judged_in_each_beam() {
    let second = extrusion(
        beam(),
        "b2",
        "beam",
        [0.0, 0.8, 0.0],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        6.0,
        "i-shape",
        &[
            ("OverallWidth", 0.3),
            ("OverallDepth", 0.3),
            ("WebThickness", 0.01),
            ("FlangeThickness", 0.02),
        ],
    );
    let model = duct(second, "inside", 3.0, 0.0, &["b", "b2"]);
    let model = duct(model, "near-end", 0.2, 0.0, &["b", "b2"]);
    let model = duct(model, "in-flange", 4.5, 0.1, &["b"]);
    // Running past both beams, it reaches none and is not checked.
    let model = duct(model, "beside", 5.0, 1.0, &[]);
    let evaluation = penetrations(model);
    assert_eq!(
        sorted(&evaluation),
        [
            (
                "in-flange".into(),
                "opening reaches 0.02 m into the flanges of its host b; 0 m clear required".into()
            ),
            (
                "near-end".into(),
                "opening is 0.15 m from an end of its host b2; 0.3 m required".into()
            ),
            (
                "near-end".into(),
                "opening is 0.15 m from an end of its host b; 0.3 m required".into()
            ),
        ]
    );
    assert!(
        unevaluated(&evaluation).is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    // A host whose body cannot be read leaves only that placement open.
    let unreadable = extrusion(
        beam(),
        "b2",
        "beam",
        [0.0, 0.8, 0.0],
        [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        6.0,
        "i-shape",
        &[("OverallWidth", 0.3)],
    );
    let evaluation = penetrations(duct(unreadable, "near-end", 0.2, 0.0, &["b", "b2"]));
    assert_eq!(
        sorted(&evaluation),
        [(
            "near-end".into(),
            "opening is 0.15 m from an end of its host b; 0.3 m required".into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("near-end".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}
