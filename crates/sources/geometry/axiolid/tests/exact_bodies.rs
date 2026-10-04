//! Exact bodies beyond one solid: walls less their openings
//! (axiolid/kernel#228, #236), walls clipped by roof planes (#234) and
//! bodies of several items (#229).
//!
//! Each body is built from a geometry graph as a host lowers it, meshed by
//! the kernel's mesh compiler and built exactly by `exact_boundary`; the
//! two must agree, and the certified distances measured on the exact body
//! must hold the closed form where the chord meshes leave it open.

use axiolid_brep::ExactBRep;
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Mat3, Plane3, Point2, Point3, Tolerance, Transform3, Vec3};
use axiolid_curve::{Curve2, Curve3, Polyline, Polyline2};
use axiolid_measure::exact_properties;
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_compile::ReferenceMeshCompiler;
use axiolid_mesh_compile_contract::MeshCompiler;
use axiolid_model::{
    GeometryGraph, GeometryGraphBuilder, GeometryNode, Instance, NodeId, SolidOperation,
};
use axiolid_primitive::HalfSpace;
use axiolid_profile::{CircleProfile, Profile, RectangleProfile, SectionProfile};
use axioval_axiolid::proximity::CERTIFIED_ACCURACY_METRES;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService, ExactBoundary, exact_boundary};
use axioval_engine::{
    ProximityError, ProximityProjection, ProximityRequest, ProximityService, SurfaceBasis,
    SurfaceDistanceRequest,
};
use axioval_ir::{ObjectId, SourceId};

/// The chord budget the mesh compiler keeps to, declared for its meshes.
const DEVIATION: f64 = 1e-3;

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
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

fn rectangle(x: f64, y: f64) -> Profile {
    Profile::Rectangle(RectangleProfile {
        x,
        y,
        thickness: None,
        outer_radius: None,
        inner_radius: None,
    })
}

fn circle(radius: f64) -> Profile {
    Profile::Circle(CircleProfile {
        radius,
        thickness: None,
    })
}

/// `profile` extruded `depth` along its normal, placed by `transform`.
fn extrusion(
    builder: &mut GeometryGraphBuilder,
    profile: Profile,
    depth: f64,
    transform: Transform3,
) -> NodeId {
    let profile = push(builder, GeometryNode::Profile(profile));
    let solid = push(
        builder,
        GeometryNode::SolidOperation(SolidOperation::Extrusion {
            profile,
            direction: Vec3::Z,
            depth,
        }),
    );
    placed(builder, solid, transform)
}

fn difference(builder: &mut GeometryGraphBuilder, left: NodeId, right: NodeId) -> NodeId {
    push(
        builder,
        GeometryNode::SolidOperation(SolidOperation::Boolean {
            left,
            right,
            operator: BooleanOperator::Difference,
        }),
    )
}

/// An opening `width` by `height` whose bottom centre stands at `at`,
/// extruded `depth` across the wall (along `+y`) from `at.y`, as a door
/// or window opening is extruded perpendicular to its wall.
fn opening(
    builder: &mut GeometryGraphBuilder,
    (width, height, depth): (f64, f64, f64),
    at: Vec3,
) -> NodeId {
    // The profile plane turned up: its z onto +y and its y onto -z, by
    // axes of zeros and ones as a placement along coordinate axes reads.
    // The profile is centred, so its centre goes half the height up.
    let turn = Transform3::from_cols(Vec3::X, -Vec3::Z, Vec3::Y, Vec3::ZERO);
    let transform = Transform3::from_translation(at + Vec3::Z * (height / 2.0)) * turn;
    extrusion(builder, rectangle(width, height), depth, transform)
}

/// A wall 4 m long (`x` in `[0, 4]`), 0.2 m thick (`y` in `[0, 0.2]`)
/// and 3 m high, less a door 1 m wide and 2.1 m high (`x` in
/// `[0.5, 1.5]`) and a window 1.2 m wide (`x` in `[2.2, 3.4]`, `z` in
/// `[1, 2]`), both standing 0.1 m out of each face.
fn wall_with_openings(builder: &mut GeometryGraphBuilder) -> NodeId {
    let wall = extrusion(
        builder,
        rectangle(4.0, 0.2),
        3.0,
        Transform3::from_translation(Vec3::new(2.0, 0.1, 0.0)),
    );
    // The door reaches below the wall's foot.
    let door = opening(builder, (1.0, 2.2, 0.4), Vec3::new(1.0, -0.1, -0.1));
    let window = opening(builder, (1.2, 1.0, 0.4), Vec3::new(2.8, -0.1, 1.0));
    let less_door = difference(builder, wall, door);
    difference(builder, less_door, window)
}

const WALL_VOLUME: f64 = 4.0 * 0.2 * 3.0 - 1.0 * 2.1 * 0.2 - 1.2 * 1.0 * 0.2;

/// A pipe of radius 0.05 m along `y` through the window, its axis at
/// `x` 2.8, `z` 1.3: 0.25 m above the sill, which is what it comes
/// nearest.
fn pipe(builder: &mut GeometryGraphBuilder) -> NodeId {
    let line = push(
        builder,
        GeometryNode::Curve3(Curve3::Polyline(Polyline {
            points: vec![Point3::new(2.8, -1.0, 1.3), Point3::new(2.8, 1.2, 1.3)],
            closed: false,
        })),
    );
    push(
        builder,
        GeometryNode::SolidOperation(SolidOperation::SweptDisk {
            directrix: line,
            radius: 0.05,
            inner_radius: None,
            parameter_range: None,
            fillet_radius: None,
        }),
    )
}

const PIPE_TO_SILL: f64 = 0.3 - 0.05;

fn mesh(graph: &GeometryGraph, root: NodeId) -> TriMesh {
    ReferenceMeshCompiler::new(BoolmeshBoolean)
        .compile_mesh(graph, root, &ExecutionOptions::new(Tolerance::MILLIMETRE))
        .unwrap()
}

/// The exact boundary of `root`, checked against the kernel's mesh.
fn agreeing(graph: &GeometryGraph, root: NodeId) -> ExactBoundary {
    let boundary = exact_boundary(graph, root).unwrap();
    AxiolidGeometry::new()
        .with_tessellated_mesh(id("body"), mesh(graph, root), DEVIATION)
        .check_exact_boundary(&id("body"), &boundary)
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

/// The wall (planar, so its mesh is exact) and the pipe (tessellated),
/// with their exact bodies when `exact` is set, and how far the wall's
/// exact body may lie from the model's (its booleans' rounding).
fn wall_and_pipe(exact: bool) -> (AxiolidGeometry, f64) {
    let mut builder = GeometryGraphBuilder::new();
    let wall = wall_with_openings(&mut builder);
    let pipe = pipe(&mut builder);
    let graph = builder.finish(vec![wall, pipe]).unwrap();
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), mesh(&graph, wall))
        .with_tessellated_mesh(id("pipe"), mesh(&graph, pipe), DEVIATION);
    if !exact {
        return (geometry, 0.0);
    }
    let (wall, pipe) = (agreeing(&graph, wall), agreeing(&graph, pipe));
    let widening = wall.body().widening_metres();
    let geometry = geometry
        .with_exact_body(id("wall"), wall.into_body())
        .with_exact_body(id("pipe"), pipe.into_body());
    (geometry, widening)
}

fn pipe_to_wall(geometry: AxiolidGeometry) -> (f64, f64) {
    let request = ProximityRequest::try_new(id("pipe"), id("wall")).unwrap();
    let measured = AxiolidProximityService::new(geometry)
        .measure_distance(&request)
        .unwrap();
    assert!(!measured.evidence().exact, "a tessellated pair is inexact");
    measured.interval_metres()
}

#[test]
fn a_wall_less_a_door_and_a_window_along_the_axes_is_exact() {
    let mut builder = GeometryGraphBuilder::new();
    let wall = wall_with_openings(&mut builder);
    let graph = builder.finish(vec![wall]).unwrap();
    let boundary = agreeing(&graph, wall);
    // Openings placed by axis matrices meet the wall's faces exactly or
    // cross them transversally: the kernel's general boolean decides
    // nothing within a tolerance (axiolid/kernel#236), so the body is the
    // exact difference of the operands as given. Its two booleans may
    // still merge constructed points within 2^-40 of the operands' largest
    // coordinate (a few metres here), which widens its distances.
    assert!(boundary.body().is_exact());
    let rounding = boundary.body().rounding_metres();
    assert!(
        rounding > 0.0 && rounding <= 2.0 * 10.0 / 1_099_511_627_776.0,
        "{rounding}"
    );
    assert_close(volume(boundary.brep().unwrap()), WALL_VOLUME, 1e-9);
}

/// A wall 4 m by 0.2 m by 3 m less a window 1.2 m by 1 m standing 0.1 m
/// out of its front face and flush with its back face, both turned
/// 0.3 rad about z and moved to (100, 50): the window's end face agrees
/// with the wall's back face only up to rounding.
fn turned_wall_with_flush_window(builder: &mut GeometryGraphBuilder) -> NodeId {
    let world = Transform3::from_translation(Vec3::new(100.0, 50.0, 0.0))
        * Transform3::from_rotation_z(0.3);
    let wall = extrusion(
        builder,
        rectangle(4.0, 0.2),
        3.0,
        world * Transform3::from_translation(Vec3::new(2.0, 0.1, 0.0)),
    );
    let turn = Transform3::from_cols(Vec3::X, -Vec3::Z, Vec3::Y, Vec3::ZERO);
    let window = extrusion(
        builder,
        rectangle(1.2, 1.0),
        0.3,
        world * Transform3::from_translation(Vec3::new(2.8, -0.1, 1.5)) * turn,
    );
    difference(builder, wall, window)
}

#[test]
fn a_turned_wall_with_a_flush_window_is_perturbed_by_what_the_kernel_reports() {
    let mut builder = GeometryGraphBuilder::new();
    let wall = turned_wall_with_flush_window(&mut builder);
    let graph = builder.finish(vec![wall]).unwrap();
    let boundary = agreeing(&graph, wall);
    // Without a tolerance the kernel refuses the near-coincident faces;
    // within one it reads them as one and reports how far that moved them
    // (#236). The body is the exact difference of operands moved by that
    // much, far less than the tolerance itself: marked perturbed by it,
    // never exact.
    let perturbation = boundary.body().perturbation_metres();
    assert!(!boundary.body().is_exact());
    assert!(perturbation > 0.0 && perturbation < 1e-9, "{perturbation}");
    assert_close(
        volume(boundary.brep().unwrap()),
        4.0 * 0.2 * 3.0 - 1.2 * 1.0 * 0.2,
        1e-9,
    );
}

#[test]
fn the_window_decides_a_pipes_distance_the_mesh_left_open() {
    // A minimum just under the true 0.25 m: within the pipe's chord band.
    let minimum = PIPE_TO_SILL - 1e-4;
    let (lower, upper) = pipe_to_wall(wall_and_pipe(false).0);
    assert!(
        lower <= PIPE_TO_SILL && PIPE_TO_SILL <= upper,
        "[{lower}, {upper}]"
    );
    assert!(
        lower < minimum,
        "the mesh alone leaves it open: [{lower}, {upper}]"
    );

    // Through the window the exact bodies certify it, widened by the
    // wall's rounding on both sides.
    let (geometry, widening) = wall_and_pipe(true);
    let (lower, upper) = pipe_to_wall(geometry);
    assert!(
        lower <= PIPE_TO_SILL - widening && PIPE_TO_SILL + widening <= upper,
        "[{lower}, {upper}]"
    );
    assert!(
        upper - lower <= 2.0 * widening + 4.0 * CERTIFIED_ACCURACY_METRES,
        "[{lower}, {upper}]"
    );
    assert!(lower > minimum, "certified: [{lower}, {upper}]");
}

#[test]
fn a_shaft_through_a_slab_is_exact() {
    // Two bare sharp rectangles along +z go the kernel's prism path, whose
    // predicates are exact: nothing is decided within a tolerance.
    let mut builder = GeometryGraphBuilder::new();
    let bare = |builder: &mut GeometryGraphBuilder, x: f64, y: f64| {
        let profile = push(builder, GeometryNode::Profile(rectangle(x, y)));
        push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction: Vec3::Z,
                depth: 0.3,
            }),
        )
    };
    let (slab, shaft) = (bare(&mut builder, 4.0, 3.0), bare(&mut builder, 1.0, 1.0));
    let root = difference(&mut builder, slab, shaft);
    let graph = builder.finish(vec![root]).unwrap();
    let boundary = agreeing(&graph, root);
    assert!(boundary.body().is_exact());
    assert_close(volume(boundary.brep().unwrap()), (12.0 - 1.0) * 0.3, 1e-12);
}

/// A round column of radius 0.2 m standing on a square footing 1 m wide
/// and 0.5 m high, as two items of one body under one placement at
/// `(x, y)`: the column's base stands at `base`, on the footing's top at
/// 0.5.
fn column_on_footing(builder: &mut GeometryGraphBuilder, at: [f64; 2], base: f64) -> NodeId {
    let footing = extrusion(builder, rectangle(1.0, 1.0), 0.5, Transform3::IDENTITY);
    let column = extrusion(
        builder,
        circle(0.2),
        3.0,
        Transform3::from_translation(Vec3::Z * base),
    );
    let items = push(builder, GeometryNode::Collection(vec![footing, column]));
    placed(
        builder,
        items,
        Transform3::from_translation(Vec3::new(at[0], at[1], 0.0))
            * Transform3::from_rotation_z(0.4),
    )
}

/// A wall `x` in `[1.5, 1.7]` along `y`, 3.5 m high.
fn plain_wall(builder: &mut GeometryGraphBuilder) -> NodeId {
    extrusion(
        builder,
        rectangle(0.2, 6.0),
        3.5,
        Transform3::from_translation(Vec3::new(1.6, 0.0, 0.0)),
    )
}

#[test]
fn a_column_on_its_footing_is_measured_exactly_against_a_wall() {
    let mut builder = GeometryGraphBuilder::new();
    let body = column_on_footing(&mut builder, [0.0, 0.0], 0.5);
    let wall = plain_wall(&mut builder);
    let graph = builder.finish(vec![body, wall]).unwrap();
    let boundary = agreeing(&graph, body);
    assert_eq!(boundary.body().items().len(), 2);
    assert!(boundary.body().is_exact());
    assert!(
        boundary.brep().is_none(),
        "several items stay in their frame"
    );

    // The column's surface is 1.5 - 0.2 from the wall; the footing,
    // turned 0.4 rad, reaches cos 0.4 + sin 0.4 of its half width.
    let footing = 0.5 * (0.4_f64.cos() + 0.4_f64.sin());
    let expected = (1.5 - 0.2_f64).min(1.5 - footing);
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("column"), mesh(&graph, body), DEVIATION)
        .with_mesh(id("wall"), mesh(&graph, wall))
        .with_exact_body(id("column"), boundary.into_body())
        .with_exact_body(id("wall"), agreeing(&graph, wall).into_body());
    let request = ProximityRequest::try_new(id("column"), id("wall")).unwrap();
    let measured = AxiolidProximityService::new(geometry)
        .measure_proximity(&request)
        .unwrap();
    let certified = measured.certified_separation().expect("certified");
    assert!(
        certified.lower_metres() <= expected && expected <= certified.upper_metres(),
        "{certified:?} must hold {expected}"
    );
    assert!(
        certified.upper_metres() - certified.lower_metres() <= 4.0 * CERTIFIED_ACCURACY_METRES,
        "{certified:?}"
    );
}

#[test]
fn a_moved_column_on_its_footing_is_compared_between_its_exact_bodies() {
    // Two revisions of the same body, the second moved 5 mm along x: the
    // surface distance between their unions' boundaries is 5 mm.
    let mut builder = GeometryGraphBuilder::new();
    let before = column_on_footing(&mut builder, [0.0, 0.0], 0.5);
    let after = column_on_footing(&mut builder, [0.005, 0.0], 0.5);
    let graph = builder.finish(vec![before, after]).unwrap();
    let session = |object: &ObjectId, root: NodeId| {
        AxiolidProximityService::new(
            AxiolidGeometry::new()
                .with_tessellated_mesh(object.clone(), mesh(&graph, root), DEVIATION)
                .with_exact_body(object.clone(), agreeing(&graph, root).into_body()),
        )
    };
    let revised = ObjectId::new(SourceId::new("cad", "revised").unwrap(), "column").unwrap();
    let (before, after) = (session(&id("column"), before), session(&revised, after));
    let surface = after.body_surface(&revised).unwrap();
    assert!(surface.exact_boundary().is_some());
    let request =
        SurfaceDistanceRequest::try_new(id("column"), std::sync::Arc::new(surface), 1e-4).unwrap();
    let measured = before.measure_surface_distance(&request).unwrap();
    assert_eq!(measured.basis(), SurfaceBasis::ExactBoundary);
    assert!(measured.evidence().exact);
    let (lower, upper) = (
        measured.distance().lower_metres(),
        measured.distance().upper_metres(),
    );
    assert!(lower <= 0.005 && 0.005 <= upper, "[{lower}, {upper}]");
    assert!(upper - lower <= 2e-4, "[{lower}, {upper}]");
}

#[test]
fn items_the_kernel_cannot_unite_keep_the_mesh() {
    // A column sunk 0.1 m into its footing: the items overlap, so the
    // boundary of their union is not formed (`ItemsOverlap`) and the
    // comparison falls back to the meshes, which refuse a tessellated
    // pair rather than guess. The distance in space needs no union and is
    // still certified.
    let mut builder = GeometryGraphBuilder::new();
    let before = column_on_footing(&mut builder, [0.0, 0.0], 0.4);
    let after = column_on_footing(&mut builder, [0.005, 0.0], 0.4);
    let wall = plain_wall(&mut builder);
    let graph = builder.finish(vec![before, after, wall]).unwrap();
    let revised = ObjectId::new(SourceId::new("cad", "revised").unwrap(), "column").unwrap();
    let session = |object: &ObjectId, root: NodeId| {
        AxiolidProximityService::new(
            AxiolidGeometry::new()
                .with_tessellated_mesh(object.clone(), mesh(&graph, root), DEVIATION)
                .with_exact_body(object.clone(), agreeing(&graph, root).into_body())
                .with_mesh(id("wall"), mesh(&graph, wall))
                .with_exact_body(id("wall"), agreeing(&graph, wall).into_body()),
        )
    };
    let (base, revision) = (session(&id("column"), before), session(&revised, after));
    let surface = revision.body_surface(&revised).unwrap();
    let request =
        SurfaceDistanceRequest::try_new(id("column"), std::sync::Arc::new(surface), 1e-4).unwrap();
    assert_eq!(
        base.measure_surface_distance(&request).unwrap_err(),
        ProximityError::EvidenceFidelityMismatch
    );

    let request = ProximityRequest::try_new(id("column"), id("wall")).unwrap();
    let measured = base.measure_proximity(&request).unwrap();
    assert!(measured.certified_separation().is_some());
}

#[test]
fn a_wall_built_within_a_tolerance_is_compared_on_its_meshes() {
    // The surface distance between revisions must be measured on exact
    // surfaces: a turned wall less a flush window is perturbed, so its
    // exact (planar) meshes are compared instead, exactly.
    let mut builder = GeometryGraphBuilder::new();
    let wall = turned_wall_with_flush_window(&mut builder);
    let graph = builder.finish(vec![wall]).unwrap();
    let revised = ObjectId::new(SourceId::new("cad", "revised").unwrap(), "wall").unwrap();
    let session = |object: &ObjectId| {
        AxiolidProximityService::new(
            AxiolidGeometry::new()
                .with_mesh(object.clone(), mesh(&graph, wall))
                .with_exact_body(object.clone(), agreeing(&graph, wall).into_body()),
        )
    };
    let (base, revision) = (session(&id("wall")), session(&revised));
    let surface = revision.body_surface(&revised).unwrap();
    assert!(surface.exact_boundary().is_some());
    let request =
        SurfaceDistanceRequest::try_new(id("wall"), std::sync::Arc::new(surface), 1e-4).unwrap();
    let measured = base.measure_surface_distance(&request).unwrap();
    assert_eq!(measured.basis(), SurfaceBasis::Mesh);
    assert!(measured.evidence().exact);
    assert!(measured.distance().upper_metres() < 1e-9, "{measured:?}");
}

#[test]
fn a_wall_less_its_openings_is_compared_between_its_exact_bodies() {
    // An exact difference (openings along the axes) feeds the surface
    // distance: the revision moved 5 mm across the wall is 5 mm away,
    // measured between the boundaries and cited as exact, the interval
    // widened by both bodies' rounding.
    let mut builder = GeometryGraphBuilder::new();
    let before = wall_with_openings(&mut builder);
    let moved = wall_with_openings(&mut builder);
    let after = placed(
        &mut builder,
        moved,
        Transform3::from_translation(Vec3::new(0.0, 0.005, 0.0)),
    );
    let graph = builder.finish(vec![before, after]).unwrap();
    let revised = ObjectId::new(SourceId::new("cad", "revised").unwrap(), "wall").unwrap();
    let session = |object: &ObjectId, root: NodeId| {
        AxiolidProximityService::new(
            AxiolidGeometry::new()
                .with_mesh(object.clone(), mesh(&graph, root))
                .with_exact_body(object.clone(), agreeing(&graph, root).into_body()),
        )
    };
    let (base, revision) = (session(&id("wall"), before), session(&revised, after));
    let surface = revision.body_surface(&revised).unwrap();
    let request =
        SurfaceDistanceRequest::try_new(id("wall"), std::sync::Arc::new(surface), 1e-4).unwrap();
    let measured = base.measure_surface_distance(&request).unwrap();
    assert_eq!(measured.basis(), SurfaceBasis::ExactBoundary);
    assert!(measured.evidence().exact);
    let (lower, upper) = (
        measured.distance().lower_metres(),
        measured.distance().upper_metres(),
    );
    assert!(lower <= 0.005 && 0.005 <= upper, "[{lower}, {upper}]");
    assert!(upper - lower <= 2e-4, "[{lower}, {upper}]");
}

/// A gable wall 6 m long (`x` in `[-3, 3]`), 0.3 m thick and 3 m high
/// under two roof planes `z = 2.4 -+ 0.3 x` (the half-spaces above them
/// taken away, as an `IfcBooleanClippingResult` clips a wall), less a
/// round window of radius 0.3 m centred at `x = -1.2`, `z = 1.2` and
/// extruded across it.
fn gable_wall_with_round_window(builder: &mut GeometryGraphBuilder) -> NodeId {
    let mut wall = extrusion(builder, rectangle(6.0, 0.3), 3.0, Transform3::IDENTITY);
    for slope in [0.3, -0.3] {
        let roof = push(
            builder,
            GeometryNode::HalfSpace(HalfSpace {
                boundary: Plane3 {
                    origin: Point3::new(0.0, 0.0, 2.4),
                    normal: Vec3::new(-slope, 0.0, 1.0),
                },
                agreement: true,
            }),
        );
        wall = difference(builder, wall, roof);
    }
    let turn = Transform3::from_cols(Vec3::X, -Vec3::Z, Vec3::Y, Vec3::ZERO);
    let window = extrusion(
        builder,
        circle(0.3),
        0.5,
        Transform3::from_translation(Vec3::new(-1.2, -0.25, 1.2)) * turn,
    );
    difference(builder, wall, window)
}

/// The gable wall's area, `2 (2.4 * 3 - 0.3 * 9 / 2)`, times its
/// thickness, less the window's cylinder.
const GABLE_VOLUME: f64 = 11.7 * 0.3 - std::f64::consts::PI * 0.09 * 0.3;

#[test]
fn a_roof_clipped_wall_with_a_window_is_exact() {
    let mut builder = GeometryGraphBuilder::new();
    let wall = gable_wall_with_round_window(&mut builder);
    let graph = builder.finish(vec![wall]).unwrap();
    // Its extent is the clipped one, from its edges: the ridge at 2.4 m,
    // not the uncut wall's 3 m, so it agrees with the mesh.
    let boundary = agreeing(&graph, wall);
    let (low, high) = boundary.extent();
    assert_close(low[2], 0.0, 1e-12);
    assert_close(high[2], 2.4, 1e-12);
    assert!(boundary.body().is_exact(), "{:?}", boundary.body());
    assert!(boundary.body().rounding_metres() > 0.0);
    assert_close(volume(boundary.brep().unwrap()), GABLE_VOLUME, 1e-9);
}

#[test]
fn a_roof_slope_certifies_a_pipes_distance() {
    // A pipe of radius 0.05 m along y over the right-hand slope, its axis
    // at x = 1, z = 2.6: 0.5 / sqrt(1.09) from the plane z = 2.4 - 0.3 x,
    // less its radius.
    let expected = 0.5 / 1.09_f64.sqrt() - 0.05;
    let mut builder = GeometryGraphBuilder::new();
    let wall = gable_wall_with_round_window(&mut builder);
    let line = push(
        &mut builder,
        GeometryNode::Curve3(Curve3::Polyline(Polyline {
            points: vec![Point3::new(1.0, -1.0, 2.6), Point3::new(1.0, 1.2, 2.6)],
            closed: false,
        })),
    );
    let pipe = push(
        &mut builder,
        GeometryNode::SolidOperation(SolidOperation::SweptDisk {
            directrix: line,
            radius: 0.05,
            inner_radius: None,
            parameter_range: None,
            fillet_radius: None,
        }),
    );
    let graph = builder.finish(vec![wall, pipe]).unwrap();
    let (wall_body, pipe_body) = (agreeing(&graph, wall), agreeing(&graph, pipe));
    let widening = wall_body.body().widening_metres();
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("wall"), mesh(&graph, wall), DEVIATION)
        .with_tessellated_mesh(id("pipe"), mesh(&graph, pipe), DEVIATION)
        .with_exact_body(id("wall"), wall_body.into_body())
        .with_exact_body(id("pipe"), pipe_body.into_body());
    let (lower, upper) = pipe_to_wall(geometry);
    assert!(lower <= expected && expected <= upper, "[{lower}, {upper}]");
    assert!(
        upper - lower <= 2.0 * widening + 4.0 * CERTIFIED_ACCURACY_METRES,
        "[{lower}, {upper}]"
    );
}

#[test]
fn a_wall_clipped_by_a_bounded_half_space_is_exact() {
    // The wall above z = 2 taken away over x in [0, 3.5] only: a
    // polygonally bounded half-space, its boundary framed in the plane.
    let mut builder = GeometryGraphBuilder::new();
    let wall = extrusion(&mut builder, rectangle(6.0, 0.3), 3.0, Transform3::IDENTITY);
    let plane = push(
        &mut builder,
        GeometryNode::HalfSpace(HalfSpace {
            boundary: Plane3 {
                origin: Point3::new(0.0, 0.0, 2.0),
                normal: Vec3::Z,
            },
            agreement: true,
        }),
    );
    let boundary = push(
        &mut builder,
        GeometryNode::Curve2(Curve2::Polyline(Polyline2 {
            points: vec![
                Point2::new(0.0, -1.0),
                Point2::new(3.5, -1.0),
                Point2::new(3.5, 1.0),
                Point2::new(0.0, 1.0),
            ],
            closed: true,
        })),
    );
    let bounded = push(
        &mut builder,
        GeometryNode::SolidOperation(SolidOperation::BoundedHalfSpace {
            half_space: plane,
            boundary,
            placement: Transform3::IDENTITY,
        }),
    );
    let clipped = difference(&mut builder, wall, bounded);
    let graph = builder.finish(vec![clipped]).unwrap();
    let boundary = agreeing(&graph, clipped);
    assert!(boundary.body().is_exact());
    assert_close(
        volume(boundary.brep().unwrap()),
        6.0 * 0.3 * 3.0 - 3.0 * 0.3 * 1.0,
        1e-9,
    );
}

#[test]
fn a_column_on_its_footing_is_certified_against_a_wall_in_plan() {
    // A body of two items in plan (axiolid/kernel#237): the footing,
    // turned 0.4 rad, comes nearest the wall's face at x = 1.5.
    let mut builder = GeometryGraphBuilder::new();
    let body = column_on_footing(&mut builder, [0.0, 0.0], 0.5);
    let wall = plain_wall(&mut builder);
    let graph = builder.finish(vec![body, wall]).unwrap();
    let expected = 1.5 - 0.5 * (0.4_f64.cos() + 0.4_f64.sin());
    let geometry = |exact: bool| {
        let geometry = AxiolidGeometry::new()
            .with_tessellated_mesh(id("column"), mesh(&graph, body), DEVIATION)
            .with_mesh(id("wall"), mesh(&graph, wall));
        if !exact {
            return geometry;
        }
        geometry
            .with_exact_body(id("column"), agreeing(&graph, body).into_body())
            .with_exact_body(id("wall"), agreeing(&graph, wall).into_body())
    };
    let horizontal = |exact: bool| {
        let request =
            ProximityRequest::projected(id("column"), id("wall"), ProximityProjection::Horizontal)
                .unwrap();
        AxiolidProximityService::new(geometry(exact))
            .measure_distance(&request)
            .unwrap()
            .interval_metres()
    };
    let (lower, upper) = horizontal(false);
    assert!(lower <= expected && expected <= upper, "[{lower}, {upper}]");
    assert!(
        upper - lower >= DEVIATION,
        "the chord band: [{lower}, {upper}]"
    );
    let (lower, upper) = horizontal(true);
    assert!(lower <= expected && expected <= upper, "[{lower}, {upper}]");
    assert!(
        upper - lower <= 4.0 * CERTIFIED_ACCURACY_METRES,
        "certified: [{lower}, {upper}]"
    );
}

/// A slab 4 m by 3 m and 0.3 m thick less a 1 m square shaft at its
/// centre, both bare prisms along `+z`.
fn slab_with_shaft(builder: &mut GeometryGraphBuilder) -> NodeId {
    let bare = |builder: &mut GeometryGraphBuilder, x: f64, y: f64| {
        let profile = push(builder, GeometryNode::Profile(rectangle(x, y)));
        push(
            builder,
            GeometryNode::SolidOperation(SolidOperation::Extrusion {
                profile,
                direction: Vec3::Z,
                depth: 0.3,
            }),
        )
    };
    let (slab, shaft) = (bare(builder, 4.0, 3.0), bare(builder, 1.0, 1.0));
    difference(builder, slab, shaft)
}

#[test]
fn a_column_over_a_shafts_edge_overlaps_the_slab_in_plan() {
    // A round column standing in the shaft and reaching 0.5 mm over its edge
    // at x = 0.5 onto the slab: within the chord deviation, so the meshes
    // leave the plan overlap open, while the exact bodies (the slab an
    // exact difference) show it.
    let mut builder = GeometryGraphBuilder::new();
    let slab = slab_with_shaft(&mut builder);
    let column = extrusion(
        &mut builder,
        circle(0.2),
        2.7,
        Transform3::from_translation(Vec3::new(0.3005, 0.0, 0.3)),
    );
    let graph = builder.finish(vec![slab, column]).unwrap();
    let overlap = |exact: bool| {
        let mut geometry = AxiolidGeometry::new()
            .with_mesh(id("slab"), mesh(&graph, slab))
            .with_tessellated_mesh(id("column"), mesh(&graph, column), DEVIATION);
        if exact {
            geometry = geometry
                .with_exact_body(id("slab"), agreeing(&graph, slab).into_body())
                .with_exact_body(id("column"), agreeing(&graph, column).into_body());
        }
        let request =
            ProximityRequest::projected(id("column"), id("slab"), ProximityProjection::PlanOverlap)
                .unwrap();
        AxiolidProximityService::new(geometry)
            .measure_distance(&request)
            .unwrap()
            .interval_metres()
    };
    assert_eq!(overlap(false), (0.0, f64::INFINITY));
    assert_eq!(overlap(true), (0.0, 0.0));
}

#[test]
fn a_body_perturbed_by_zero_certifies_neither_plan_overlap_nor_surface_distance() {
    // A boolean's report may name a reading within tolerance whose
    // magnitudes are zero (at no tolerance the kernel refuses one since
    // axiolid/kernel#251). Such a body is perturbed all the same: it shows
    // no plan overlap and feeds no exact surface distance.
    let mut builder = GeometryGraphBuilder::new();
    let slab = slab_with_shaft(&mut builder);
    let column = extrusion(
        &mut builder,
        circle(0.2),
        2.7,
        Transform3::from_translation(Vec3::new(0.3005, 0.0, 0.3)),
    );
    let graph = builder.finish(vec![slab, column]).unwrap();
    let decided = agreeing(&graph, slab).into_body().with_perturbation(0.0);
    assert!(!decided.is_exact());
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), mesh(&graph, slab))
        .with_tessellated_mesh(id("column"), mesh(&graph, column), DEVIATION)
        .with_exact_body(id("slab"), decided.clone())
        .with_exact_body(id("column"), agreeing(&graph, column).into_body());
    let request =
        ProximityRequest::projected(id("column"), id("slab"), ProximityProjection::PlanOverlap)
            .unwrap();
    let overlap = AxiolidProximityService::new(geometry)
        .measure_distance(&request)
        .unwrap()
        .interval_metres();
    assert_eq!(overlap, (0.0, f64::INFINITY));

    let revised = ObjectId::new(SourceId::new("cad", "revised").unwrap(), "slab").unwrap();
    let session = |object: &ObjectId| {
        AxiolidProximityService::new(
            AxiolidGeometry::new()
                .with_mesh(object.clone(), mesh(&graph, slab))
                .with_exact_body(object.clone(), decided.clone()),
        )
    };
    let (base, revision) = (session(&id("slab")), session(&revised));
    let surface = revision.body_surface(&revised).unwrap();
    let request =
        SurfaceDistanceRequest::try_new(id("slab"), std::sync::Arc::new(surface), 1e-4).unwrap();
    let measured = base.measure_surface_distance(&request).unwrap();
    assert_eq!(measured.basis(), SurfaceBasis::Mesh);
}

#[test]
fn a_union_with_a_half_space_is_refused_by_name() {
    let mut builder = GeometryGraphBuilder::new();
    let wall = extrusion(&mut builder, rectangle(6.0, 0.3), 3.0, Transform3::IDENTITY);
    let roof = push(
        &mut builder,
        GeometryNode::HalfSpace(HalfSpace {
            boundary: Plane3 {
                origin: Point3::new(0.0, 0.0, 2.4),
                normal: Vec3::Z,
            },
            agreement: true,
        }),
    );
    let union = push(
        &mut builder,
        GeometryNode::SolidOperation(SolidOperation::Boolean {
            left: wall,
            right: roof,
            operator: BooleanOperator::Union,
        }),
    );
    let graph = builder.finish(vec![union]).unwrap();
    let refusal = exact_boundary(&graph, union).unwrap_err();
    assert!(refusal.contains("union"), "{refusal}");
}

#[test]
fn a_whole_of_parts_with_exact_bodies_is_certified_as_their_union() {
    // The column and its footing as two parts of one whole, each placed in
    // the world on its own: the whole's exact body holds both items, and
    // its distance to the wall is the nearer part's.
    let mut builder = GeometryGraphBuilder::new();
    let turn = Transform3::from_rotation_z(0.4);
    let footing = extrusion(&mut builder, rectangle(1.0, 1.0), 0.5, turn);
    let column = extrusion(
        &mut builder,
        circle(0.2),
        3.0,
        turn * Transform3::from_translation(Vec3::Z * 0.5),
    );
    let wall = plain_wall(&mut builder);
    let graph = builder.finish(vec![footing, column, wall]).unwrap();
    let parts = AxiolidGeometry::new()
        .with_mesh(id("footing"), mesh(&graph, footing))
        .with_tessellated_mesh(id("column"), mesh(&graph, column), DEVIATION)
        .with_mesh(id("wall"), mesh(&graph, wall))
        .with_exact_body(id("footing"), agreeing(&graph, footing).into_body())
        .with_exact_body(id("column"), agreeing(&graph, column).into_body())
        .with_exact_body(id("wall"), agreeing(&graph, wall).into_body());
    let body = parts.compose(&[id("footing"), id("column")]).unwrap();
    assert!(body.has_exact_body());
    assert!(!body.is_exact(), "the column is tessellated");
    assert_close(body.deviation_metres(), DEVIATION, 0.0);
    let geometry = parts.with_composed_body(id("whole"), body);
    let exact = geometry.exact_boundary(&id("whole")).unwrap();
    assert_eq!(exact.items().len(), 2);
    assert!(exact.is_exact());

    let footing_reach = 0.5 * (0.4_f64.cos() + 0.4_f64.sin());
    let expected = (1.5 - 0.2_f64).min(1.5 - footing_reach);
    let request = ProximityRequest::try_new(id("whole"), id("wall")).unwrap();
    let measured = AxiolidProximityService::new(geometry)
        .measure_proximity(&request)
        .unwrap();
    let certified = measured.certified_separation().expect("certified");
    assert!(
        certified.lower_metres() <= expected && expected <= certified.upper_metres(),
        "{certified:?} must hold {expected}"
    );
    assert!(
        certified.upper_metres() - certified.lower_metres() <= 4.0 * CERTIFIED_ACCURACY_METRES,
        "{certified:?}"
    );
}

/// A dyadic I-beam (0.5 m deep, 0.25 m wide, 0.125 m web, 0.0625 m
/// flanges, root fillets of 1/32 m) 2 m long along `z`, placed by
/// `placement`, less a round web hole of radius 0.125 m along `x` past
/// both flange tips, `gap` above touching the top flange's inner face.
fn filleted_beam_with_a_web_hole(
    builder: &mut GeometryGraphBuilder,
    gap: f64,
    placement: Transform3,
) -> NodeId {
    let section = Profile::Section(SectionProfile::I {
        depth: 0.5,
        width: 0.25,
        web_thickness: 0.125,
        flange_thickness: 0.0625,
        fillet_radius: Some(0.031_25),
        flange_edge_radius: None,
        flange_slope: None,
    });
    let beam = extrusion(builder, section, 2.0, placement);
    // The hole's +z onto +x by an axis matrix of zeros and ones; its axis
    // at `0.25 - 0.0625 - 0.125` touches the flange's inner face.
    let onto_x = Transform3::from_mat3(Mat3::from_cols(-Vec3::Z, Vec3::Y, Vec3::X));
    let at = Transform3::from_translation(Vec3::new(-0.1875, 0.0625 + gap, 1.0));
    let hole = extrusion(builder, circle(0.125), 0.375, placement * at * onto_x);
    difference(builder, beam, hole)
}

/// The filleted beam's volume less the hole's whole cylinder across the
/// flange width, and less the web hole alone: the true volume lies
/// between (the hole also takes a sliver of each root fillet).
fn filleted_beam_volume_bounds() -> (f64, f64) {
    let fillets = 4.0 * 0.031_25_f64.powi(2) * (1.0 - std::f64::consts::FRAC_PI_4);
    let area = 2.0 * 0.25 * 0.0625 + (0.5 - 2.0 * 0.0625) * 0.125 + fillets;
    let disc = std::f64::consts::PI * 0.125 * 0.125;
    (area * 2.0 - disc * 0.25, area * 2.0 - disc * 0.125)
}

#[test]
fn a_web_hole_touching_the_flange_of_a_filleted_beam_is_exact() {
    // axiolid/kernel#243, #249: the hole touches the flange's inner face,
    // and each root fillet where the fillet meets it. Placed by axis
    // matrices with dyadic sizes, the kernel decides the contact exactly
    // at no tolerance: an empty report, so the body is exact.
    let mut builder = GeometryGraphBuilder::new();
    let beam = filleted_beam_with_a_web_hole(&mut builder, 0.0, Transform3::IDENTITY);
    let graph = builder.finish(vec![beam]).unwrap();
    let boundary = agreeing(&graph, beam);
    assert!(boundary.body().is_exact());
    let (lower, upper) = filleted_beam_volume_bounds();
    let volume = volume(boundary.brep().unwrap());
    assert!(
        lower < volume && volume < upper,
        "{volume} in ({lower}, {upper})"
    );
}

#[test]
fn a_turned_filleted_beam_with_a_web_hole_touching_its_flange_is_perturbed() {
    // Under a building placement the contact holds only to rounding: an
    // empty report at no tolerance would claim it exact, so the kernel
    // refuses it there (#251) and reads the flange as touching the hole
    // within its tolerance (`PlaneTouchesCylinder`). The body is built,
    // perturbed by what it reports, never exact.
    let placement =
        Transform3::from_translation(Vec3::new(12.5, -4.0, 3.2)) * Transform3::from_rotation_z(0.6);
    let mut builder = GeometryGraphBuilder::new();
    let beam = filleted_beam_with_a_web_hole(&mut builder, 0.0, placement);
    let graph = builder.finish(vec![beam]).unwrap();
    let boundary = agreeing(&graph, beam);
    assert!(!boundary.body().is_exact());
    let perturbation = boundary.body().perturbation_metres();
    assert!(perturbation < 1e-6, "{perturbation}");
    let (lower, upper) = filleted_beam_volume_bounds();
    let volume = volume(boundary.brep().unwrap());
    assert!(
        lower < volume && volume < upper,
        "{volume} in ({lower}, {upper})"
    );
}

#[test]
fn a_web_hole_a_fraction_of_the_tolerance_off_a_filleted_flange_is_built_at_no_tolerance() {
    // Read as touching the flange within the kernel's tolerance, a hole
    // half a micrometre into or short of it would touch the fillets too,
    // which it meets in two arcs instead, and the kernel refuses that
    // reading by name (axiolid/kernel#249). The adapter asks for no
    // tolerance first, where the exact predicates decide the crossing as
    // given: the body is exact, and the hole reaching into the flange
    // takes the more material.
    let built = |gap: f64| {
        let mut builder = GeometryGraphBuilder::new();
        let beam = filleted_beam_with_a_web_hole(&mut builder, gap, Transform3::IDENTITY);
        let graph = builder.finish(vec![beam]).unwrap();
        let boundary = agreeing(&graph, beam);
        assert!(boundary.body().is_exact(), "{gap}");
        volume(boundary.brep().unwrap())
    };
    let (into, short) = (built(5e-7), built(-5e-7));
    let (lower, upper) = filleted_beam_volume_bounds();
    assert!(
        lower < into && into < short && short < upper,
        "{into} {short}"
    );
}

#[test]
fn an_ipe_beam_less_a_web_hole_is_exact() {
    // axiolid/kernel#250: an I section of decimal (IPE 300) sizes, root
    // fillets included, closes its contour at no tolerance since
    // axiolid-construct 0.3.14, so a web hole clear of the fillets, placed
    // by axis matrices, is cut exactly: an empty report, never a body
    // perturbed by a run within the tolerance.
    let (h, b, tw, tf, rf, r, len) = (0.3, 0.15, 0.0071, 0.0107, 0.015, 0.05, 2.0);
    let mut builder = GeometryGraphBuilder::new();
    let section = Profile::Section(SectionProfile::I {
        depth: h,
        width: b,
        web_thickness: tw,
        flange_thickness: tf,
        fillet_radius: Some(rf),
        flange_edge_radius: None,
        flange_slope: None,
    });
    let beam = extrusion(&mut builder, section, len, Transform3::IDENTITY);
    let onto_x = Transform3::from_mat3(Mat3::from_cols(-Vec3::Z, Vec3::Y, Vec3::X));
    let at = Transform3::from_translation(Vec3::new(-b, 0.0, len / 2.0));
    let hole = extrusion(&mut builder, circle(r), 2.0 * b, at * onto_x);
    let beam = difference(&mut builder, beam, hole);
    let graph = builder.finish(vec![beam]).unwrap();
    let boundary = agreeing(&graph, beam);
    assert!(boundary.body().is_exact());
    let area =
        2.0 * b * tf + (h - 2.0 * tf) * tw + 4.0 * rf * rf * (1.0 - std::f64::consts::FRAC_PI_4);
    let expected = area * len - std::f64::consts::PI * r * r * tw;
    assert_close(volume(boundary.brep().unwrap()), expected, 1e-12);
}
