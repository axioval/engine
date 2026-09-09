//! Staging probe: does local Axiolid pathfinding actually route?
//!
//! This crate exists to develop `MetricRoutingService` against the local
//! kernel while `axiolid-route` and `axiolid-predicates` are unpublished. It
//! is deliberately unpublishable (`publish = false`, path dependencies) and is
//! not a workspace member.
//!
//! **Every axiolid crate here must come from the same source.** Mixing a
//! published `axiolid-core` with a local `axiolid-route` produces two distinct
//! copies of the same types, and rustc rejects passing one into the other:
//! "two different versions of crate `axiolid_overlay` are being used".

use axiolid_core::Point2;
use axiolid_overlay::{Polygon, Ring};
use axiolid_route::{Route, Unreachable, shortest_path};

/// A rectangular room as a single region polygon.
fn room(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
    Polygon {
        outer: Ring {
            points: vec![
                Point2::new(x0, y0),
                Point2::new(x1, y0),
                Point2::new(x1, y1),
                Point2::new(x0, y1),
            ],
        },
        holes: Vec::new(),
    }
}

/// Shortest walkable path between two points inside `region`.
///
/// Returns `Ok(None)` when the geometry proves no path exists, which is a
/// measurement, not a failure.
pub fn route_between(
    region: &[Polygon],
    barriers: &[Vec<Point2>],
    start: Point2,
    goal: Point2,
) -> Result<Option<Route>, String> {
    match shortest_path(region, barriers, start, goal) {
        Ok(Ok(route)) => Ok(Some(route)),
        Ok(Err(reason)) => {
            let _: Unreachable = reason;
            Ok(None)
        }
        Err(error) => Err(format!("{error:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Point2, room, route_between};

    /// A straight walk across an empty room is the direct distance.
    #[test]
    fn an_empty_room_routes_directly() {
        let region = [room(0.0, 0.0, 10.0, 10.0)];
        let route = route_between(&region, &[], Point2::new(1.0, 1.0), Point2::new(9.0, 1.0))
            .expect("valid query")
            .expect("a path exists");
        assert!(
            (route.length - 8.0).abs() < 1e-9,
            "direct walk is 8 m, got {}",
            route.length
        );
    }

    /// A barrier across the room forces a longer path than the direct line.
    ///
    /// This is the property routing exists for: the detour must be measured,
    /// not assumed.
    #[test]
    fn a_barrier_forces_a_detour() {
        let region = [room(0.0, 0.0, 10.0, 10.0)];
        // A wall from the south edge to y = 8, leaving a gap at the north.
        let barrier = vec![Point2::new(5.0, 0.0), Point2::new(5.0, 8.0)];
        let route = route_between(
            &region,
            std::slice::from_ref(&barrier),
            Point2::new(1.0, 1.0),
            Point2::new(9.0, 1.0),
        )
        .expect("valid query")
        .expect("a path around the barrier exists");
        assert!(
            route.length > 8.0,
            "the detour must exceed the blocked direct line, got {}",
            route.length
        );
    }

    /// A point outside the region is unreachable, not an error.
    #[test]
    fn a_goal_outside_the_region_is_unreachable() {
        let region = [room(0.0, 0.0, 10.0, 10.0)];
        let outcome = route_between(&region, &[], Point2::new(1.0, 1.0), Point2::new(99.0, 99.0))
            .expect("valid query");
        assert!(
            outcome.is_none(),
            "outside the region is no path, not a failure"
        );
    }
}
