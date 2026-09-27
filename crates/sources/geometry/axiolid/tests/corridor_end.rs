//! Corridor ends over real Axiolid geometry: the region skeleton of a
//! space's footprint, the wall each end runs into, and windows against it.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanSpanService};
use axioval_engine::{
    CorridorEndRequest, CorridorEnds, EndWall, PlanSpanError, PlanSpanService, WallContact,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(x0: f64, y0: f64, x1: f64, y1: f64, height: f64) -> TriMesh {
    let points = vec![
        Point3::new(x0, y0, 0.0),
        Point3::new(x1, y0, 0.0),
        Point3::new(x1, y1, 0.0),
        Point3::new(x0, y1, 0.0),
        Point3::new(x0, y0, height),
        Point3::new(x1, y0, height),
        Point3::new(x1, y1, height),
        Point3::new(x0, y1, height),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // floor, facing down
        4, 5, 6, 4, 6, 7, // ceiling, facing up
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn ends(geometry: AxiolidGeometry, space: &str, subjects: &[&str]) -> CorridorEnds {
    let request =
        CorridorEndRequest::try_new(id(space), subjects.iter().map(|subject| id(subject))).unwrap();
    AxiolidPlanSpanService::new(geometry, source())
        .measure_corridor_ends(&request)
        .unwrap()
}

/// A decided end wall nearest `point`, and its contacts.
fn wall_near(ends: &CorridorEnds, point: [f64; 2]) -> ([f64; 2], [f64; 2], Vec<WallContact>) {
    let near = |start: &[f64; 2], end: &[f64; 2]| {
        let mid = [
            f64::midpoint(start[0], end[0]),
            f64::midpoint(start[1], end[1]),
        ];
        (mid[0] - point[0]).hypot(mid[1] - point[1])
    };
    ends.ends()
        .iter()
        .filter_map(|end| match end.wall() {
            EndWall::Decided {
                start,
                end: last,
                contacts,
            } => Some((*start, *last, contacts.clone())),
            EndWall::Undecided(_) => None,
        })
        .min_by(|a, b| near(&a.0, &a.1).total_cmp(&near(&b.0, &b.1)))
        .expect("a decided end wall")
}

fn contact<'a>(contacts: &'a [WallContact], subject: &str) -> &'a WallContact {
    contacts
        .iter()
        .find(|contact| contact.subject() == &id(subject))
        .expect("the subject was measured")
}

/// Asserts a segment joins `a` and `b`, either way round.
#[allow(clippy::float_cmp)]
fn joins(start: [f64; 2], end: [f64; 2], a: [f64; 2], b: [f64; 2]) {
    assert!(
        (start == a && end == b) || (start == b && end == a),
        "{start:?}-{end:?} is not {a:?}-{b:?}"
    );
}

/// A 20 x 2 m corridor with a window in its east end wall, one in its
/// south side wall, and one in the side wall right next to the east corner.
fn straight() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("hall"), cuboid(0.0, 0.0, 20.0, 2.0, 3.0))
        .with_mesh(id("end"), cuboid(20.0, 0.5, 20.3, 1.5, 2.0))
        .with_mesh(id("side"), cuboid(10.0, -0.3, 11.0, 0.0, 2.0))
        .with_mesh(id("corner"), cuboid(19.0, -0.3, 20.0, 0.0, 2.0))
}

#[test]
fn a_straight_corridor_ends_at_both_end_walls() {
    let ends = ends(straight(), "hall", &["end", "side", "corner"]);
    assert!(!ends.evidence().exact);
    assert_eq!(ends.ends().len(), 2, "{ends:#?}");
    for end in ends.ends() {
        // Clearance is certified; the end lies about half a width in.
        let (lower, upper) = end.clearance_metres();
        assert!(lower <= upper && (0.8..=1.0).contains(&upper), "{end:?}");
    }
    let (start, last, contacts) = wall_near(&ends, [20.0, 1.0]);
    joins(start, last, [20.0, 0.0], [20.0, 2.0]);
    // The end window sits in the wall and faces a metre of it.
    let window = contact(&contacts, "end");
    assert!(window.gap().is_exact() && window.facing().is_exact());
    assert!(window.gap().upper_metres() < 1e-6);
    assert!((window.facing().lower_metres() - 1.0).abs() < 1e-6);
    // The side window faces none of it and lies far from it.
    let side = contact(&contacts, "side");
    assert!(side.facing().upper_metres() < 1e-6);
    assert!((side.gap().lower_metres() - 9.0).abs() < 1e-6);
    // The corner window touches the wall's end but faces none of it.
    let corner = contact(&contacts, "corner");
    assert!(corner.gap().upper_metres() < 1e-6);
    assert!(corner.facing().upper_metres() < 1e-6);
    let (start, last, contacts) = wall_near(&ends, [0.0, 1.0]);
    joins(start, last, [0.0, 0.0], [0.0, 2.0]);
    assert!((contact(&contacts, "end").gap().lower_metres() - 20.0).abs() < 1e-6);
}

#[test]
fn an_l_shaped_corridor_ends_at_the_far_end_of_each_leg() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("long"), cuboid(0.0, 0.0, 8.0, 1.5, 3.0))
        .with_mesh(id("short"), cuboid(6.5, 1.5, 8.0, 8.0, 3.0))
        .with_group(id("ell"), [id("long"), id("short")])
        .with_mesh(id("north"), cuboid(6.8, 8.0, 7.8, 8.3, 2.0))
        .with_mesh(id("bend"), cuboid(8.0, 0.2, 8.3, 1.2, 2.0));
    let ends = ends(geometry, "ell", &["bend", "north"]);
    assert_eq!(ends.ends().len(), 2, "{ends:#?}");
    let (start, last, contacts) = wall_near(&ends, [7.25, 8.0]);
    joins(start, last, [6.5, 8.0], [8.0, 8.0]);
    let north = contact(&contacts, "north");
    assert!(north.gap().upper_metres() < 1e-6);
    assert!((north.facing().lower_metres() - 1.0).abs() < 1e-6);
    // The window at the outside of the bend is in no end wall.
    let bend = contact(&contacts, "bend");
    assert!(bend.facing().upper_metres() < 1e-6);
    let (start, last, contacts) = wall_near(&ends, [0.0, 0.75]);
    joins(start, last, [0.0, 0.0], [0.0, 1.5]);
    assert!(contact(&contacts, "bend").gap().lower_metres() > 7.0);
}

#[test]
fn a_room_names_no_end_wall() {
    // A 6 x 5 m room: its skeleton is a short stub whose direction the
    // approximation cannot give, so no wall is named for its ends.
    let geometry = AxiolidGeometry::new().with_mesh(id("room"), cuboid(0.0, 0.0, 6.0, 5.0, 3.0));
    let ends = ends(geometry, "room", &[]);
    assert!(
        ends.ends()
            .iter()
            .all(|end| matches!(end.wall(), EndWall::Undecided(_))),
        "{ends:#?}"
    );
}

#[test]
fn a_tessellated_subject_widens_its_contact() {
    let deviation = 0.01;
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("hall"), cuboid(0.0, 0.0, 20.0, 2.0, 3.0))
        .with_tessellated_mesh(id("end"), cuboid(20.0, 0.5, 20.3, 1.5, 2.0), deviation);
    let ends = ends(geometry, "hall", &["end"]);
    let (_, _, contacts) = wall_near(&ends, [20.0, 1.0]);
    let window = contact(&contacts, "end");
    assert!(!window.gap().is_exact() && !window.facing().is_exact());
    assert!(window.gap().lower_metres() < 1e-12);
    assert!((window.gap().upper_metres() - deviation).abs() < 1e-6);
    assert!((window.facing().lower_metres() - (1.0 - 2.0 * deviation)).abs() < 1e-6);
    assert!((window.facing().upper_metres() - (1.0 + 2.0 * deviation)).abs() < 1e-6);
}

#[test]
fn a_tessellated_space_or_an_unknown_subject_is_refused() {
    let service = AxiolidPlanSpanService::new(
        AxiolidGeometry::new()
            .with_tessellated_mesh(id("round"), cuboid(0.0, 0.0, 20.0, 2.0, 3.0), 0.01)
            .with_mesh(id("hall"), cuboid(0.0, 0.0, 20.0, 2.0, 3.0)),
        source(),
    );
    let request = CorridorEndRequest::try_new(id("round"), []).unwrap();
    assert!(matches!(
        service.measure_corridor_ends(&request),
        Err(PlanSpanError::Unavailable(_))
    ));
    let request = CorridorEndRequest::try_new(id("hall"), [id("ghost")]).unwrap();
    assert!(matches!(
        service.measure_corridor_ends(&request),
        Err(PlanSpanError::UnknownObject(_))
    ));
}
