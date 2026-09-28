# Walkability topology

`WalkabilityService` converts a complete geometry scene into a source-neutral graph of derived walkable regions. Axiolid or another trusted geometry backend owns free-space extraction, obstacle subtraction, portal verification, and region decomposition.

The request supplies deterministic source-qualified sets of:

- walkable surface objects;
- entrance or portal objects;
- obstacle objects;
- the required clear width and optional elevation band;
- verified-portal and moving-envelope policy;
- typed vertical connectors (`VerticalConnector`: an object and its
  `VerticalConnectorKind`, `Lift`, `Ramp` or `Stair`), given with
  `WalkabilityRequest::with_connectors`;
- optionally, stated clear widths of entrances
  (`WalkabilityRequest::with_stated_clear_widths`): what the rule reads from
  its source as the width a door's leaf and lining leave. Each must be
  finite, positive and name a requested entrance once, or the request is
  refused (`InvalidStatedClearWidth`);
- optionally, swept doors (`WalkabilityRequest::with_swept_doors`): the
  doors whose leaves' swing sectors are obstacles (see
  [Door swings as obstacles](./free-space.md#door-swings-as-obstacles));
- optionally, an obstruction depth (`with_obstruction_depth`, metres):
  whatever an obstacle occupies within that distance of a surface's plan
  boundary does not obstruct it, so a skirting or a pipe along the wall
  leaves the width between the walls;
- optionally, a surface gap (`with_surface_gap`, metres): surfaces whose
  plans lie at most that far apart at overlapping heights are joined as if
  they touched, the gap walkable unless an obstacle fills it. Both
  tolerances default to zero and must be finite and non-negative
  (`InvalidTolerance`).

Semantic selectors run before this service. IFC placements, meshes, B-reps, and native kernel types never enter the request.

## Snapshot guarantees

A `WalkabilitySnapshot` is accepted only when it carries complete exact provenance. Every passage must have exact reviewable evidence, declared endpoints, and a conservative clear-width interval. Duplicate passages, unrequested object mappings, and portals outside the request policy are rejected.

Object-to-region membership lets reusable capabilities ask whether selected spaces, entrances, or components share traversable free space without exposing backend cells as model objects.

A passage inside one surface that no body of the request's width can pass
may carry a `WalkableStretch` (`VerifiedWalkablePassage::with_stretch`):
the surface, a position (plan coordinates and the floor's elevation, in the
model's coordinates), what limits it (`StretchLimit`: `Narrow` when the
surface's own shape does, `Obstructed` when obstacles standing on the floor
or swung doors do, `Low` when obstacles hanging in the headroom band do),
the obstacles found there that the limit depends on, and for a low stretch
the headroom they leave. The snapshot accepts a stretch only on a passage
that names no portal or connector, whose upper width bound lies below the
request's width, on a requested surface, relating requested obstacles or
swept doors (`InvalidStretch`). The position locates the stretch for a
reviewer; it is never a measurement.

## Three-valued routes

For the request minimum width, the engine evaluates two deterministic graphs:

1. **definite graph:** passage lower bound meets the width;
2. **possible graph:** passage upper bound meets the width.

A passage that climbs through a connector names it
(`VerifiedWalkablePassage::with_connector`); a passage names a portal or a
connector, never both, and the snapshot accepts a connector passage only for a
requested connector of the same kind. `route_between_avoiding` evaluates both
graphs without the passages of the forbidden kinds, so a rule that forbids
`Stair` sees a stairs-only connection as `Unreachable`.

A route in the definite graph is `Reachable`. No route in the possible graph is `Unreachable`. A route only in the possible graph is `Indeterminate`. Approximate width evidence therefore cannot become a pass or a false negative.

A rule can judge passages itself on top of their widths with
`route_between_admitting`: a `PassageAdmission` per passage, `Admitted`,
`Undecided` or `Refused`. The definite graph keeps only admitted passages and
the possible graph drops only refused ones, so a passage the rule cannot
decide (a door whose required width it cannot read) can only make a route
`Indeterminate`. `route_between_avoiding` is the admission that refuses the
forbidden connector kinds.

When a route is `Unreachable`, `blocking_passages` names what blocks it: the
passages leaving the regions the possible graph reaches from the origin
that lead, widths and admission ignored, towards the destination without
re-entering those regions. Every route crosses one of them after it last
leaves the reached regions, so they form a cut; a block that only guards
some other region is left out. The list is empty when nothing joins the
two at all.

## The Axiolid backend

`AxiolidWalkabilityService` (in `axioval-axiolid`) implements this contract
over host-supplied meshes. Surfaces, entrances, obstacles and connectors are
the request's; the host declares only what a mesh cannot show: the void of a
bodiless opening and, optionally, a door's clear width. A clear width the
request states counts as one the host states; where both do, the narrower
counts, and a stated width bounds even an opening's void from above.

- **Surfaces.** Each selected surface must be an exact closed body whose
  underside is one horizontal floor. Its free region is its plan footprint
  minus, for every obstacle, the plan projection of the part of the obstacle
  inside the headroom band: the request's band measured from the floor, or
  the surface's own height without one. An obstacle touching the band only at
  its ends does not obstruct, so a lintel at the band's top leaves the door
  open.
- **Portals.** A portal's corridor is its plan extent along the wall,
  extruded through its thickness until the first selected surface on each
  side (within 1 m), minus the obstacles in the band above its own sill. The
  portal's own body is open: its leaf and lining are not obstacles. A selected surface or entrance never obstructs, even when the
  obstacle selection also picks it.
- **Regions.** `surface:{id}` per surface and `portal:{id}:-`,
  `portal:{id}:+` per entrance, mapped to their objects. Each surface stands
  for a hub where the body provably fits; each portal face for the landing
  in front of it.
- **Passages.** A *spoke* joins a surface to the portal face opening onto
  it. A *crossing* joins a portal's two faces and names it; its width is at
  most the longest free interval of the portal's mid-line, since a crossing
  body covers a chord of its own diameter there, and at most a stated clear
  width. Surfaces and portal faces that touch in plan at overlapping heights
  are joined without a width bound, and a connector joins every pair of
  surfaces whose floor lies within 1 m of its height range and plan.
- **Climbs and rides.** A stair or ramp is measured as for metric routing
  (see [Across levels in the Axiolid backend](./metric-routing.md#across-levels-in-the-axiolid-backend)):
  an exact straight flight or one-run ramp whose ends fill a rectangle,
  its landings half the width plus 1 mm outside its walking line's ends,
  each on the one joined surface whose footprint holds it and whose floor
  lies at the end's elevation (within 1 µm). The passage between those two
  surfaces is bounded above by the flight's or run's narrowest width, and
  from below by it when the width admits the body, the walking-surface
  headroom above it against the request's obstacles clears the band's top
  (the lower surface's own height without a band), and the sweeps from the
  lower hub to the lower landing and from the upper landing to the upper
  hub are proven. Headroom surely too low bounds it at zero. A lift rides
  between two joined surfaces within its height range when the body at
  the first one's hub lies inside the lift's exact plan and a sweep from
  that point to the second one's hub is proven on it: a definite passage
  of the request width. Every other connector passage has no lower bound.
- **Definite passages.** A passage's lower bound is the request width only
  when the sweep of the body along a witness path (hub to landing, or
  landing to mid-line to landing) is proven inside the free region, and, for
  a crossing, the leaf and lining admit the body: a bodiless opening does, a
  door only through a stated clear width. Otherwise the lower bound is zero.
  Every definite passage ends where the next begins, so a definite route
  concatenates proven sweeps.
- **Proof without one-sided erosion.** `axiolid-overlay` 0.3.0 erodes by a
  round-joined approximation of a disc without stating on which side of the
  exact erosion it lies, so the backend does not use it. Paths are proposed
  by `axiolid-route` through the free region less an enclosure of its
  boundary's disc sweep, which lies inside the exact erosion, and only
  proposed. A path is a witness once an outer
  polygon enclosing its exact sweep (segment rectangles, polygons
  circumscribing the discs at its vertices, all grown by 0.1 mm) leaves
  nothing outside the free region under exact booleans.
- **Evidence.** Every passage cites the source of the object it measures (the
  surface of a spoke or surface pair, the portal of a crossing or portal
  face, the connector of a climb), so in a set federating several files it
  names that object's own file. The snapshot's completeness is set-level and
  cites the source the host gave the service.
- **Door swings.** The request's swept doors (`with_swept_doors`, see
  [Door swings as obstacles](./free-space.md#door-swings-as-obstacles)) are
  obstacles on every surface a sector stands on. Hubs stand clear of every
  swing's circumscribed polygon; a portal's landings, spokes and crossing
  are proven clear of every swing but the portal's own, which the body
  walks through. Mid-line width bounds ignore swings, so they stay upper
  bounds.
- **Pieces.** A surface that touches no other selected surface and that no
  connector joins is split into its possible pieces: its footprint eroded
  from outside (`Region::erode_outer`) by half the width less 0.1 mm, less
  the obstacles' band footprint and every swing's inscribed polygon grown
  from inside by as much. Every centre of the body clear of walls,
  obstacles and swings lies in one piece with a 0.1 mm disc around it.
  `surface:{id}` is the piece holding the hub (the first when there is no
  hub) and `surface:{id}#{k}` every other one, all mapped to the surface. A
  body leaves the surface only through a portal's half on that side or the
  portal's own swing, so a portal face is joined to each piece within half
  the width (and 0.1 mm) of them, grown from outside, and to the hub's piece
  always, with no width (`:separated` in its locator) when it is not within
  reach. Two faces whose such zones meet are joined without a width bound,
  since a body may pass between them without standing wholly on the
  surface. A surface touching another, or joined by a connector, stays one
  region.
- **Stretches.** Two pieces lying in one polygon of the surface's footprint
  are joined by floor too narrow for the body. Each such pair is joined by
  a passage with an upper width bound of the width less 0.2 mm (the pieces
  are separated at half the width less 0.1 mm) and a stretch: where the
  footprint that bodies centred in neither piece cover (each piece grown
  from outside by half the width) comes next to both pieces, or, failing
  that, where the pieces come closest. The limit is `Low` when the pieces
  merge once the obstacles hanging above the floor are left out, else
  `Obstructed` when they merge without any obstacle or swing, else
  `Narrow`; the stretch relates the obstacles (and, when obstructed, the
  swung doors) of that kind meeting its area, and a low one the headroom
  under the lowest of them. The hub's piece is then joined to a portal face
  only through the stretches, not by a passage of width zero. A face whose
  zone reaches no piece at all keeps its passage of width zero to the hub's
  piece, with a stretch in front of the face classified the same way
  (`:separated:<limit>` in its locator).
- **Obstruction depth.** An obstacle's band footprint counts on a surface
  only beyond the depth from the footprint's boundary. Where that lies is
  bracketed by the one-sided erosions of the footprint: witnesses are
  proven against what lies beyond `erode_outer` (tolerating less than the
  exact depth), while mid-line width bounds and possible pieces use what
  lies beyond `erode_inner` (tolerating more). Portal corridors are not
  tolerated.
- **Surface gap.** Surfaces within the gap of each other at overlapping
  heights are joined as touching surfaces are, and stay whole. The points
  within the gap of both footprints (`dilate_inner` of each, intersected)
  are floor for the witness between their hubs, less what any obstacle
  occupies there (obstacles are gathered over each floor grown by the gap,
  nothing tolerated) and every circumscribed swing; the passage's locator
  says `gap<=`.
- **Refusals.** `WalkabilityError::Unavailable` for moving envelopes (use
  swept doors instead), unmeasured or undescribed obstacles, tessellated
  surfaces, portals or obstacles inside a band, surfaces that are not closed
  or not flat underneath, portals without a single through-direction and
  portals opening onto two surfaces at the same distance.

Verdicts available today: `Reachable` wherever a sweep is proven;
`Unreachable` where every connection needs a portal narrower than the width
(geometrically or by a stated clear width), a forbidden connector kind, or
nothing joins the surfaces at all; `Indeterminate` otherwise, notably for a
door whose clear width is not stated, a crossing whose landing is obstructed,
and a route through a connector that is not measured as above. A gap narrower than the width inside a
surface that touches no other surface splits it into pieces, so a route
needing that gap is `Unreachable`, its cut the located stretch between the
pieces (or, in front of a face where the body fits nowhere, the stretch
there); inside surfaces that touch, or that the surface gap joins, such a
gap is still not detected and can only make a route `Indeterminate`.

The `accessible-route` capability (see [capabilities](./capabilities.md#accessible-route)) is built on this contract: one snapshot for its mobility profile, its own admission per passage, and the blocking passages as the related elements of a finding.

Corridor metric lengths remain owned by the separate metric-routing service. End-clearance placement remains owned by the free-space placement service.
