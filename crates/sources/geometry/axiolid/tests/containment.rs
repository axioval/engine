//! Whether a clearance footprint lies inside its scopes, from real geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, ClearanceShape, ContainmentOutcome, ContainmentRequest, CylinderClearance,
    FreeSpaceError, FreeSpaceService, MetricDirection, MetricFrame, MetricPoint,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
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
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

fn frame_at(x: f64, y: f64) -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(id("wc"), [x, y, 0.0]).expect("valid point"),
        MetricDirection::try_new([1.0, 0.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 1.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 0.0, 1.0]).expect("valid direction"),
    )
    .expect("valid frame")
}

fn square(side: f64) -> ClearanceShape {
    ClearanceShape::Box(BoxClearance::try_new(side, side, 2.0).expect("valid box"))
}

/// Two rooms side by side: `a` x 0..2, `b` x 2..4, both y 0..2.
fn rooms() -> AxiolidFreeSpaceService {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [2.0, 2.0, 2.5]))
        .with_mesh(id("b"), cuboid([2.0, 0.0, 0.0], [4.0, 2.0, 2.5]))
        .with_no_body(id("zone"));
    AxiolidFreeSpaceService::new(geometry, source())
}

#[test]
fn a_box_flush_with_the_walls_lies_inside() {
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), square(2.0), vec![id("a")]);
    assert!(matches!(
        rooms().assess_containment(&request),
        Ok(ContainmentOutcome::Inside(_))
    ));
}

#[test]
fn a_box_reaching_past_the_room_lies_outside() {
    let request = ContainmentRequest::new(frame_at(1.5, 1.0), square(1.2), vec![id("a")]);
    assert!(matches!(
        rooms().assess_containment(&request),
        Ok(ContainmentOutcome::Outside(_))
    ));
}

#[test]
fn merged_rooms_cover_a_box_across_their_shared_wall() {
    let request = ContainmentRequest::new(frame_at(2.0, 1.0), square(1.2), vec![id("a"), id("b")]);
    assert!(matches!(
        rooms().assess_containment(&request),
        Ok(ContainmentOutcome::Inside(_))
    ));
}

#[test]
fn no_scope_covers_nothing() {
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), square(1.0), Vec::new());
    assert!(matches!(
        rooms().assess_containment(&request),
        Ok(ContainmentOutcome::Outside(_))
    ));
}

#[test]
fn a_disc_touching_the_wall_lies_in_the_band_and_is_refused() {
    // Radius 1 about the room's centre: the inscribed polygon is inside, the
    // circumscribed one reaches past the walls.
    let shape =
        ClearanceShape::Cylinder(CylinderClearance::try_new(1.0, 2.0).expect("valid cylinder"));
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), shape, vec![id("a")]);
    assert!(matches!(
        rooms().assess_containment(&request),
        Err(FreeSpaceError::Unavailable(_))
    ));
    let smaller =
        ClearanceShape::Cylinder(CylinderClearance::try_new(0.9, 2.0).expect("valid cylinder"));
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), smaller, vec![id("a")]);
    assert!(matches!(
        rooms().assess_containment(&request),
        Ok(ContainmentOutcome::Inside(_))
    ));
}

#[test]
fn a_scope_without_a_body_or_tessellated_is_refused() {
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), square(1.0), vec![id("zone")]);
    assert!(matches!(
        rooms().assess_containment(&request),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
    let geometry = AxiolidGeometry::new().with_tessellated_mesh(
        id("round"),
        cuboid([0.0, 0.0, 0.0], [2.0, 2.0, 2.5]),
        0.001,
    );
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), square(1.0), vec![id("round")]);
    assert!(matches!(
        service.assess_containment(&request),
        Err(FreeSpaceError::Unavailable(_))
    ));
}

/// A room whose walls stand upright up to rounding (#377): its top corners
/// lie a few picometres off its bottom ones in plan, so each wall projects
/// to a sliver the overlay refuses. Containment is decided around them.
#[test]
fn a_room_whose_walls_project_to_slivers_decides_containment() {
    let mut room = cuboid([0.0, 0.0, 0.0], [2.0, 2.0, 2.5]);
    for point in &mut room.positions {
        if point.z > 0.0 {
            point.x += 3.0e-12;
            point.y -= 2.0e-12;
        }
    }
    let service =
        AxiolidFreeSpaceService::new(AxiolidGeometry::new().with_mesh(id("a"), room), source());
    let inside = ContainmentRequest::new(frame_at(1.0, 1.0), square(1.9), vec![id("a")]);
    assert!(matches!(
        service.assess_containment(&inside),
        Ok(ContainmentOutcome::Inside(_))
    ));
    let outside = ContainmentRequest::new(frame_at(1.5, 1.0), square(1.2), vec![id("a")]);
    assert!(matches!(
        service.assess_containment(&outside),
        Ok(ContainmentOutcome::Outside(_))
    ));
}

/// A scope whose 3,000 long, thin shadows lie over one another meets in
/// more pairs of edges than one overlay is handed: it is refused by name
/// before the overlay is built, never measured until memory runs out
/// (#432).
#[test]
fn a_scope_past_the_overlay_budget_is_refused_by_name() {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for index in 0..3000_u32 {
        let y = f64::from(index) * 1e-4;
        let first = u32::try_from(positions.len()).unwrap();
        positions.extend([
            Point3::new(0.0, y, 0.0),
            Point3::new(10.0, y + 0.5, 0.0),
            Point3::new(0.0, y + 1.0, 0.0),
        ]);
        indices.extend([first, first + 1, first + 2]);
    }
    let service = AxiolidFreeSpaceService::new(
        AxiolidGeometry::new().with_mesh(id("a"), TriMesh::new(positions, indices)),
        source(),
    );
    let request = ContainmentRequest::new(frame_at(1.0, 1.0), square(0.5), vec![id("a")]);
    let Err(FreeSpaceError::Unavailable(message)) = service.assess_containment(&request) else {
        panic!("a scope past the overlay budget was measured");
    };
    assert!(
        message.ends_with("past the overlay budget of 4000000 pairs"),
        "{message}"
    );
}
