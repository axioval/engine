//! Shelf capacity measured from real geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidLinearQuantityService};
use axioval_engine::{
    LinearQuantityError, LinearQuantityKind, LinearQuantityRequest, LinearQuantityService,
    ShelfGeometry,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid object id")
}

/// A room as a box: `width` x `depth` in plan, `height` tall.
fn room(width: f64, depth: f64, height: f64) -> TriMesh {
    let (w, d, h) = (width, depth, height);
    let positions = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(w, 0.0, 0.0),
        Point3::new(w, d, 0.0),
        Point3::new(0.0, d, 0.0),
        Point3::new(0.0, 0.0, h),
        Point3::new(w, 0.0, h),
        Point3::new(w, d, h),
        Point3::new(0.0, d, h),
    ];
    // Floor and ceiling only: the measurement uses the extent, and a closed
    // box is not needed to establish it.
    let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
    TriMesh::new(positions, indices)
}

/// Shelving 0.3 m deep, 1 m pitch, tiers every 0.4 m up to 2 m.
fn shelf() -> ShelfGeometry {
    ShelfGeometry::try_new(0.3, 1.0, 0.4, 0.0, 2.0, 0.9).expect("valid shelf geometry")
}

fn measure(geometry: AxiolidGeometry, scope: ObjectId) -> Result<f64, LinearQuantityError> {
    let service = AxiolidLinearQuantityService::new(geometry, source());
    let request =
        LinearQuantityRequest::new(scope, LinearQuantityKind::ShelfRunningLength(shelf()));
    service
        .measure_linear_quantity(&request)
        .map(|evidence| evidence.measured().upper_metres())
}

/// A bigger room holds more shelving than a smaller one.
///
/// The weakest possible property, and the one a constant would fail: the
/// measurement must actually depend on the geometry it was given.
#[test]
fn a_larger_room_takes_more_shelving() {
    let small = AxiolidGeometry::new().with_mesh(id("small"), room(4.0, 4.0, 3.0));
    let large = AxiolidGeometry::new().with_mesh(id("large"), room(8.0, 8.0, 3.0));

    let small_metres = measure(small, id("small")).expect("small room measurable");
    let large_metres = measure(large, id("large")).expect("large room measurable");

    assert!(
        large_metres > small_metres,
        "larger room must take more shelving, got {large_metres} vs {small_metres}"
    );
}

/// Doorways consume wall that cannot carry a run.
#[test]
fn doorways_reduce_the_usable_wall() {
    let clear = AxiolidGeometry::new().with_mesh(id("room"), room(6.0, 6.0, 3.0));
    let with_doors = AxiolidGeometry::new()
        .with_mesh(id("room"), room(6.0, 6.0, 3.0))
        .with_doorways(id("room"), 3);

    let clear_metres = measure(clear, id("room")).expect("clear room measurable");
    let door_metres = measure(with_doors, id("room")).expect("room with doors measurable");

    assert!(
        door_metres < clear_metres,
        "doorways must reduce capacity, got {door_metres} vs {clear_metres}"
    );
}

/// A room too shallow for the shelf depth holds nothing.
///
/// Compared exactly: "nothing fits" is a definite measurement, and an epsilon
/// would let a tiny positive capacity masquerade as none.
#[test]
#[allow(clippy::float_cmp)]
fn a_room_shallower_than_the_shelf_holds_none() {
    let geometry = AxiolidGeometry::new().with_mesh(id("slot"), room(6.0, 0.2, 3.0));
    let metres = measure(geometry, id("slot")).expect("slot measurable");
    assert_eq!(metres, 0.0, "a room shallower than the shelf holds nothing");
}

/// A ceiling below the first tier holds no shelving.
///
/// The tier count must come from the room's real height, not from the
/// requested top elevation: a crawlspace cannot hold a 2 m stack.
#[test]
#[allow(clippy::float_cmp)]
fn a_ceiling_below_the_first_tier_holds_none() {
    let geometry = AxiolidGeometry::new().with_mesh(id("crawl"), room(6.0, 6.0, 0.3));
    let metres = measure(geometry, id("crawl")).expect("crawlspace measurable");
    assert_eq!(metres, 0.0, "no tier fits under a 0.3 m ceiling");
}

/// An unknown object is unavailable rather than silently zero.
///
/// Zero is a measurement meaning "no shelving fits"; absence is not.
#[test]
fn an_unknown_object_is_unavailable() {
    let geometry = AxiolidGeometry::new();
    assert_eq!(
        measure(geometry, id("absent")),
        Err(LinearQuantityError::Unavailable)
    );
}

/// The measurement is reported as an upper bound, not as exact.
///
/// It derives from a bounding footprint, so a non-rectangular room's true
/// usable wall is at most this. Claiming exactness would assert more than
/// the geometry supports, and the capability fails closed on that ambiguity.
#[test]
fn a_footprint_bound_is_not_reported_as_exact() {
    let geometry = AxiolidGeometry::new().with_mesh(id("room"), room(6.0, 6.0, 3.0));
    let service = AxiolidLinearQuantityService::new(geometry, source());
    let request =
        LinearQuantityRequest::new(id("room"), LinearQuantityKind::ShelfRunningLength(shelf()));
    let evidence = service
        .measure_linear_quantity(&request)
        .expect("room measurable");
    assert!(
        !evidence.measured().is_exact(),
        "a bound derived from a footprint must not claim to be exact"
    );
}
