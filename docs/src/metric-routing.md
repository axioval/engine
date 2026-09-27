# Metric routing

Metric routing is a typed host service, not an algorithm embedded in the rule engine. `MetricRoutingService` may be implemented by Axiolid or another trusted geometry backend. OpenBIM may supply semantic route candidates, but it does not own geometry and no combined OpenBIM–Axiolid adapter is required.

## Canonical request

A `MetricRouteRequest` carries:

- source-qualified, object-grounded origin and destination points;
- coordinates in canonical metres;
- a validated mobility profile: radius, clear height, maximum step, and maximum slope.

NaN, infinity, and negative dimensions are rejected before backend execution. `MetricRoutingServiceHandle` validates that a backend response starts and ends at the requested points. The handle is a concrete type that can be registered in `ServiceRegistry` and consumed through `RuleContext`.

## Bounded shortest-distance evidence

`LengthInterval` stores inclusive lower and upper bounds. Exact evidence has equal bounds. Policy comparison is intentionally three-valued:

- `Satisfied`: the upper bound meets the maximum;
- `Violated`: the lower bound exceeds the maximum;
- `Indeterminate`: the interval straddles the threshold.

A known longer route plus an unavailable shortcut can therefore prove route existence, but cannot produce an exact shortest-distance verdict. Raster or tolerance-based backends may return conservative bounds rather than silently upgrading an approximation.

## Negative results

`MetricRouteOutcome::Blocked` requires `CompleteMetricEvidence`: exact, reviewable provenance that topology and relevant obstacles were complete for the query. `BlockedMetricRouteEvidence` binds that proof to the complete request, including endpoints and mobility profile; the service handle rejects evidence returned for another request. Missing geometry, partial obstacle composition, unsupported connectors, resource limits, and backend failures return `MetricRoutingError`; they never become `Blocked` or a passing rule result.

## Many targets

Two further queries measure against several targets at once. Both are
default methods of `MetricRoutingService` that refuse, so a backend that
cannot search many targets never answers with a single pair.

- `nearest_target(NearestTargetRequest)`: from one origin to the nearest of
  several `MetricPoint` targets (at least one, `NoTargets` otherwise).
  `Reached(NearestTargetEvidence)` carries the index of the target a known
  route reaches, that route's waypoints, and a `LengthInterval` that bounds
  the distance to the nearest of *all* targets, which may be another one.
  `Unreachable(UnreachableTargetsEvidence)` needs complete evidence bound to
  the request, as `Blocked` does.
- `farthest_point(FarthestPointRequest)`: the largest distance from any
  point of a region, an object's walkable area, to its nearest target,
  asked to within `tolerance_metres` (`InvalidTolerance` for a negative or
  non-finite one). `Bounded(FarthestPointEvidence)` carries an interval that
  contains the maximum, a witness point of the region at least the lower
  bound from every target, and whether the interval converged to the
  tolerance. `Unreachable(UnreachableRegionEvidence)` names a point of the
  region no target reaches, with complete evidence; the largest distance is
  then unbounded.

The handle checks every answer against its request: a reached target must
exist and end the route that starts at the origin, a witness must lie on the
requested region, a claimed convergence must hold for the requested
tolerance (`InconsistentResponse` otherwise), and an unreachable verdict
must name the request it answers.

The engine contract contains no mesh, B-rep, IFC entity, Axiolid kernel, OpenCascade, or vendor type.

## The Axiolid backend

`AxiolidMetricRoutingService` (in `axioval-axiolid`) routes over declared
walkable surfaces (closed exact bodies with one horizontal floor), portals
(doors and bodiless openings with their voids) and vertical connectors. Every
other body obstructs where it enters the band between the profile's maximum
step and its clear height above a floor; an unmeasured body refuses every
route. Lengths are measured in plan.

A route stays on the origin's **level**: the surfaces reachable from it
through portals whose sill lies within a step of both floors, and through
shared boundaries whose floors differ by at most a step. End points must lie
on exactly one surface.

- `Reachable`: a path proposed through the level's free region, less an
  enclosure of its boundary's disc sweep, is accepted only once the body's
  sweep along
  it is proven inside the free region with exact booleans, and only if every
  door it crosses admits the body (a bodiless opening does; a door only by a
  stated clear width). Its length is the upper bound. The lower bound is the
  `axiolid-route` shortest path for a point through the free region, with
  every portal too narrow for the body cut at its mid-line; with incomplete
  evidence it is the straight line. The route kernel may accept an edge along
  collinear boundary edges across an outside gap, which only shortens its
  path, so it is used as a bound and never as a witness.
- `Blocked` with `CompleteMetricEvidence` when the destination is off the
  level, or when the level's free region, less a band around the mid-line of
  every portal whose free chord is shorter than the body, separates the two
  points. No body centre lies in that band: a centre at distance `t` from the
  mid-line covers a chord `2·sqrt(r² − t²)` of it. Blocked needs every
  declared surface and portal measured and no declared connector touching
  the level, since a connector could lead round through another level.
- A reachable route's evidence cites the source of its origin's object; a
  blocked verdict's completeness is set-level and cites the source the host
  gave the service.
- `MetricRoutingError::Unavailable` otherwise, with the lower bound in the
  reason. A gap narrower than the body between obstacles inside a room
  cannot be proven blocking with `axiolid-overlay` 0.3.0, whose erosion does
  not state its side; such a route is refused, not blocked, until one-sided
  erosion (`Region::erode_inner`, 0.3.1) is published. Routes across vertical
  connectors are not measured.

### Many targets in the Axiolid backend

Both queries build one `axiolid-route` distance map (axiolid/kernel#186)
over the level's free region from every target placed on the level at once.
A point is placed on one surface, or else inside the thickness of one portal
opening onto a surface, so an exit door is a target; the level then also
holds the corridor of every portal that opens from it onto nothing else of
it, such as a door to the outside. A target placed off a closed level is
unreachable; one the service cannot place, or placed off a level that is not
closed, counts only by its straight-line distance, which no route beats.

- **Nearest target.** The map's point distance is the lower bound. For a
  point body it is also the upper bound: the kernel's path stays in the
  closed free region (axiolid/kernel#187, #189: routes no longer run
  across gaps along collinear walls or squeeze between touching
  obstacles). For a body with a radius the upper bound is a proven sweep
  to one target, the map's nearest first, as for a single route; a body
  cannot stand in a doorway that leads outside, so such a target refuses.
  Unreachable needs a closed level, every target placed, and every placed
  target separated from the origin as `Blocked` requires.
- **Farthest point.** `axiolid-route`'s `farthest_point` brackets the
  largest map distance over the region's footprint, polygon by polygon,
  with the kernel's certified interval; convergence is reported as the
  interval's width against the tolerance. The upper bound holds whatever
  the level lacks, because missing free space only lengthens map routes.
  The lower bound is the bracket's only on a closed level with every
  target placed; otherwise it falls back to the witness's straight-line
  distance to the nearest target. A part of the region that reaches no
  target (`FarthestError::Unreachable`) is reported, with a point inside it,
  only on a closed level with every target placed, and refused otherwise.
  It is measured for a point only: where a body with a radius can stand
  needs one-sided erosion, so a positive radius is refused. Crossing
  barriers (`CrossingObstacles`) and a map over `axiolid-route`'s vertex
  budget refuse as well.

These need `axiolid-route` 0.3.2 (`distance_map`, `farthest_point`), not yet
published.

## Consumers

`space-distance` walks between spaces with `nearest_target`: from the
centroid of one space's footprint, on its floor, to those of its
destinations, refusing a space whose centroid lies outside its footprint.
Unreachable destinations are none; a refusal leaves the distance unknown.
See [Distances and connections between spaces](./capabilities.md#distances-and-connections-between-spaces).

`escape-route` measures travel to the nearest exit with `farthest_point`
from a space's walkable area, or with `nearest_target` from its doors.
Neither answer names the objects a walk crosses, so where metres on a stair
or a shared section count several times, the multiplied travel is bracketed
between the plain walk and its upper bound times the largest factor of a
section within that bound's reach in plan. See
[Escape routes](./capabilities.md#escape-routes).
