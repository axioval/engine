//! Least-area rectangles and bands between footprints over real Axiolid
//! geometry.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService, AxiolidPlanSpanService};
use axioval_engine::{
    PlanAreaServiceHandle, PlanBand, PlanSpanError, PlanSpanServiceHandle, RectangleOrientation,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented prism over the counter-clockwise plan
/// polygon `plan`, `height` high.
fn prism(plan: &[[f64; 2]], height: f64) -> TriMesh {
    let n = u32::try_from(plan.len()).unwrap();
    let mut points: Vec<Point3> = plan.iter().map(|[x, y]| Point3::new(*x, *y, 0.0)).collect();
    points.extend(plan.iter().map(|[x, y]| Point3::new(*x, *y, height)));
    let mut indices = Vec::new();
    for i in 1..n - 1 {
        indices.extend([0, i + 1, i]);
        indices.extend([n, n + i, n + i + 1]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, n + j, i, n + j, n + i]);
    }
    TriMesh::new(points, indices)
}

/// A `length` x `width` rectangle centred at `centre`, its length turned
/// `degrees` from the x-axis.
fn turned(centre: [f64; 2], length: f64, width: f64, degrees: f64) -> Vec<[f64; 2]> {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(a, b)| {
            let (u, v) = (a * length / 2.0, b * width / 2.0);
            [centre[0] + cos * u - sin * v, centre[1] + sin * u + cos * v]
        })
        .to_vec()
}

fn spans(geometry: AxiolidGeometry) -> PlanSpanServiceHandle {
    PlanSpanServiceHandle::new(Arc::new(AxiolidPlanSpanService::new(geometry, source())))
}

fn areas(geometry: AxiolidGeometry) -> PlanAreaServiceHandle {
    PlanAreaServiceHandle::new(Arc::new(AxiolidPlanAreaService::new(geometry, source())))
}

/// Whether `interval` holds `value` and is at most a nanometre wide.
fn within(interval: (f64, f64), value: f64) -> bool {
    interval.0 <= value && value <= interval.1 && interval.1 - interval.0 < 1e-9
}

#[test]
#[allow(clippy::float_cmp)]
fn an_axis_aligned_bay_is_measured_along_its_own_axes() {
    let geometry = AxiolidGeometry::new().with_mesh(
        id("bay"),
        prism(&[[0.0, 0.0], [2.5, 0.0], [2.5, 5.0], [0.0, 5.0]], 2.2),
    );
    let rectangle = spans(geometry).measure_rectangle(&id("bay")).unwrap();
    assert_eq!(rectangle.orientation(), RectangleOrientation::Unique);
    assert_eq!(rectangle.centre(), [1.25, 2.5]);
    // Along the coordinate axes nothing was rounded.
    assert!(rectangle.is_exact(), "{rectangle:?}");
    assert_eq!(
        rectangle.width_and_length().unwrap(),
        [(2.5, 2.5), (5.0, 5.0)]
    );
    assert_eq!(rectangle.axes()[rectangle.long_axis().unwrap()], [0.0, 1.0]);
    assert!(rectangle.evidence().locator.starts_with("plan-rectangle:"));
}

#[test]
fn a_turned_bay_is_measured_along_its_own_axes_not_its_bounding_box() {
    // 2.5 x 4.5 m turned 30 degrees: its box is about 5.15 m long.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("bay"), prism(&turned([10.0, 3.0], 4.5, 2.5, 30.0), 2.2))
        .with_mesh(id("aisle"), prism(&turned([0.0, 0.0], 20.0, 6.0, 0.0), 2.2));
    let service = spans(geometry);
    let bay = service.measure_rectangle(&id("bay")).unwrap();
    assert_eq!(bay.orientation(), RectangleOrientation::Unique);
    assert!(!bay.is_exact(), "turned axes are rounded");
    let [width, length] = bay.width_and_length().unwrap();
    assert!(within(width, 2.5) && within(length, 4.5), "{bay:?}");
    let aisle = service.measure_rectangle(&id("aisle")).unwrap();
    let (low, high) = bay.long_axis_angle(&aisle).unwrap();
    assert!(
        low <= 30.0 && 30.0 <= high && high - low < 1e-6,
        "{low} {high}"
    );
}

#[test]
fn a_square_has_no_long_axis_but_has_its_sides() {
    let geometry = AxiolidGeometry::new().with_mesh(
        id("square"),
        prism(&[[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0]], 1.0),
    );
    let rectangle = spans(geometry).measure_rectangle(&id("square")).unwrap();
    assert_eq!(rectangle.orientation(), RectangleOrientation::Unique);
    assert_eq!(
        rectangle.width_and_length().unwrap(),
        [(3.0, 3.0), (3.0, 3.0)]
    );
    let reason = rectangle.long_axis().unwrap_err();
    assert!(reason.contains("too close to equal"), "{reason}");
    assert!(rectangle.long_axis_angle(&rectangle).is_err());
}

#[test]
fn a_tessellated_footprint_has_no_proven_orientation() {
    let geometry = AxiolidGeometry::new().with_tessellated_mesh(
        id("round"),
        prism(&turned([0.0, 0.0], 4.0, 1.0, 0.0), 1.0),
        0.01,
    );
    let rectangle = spans(geometry).measure_rectangle(&id("round")).unwrap();
    assert_eq!(rectangle.orientation(), RectangleOrientation::Unproven);
    assert!(!rectangle.is_exact());
    let [(low, high), _] = rectangle.half_extents_metres();
    assert!(low <= 1.99 && high >= 2.01, "{low} {high}");
    assert!(
        rectangle
            .width_and_length()
            .unwrap_err()
            .contains("tessellated")
    );
}

#[test]
#[allow(clippy::float_cmp)]
fn rounded_sides_along_the_axes_are_intervals_holding_the_exact_length() {
    // 0.7 - 0.1 is not a double: the width is an interval around 0.6.
    let geometry = AxiolidGeometry::new().with_mesh(
        id("thin"),
        prism(&[[0.1, 0.0], [0.7, 0.0], [0.7, 4.0], [0.1, 4.0]], 1.0),
    );
    let rectangle = spans(geometry).measure_rectangle(&id("thin")).unwrap();
    assert!(!rectangle.is_exact());
    assert_eq!(rectangle.axis_error_radians(), 0.0);
    let [(low, high), length] = rectangle.half_extents_metres();
    assert!(low < high && high - low < 1e-16, "{low} {high}");
    assert_eq!(length, (2.0, 2.0));
}

#[test]
fn objects_without_a_footprint_are_refused() {
    let geometry = AxiolidGeometry::new()
        .with_no_body(id("storey"))
        .with_unmeasured(id("broken"), "no mesh");
    let service = spans(geometry);
    assert!(matches!(
        service.measure_rectangle(&id("storey")),
        Err(PlanSpanError::Unavailable(_))
    ));
    assert!(service.measure_rectangle(&id("broken")).is_err());
    assert!(matches!(
        service.measure_rectangle(&id("missing")),
        Err(PlanSpanError::UnknownObject(_))
    ));
}

/// A 10 x 8 m slab with two walls 0.2 m thick along x, at y 0..0.2 and
/// 5..5.2, the second only from x 2 to 10.
fn slab_and_walls() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(
            id("slab"),
            prism(&[[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]], 0.3),
        )
        .with_mesh(
            id("south"),
            prism(&[[0.0, 0.0], [10.0, 0.0], [10.0, 0.2], [0.0, 0.2]], 3.0),
        )
        .with_mesh(
            id("middle"),
            prism(&[[2.0, 5.0], [10.0, 5.0], [10.0, 5.2], [2.0, 5.2]], 3.0),
        )
        .with_tessellated_mesh(
            id("curved"),
            prism(&[[0.0, 7.8], [10.0, 7.8], [10.0, 8.0], [0.0, 8.0]], 3.0),
            0.01,
        )
}

#[test]
fn a_band_covers_the_strip_between_two_walls_over_their_shared_length() {
    let areas = areas(slab_and_walls());
    let band = PlanBand::try_new(id("south"), id("middle"), [1.0, 0.0]).unwrap();
    let outside = areas
        .measure_outside_bands(&id("slab"), std::slice::from_ref(&band))
        .unwrap();
    // The band spans x 2..10 and y 0..5.2: 41.6 of the slab's 80 m².
    assert!(
        outside.lower_square_metres() <= 38.4 + 1e-6
            && outside.upper_square_metres() >= 38.4 - 1e-6
            && outside.upper_square_metres() - outside.lower_square_metres() < 1e-5,
        "{outside:?}"
    );
    // Along y the two reach no common position: the band is empty.
    let across = PlanBand::try_new(id("south"), id("middle"), [0.0, 1.0]).unwrap();
    let outside = areas.measure_outside_bands(&id("slab"), &[across]).unwrap();
    assert!(
        (outside.lower_square_metres() - 80.0).abs() < 1e-6,
        "{outside:?}"
    );
    // No band leaves the whole footprint outside.
    let outside = areas.measure_outside_bands(&id("slab"), &[]).unwrap();
    assert!(outside.is_exact());
    assert!((outside.upper_square_metres() - 80.0).abs() < 1e-6);
}

#[test]
fn a_band_refuses_a_tessellated_wall_and_its_own_subject() {
    let areas = areas(slab_and_walls());
    let curved = PlanBand::try_new(id("south"), id("curved"), [1.0, 0.0]).unwrap();
    assert!(areas.measure_outside_bands(&id("slab"), &[curved]).is_err());
    let own = PlanBand::try_new(id("slab"), id("south"), [1.0, 0.0]).unwrap();
    assert!(areas.measure_outside_bands(&id("slab"), &[own]).is_err());
    assert!(PlanBand::try_new(id("south"), id("south"), [1.0, 0.0]).is_err());
    assert!(PlanBand::try_new(id("south"), id("middle"), [0.0, 0.0]).is_err());
}
