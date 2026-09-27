//! Free space measured from real geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, ClearanceOutcome, ClearanceRequest, ClearanceShape, CylinderClearance,
    FreeAreaRequest, FreeSpaceError, FreeSpaceService, MetricDirection, MetricFrame, MetricPoint,
    MobilityProfile, PlacementOrientation, PlacementRequest, PlacementShape,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

fn body(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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
        vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
    )
}

/// A clearance volume centred at `(x, y, z)`.
fn frame_at(x: f64, y: f64, z: f64) -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(id("scope"), [x, y, z]).expect("valid point"),
        MetricDirection::try_new([1.0, 0.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 1.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 0.0, 1.0]).expect("valid direction"),
    )
    .expect("valid frame")
}

/// A walking profile: 0.3 m radius, 1.8 m tall, modest step and slope.
fn profile() -> MobilityProfile {
    MobilityProfile::try_new(0.3, 1.8, 0.15, 0.08).expect("valid profile")
}

fn box_shape(w: f64, d: f64, h: f64) -> ClearanceShape {
    ClearanceShape::Box(BoxClearance::try_new(w, d, h).expect("valid box"))
}

/// A volume with nothing in it is clear, and the claim is earned: every named
/// obstacle was measured.
#[test]
fn an_empty_volume_is_clear() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("column"), body(8.0, 9.0, 8.0, 9.0, 0.0, 3.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("column")],
    );
    assert!(matches!(
        service.assess_clearance(&request).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// A column inside the volume obstructs it and is named as the blocker.
#[test]
fn an_intersecting_body_obstructs_and_is_named() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("column"), closed_box(0.9, 1.1, 0.9, 1.1, 0.0, 3.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("column")],
    );
    match service.assess_clearance(&request).expect("measurable") {
        ClearanceOutcome::Obstructed(evidence) => {
            assert_eq!(evidence.blockers(), &[id("column")]);
        }
        ClearanceOutcome::Clear(_) => panic!("a column in the volume obstructs it"),
    }
}

/// A body sharing plan area but sitting above the volume does not obstruct it.
#[test]
fn a_body_above_the_volume_does_not_obstruct_it() {
    let geometry = AxiolidGeometry::new().with_mesh(id("beam"), body(0.0, 2.0, 0.0, 2.0, 5.0, 5.4));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("beam")],
    );
    assert!(matches!(
        service.assess_clearance(&request).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// Free floor is the scope minus what obstructs it.
#[test]
fn free_area_subtracts_obstacles_from_the_scope() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        .with_mesh(id("island"), body(0.0, 2.0, 0.0, 10.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("island")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    // 100 m2 of room, 20 m2 taken by the island.
    assert!(
        (evidence.available_area().upper_square_metres() - 80.0).abs() < 1e-6,
        "got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// Two overlapping obstacles do not subtract their shared area twice.
#[test]
fn overlapping_obstacles_are_not_double_counted() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        .with_mesh(id("crate-a"), body(0.0, 4.0, 0.0, 10.0, 0.0, 1.0))
        .with_mesh(id("crate-b"), body(2.0, 6.0, 0.0, 10.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("crate-a"), id("crate-b")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    // Union of the two crates spans x 0..6, so 60 m2 obstructed, 40 free.
    assert!(
        (evidence.available_area().upper_square_metres() - 40.0).abs() < 1e-6,
        "overlapping obstacles must union, got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// An obstacle extending beyond the room does not consume floor the room
/// never had.
#[test]
fn an_obstacle_is_clipped_to_the_scope() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        // Twice the room's width, half its depth.
        .with_mesh(id("overhang"), body(-10.0, 20.0, 0.0, 5.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("overhang")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    assert!(
        (evidence.available_area().upper_square_metres() - 50.0).abs() < 1e-6,
        "only the overlap inside the room counts, got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// Placement refuses rather than guessing.
///
/// `NoPlacement` asserts an EXHAUSTIVE search found nowhere the shape fits. A
/// sampled sweep can only fail to find a witness, which is a weaker statement.
/// Returning `NoPlacement` from a sampled search would launder "did not find"
/// into "does not exist" -- the exact failure mode the outcome type exists to
/// prevent.
#[test]
fn placement_refuses_rather_than_claiming_an_exhaustive_search() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let shape = PlacementShape::Box {
        shape: BoxClearance::try_new(0.8, 0.8, 2.0).unwrap(),
        orientation: PlacementOrientation::Any,
    };
    let request = PlacementRequest::new(id("room"), shape, Vec::new());
    assert!(
        matches!(
            service.find_placement(&request),
            Err(FreeSpaceError::Unavailable(_))
        ),
        "an unimplementable completeness claim must be refused, not faked"
    );
}

/// An obstacle without geometry cannot be cleared.
///
/// `Clear` asserts nothing obstructs the volume. An obstacle the adapter
/// cannot see has not been shown to be elsewhere, so reporting `Clear` would
/// assert more than was measured. The error names the object to look at.
#[test]
fn an_obstacle_without_geometry_is_not_silently_clear() {
    let service = AxiolidFreeSpaceService::new(AxiolidGeometry::new(), source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("unknown-column")],
    );
    match service.assess_clearance(&request) {
        Err(FreeSpaceError::MissingGeometry(object)) => {
            assert_eq!(*object, id("unknown-column"), "the error names the object");
        }
        other => panic!("expected MissingGeometry, got {other:?}"),
    }
}

/// Free area is reported as an upper bound, never as exact.
///
/// Plan area counts floor a mobility profile cannot occupy, such as a strip
/// too narrow to turn in. Claiming exactness would assert usable space that
/// was never measured.
#[test]
fn free_area_is_an_upper_bound_not_an_exact_value() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), Vec::new());
    let evidence = service.measure_free_area(&request).expect("measurable");
    let area = evidence.available_area();
    assert!(
        area.lower_square_metres() < area.upper_square_metres(),
        "an unrefined plan measurement is a bound, not a value"
    );
}

/// A closed, outward-oriented box, as real exports produce: its bottom face
/// winds opposite to its top when seen from above.
fn closed_box(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

/// A closed solid's top and bottom faces project with opposite windings. The
/// footprint must be their union, not their cancellation.
#[test]
fn closed_bodies_keep_their_footprint() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), closed_box(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        .with_mesh(id("island"), closed_box(0.0, 2.0, 0.0, 10.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("island")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    assert!(
        (evidence.available_area().upper_square_metres() - 80.0).abs() < 1e-6,
        "got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// A storey or zone names no volume; declaring it bodiless lets a clearance
/// be assessed around it, where an undeclared one refuses (above).
#[test]
fn a_bodiless_candidate_is_no_obstacle() {
    let geometry = AxiolidGeometry::new().with_no_body(id("storey"));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("storey")],
    );
    assert!(
        service.assess_clearance(&request).is_ok(),
        "a bodiless candidate obstructs nothing"
    );
}

/// An unmeasured obstacle is not bodiless: it still refuses.
#[test]
fn an_unmeasured_obstacle_still_refuses() {
    let geometry =
        AxiolidGeometry::new().with_unmeasured(id("column"), "unsupported representation");
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("column")],
    );
    assert!(matches!(
        service.assess_clearance(&request),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
}

/// A 2 m × 0.4 m box turned by 45°. Its axis-aligned bounding square
/// (about 1.56 m) would reach a column at its corner; the turned box does not.
#[test]
fn a_turned_box_follows_its_frame_axes() {
    let (s, c) = std::f64::consts::FRAC_PI_4.sin_cos();
    let frame = MetricFrame::try_new(
        MetricPoint::try_new(id("scope"), [0.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([c, s, 0.0]).unwrap(),
        MetricDirection::try_new([-s, c, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    )
    .unwrap();
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), closed_box(0.6, 0.75, -0.75, -0.6, 0.0, 3.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(frame, box_shape(2.0, 0.4, 2.0), vec![id("column")]);
    assert!(matches!(
        service.assess_clearance(&request).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// A column in the corner of a cylinder's bounding square is outside the
/// cylinder, and must not be named as a blocker.
#[test]
fn a_column_in_the_corner_of_a_cylinders_square_does_not_obstruct_it() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), closed_box(0.65, 0.75, 0.65, 0.75, 0.0, 3.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let cylinder = ClearanceShape::Cylinder(CylinderClearance::try_new(0.75, 2.0).unwrap());
    let request = ClearanceRequest::new(frame_at(0.0, 0.0, 0.0), cylinder, vec![id("column")]);
    assert!(matches!(
        service.assess_clearance(&request).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// An obstacle grazing the disc between the inscribed and circumscribed
/// polygons is neither a proven blocker nor proven clear.
#[test]
fn an_obstacle_in_the_disc_band_refuses() {
    // Both polygons have a vertex at angle 0, so at angle pi/64 the inscribed
    // one's edge is about 0.9 mm inside the circle and the circumscribed
    // one's touches it. A 0.1 mm post centred 0.5 mm inside is in the band.
    let angle = std::f64::consts::PI / 64.0;
    let (cx, cy) = (0.7495 * angle.cos(), 0.7495 * angle.sin());
    let geometry = AxiolidGeometry::new().with_mesh(
        id("post"),
        closed_box(
            cx - 0.00005,
            cx + 0.00005,
            cy - 0.00005,
            cy + 0.00005,
            0.0,
            3.0,
        ),
    );
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let cylinder = ClearanceShape::Cylinder(CylinderClearance::try_new(0.75, 2.0).unwrap());
    let request = ClearanceRequest::new(frame_at(0.0, 0.0, 0.0), cylinder, vec![id("post")]);
    assert!(matches!(
        service.assess_clearance(&request),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

/// Several closed bodies as one mesh, like a table exported in one piece.
fn merged(parts: &[TriMesh]) -> TriMesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for part in parts {
        let offset = u32::try_from(positions.len()).expect("small mesh");
        indices.extend(part.indices.iter().map(|index| index + offset));
        positions.extend(part.positions.iter().copied());
    }
    TriMesh::new(positions, indices)
}

/// A profile extruded from `w0` to `w1` into a closed, outward-oriented
/// prism, `place` putting a profile point `(u, v)` at depth `w` in space.
/// `fan` triangulates the profile with the profile's own winding.
fn extruded(
    profile: &[(f64, f64)],
    fan: &[[u32; 3]],
    [w0, w1]: [f64; 2],
    place: fn(f64, f64, f64) -> Point3,
) -> TriMesh {
    let n = u32::try_from(profile.len()).expect("small profile");
    let mut positions: Vec<Point3> = profile.iter().map(|&(u, v)| place(u, v, w0)).collect();
    positions.extend(profile.iter().map(|&(u, v)| place(u, v, w1)));
    let mut indices = Vec::new();
    for &[a, b, c] in fan {
        indices.extend([a, b, c, n + a, n + c, n + b]);
    }
    for a in 0..n {
        let b = (a + 1) % n;
        indices.extend([b, a, n + a, b, n + a, n + b]);
    }
    let mesh = TriMesh::new(positions, indices);
    // The winding is consistent; which way it faces depends on the profile.
    if signed_volume(&mesh) > 0.0 {
        mesh
    } else {
        inverted(&mesh)
    }
}

fn signed_volume(mesh: &TriMesh) -> f64 {
    mesh.indices
        .chunks(3)
        .map(|t| {
            let [a, b, c] = [0, 1, 2].map(|i| mesh.positions[t[i] as usize]);
            a.dot(b.cross(c))
        })
        .sum()
}

/// The same surface wound the other way, so it faces inward.
fn inverted(mesh: &TriMesh) -> TriMesh {
    TriMesh::new(
        mesh.positions.clone(),
        mesh.indices
            .chunks(3)
            .flat_map(|t| [t[0], t[2], t[1]])
            .collect(),
    )
}

/// An L-shaped body 0.8 m deep (y 0.6 to 1.4): a foot from x 0.6 to 2.0 up
/// to `foot_top`, and a 3 m column from x 1.6 to 2.0.
fn l_shaped(foot_top: f64) -> TriMesh {
    let profile = [
        (0.6, 0.0),
        (2.0, 0.0),
        (2.0, 3.0),
        (1.6, 3.0),
        (1.6, foot_top),
        (0.6, foot_top),
    ];
    // Fanned from the reflex corner, which sees every other edge.
    let fan = [[4, 5, 0], [4, 0, 1], [4, 1, 2], [4, 2, 3]];
    extruded(&profile, &fan, [0.6, 1.4], across)
}

/// A profile `(x, z)` extruded along y.
fn across(x: f64, z: f64, y: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// A plan outline `(x, y)` extruded upward.
fn upright(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z)
}

/// A 2 m square table 2.7 m high on four corner legs, one mesh.
fn table() -> TriMesh {
    let leg = |x: f64, y: f64| closed_box(x, x + 0.2, y, y + 0.2, 0.0, 2.6);
    merged(&[
        leg(0.0, 0.0),
        leg(1.8, 0.0),
        leg(0.0, 1.8),
        leg(1.8, 1.8),
        closed_box(0.0, 2.0, 0.0, 2.0, 2.6, 2.7),
    ])
}

/// The box the tests below share: 0.8 m square (x and y 0.6 to 1.4), from
/// 0.5 m up to 2.5 m.
fn raised_box(obstacles: &[(&str, TriMesh)]) -> Result<ClearanceOutcome, FreeSpaceError> {
    raised(box_shape(0.8, 0.8, 2.0), obstacles)
}

/// `shape` centred on x = y = 1 m, standing at 0.5 m.
fn raised(
    shape: ClearanceShape,
    obstacles: &[(&str, TriMesh)],
) -> Result<ClearanceOutcome, FreeSpaceError> {
    let geometry = obstacles
        .iter()
        .fold(AxiolidGeometry::new(), |geometry, (name, mesh)| {
            geometry.with_mesh(id(name), mesh.clone())
        });
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.5),
        shape,
        obstacles.iter().map(|(name, _)| id(name)).collect(),
    );
    AxiolidFreeSpaceService::new(geometry, source()).assess_clearance(&request)
}

fn blockers(outcome: Result<ClearanceOutcome, FreeSpaceError>) -> Vec<ObjectId> {
    match outcome.expect("measurable") {
        ClearanceOutcome::Obstructed(evidence) => evidence.blockers().to_vec(),
        ClearanceOutcome::Clear(_) => Vec::new(),
    }
}

/// The L's foot lies under the volume and its column beside it. Its height
/// overlaps the volume's band and its plan outline the volume's footprint,
/// but no part of it is inside the volume. Testing the two separately, as
/// this adapter once did, named it as a blocker.
#[test]
fn an_l_shaped_body_under_and_beside_the_volume_is_clear() {
    assert!(matches!(
        raised_box(&[("l", l_shaped(0.3))]).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
    let cylinder = ClearanceShape::Cylinder(CylinderClearance::try_new(0.4, 2.0).unwrap());
    assert!(matches!(
        raised(cylinder, &[("l", l_shaped(0.3))]).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// A foot rising into the volume obstructs it.
#[test]
fn an_l_shaped_body_reaching_into_the_volume_obstructs_it() {
    assert_eq!(blockers(raised_box(&[("l", l_shaped(0.8))])), [id("l")]);
    let cylinder = ClearanceShape::Cylinder(CylinderClearance::try_new(0.4, 2.0).unwrap());
    assert_eq!(
        blockers(raised(cylinder, &[("l", l_shaped(0.8))])),
        [id("l")]
    );
}

/// The table's top covers the volume's footprint above its band, and its legs
/// stand in the band outside the footprint: the volume beneath is clear.
#[test]
fn a_table_overhanging_the_volume_above_its_band_is_clear() {
    assert!(matches!(
        raised_box(&[("table", table())]).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
    let cylinder = ClearanceShape::Cylinder(CylinderClearance::try_new(0.75, 2.0).unwrap());
    assert!(matches!(
        raised(cylinder, &[("table", table())]).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// A body entering the volume through a side obstructs it and is named.
#[test]
fn a_body_entering_the_volume_obstructs_it() {
    assert_eq!(
        blockers(raised_box(&[
            ("crate", closed_box(1.2, 2.0, 0.6, 1.4, 0.0, 1.0)),
            ("far", closed_box(5.0, 6.0, 5.0, 6.0, 0.0, 1.0)),
        ])),
        [id("crate")]
    );
}

/// Bodies against a side, under the base and on the top touch the volume
/// but do not enter it.
#[test]
fn bodies_in_contact_with_its_faces_leave_the_volume_clear() {
    assert!(matches!(
        raised_box(&[
            ("wall", closed_box(1.4, 2.0, 0.0, 2.0, 0.0, 3.0)),
            ("floor", closed_box(0.0, 2.0, 0.0, 2.0, 0.0, 0.5)),
            ("ceiling", closed_box(0.0, 1.4, 0.0, 2.0, 2.5, 3.0)),
        ])
        .expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// No surface meets a volume buried in a solid, yet it is obstructed.
#[test]
fn a_volume_inside_a_body_is_obstructed() {
    assert_eq!(
        blockers(raised_box(&[(
            "block",
            closed_box(-5.0, 5.0, -5.0, 5.0, -5.0, 5.0)
        )])),
        [id("block")]
    );
}

/// A body wholly inside the volume obstructs it.
#[test]
fn a_body_inside_the_volume_obstructs_it() {
    assert_eq!(
        blockers(raised_box(&[(
            "box",
            closed_box(0.9, 1.1, 0.9, 1.1, 1.0, 1.2)
        )])),
        [id("box")]
    );
}

/// A slab leaning over the volume: in the band it stays beyond x = 1.4, and
/// it reaches over the footprint only above the band. Its faces cross the
/// band's top, so only their part inside the band may be compared in plan.
#[test]
fn a_slab_leaning_over_the_volume_is_clear() {
    let profile = [(2.2, 2.0), (2.5, 2.0), (0.9, 3.0), (0.6, 3.0)];
    let slab = extruded(&profile, &[[0, 1, 2], [0, 2, 3]], [0.6, 1.4], across);
    assert!(matches!(
        raised_box(&[("slab", slab)]).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// A column turned by 45° off the volume's corner. Neither of the volume's
/// sides separates them in plan, only the column's own diagonal side does.
#[test]
fn a_turned_column_off_the_corner_is_clear() {
    let diamond = [(1.85, 1.6), (1.6, 1.85), (1.35, 1.6), (1.6, 1.35)];
    let column = extruded(&diamond, &[[0, 1, 2], [0, 2, 3]], [0.0, 3.0], upright);
    assert!(matches!(
        raised_box(&[("column", column)]).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// Only a closed, outward surface bounds a solid. An open or inward-facing
/// mesh near the volume refuses rather than being guessed either way.
#[test]
fn an_open_or_inward_obstacle_near_the_volume_refuses() {
    for mesh in [
        body(0.9, 1.1, 0.9, 1.1, 0.0, 3.0),
        body(1.3, 1.8, 0.9, 1.1, 0.0, 3.0),
        inverted(&closed_box(1.3, 1.8, 0.9, 1.1, 0.0, 3.0)),
    ] {
        assert!(matches!(
            raised_box(&[("column", mesh)]),
            Err(FreeSpaceError::Unavailable(_))
        ));
    }
}
