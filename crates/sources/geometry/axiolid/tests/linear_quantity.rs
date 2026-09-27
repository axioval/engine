//! Shelf capacity measured from real geometry: parallel bands on the
//! footprint less door clearances.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidLinearQuantityService};
use axioval_engine::{
    LinearInterval, LinearQuantityError, LinearQuantityKind, LinearQuantityRequest,
    LinearQuantityService, ShelfGeometry,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid object id")
}

/// A closed, outward-wound box from `min` to `max`.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let positions = vec![
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // bottom, facing down
        4, 5, 6, 4, 6, 7, // top, facing up
        0, 1, 5, 0, 5, 4, // south
        1, 2, 6, 1, 6, 5, // east
        2, 3, 7, 2, 7, 6, // north
        3, 0, 4, 3, 4, 7, // west
    ];
    TriMesh::new(positions, indices)
}

/// A 6 m × 4 m room, `height` tall.
fn room(height: f64) -> TriMesh {
    cuboid([0.0, 0.0, 0.0], [6.0, 4.0, height])
}

/// A 1 m wide door leaf in the south wall, its inner face flush with the
/// room.
fn door() -> TriMesh {
    cuboid([2.5, -0.2, 0.0], [3.5, 0.0, 2.1])
}

/// Bands 0.5 m deep with 1 m aisles, tiers every 0.5 m up to 2 m, and a
/// 1 m clearance at doors.
fn shelf() -> ShelfGeometry {
    ShelfGeometry::try_new(0.5, 1.0, 0.5, 0.0, 2.0, 1.0).expect("valid shelf geometry")
}

fn measure(
    geometry: AxiolidGeometry,
    doors: &[&str],
) -> Result<(LinearInterval, LinearInterval), LinearQuantityError> {
    measure_with(&AxiolidLinearQuantityService::new(geometry), doors, shelf())
}

fn measure_with(
    service: &AxiolidLinearQuantityService,
    doors: &[&str],
    shelf: ShelfGeometry,
) -> Result<(LinearInterval, LinearInterval), LinearQuantityError> {
    let request =
        LinearQuantityRequest::new(id("room"), LinearQuantityKind::ShelfRunningLength(shelf))
            .with_doors(doors.iter().map(|door| id(door)));
    let evidence = service.measure_linear_quantity(&request)?;
    assert_eq!(evidence.request(), &request);
    Ok((
        evidence.measured(),
        evidence.clear_height().expect("a clear height"),
    ))
}

fn assert_near(interval: LinearInterval, expected: f64, slack: f64) {
    assert!(
        interval.lower_metres() <= expected && expected <= interval.upper_metres(),
        "{interval:?} must hold {expected}"
    );
    assert!(
        expected - interval.lower_metres() < slack && interval.upper_metres() - expected < slack,
        "{interval:?} must lie within {slack} of {expected}"
    );
}

/// Without doors, four 6 m bands fit across the 4 m: a band against each
/// long wall and two back to back, with an aisle between each pair. Four
/// tiers of 0.5 m fit under 2 m: 96 m.
#[test]
fn a_rectangular_room_holds_four_bands_in_four_tiers() {
    let geometry = AxiolidGeometry::new().with_mesh(id("room"), room(3.0));
    let (length, height) = measure(geometry, &[]).expect("measurable");
    assert_near(length, 96.0, 1e-4);
    assert!(height.is_exact());
    assert!(height.definitely_at_least(3.0));
}

/// The door's clearance reaches 1 m from the leaf in every direction, so
/// the band against the south wall loses the 3 m between x = 1.5 and 4.5;
/// the band behind the aisle is beyond its reach. 21 m in four tiers: 84 m,
/// with a lower bound close enough to pass a rule asking for 80 m.
#[test]
fn a_door_removes_its_clearance_from_the_bands_it_reaches() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), room(3.0))
        .with_mesh(id("door"), door());
    let (length, _) = measure(geometry, &["door"]).expect("measurable");
    assert_near(length, 84.0, 5e-3);
    assert!(length.definitely_at_least(80.0));
}

/// A leaf standing back from the room in a thick wall places its clearance
/// from the room's side of the wall, not from the leaf: the gap is added.
#[test]
fn a_recessed_leaf_reaches_as_far_into_the_room() {
    let flush = AxiolidGeometry::new()
        .with_mesh(id("room"), room(3.0))
        .with_mesh(id("door"), door());
    let set_back = AxiolidGeometry::new()
        .with_mesh(id("room"), room(3.0))
        .with_mesh(id("door"), cuboid([2.5, -0.2, 0.0], [3.5, -0.1, 2.1]));
    let (flush, _) = measure(flush, &["door"]).expect("measurable");
    let (set_back, _) = measure(set_back, &["door"]).expect("measurable");
    assert!(set_back.upper_metres() <= flush.upper_metres() + 1e-3);
    assert!(set_back.lower_metres() < 84.0);
}

/// A bodiless opening places its clearance through the void the host gives.
#[test]
fn an_opening_is_placed_through_its_void() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), room(3.0))
        .with_no_body(id("door"));
    let service =
        AxiolidLinearQuantityService::new(geometry.clone()).with_opening_void(id("door"), door());
    let (length, _) = measure_with(&service, &["door"], shelf()).expect("measurable");
    assert_near(length, 84.0, 5e-3);

    // Without the void, or with an unmeasured one, the space is not
    // measured: its clearance is unknown.
    assert_eq!(
        measure(geometry.clone(), &["door"]),
        Err(LinearQuantityError::Unavailable)
    );
    let unmeasured =
        AxiolidLinearQuantityService::new(geometry).with_unmeasured_opening_void(id("door"));
    assert_eq!(
        measure_with(&unmeasured, &["door"], shelf()),
        Err(LinearQuantityError::Unavailable)
    );
}

/// A door far from the space is not on its boundary; its clearance is not
/// guessed.
#[test]
fn a_door_away_from_the_space_refuses() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), room(3.0))
        .with_mesh(id("door"), cuboid([2.5, -3.2, 0.0], [3.5, -3.0, 2.1]));
    assert_eq!(
        measure(geometry, &["door"]),
        Err(LinearQuantityError::Unavailable)
    );
}

/// A space under the shelving's top holds only the tiers that fit, and
/// says how high it is so the rule can report it too low.
#[test]
fn a_low_room_reports_its_height_and_fewer_tiers() {
    let geometry = AxiolidGeometry::new().with_mesh(id("room"), room(1.2));
    let (length, height) = measure(geometry, &[]).expect("measurable");
    assert!(height.is_exact());
    assert!(height.definitely_below(2.0));
    // Two tiers of 0.5 m fit under 1.2 m.
    assert_near(length, 48.0, 1e-4);
}

/// A room narrower than a band and its aisle holds nothing.
#[test]
#[allow(clippy::float_cmp)]
fn a_room_narrower_than_a_band_and_its_aisle_holds_none() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [1.2, 1.2, 3.0]));
    let (length, _) = measure(geometry, &[]).expect("measurable");
    assert_eq!(length.upper_metres(), 0.0);
}

/// An L-shaped room holds bands only where their whole depth and aisle lie
/// inside it: less than its bounding rectangle would.
#[test]
fn an_l_shaped_room_holds_less_than_its_bounding_box() {
    // 6 × 2 along the south, and 2 × 4 more up the west side.
    let l_shape = axiolid_mesh::compose(&[
        cuboid([0.0, 0.0, 0.0], [6.0, 2.0, 3.0]),
        cuboid([0.0, 2.0, 0.0], [2.0, 6.0, 3.0]),
    ]);
    let geometry = AxiolidGeometry::new().with_mesh(id("room"), l_shape);
    let (length, _) = measure(geometry, &[]).expect("measurable");
    let bounding =
        AxiolidGeometry::new().with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [6.0, 6.0, 3.0]));
    let (bounding, _) = measure(bounding, &[]).expect("measurable");
    assert!(length.lower_metres() > 0.0);
    assert!(length.upper_metres() < bounding.lower_metres());
    assert!(length.upper_metres() - length.lower_metres() < 1e-3);
}

/// An unknown object is unavailable rather than silently zero.
#[test]
fn an_unknown_object_is_unavailable() {
    assert_eq!(
        measure(AxiolidGeometry::new(), &[]),
        Err(LinearQuantityError::Unavailable)
    );
}

/// One geometry set feeds several services, and measures identically.
#[test]
fn one_geometry_set_serves_several_services() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), room(3.0))
        .with_mesh(id("door"), door());
    let shared = geometry.clone();
    assert_eq!(measure(geometry, &["door"]), measure(shared, &["door"]));
}
