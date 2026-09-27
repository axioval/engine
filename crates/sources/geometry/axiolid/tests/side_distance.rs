//! What lies beside each side of a footprint's least-area rectangle, over
//! real Axiolid geometry.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanSpanService};
use axioval_engine::{
    PlanSpanError, PlanSpanServiceHandle, RectangleSide, SideDistanceRequest, SideDistances,
    SidePresence,
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

fn block(min: [f64; 2], max: [f64; 2], height: f64) -> TriMesh {
    prism(
        &[
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ],
        height,
    )
}

/// The plan turned `degrees` about `pivot`.
fn turned(plan: &[[f64; 2]], degrees: f64, pivot: [f64; 2]) -> Vec<[f64; 2]> {
    let (sin, cos) = degrees.to_radians().sin_cos();
    plan.iter()
        .map(|[x, y]| {
            let (dx, dy) = (x - pivot[0], y - pivot[1]);
            [
                pivot[0] + cos * dx - sin * dy,
                pivot[1] + sin * dx + cos * dy,
            ]
        })
        .collect()
}

/// A WC (x 1 to 1.4, y 0 to 0.7) against a south wall, a west wall 1 m
/// from its west face, and nothing east or north.
fn washroom() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("wc"), block([1.0, 0.0], [1.4, 0.7], 0.4))
        .with_mesh(id("south"), block([-0.2, -0.2], [3.2, 0.0], 2.5))
        .with_mesh(id("west"), block([-0.2, -0.2], [0.0, 3.2], 2.5))
}

fn measure(
    geometry: AxiolidGeometry,
    inset: f64,
    reach: f64,
) -> Result<SideDistances, PlanSpanError> {
    let spans =
        PlanSpanServiceHandle::new(Arc::new(AxiolidPlanSpanService::new(geometry, source())));
    let request =
        SideDistanceRequest::try_new(id("wc"), [id("south"), id("west")], reach, inset).unwrap();
    spans.measure_side_distances(&request)
}

/// `(candidate, presence, lower, upper)` beside `side`.
fn beside(measured: &SideDistances, side: RectangleSide) -> Vec<(String, SidePresence, f64, f64)> {
    measured
        .beside(side)
        .map(|distance| {
            (
                distance.candidate().local_id.clone(),
                distance.presence(),
                distance.distance().lower_metres(),
                distance.distance().upper_metres(),
            )
        })
        .collect()
}

fn near(value: f64, expected: f64) -> bool {
    (value - expected).abs() < 1e-6
}

#[test]
fn walls_are_found_beside_the_sides_they_face() {
    let measured = measure(washroom(), 0.01, 2.0).unwrap();
    assert_eq!(measured.rectangle().axes(), [[1.0, 0.0], [0.0, 1.0]]);
    assert!(!measured.evidence().exact);
    // South: the wall the WC stands against, 0.35 m from its centre line.
    let south = beside(&measured, RectangleSide::AgainstSecond);
    assert_eq!(south.len(), 1, "{south:?}");
    assert_eq!(south[0].0, "south");
    assert_eq!(south[0].1, SidePresence::Sure);
    assert!(
        near(south[0].2, 0.35) && near(south[0].3, 0.35),
        "{south:?}"
    );
    assert!(south[0].2 <= 0.35 && 0.35 <= south[0].3, "{south:?}");
    // West: the side wall, 1.2 m from the centre line; the south wall only
    // touches this strip's edge, which the inset keeps out.
    let west = beside(&measured, RectangleSide::AgainstFirst);
    assert_eq!(west.len(), 1, "{west:?}");
    assert_eq!(west[0].0, "west");
    assert!(near(west[0].2, 1.2) && near(west[0].3, 1.2), "{west:?}");
    // Nothing east or north within reach.
    assert!(beside(&measured, RectangleSide::AlongFirst).is_empty());
    assert!(beside(&measured, RectangleSide::AlongSecond).is_empty());
}

#[test]
fn without_an_inset_a_flush_wall_may_touch_the_neighbouring_strips() {
    let measured = measure(washroom(), 0.0, 2.0).unwrap();
    let west = beside(&measured, RectangleSide::AgainstFirst);
    let south = west.iter().find(|entry| entry.0 == "south").unwrap();
    // It may reach into the strip right at the WC's centre line.
    assert_eq!(south.1, SidePresence::Possible, "{west:?}");
    assert!(south.2 < 1e-6, "{west:?}");
}

#[test]
fn a_wall_beyond_the_reach_is_not_listed() {
    let measured = measure(washroom(), 0.01, 1.0).unwrap();
    assert!(beside(&measured, RectangleSide::AgainstFirst).is_empty());
}

#[test]
fn a_turned_footprint_measures_within_its_slack() {
    let pivot = [1.2, 0.35];
    let plan = |min: [f64; 2], max: [f64; 2]| {
        turned(
            &[
                [min[0], min[1]],
                [max[0], min[1]],
                [max[0], max[1]],
                [min[0], max[1]],
            ],
            20.0,
            pivot,
        )
    };
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wc"), prism(&plan([1.0, 0.0], [1.4, 0.7]), 0.4))
        .with_mesh(id("south"), prism(&plan([-0.2, -0.2], [3.2, 0.0]), 2.5))
        .with_mesh(id("west"), prism(&plan([-0.2, -0.2], [0.0, 3.2]), 2.5));
    let measured = measure(geometry, 0.01, 2.0).unwrap();
    let sides: Vec<(String, f64, f64)> = measured
        .distances()
        .iter()
        .map(|distance| {
            (
                distance.candidate().local_id.clone(),
                distance.distance().lower_metres(),
                distance.distance().upper_metres(),
            )
        })
        .collect();
    assert_eq!(sides.len(), 2, "{sides:?}");
    for (candidate, expected) in [("south", 0.35), ("west", 1.2)] {
        let (_, lower, upper) = sides.iter().find(|entry| entry.0 == candidate).unwrap();
        assert!(*lower <= expected && expected <= *upper, "{sides:?}");
        assert!(upper - lower < 1e-6, "{sides:?}");
    }
}
