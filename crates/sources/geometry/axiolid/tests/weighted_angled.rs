//! Weighted travel over cost regions at an angle, over real Axiolid
//! geometry (axiolid/kernel#198, `axiolid-route` 0.3.5).
//!
//! Each scene is a room surface (a box, 3 m high) and a costed slab under
//! its floor, both turned about the origin, so that the cost edges and the
//! walls run at an angle. The slab's footprint is cut to the room's free
//! region by the overlay, whose output is rounded anew (axiolid/kernel#173):
//! along a turned wall the cut edge meets the wall only up to rounding.
//! Every bracket must hold the scene's closed form.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidMetricRoutingService};
use axioval_engine::{
    MetricPoint, MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome,
    NearestTargetRequest, TravelCost,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A turn about the origin by `(cos, sin)`.
#[derive(Clone, Copy)]
struct Turn(f64, f64);

impl Turn {
    fn degrees(degrees: f64) -> Self {
        let (s, c) = degrees.to_radians().sin_cos();
        Self(c, s)
    }

    fn point(self, x: f64, y: f64) -> [f64; 2] {
        let Self(c, s) = self;
        [c * x - s * y, s * x + c * y]
    }

    /// A closed, outward-oriented box over the turned rectangle
    /// `(x0, y0, x1, y1)`, from `z0` to `z1`.
    fn cuboid(self, (x0, y0, x1, y1): (f64, f64, f64, f64), z0: f64, z1: f64) -> TriMesh {
        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| self.point(x, y));
        let points: Vec<Point3> = [z0, z1]
            .iter()
            .flat_map(|z| corners.iter().map(move |[x, y]| Point3::new(*x, *y, *z)))
            .collect();
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
}

type Rectangle = (f64, f64, f64, f64);

/// The weighted walk in `room` from `from` to `to` with `cost` counting
/// twice, turned: its bracket.
fn walk(turn: Turn, room: Rectangle, cost: Rectangle, from: [f64; 2], to: [f64; 2]) -> (f64, f64) {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), turn.cuboid(room, 0.0, 3.0))
        .with_mesh(id("slab"), turn.cuboid(cost, -0.25, -0.05));
    let service = MetricRoutingServiceHandle::new(Arc::new(
        AxiolidMetricRoutingService::new(geometry, source()).with_surface(id("room")),
    ));
    let at = |[x, y]: [f64; 2]| {
        let [x, y] = turn.point(x, y);
        MetricPoint::try_new(id("room"), [x, y, 0.0]).unwrap()
    };
    let request = NearestTargetRequest::try_new(
        at(from),
        vec![at(to)],
        MobilityProfile::try_new(0.0, 2.0, 0.02, 0.0).unwrap(),
    )
    .unwrap()
    .with_costs(vec![TravelCost::try_new(id("slab"), 2.0).unwrap()]);
    let outcome = service
        .nearest_target(&request)
        .unwrap_or_else(|error| panic!("the weighted walk is measured: {error}"));
    let NearestTargetOutcome::Reached(reached) = outcome else {
        panic!("the target is reachable, got {outcome:?}");
    };
    let distance = reached.shortest_distance();
    (distance.lower_metres(), distance.upper_metres())
}

/// How far the overlay's grid snapping (axiolid/kernel#173) may move a
/// corner of the free region or of a cut cost region, and so a walk.
const SNAP: f64 = 1e-7;

fn holds((lower, upper): (f64, f64), exact: f64, gap: f64, scene: &str) {
    assert!(
        lower <= exact + SNAP && exact <= upper + SNAP,
        "{scene}: [{lower}, {upper}] misses {exact}"
    );
    assert!(upper - lower <= gap, "{scene}: [{lower}, {upper}] is loose");
}

#[test]
fn a_costed_corridor_cut_flush_with_a_turned_wall_is_weighed() {
    // A factor-2 corridor overshooting the room on three sides, cut to it:
    // along the bottom wall, turned 30 degrees, it meets the wall only up
    // to rounding, which `axiolid-route` 0.3.4 refused as a crossing. From
    // near one bottom corner to near the other the cheapest walk leaves the
    // corridor at the critical angle, runs along its edge and comes back:
    // 9.8 m along, plus sqrt 3 per metre risen on each leg. A sliver along
    // the wall costing 1 would allow 9.8.
    holds(
        walk(
            Turn::degrees(30.0),
            (0.0, 0.0, 10.0, 4.0),
            (-1.0, -1.0, 11.0, 1.5),
            [9.9, 0.1],
            [0.1, 0.1],
        ),
        9.8 + 2.0 * 3.0_f64.sqrt() * 1.4,
        0.5,
        "corridor",
    );
}

#[test]
fn brackets_hold_across_cost_edges_at_an_angle() {
    // Round a factor-2 square: to its near corner, along its edge (the
    // free side counting), and on; straight through costs more. With
    // `axiolid-route` 0.3.4, the square standing on a turned wall was
    // refused, or its cut left a sliver along the wall costing 1, so that
    // even the upper bound (9.28 m) fell short of the cost; the square clear
    // of the walls at 17 degrees was bracketed 0.25 m wide.
    let on_wall = (
        (0.0, 0.0, 10.0, 4.0),
        (4.0, 0.0, 6.0, 3.0),
        ([0.5, 1.0], [9.5, 1.0]),
        2.0 * 3.5_f64.hypot(2.0) + 2.0,
    );
    let clear = (
        (-1.0, -1.0, 11.0, 5.0),
        (4.0, 1.0, 6.0, 3.0),
        ([0.0, 2.0], [10.0, 2.0]),
        2.0 * 17.0_f64.sqrt() + 2.0,
    );
    for ((room, cost, (from, to), exact), turn) in [
        (on_wall, Turn(0.8, 0.6)),
        (on_wall, Turn::degrees(17.0)),
        (on_wall, Turn::degrees(45.0)),
        (clear, Turn::degrees(17.0)),
    ] {
        let Turn(c, s) = turn;
        holds(
            walk(turn, room, cost, from, to),
            exact,
            0.1,
            &format!("square {cost:?} turned ({c}, {s})"),
        );
    }
}
