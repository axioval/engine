//! Exact bodies beyond one solid: walls less their openings
//! (axiolid/kernel#228) and bodies of several items (#229).
//!
//! Each body is built from a geometry graph as a host lowers it, meshed by
//! the kernel's mesh compiler and built exactly by `exact_boundary`; the
//! two must agree, and the certified distances measured on the exact body
//! must hold the closed form where the chord meshes leave it open.

use axiolid_brep::ExactBRep;
use axiolid_contracts::ExecutionOptions;
use axiolid_core::{BooleanOperator, Point3, Tolerance, Transform3, Vec3};
use axiolid_curve::{Curve3, Polyline};
use axiolid_measure::exact_properties;
use axiolid_mesh::TriMesh;
use axiolid_mesh_boolean_boolmesh::BoolmeshBoolean;
use axiolid_mesh_compile::ReferenceMeshCompiler;
use axiolid_mesh_compile_contract::MeshCompiler;
use axiolid_model::{
    GeometryGraph, GeometryGraphBuilder, GeometryNode, Instance, NodeId, SolidOperation,
};
use axiolid_profile::{CircleProfile, Profile, RectangleProfile};
use axioval_axiolid::proximity::CERTIFIED_ACCURACY_METRES;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService, ExactBoundary, exact_boundary};
use axioval_engine::{
    ProximityError, ProximityRequest, ProximityService, SurfaceBasis, SurfaceDistanceRequest,
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
/// with their exact bodies when `exact` is set, and the wall's
/// perturbation.
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
    let perturbation = wall.body().perturbation_metres();
    let geometry = geometry
        .with_exact_body(id("wall"), wall.into_body())
        .with_exact_body(id("pipe"), pipe.into_body());
    (geometry, perturbation)
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
fn a_wall_less_a_door_and_a_window_is_built_within_the_kernels_tolerance() {
    let mut builder = GeometryGraphBuilder::new();
    let wall = wall_with_openings(&mut builder);
    let graph = builder.finish(vec![wall]).unwrap();
    let boundary = agreeing(&graph, wall);
    // The kernel's general boolean takes placed operands only within a
    // tolerance, so the body is the exact difference of operands moved by
    // at most a micrometre (and turned by a nanoradian over its 5 m
    // extent): marked perturbed, never exact.
    let perturbation = boundary.body().perturbation_metres();
    assert!(!boundary.body().is_exact());
    assert!(
        (1e-6..1e-6 + 6e-9).contains(&perturbation),
        "{perturbation}"
    );
    assert_close(volume(boundary.brep().unwrap()), WALL_VOLUME, 1e-9);
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
    // wall's perturbation on both sides.
    let (geometry, perturbation) = wall_and_pipe(true);
    let (lower, upper) = pipe_to_wall(geometry);
    assert!(
        lower <= PIPE_TO_SILL - perturbation && PIPE_TO_SILL + perturbation <= upper,
        "[{lower}, {upper}]"
    );
    assert!(
        upper - lower <= 2.0 * perturbation + 4.0 * CERTIFIED_ACCURACY_METRES,
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
    // surfaces: a wall less its openings is perturbed, so its exact
    // (planar) meshes are compared instead, exactly.
    let mut builder = GeometryGraphBuilder::new();
    let wall = wall_with_openings(&mut builder);
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
