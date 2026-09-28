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

## Walks around objects, and walks traced over objects

Two further default methods let a consumer establish what a walk crosses
without a backend naming "the" objects of a route, which would be one
shortest walk among possibly several.

- `NearestTargetRequest::with_avoided(objects)` asks for the nearest target
  by walks that keep out of the named objects: each is an obstacle wherever
  its body stands in the walking band, even a surface or portal. A backend
  answers such a request only if `avoids_objects()` says so; the default is
  `false`, and the handle then refuses the request (`Unavailable`) rather
  than let a backend answer the plain walk. The answer's lower bound bounds
  every walk avoiding the objects, so a lower bound beyond the plain walk's
  upper bound, or an unreachable verdict, proves that every shortest walk
  enters one of them, however many shortest walks there are.
- `trace_path(PathTraceRequest)`: how much of a polyline (a route's
  waypoints) lies over each requested object's plan footprint, in plan,
  whatever the heights. Each `PathTrace` length is an interval whose upper
  bound counts every part that may lie over the object, along its boundary
  included, and whose lower bound only parts surely inside; an object whose
  footprint is unknown answers why. The handle checks one answer per
  requested object (sorted, deduplicated) and none whose lower bound
  exceeds the polyline's own length. The default refuses.

## Weighted travel and forced walks

- `NearestTargetRequest::with_costs` and `FarthestPointRequest::with_costs`
  take `TravelCost`s: an object and a factor of at least one
  (`InvalidCostFactor` otherwise). A metre walked over the object's plan
  footprint counts `factor` metres; where footprints overlap the greatest
  factor counts, along an edge the cheaper side. The answer then brackets
  the least weighted cost, and a nearest answer's route is a walk whose
  weighted cost is at most its upper bound. The costs are sorted by
  object, each object's greatest factor kept, a factor of one dropped. A
  backend answers only if `weighs_travel()` says so; the default is
  `false`, and the handle refuses a weighted request rather than let a
  backend answer the plain length.
- `forced_walk(ForcedWalkRequest)` brackets the shortest walk from a point
  to the nearest of several targets that enters an object's plan footprint
  (touching counts), optionally keeping out of objects (`with_avoided`,
  asked only of a backend that avoids objects). `ForcedWalkEvidence` holds a
  finite lower bound and an upper bound that is infinite when no entering
  walk is known; `NeverEntered` says, under complete evidence, that no walk
  to a target enters the object. A lower bound beyond the upper bound of the
  plain walk proves that no shortest walk enters the object. The handle
  checks a claimed convergence against the tolerance and that a
  never-entered verdict names its request. The default refuses.

In the Axiolid backend (`axiolid-route` 0.3.4, axiolid/kernel#195 and
#196):

- **Weighted maps.** Each costed object's exact plan footprint, cut to the
  level's free region, becomes a cost region of a weighted distance map; the
  cut's vertices within a micrometre of an axis-parallel wall are moved onto
  it, since overlay output is rounded anew on each operation
  (axiolid/kernel#173) and a cost edge crossing a wall by nanometres is
  refused. A tessellated or bodiless costed object, footprints crossing
  each other or a wall at an angle, a body with a radius, and a request
  across levels are refused. Points along cost edges stand half the
  tolerance apart (5 mm for a nearest target), or as close as 1024 points
  over all cost edges allow; the bracket is first order in that spacing.
  Lower bounds come from the map only on a closed level, as for plain maps.
- **Forced walks.** One distance map out of the origin and one out of the
  targets, over the same free region (narrow portals cut as for nearest
  targets), bracket the forced walk into each polygon of the object's
  footprint (a tessellation's grown plan box, which only lowers the bound).
  It is answered only on a closed level with every target placed, else
  refused; its upper bound only for a point and an exact footprint.

## Routes across levels

`MetricRouteRequest`, `NearestTargetRequest` and `FarthestPointRequest`
take `with_connectors(ConnectorRouting)`: the typed vertical connectors
(`VerticalConnector`, as in walkability) a route may climb, and a
`ClimbLength` saying how a climb counts. A route enters and leaves a
connector at its **landings**, the two ends of its walking line, and counts
the climb between them:

- `StairLength::Slope`: `sqrt(h² + (f·v)²)`, the slope length for a vertical
  factor `f` of one;
- `StairLength::HorizontalPlusVertical`: `h + f·v`, the horizontal length
  plus the rise times the vertical factor.

`h` is the climb's plan length between the landings and `v` its rise;
`ClimbLength::length` bounds the result from both intervals' ends, rounded
outwards. A negative or non-finite factor is `InvalidClimb`, one object given
two kinds `ConflictingConnector`. A request carrying connectors climbs only
through them: any other connector is no way between levels for it. A
backend answers such a request only if `climbs_connectors()` says so; the
default is `false`, and the handle then refuses the request (`Unavailable`)
rather than let a backend answer a walk on one level. A connector whose
length or passability a backend cannot prove leaves every route through it
undecided: never shorter, never blocked.

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
  erosion (`Region::erode_inner`, 0.3.1) is published. A request without
  connectors stays on its level; one with them is measured as below.

### Many targets in the Axiolid backend

Both queries build one `axiolid-route` distance map (axiolid/kernel#186)
over the level's free region from every target placed on the level at once.
A point is placed on one surface, or else inside the thickness of one portal
opening onto a surface, so an exit door is a target; the level then also
holds the corridor of every portal that opens from it onto nothing else of
it, such as a door to the outside. A target placed off a closed level is
unreachable; one the service cannot place, or placed off a level that is not
closed, counts only by its straight-line distance, which no route beats.

- **Nearest target.** On a closed level the map's point distance is the
  lower bound; on a level that is not closed (a declared surface or portal
  unmeasured, a connector leaving it) the map may miss a shortcut, so the
  lower bound is the straight line to the nearest target. For a point
  body the map's distance is also the upper bound: the kernel's path stays in the
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

- **Avoided objects.** The backend avoids objects: each avoided object's
  body joins the obstacles, cut to the walking band as any obstacle is, so
  a surface or a door avoided closes the free region it would add. A
  tessellated or unmeasured avoided body refuses, as an obstacle does.
- **Traces.** A declared surface's footprint is its measured floor; any
  other body's is the union of its triangles projected to plan, and a
  tessellation's the plan box enclosing its true body grown by the margin
  (0.1 mm), which bounds only from above (lower bound zero). A bodiless
  object lies under nothing. Each segment is cut where it meets the
  footprint's boundary; a piece counts in the upper bound when its middle
  lies inside or within the margin of the boundary, in the lower bound when
  it lies inside and farther than the margin from it.

These need `axiolid-route` 0.3.2 or later (`distance_map`, `farthest_point`);
the workspace requires 0.3.4.

### Across levels in the Axiolid backend

A request with connectors climbs only through its own; the host's
`with_connector` declarations are no way for it and no longer leave a level
open. Each connector is measured from its body with the walking-surface
service's own measurements:

- a **stair** must be one exactly measured, straight flight whose first and
  last treads fill a rectangle: its walking line runs from the first
  nosing, at the base, to the back of the last tread, at the top, midway
  between each tread's sides;
- a **ramp** must be one exactly measured planar run filling a rectangle:
  its walking line runs along the run's direction from its bottom to its top
  end, midway between its sides;
- a **lift** is ridden, not walked: no walking length is measured for it.

The landings stand 1 mm plus the body's radius outside the line's ends,
along the walking direction, each on the one declared surface whose
footprint holds it and whose floor lies within a step of the end's
elevation. The climb's plan length is the distance between the landings and
its rise the flight's (base to top) or the run's. A body passes when the
narrowest tread or the run is at least as wide as the body and the
walking-surface headroom above the connector (against every other body but
the surfaces and portals) clears the profile's height: a connector surely
too narrow or too low is no way, one that may be either keeps its lower
bound and gives no upper bound. A connector that cannot be measured or
whose landings cannot be placed is not climbed, and every level it touches
(within 1 m, as before) is not closed.

The levels a walk may reach are the start's and, transitively, every level
a climbed connector lands on. On each, one `axiolid-route` distance map per
landing and one from the level's targets give the walks between the start,
the landings and the targets: a lower bound from the point map on a closed
level (infinite only where the free region, less the narrow portals' bands,
proves the two points apart), an upper bound from the point path, or from a
proven sweep for a body with a radius. These level walks and the climbs
form a small graph; its shortest path on lower bounds bounds every walk
through the connectors from below, and its shortest path on upper bounds is
a walk, whose waypoints stand each landing on its connector. When any
reached level is not closed or a target cannot be placed, the lower bound
falls back to the straight line to the nearest target (a climb is never
shorter than its plan length). `Unreachable` and `Blocked` need every
reached level closed, every target placed, and no walk that is not proven
apart.

The **farthest point** (for a point only) weighs each landing on the
region's level by the walk beyond it, bounded both ways by the graph.
`axiolid-route` 0.3.4 seeds a distance map with a starting distance per
target (axiolid/kernel#197), so the sources (the level's targets at no
weight, each landing at the walk beyond it) share one map: seeded with the
walks' lower bounds it brackets the largest walk from below (and proves a
part of the region unreachable), seeded with their upper bounds (a landing
without one left out) from above; where both agree one map does both. The
walk from every witness found to its nearest source plus its weight may
raise the lower bound. The bracket converges to the tolerance plus the gap
between the walks' own bounds beyond the landings, however many sources
the region's level holds; only if no weighted map can be built do the
per-source maps bound it from above, their farthest distance plus their
weight.

## Consumers

`space-distance` walks between spaces with `nearest_target`: from the
centroid of one space's footprint, on its floor, to those of its
destinations, refusing a space whose centroid lies outside its footprint.
Unreachable destinations are none; a refusal leaves the distance unknown.
See [Distances and connections between spaces](./capabilities.md#distances-and-connections-between-spaces).

`escape-route` builds its targets from the exits and, with compartments,
from the doors out of the start's compartment: each is a door standing in
its portal, a target like an exit door, so a compartment boundary needs no
target kind of its own. It measures travel to the nearest exit with `farthest_point`
from a space's walkable area, or with `nearest_target` from its doors.
Where metres on a stair or a shared section count several times, the
multiplied travel from a door is bounded from above by the answer's own
walk, traced over the sections; from the farthest point, whose answer is a
point and not a walk, by the plain upper bound times the largest factor of
a section within that bound's reach in plan. Where the backend weighs
travel, the weighted walk and the weighted farthest point bound it from
above over every possible section and from below over the sure ones. With
`walked_passages`, a passage is one every shortest walk from a door crosses
when the walk around it is longer than the plain walk, and off every
shortest walk when the walk forced through it is. See
[Escape routes](./capabilities.md#escape-routes).

Both take `stair_selector`, `ramp_selector` and `lift_selector`,
`stair_length` and `vertical_factor`, and send every query with the
selected connectors (`with_connectors`), so an escape route from an upper
storey is walked down its stairs and a walking distance reaches another
storey without `same_storey`. An escape walk that climbs is never traced:
its plan trace would undercount the climb.
