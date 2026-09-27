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

## Consumers

`space-distance` walks between spaces with this service: from the centroid
of one space's footprint, on its floor, to that of another, refusing a space
whose centroid lies outside its footprint. A `Blocked` route is no
destination; a refusal leaves the distance unknown. See
[Distances and connections between spaces](./capabilities.md#distances-and-connections-between-spaces).
