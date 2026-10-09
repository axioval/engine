//! Exact boundaries built from geometry graphs under rigid placements.
//!
//! Each body is built twice from one graph: meshed by the kernel's mesh
//! compiler, as a host meshes it, and as an exact solid by
//! `exact_boundary`. The two must agree (`check_exact_boundary`), the
//! solid's volume must be the closed form, and the distance between exact
//! solids the closed form to within the certified accuracy.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use axiolid_brep::ExactBRep;
use axiolid_construct::profile_lower::lower_derived;
use axiolid_construct::section_lower::rectangle_contour;
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{
    BooleanOperator, Frame3, Point3, Tolerance, Transform2, Transform3, Vec2, Vec3,
};
use axiolid_curve::{Circle3, Curve3, Polyline};
use axiolid_measure::{boundary_distance, exact_properties};
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_compile::ReferenceMeshCompiler;
use axiolid_mesh_compile_contract::MeshCompiler;
use axiolid_model::{
    CurveRelation, GeometryGraph, GeometryGraphBuilder, GeometryNode, Instance, NodeId,
    SolidOperation, TrimSelector, TrimmingPreference,
};
use axiolid_profile::{CircleProfile, ContourProfile, EllipseProfile, Profile, RectangleProfile};
use axioval_axiolid::{AxiolidGeometry, ExactBoundary, exact_boundary};
use axioval_ir::{ObjectId, SourceId};

/// The chord deviation the mesh compiler keeps to, declared for its meshes.
const DEVIATION: f64 = 1e-3;
const ACCURACY: f64 = 1e-6;

fn id() -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), "body").unwrap()
}

/// A graph whose root `build` pushes.
fn graph(build: impl FnOnce(&mut GeometryGraphBuilder) -> NodeId) -> (GeometryGraph, NodeId) {
    let mut builder = GeometryGraphBuilder::new();
    let root = build(&mut builder);
    (builder.finish(vec![root]).unwrap(), root)
}

fn push(builder: &mut GeometryGraphBuilder, node: GeometryNode) -> NodeId {
    builder.push(node).unwrap()
}

fn placed(builder: &mut GeometryGraphBuilder, source: NodeId, transform: Transform3) -> NodeId {
    push(
        builder,
        GeometryNode::Instance(Instance { source, transform }),
    )
}

/// `profile` extruded by `depth` along `direction`, placed by `transform`.
fn extrusion(
    profile: Profile,
    direction: Vec3,
    depth: f64,
    transform: Transform3,
) -> (GeometryGraph, NodeId) {
    graph(|builder| {
        let profile = push(builder, GeometryNode::Profile(profile));
        let solid = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction,
                depth,
            }),
        );
        placed(builder, solid, transform)
    })
}

/// `profile` turned by `angle` about the local y axis through `origin`,
/// placed by `transform`.
fn revolution(
    profile: Profile,
    origin: Point3,
    angle: f64,
    transform: Transform3,
) -> (GeometryGraph, NodeId) {
    graph(|builder| {
        let profile = push(builder, GeometryNode::Profile(profile));
        let solid = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Revolution {
                profile,
                axis_origin: origin,
                axis_direction: Vec3::Y,
                angle,
            }),
        );
        placed(builder, solid, transform)
    })
}

/// A disk of `radius` (bored to `inner`) swept along `directrix`.
fn swept_disk(
    directrix: GeometryNode,
    trim: Option<(f64, f64)>,
    radius: f64,
    inner: Option<f64>,
    transform: Transform3,
) -> (GeometryGraph, NodeId) {
    graph(|builder| {
        let mut directrix = push(builder, directrix);
        if let Some((start, end)) = trim {
            directrix = push(
                builder,
                GeometryNode::CurveRelation(CurveRelation::Trimmed {
                    basis: directrix,
                    start: vec![TrimSelector::Parameter(start)],
                    end: vec![TrimSelector::Parameter(end)],
                    sense_agreement: true,
                    preference: TrimmingPreference::Parameter,
                }),
            );
        }
        let solid = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::SweptDisk {
                directrix,
                radius,
                inner_radius: inner,
                parameter_range: None,
                fillet_radius: None,
            }),
        );
        placed(builder, solid, transform)
    })
}

fn circle(radius: f64) -> Profile {
    Profile::Circle(CircleProfile {
        radius,
        thickness: None,
    })
}

fn rectangle(x: f64, y: f64) -> Profile {
    Profile::Rectangle(RectangleProfile {
        x,
        y,
        thickness: None,
        outer_radius: None,
        inner_radius: None,
    })
}

/// `profile` moved by `(x, y)` in its plane.
fn moved(profile: Profile, x: f64, y: f64) -> Profile {
    Profile::Derived {
        basis: Box::new(profile),
        transform: Transform2::from_translation(Vec2::new(x, y)),
    }
}

/// A tilt off every coordinate axis, then a move.
fn tilted(translation: Vec3) -> Transform3 {
    Transform3::from_translation(translation)
        * Transform3::from_rotation_z(0.3)
        * Transform3::from_rotation_x(0.6)
        * Transform3::from_rotation_y(-0.2)
}

/// The exact boundary of `root`, checked against the kernel's mesh of the
/// same graph.
fn agreeing(graph: &GeometryGraph, root: NodeId) -> ExactBoundary {
    let boundary = exact_boundary(graph, root).unwrap();
    let mesh = ReferenceMeshCompiler::new(BoolmeshBoolean)
        .compile_mesh(graph, root, &ExecutionOptions::new(Tolerance::MILLIMETRE))
        .unwrap();
    AxiolidGeometry::new()
        .with_tessellated_mesh(id(), mesh, DEVIATION)
        .check_exact_boundary(&id(), &boundary)
        .unwrap();
    boundary
}

fn volume(brep: &ExactBRep) -> f64 {
    exact_properties(brep, Tolerance::METRE)
        .unwrap()
        .signed_volume
}

fn assert_close(actual: f64, expected: f64, within: f64) {
    assert!(
        (actual - expected).abs() <= within,
        "{actual} is not within {within} of {expected}"
    );
}

/// An exact wall `x` in `[at, at + 0.2]`, large in `y` and `z`.
fn wall(at: f64) -> ExactBRep {
    let (graph, root) = extrusion(
        rectangle(0.2, 40.0),
        Vec3::Z,
        40.0,
        Transform3::from_translation(Vec3::new(at + 0.1, 0.0, -20.0)),
    );
    exact_boundary(&graph, root)
        .unwrap()
        .brep()
        .unwrap()
        .clone()
}

/// The certified distance from `brep` to the wall at `at`.
fn distance_to_wall(brep: &ExactBRep, at: f64) -> (f64, f64) {
    let bounds = boundary_distance(brep, &wall(at), ACCURACY, Tolerance::METRE).unwrap();
    (bounds.lower, bounds.upper)
}

#[test]
fn a_tilted_round_column_is_its_closed_form() {
    let (radius, height) = (0.2, 3.0);
    let start = Vec3::new(1.0, -0.5, 0.25);
    let transform = tilted(start);
    let (graph, root) = extrusion(circle(radius), Vec3::Z, height, transform);
    let boundary = agreeing(&graph, root);
    assert_close(
        volume(boundary.brep().unwrap()),
        PI * radius * radius * height,
        1e-9,
    );

    // The axis runs from `start` along the tilted z; the farthest point
    // in x is on the rim of the end farther along x.
    let axis = transform.transform_vector3(Vec3::Z);
    let end = start + axis * height;
    let reach = start.x.max(end.x) + radius * (1.0 - axis.x * axis.x).sqrt();
    let (min, max) = boundary.extent();
    assert_close(max[0], reach, 1e-12);
    assert!(
        min[2] < start.z && max[2] > end.z.min(start.z),
        "{min:?} {max:?}"
    );

    let (lower, upper) = distance_to_wall(boundary.brep().unwrap(), reach + 0.8);
    assert!(lower <= 0.8 && 0.8 <= upper, "[{lower}, {upper}]");
    assert!(upper - lower <= 2.0 * ACCURACY, "[{lower}, {upper}]");
}

#[test]
fn a_horizontal_beam_and_a_downward_mirrored_extrusion_agree_with_their_meshes() {
    // A circle extruded along y: a horizontal round beam.
    let horizontal = Transform3::from_translation(Vec3::new(0.0, 0.0, 3.0))
        * Transform3::from_rotation_x(-FRAC_PI_2);
    let (graph, root) = extrusion(circle(0.15), Vec3::Z, 4.0, horizontal);
    let boundary = agreeing(&graph, root);
    let (min, max) = boundary.extent();
    assert_close(min[1], 0.0, 1e-12);
    assert_close(max[1], 4.0, 1e-12);
    assert_close(max[2], 3.15, 1e-12);

    // Extruded against the profile normal, obliquely, under a mirror: a
    // hollow rectangle with rounded corners.
    let mirrored = Transform3::from_translation(Vec3::new(2.0, 1.0, 0.5))
        * Transform3::from_scale(Vec3::new(-1.0, 1.0, 1.0))
        * Transform3::from_rotation_y(0.4);
    let profile = Profile::Rectangle(RectangleProfile {
        x: 0.6,
        y: 0.4,
        thickness: Some(0.05),
        outer_radius: Some(0.08),
        inner_radius: None,
    });
    let (graph, root) = extrusion(profile, Vec3::new(0.0, 0.0, -1.0), 2.0, mirrored);
    agreeing(&graph, root);
}

#[test]
fn a_section_with_several_voids_is_built() {
    let hole = |x: f64| match lower_derived(
        &circle(0.1),
        &Transform2::from_translation(Vec2::new(x, 0.0)),
        Tolerance::METRE,
    )
    .unwrap()
    {
        Profile::Contour(contour) => contour.outer,
        other => panic!("{other:?}"),
    };
    let profile = Profile::Contour(ContourProfile {
        outer: rectangle_contour(&RectangleProfile {
            x: 1.0,
            y: 0.4,
            thickness: None,
            outer_radius: None,
            inner_radius: None,
        })
        .unwrap()
        .outer,
        holes: vec![hole(-0.25), hole(0.25)],
    });
    let (graph, root) = extrusion(profile, Vec3::Z, 2.0, tilted(Vec3::new(0.0, 0.0, 1.0)));
    let boundary = agreeing(&graph, root);
    assert_close(
        volume(boundary.brep().unwrap()),
        2.0 * (0.4 - 2.0 * PI * 0.01),
        1e-9,
    );
}

#[test]
fn a_revolved_ring_is_its_closed_form() {
    // A circle of radius r centred R from the axis, turned a full turn: a
    // torus about the placed y axis.
    let (bend, radius) = (0.5, 0.1);
    let centre = Vec3::new(1.0, 2.0, 3.0);
    let transform = tilted(centre);
    let (graph, root) = revolution(
        moved(circle(radius), bend, 0.0),
        Point3::ZERO,
        TAU,
        transform,
    );
    // The kernel keeps a doubly curved surface within the 1 mm chord budget
    // of its mesh, splitting it between the profile and the turn
    // (axiolid/kernel#231), so the torus is compared at the budget itself.
    let boundary = agreeing(&graph, root);
    assert_close(
        volume(boundary.brep().unwrap()),
        2.0 * PI * PI * bend * radius * radius,
        1e-9,
    );

    let axis = transform.transform_vector3(Vec3::Y);
    let reach = centre.x + bend * (1.0 - axis.x * axis.x).sqrt() + radius;
    assert_close(boundary.extent().1[0], reach, 1e-12);
    let (lower, upper) = distance_to_wall(boundary.brep().unwrap(), reach + 0.5);
    assert!(lower <= 0.5 && 0.5 <= upper, "[{lower}, {upper}]");
    assert!(upper - lower <= 2.0 * ACCURACY, "[{lower}, {upper}]");
}

#[test]
fn a_partial_revolution_agrees_with_its_mesh() {
    // A rectangle 0.2 by 0.4 centred 0.6 from the axis, turned a third.
    let angle = TAU / 3.0;
    let (graph, root) = revolution(
        moved(rectangle(0.2, 0.4), -0.6, 0.1),
        Point3::ZERO,
        angle,
        tilted(Vec3::new(-1.0, 0.0, 2.0)),
    );
    let boundary = agreeing(&graph, root);
    // Pappus: the area times the path of its centroid.
    assert_close(volume(boundary.brep().unwrap()), 0.08 * 0.6 * angle, 1e-9);
}

#[test]
fn swept_disks_along_a_segment_and_an_arc_are_exact() {
    let (start, end) = (Point3::new(0.0, 0.0, 1.0), Point3::new(2.0, 1.0, 2.0));
    let (graph, root) = swept_disk(
        GeometryNode::Curve3(Curve3::Polyline(Polyline {
            points: vec![start, end],
            closed: false,
        })),
        None,
        0.05,
        None,
        Transform3::IDENTITY,
    );
    let boundary = agreeing(&graph, root);
    let length = (end - start).length();
    assert_close(volume(boundary.brep().unwrap()), PI * 0.0025 * length, 1e-9);
    let direction = (end - start) / length;
    let reach = end.x + 0.05 * (1.0 - direction.x * direction.x).sqrt();
    let (lower, upper) = distance_to_wall(boundary.brep().unwrap(), reach + 0.3);
    assert!(lower <= 0.3 && 0.3 <= upper, "[{lower}, {upper}]");

    // A bored disk along a quarter of a tilted circle of radius 1, placed
    // as a mapped item.
    let arc = Circle3 {
        frame: Frame3 {
            origin: Point3::new(0.5, 0.5, 1.0),
            x: Vec3::X,
            y: Vec3::Z,
            z: -Vec3::Y,
        },
        radius: 1.0,
    };
    let (graph, root) = swept_disk(
        GeometryNode::Curve3(Curve3::Circle(arc)),
        Some((0.25, 0.25 + FRAC_PI_2)),
        0.05,
        Some(0.03),
        tilted(Vec3::new(0.0, 1.0, 0.0)),
    );
    let boundary = agreeing(&graph, root);
    assert_close(
        volume(boundary.brep().unwrap()),
        PI * (0.0025 - 0.0009) * FRAC_PI_2,
        1e-9,
    );
}

#[test]
fn nested_mapped_instances_compose_into_one_placement() {
    let (graph, root) = graph(|builder| {
        let profile = push(builder, GeometryNode::Profile(circle(0.2)));
        let solid = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction: Vec3::Z,
                depth: 2.0,
            }),
        );
        let inner = placed(builder, solid, tilted(Vec3::new(0.5, 0.0, 0.0)));
        let mirrored = placed(
            builder,
            inner,
            Transform3::from_scale(Vec3::new(1.0, -1.0, 1.0)),
        );
        let only = push(builder, GeometryNode::Collection(vec![mirrored]));
        placed(
            builder,
            only,
            Transform3::from_translation(Vec3::new(3.0, 3.0, 0.0)),
        )
    });
    let boundary = agreeing(&graph, root);
    assert_close(volume(boundary.brep().unwrap()), PI * 0.04 * 2.0, 1e-9);
}

#[test]
fn what_has_no_exact_construction_is_refused_with_the_reason() {
    let refusal =
        |(graph, root): (GeometryGraph, NodeId)| exact_boundary(&graph, root).unwrap_err();

    let scaled = refusal(extrusion(
        circle(0.2),
        Vec3::Z,
        1.0,
        Transform3::from_scale(Vec3::splat(2.0)),
    ));
    assert!(scaled.contains("not rigid"), "{scaled}");

    let ellipse = Profile::Ellipse(EllipseProfile {
        semi_axis_x: 0.2,
        semi_axis_y: 0.1,
    });
    let revolved = refusal(revolution(
        moved(ellipse, 1.0, 0.0),
        Point3::ZERO,
        TAU,
        Transform3::IDENTITY,
    ));
    assert!(revolved.contains("refused"), "{revolved}");

    let cornered = refusal(swept_disk(
        GeometryNode::Curve3(Curve3::Polyline(Polyline {
            points: vec![Point3::ZERO, Point3::X, Point3::new(1.0, 1.0, 0.0)],
            closed: false,
        })),
        None,
        0.05,
        None,
        Transform3::IDENTITY,
    ));
    assert!(cornered.contains("one segment or one arc"), "{cornered}");

    let flat = refusal(extrusion(circle(0.2), Vec3::X, 1.0, Transform3::IDENTITY));
    assert!(flat.contains("profile plane"), "{flat}");

    let (union, revolved_tool, several) = {
        let mut builder = GeometryGraphBuilder::new();
        let profile = push(&mut builder, GeometryNode::Profile(rectangle(1.0, 1.0)));
        let solid = |builder: &mut GeometryGraphBuilder| {
            push(
                builder,
                GeometryNode::SolidOperation(SolidOperation::Extrusion {
                    profile,
                    direction: Vec3::Z,
                    depth: 1.0,
                }),
            )
        };
        let (left, right) = (solid(&mut builder), solid(&mut builder));
        let moved = placed(
            &mut builder,
            right,
            Transform3::from_translation(Vec3::new(0.5, 0.3, 0.2)),
        );
        let union = push(
            &mut builder,
            GeometryNode::SolidOperation(SolidOperation::Boolean {
                left,
                right: moved,
                operator: BooleanOperator::Union,
            }),
        );
        let disk = push(&mut builder, GeometryNode::Profile(moved_circle()));
        let turned = push(
            &mut builder,
            GeometryNode::SolidOperation(SolidOperation::Revolution {
                profile: disk,
                axis_origin: Point3::ZERO,
                axis_direction: Vec3::Y,
                angle: TAU,
            }),
        );
        let revolved_tool = push(
            &mut builder,
            GeometryNode::SolidOperation(SolidOperation::Boolean {
                left,
                right: turned,
                operator: BooleanOperator::Difference,
            }),
        );
        // A collection is refused for the first item that is.
        let several = push(&mut builder, GeometryNode::Collection(vec![left, union]));
        let graph = builder.finish(vec![union, revolved_tool, several]).unwrap();
        (
            exact_boundary(&graph, union).unwrap_err(),
            exact_boundary(&graph, revolved_tool).unwrap_err(),
            exact_boundary(&graph, several).unwrap_err(),
        )
    };
    assert!(union.contains("boolean union"), "{union}");
    assert!(
        revolved_tool.contains("not an extrusion"),
        "{revolved_tool}"
    );
    assert!(several.contains("boolean union"), "{several}");
}

/// A disk a metre off the revolution axis, so its turn is a torus.
fn moved_circle() -> Profile {
    moved(circle(0.1), 1.0, 0.0)
}

#[test]
fn a_hollow_circle_keeps_its_bore() {
    let profile = Profile::Circle(CircleProfile {
        radius: 0.3,
        thickness: Some(0.05),
    });
    let (graph, root) = extrusion(profile, Vec3::Z, 2.0, tilted(Vec3::ZERO));
    let boundary = agreeing(&graph, root);
    assert_close(
        volume(boundary.brep().unwrap()),
        PI * (0.09 - 0.0625) * 2.0,
        1e-9,
    );
}

/// A block 2 m square and 1 m high, less `profile` extruded `depth` along
/// `direction` from 0.5 m up.
fn block_less(profile: Profile, direction: Vec3, depth: f64) -> (GeometryGraph, NodeId) {
    graph(|builder| {
        let block = push(builder, GeometryNode::Profile(rectangle(2.0, 2.0)));
        let block = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile: block,
                direction: Vec3::Z,
                depth: 1.0,
            }),
        );
        let profile = push(builder, GeometryNode::Profile(profile));
        let cut = push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction,
                depth,
            }),
        );
        let cut = placed(
            builder,
            cut,
            Transform3::from_translation(Vec3::new(0.0, 0.0, 0.5)),
        );
        push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Boolean {
                left: block,
                right: cut,
                operator: BooleanOperator::Difference,
            }),
        )
    })
}

/// An extrusion oblique to its profile's normal of a profile with arcs is
/// built by the kernel with exact oblique walls (axiolid/kernel#280), so
/// its exact boundary agrees with its mesh and has the volume of the
/// profile's area times the height it rises, upward and downward: a
/// rounded rectangle and a round column, placed and tilted. An oblique
/// ellipse is refused by the kernel by name.
#[test]
fn an_oblique_extrusion_of_a_profile_with_arcs_is_built_exact() {
    let rounded = || {
        Profile::Rectangle(RectangleProfile {
            x: 0.6,
            y: 0.4,
            thickness: None,
            outer_radius: Some(0.1),
            inner_radius: None,
        })
    };
    let rounded_area = 0.6 * 0.4 - (4.0 - PI) * 0.01;
    let depth = 0.75;
    for direction in [Vec3::new(0.3, -0.2, 1.0), Vec3::new(0.3, -0.2, -1.0)] {
        let rise = depth / direction.length();
        let (body, root) = extrusion(
            rounded(),
            direction,
            depth,
            tilted(Vec3::new(3.0, 1.0, 2.0)),
        );
        let boundary = agreeing(&body, root);
        assert_close(volume(boundary.brep().unwrap()), rounded_area * rise, 1e-9);

        let (body, root) = extrusion(circle(0.2), direction, depth, Transform3::IDENTITY);
        let boundary = agreeing(&body, root);
        assert_close(volume(boundary.brep().unwrap()), PI * 0.04 * rise, 1e-9);
    }

    let ellipse = Profile::Ellipse(EllipseProfile {
        semi_axis_x: 0.2,
        semi_axis_y: 0.1,
    });
    let (body, root) = extrusion(
        ellipse,
        Vec3::new(0.3, -0.2, 1.0),
        depth,
        Transform3::IDENTITY,
    );
    let refused = exact_boundary(&body, root).unwrap_err();
    assert!(refused.contains("oblique ellipse extrusion"), "{refused}");
}

/// A difference with an oblique round opening is refused by the kernel's
/// exact boolean by name (axiolid/kernel#287), never built wrong; the same
/// opening of straight edges is subtracted.
#[test]
fn a_difference_with_an_oblique_round_opening_is_refused_by_the_kernel() {
    let direction = Vec3::new(0.3, -0.2, 1.0);
    let (body, root) = block_less(circle(0.2), direction, 0.75);
    let refused = exact_boundary(&body, root).unwrap_err();
    assert!(!refused.is_empty());

    let (body, root) = block_less(rectangle(0.6, 0.4), direction, 0.75);
    agreeing(&body, root);
}

/// An extrusion along its profile's plane sweeps no volume, and the kernel
/// refuses it by name, as it does one leaving the plane by no more than
/// the exact construction's tolerance (1 um); one that leaves the plane by
/// more is built.
#[test]
fn an_extrusion_along_its_profile_plane_is_refused_by_the_kernel() {
    for direction in [Vec3::X, Vec3::new(1.0, 0.0, 5e-7)] {
        let (body, root) = extrusion(rectangle(1.0, 1.0), direction, 1.0, Transform3::IDENTITY);
        let refused = exact_boundary(&body, root).unwrap_err();
        assert!(
            refused.contains("extrusion direction in the profile plane"),
            "{direction:?}: {refused}"
        );
    }
    let (body, root) = extrusion(
        rectangle(1.0, 1.0),
        Vec3::new(1.0, 0.0, 0.01),
        1.0,
        Transform3::IDENTITY,
    );
    agreeing(&body, root);
}
