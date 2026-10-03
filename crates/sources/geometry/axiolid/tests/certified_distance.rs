//! Certified distances between curved bodies with exact boundaries.
//!
//! A round column of radius 0.2 m stands with its axis 1 m from the face of
//! a wall, so the true distance in space is 0.8 m. Its mesh has a vertex
//! facing the wall, so the mesh distance is 0.8 m too, but only the chord
//! deviation bounds the true surface: the mesh alone leaves an interval
//! millimetres wide. With both exact boundaries registered the kernel's
//! certified `boundary_distance` narrows it to about a micrometre around
//! the closed form.

use std::f64::consts::{PI, TAU};

use axiolid_construct::boolean_exact::{ArcPrism, boolean_arc_prisms_exact};
use axiolid_core::{BooleanOperator, Point2, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_overlay::ArcRing;
use axioval_axiolid::proximity::CERTIFIED_ACCURACY_METRES;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{ProximityError, ProximityRequest, ProximityService};
use axioval_ir::{ObjectId, SourceId};

const RADIUS: f64 = 0.2;
const SIDES: u32 = 16;
const HEIGHT: f64 = 3.0;
/// The column's axis to the wall's face, less the radius.
const DISTANCE: f64 = 1.0 - RADIUS;

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

/// How far the column's chords fall inside its circle.
fn chord_deviation() -> f64 {
    RADIUS * (1.0 - (PI / f64::from(SIDES)).cos())
}

/// A closed prism over `ring` between `bottom` and `top`, the exact solid
/// the kernel builds from an arc section (the intersection with a copy
/// scaled about the centre is the section itself).
fn exact_prism(section: impl Fn(f64) -> ArcRing) -> axiolid_brep::ExactBRep {
    let prism = |scale: f64| ArcPrism {
        section: section(scale),
        bottom: 0.0,
        top: HEIGHT,
    };
    boolean_arc_prisms_exact(
        &prism(1.0),
        &prism(2.0),
        BooleanOperator::Intersection,
        Tolerance::METRE,
    )
    .expect("an exact prism")
}

fn exact_column(centre: [f64; 2]) -> axiolid_brep::ExactBRep {
    exact_prism(|scale| ArcRing::circle(Point2::new(centre[0], centre[1]), RADIUS * scale))
}

/// The wall `x` in [1.0, 1.2], `y` in [-2, 2].
fn exact_wall() -> axiolid_brep::ExactBRep {
    let corners = [(1.0, -2.0), (1.2, -2.0), (1.2, 2.0), (1.0, 2.0)];
    exact_prism(|scale| {
        let centre = (1.1, 0.0);
        ArcRing::from_points(
            &corners
                .iter()
                .map(|(x, y)| {
                    Point2::new(
                        centre.0 + (x - centre.0) * scale,
                        centre.1 + (y - centre.1) * scale,
                    )
                })
                .collect::<Vec<_>>(),
        )
    })
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ],
        vec![
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            3, 7, 6, 3, 6, 2, // back
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ],
    )
}

/// The column's chord mesh, a vertex at angle zero facing the wall.
fn column_mesh(centre: [f64; 2]) -> TriMesh {
    let mut positions = Vec::new();
    for level in [0.0, HEIGHT] {
        for side in 0..SIDES {
            let angle = TAU * f64::from(side) / f64::from(SIDES);
            positions.push(Point3::new(
                centre[0] + RADIUS * angle.cos(),
                centre[1] + RADIUS * angle.sin(),
                level,
            ));
        }
    }
    positions.push(Point3::new(centre[0], centre[1], 0.0));
    positions.push(Point3::new(centre[0], centre[1], HEIGHT));
    let (bottom_centre, top_centre) = (2 * SIDES, 2 * SIDES + 1);
    let mut indices = Vec::new();
    for side in 0..SIDES {
        let next = (side + 1) % SIDES;
        let (b0, b1, t0, t1) = (side, next, side + SIDES, next + SIDES);
        indices.extend([b0, b1, t1, b0, t1, t0]);
        indices.extend([bottom_centre, b1, b0]);
        indices.extend([top_centre, t0, t1]);
    }
    TriMesh::new(positions, indices)
}

/// The column tessellated (declared deviation a little above the chords')
/// and the wall exact, meshes only.
fn meshes() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_tessellated_mesh(
            id("column"),
            column_mesh([0.0, 0.0]),
            chord_deviation() * 1.01,
        )
        .with_mesh(id("wall"), cuboid([1.0, -2.0, 0.0], [1.2, 2.0, HEIGHT]))
}

fn certified() -> AxiolidGeometry {
    meshes()
        .with_exact_boundary(id("column"), exact_column([0.0, 0.0]))
        .with_exact_boundary(id("wall"), exact_wall())
}

fn distance(geometry: AxiolidGeometry) -> Result<(f64, f64), ProximityError> {
    let request = ProximityRequest::try_new(id("column"), id("wall")).unwrap();
    AxiolidProximityService::new(geometry)
        .measure_distance(&request)
        .map(|measured| {
            assert!(
                !measured.evidence().exact,
                "a tessellated pair is never exact"
            );
            measured.interval_metres()
        })
}

#[test]
fn the_mesh_alone_leaves_the_distance_within_the_chord_deviation() {
    let (lower, upper) = distance(meshes()).unwrap();
    let deviation = chord_deviation() * 1.01;
    assert!((lower - (DISTANCE - deviation)).abs() < 1e-12, "{lower}");
    assert!((upper - (DISTANCE + deviation)).abs() < 1e-12, "{upper}");
    // A limit of 0.799 m lies inside it: the mesh cannot judge it.
    assert!(lower < 0.799 && 0.799 < upper);
}

#[test]
fn a_round_column_is_certified_against_a_wall() {
    let (lower, upper) = distance(certified()).unwrap();
    assert!(
        lower <= DISTANCE && DISTANCE <= upper,
        "[{lower}, {upper}] must hold {DISTANCE}"
    );
    assert!(
        upper - lower <= 2.0 * CERTIFIED_ACCURACY_METRES,
        "[{lower}, {upper}] is wider than the accuracy asked"
    );
    // Now the same limit is cleared: judged, not left open.
    assert!(lower > 0.799);
}

#[test]
fn the_proximity_evidence_carries_the_certified_separation() {
    let request = ProximityRequest::try_new(id("column"), id("wall")).unwrap();
    let measured = AxiolidProximityService::new(certified())
        .measure_proximity(&request)
        .unwrap();
    let certified = measured.certified_separation().expect("certified");
    assert!(certified.lower_metres() <= DISTANCE && DISTANCE <= certified.upper_metres());
    assert_eq!(
        measured.separation_interval_metres(),
        (certified.lower_metres(), certified.upper_metres())
    );
    assert!(!measured.evidence().exact);

    let uncertified = AxiolidProximityService::new(meshes())
        .measure_proximity(&request)
        .unwrap();
    assert_eq!(uncertified.certified_separation(), None);
}

#[test]
fn two_round_columns_are_their_axis_gap_less_both_radii() {
    // Axes 1.5 m apart: 1.1 m between the surfaces, along a whole line.
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("column"), column_mesh([0.0, 0.0]), chord_deviation())
        .with_tessellated_mesh(id("wall"), column_mesh([1.5, 0.0]), chord_deviation())
        .with_exact_boundary(id("column"), exact_column([0.0, 0.0]))
        .with_exact_boundary(id("wall"), exact_column([1.5, 0.0]));
    let (lower, upper) = distance(geometry).unwrap();
    let expected = 1.5 - 2.0 * RADIUS;
    assert!(lower <= expected && expected <= upper, "[{lower}, {upper}]");
    assert!(
        upper - lower < 2.0 * chord_deviation(),
        "[{lower}, {upper}]"
    );
}

#[test]
fn one_boundary_alone_certifies_nothing() {
    let geometry = meshes().with_exact_boundary(id("column"), exact_column([0.0, 0.0]));
    assert_eq!(distance(geometry), distance(meshes()));
}

#[test]
fn an_exact_pair_stays_a_point() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), cuboid([-0.2, -0.2, 0.0], [0.2, 0.2, HEIGHT]))
        .with_mesh(id("wall"), cuboid([1.0, -2.0, 0.0], [1.2, 2.0, HEIGHT]))
        .with_exact_boundary(id("column"), exact_column([0.0, 0.0]))
        .with_exact_boundary(id("wall"), exact_wall());
    let request = ProximityRequest::try_new(id("column"), id("wall")).unwrap();
    let measured = AxiolidProximityService::new(geometry)
        .measure_distance(&request)
        .unwrap();
    assert!(measured.evidence().exact);
    assert_eq!(measured.interval_metres(), (0.8, 0.8));
}

#[test]
fn a_boundary_that_misses_its_mesh_refuses() {
    // The registered boundary stands a metre further away than the mesh.
    let geometry = meshes()
        .with_exact_boundary(id("column"), exact_column([-1.0, 0.0]))
        .with_exact_boundary(id("wall"), exact_wall());
    assert_eq!(distance(geometry), Err(ProximityError::InvalidMeasurement));
}

/// `profile` extruded `HEIGHT` up from the ground and placed at `at`, as a
/// host lowers an extruded body.
fn graph_boundary(
    profile: axiolid_profile::Profile,
    at: [f64; 2],
) -> Result<axioval_axiolid::ExactBoundary, String> {
    use axiolid_core::{Transform3, Vec3};
    use axiolid_model::{GeometryGraphBuilder, GeometryNode, Instance, SolidOperation};

    let mut builder = GeometryGraphBuilder::new();
    let profile = builder.push(GeometryNode::Profile(profile)).unwrap();
    let extrusion = builder
        .push(GeometryNode::SolidOperation(SolidOperation::Extrusion {
            profile,
            direction: Vec3::Z,
            depth: HEIGHT,
        }))
        .unwrap();
    let root = builder
        .push(GeometryNode::Instance(Instance {
            source: extrusion,
            transform: Transform3::from_translation(Vec3::new(at[0], at[1], 0.0)),
        }))
        .unwrap();
    let graph = builder.finish(vec![root]).unwrap();
    axioval_axiolid::exact_boundary(&graph, root)
}

fn round(radius: f64) -> axiolid_profile::Profile {
    axiolid_profile::Profile::Circle(axiolid_profile::CircleProfile {
        radius,
        thickness: None,
    })
}

fn wall_profile() -> axiolid_profile::Profile {
    axiolid_profile::Profile::Rectangle(axiolid_profile::RectangleProfile {
        x: 0.2,
        y: 4.0,
        thickness: None,
        outer_radius: None,
        inner_radius: None,
    })
}

#[test]
fn boundaries_built_from_the_meshed_graph_certify_the_pair() {
    let column = graph_boundary(round(RADIUS), [0.0, 0.0]).unwrap();
    let wall = graph_boundary(wall_profile(), [1.1, 0.0]).unwrap();
    let geometry = meshes();
    geometry
        .check_exact_boundary(&id("column"), &column)
        .unwrap();
    geometry.check_exact_boundary(&id("wall"), &wall).unwrap();
    let geometry = geometry
        .with_exact_body(id("column"), column.into_body())
        .with_exact_body(id("wall"), wall.into_body());
    let (lower, upper) = distance(geometry).unwrap();
    assert!(lower <= DISTANCE && DISTANCE <= upper, "[{lower}, {upper}]");
    assert!(lower > 0.799, "[{lower}, {upper}]");
}

#[test]
fn a_boundary_that_does_not_match_its_mesh_fails_the_check() {
    let geometry = meshes();
    // Placed a metre away, and a column a little too wide.
    let moved = graph_boundary(round(RADIUS), [-1.0, 0.0]).unwrap();
    assert!(
        geometry
            .check_exact_boundary(&id("column"), &moved)
            .is_err()
    );
    let wider = graph_boundary(round(RADIUS + 0.01), [0.0, 0.0]).unwrap();
    assert!(
        geometry
            .check_exact_boundary(&id("column"), &wider)
            .is_err()
    );
    // Nothing to compare against.
    let column = graph_boundary(round(RADIUS), [0.0, 0.0]).unwrap();
    assert!(
        geometry
            .check_exact_boundary(&id("nothing"), &column)
            .is_err()
    );
}
