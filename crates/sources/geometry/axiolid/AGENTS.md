# `axioval-axiolid`

Geometry evidence for any source, measured with the Axiolid kernel.

- `src/geometry.rs` holds `AxiolidGeometry`, the host-supplied mesh store shared
  by every service here.
- `src/guard.rs` implements `GuardService`: barriers, landings and climbing
  aids around a walking surface's edge. Proximity is footprint-to-footprint,
  never vertex-to-vertex.
- `src/free_space.rs` implements `FreeSpaceService` for clearance, placement
  and free area. `find_placement` builds the scene (scope footprint, and each
  obstacle's `walkable::band_footprint` in the open band from the scope's
  floor up by the shape's height, never its whole projection) and hands it to
  `src/placement.rs`: the configuration space by Minkowski erosion and sum
  with the convex rectangle, one-sided disc morphology for circles, angle
  interval search for `Any`. Witnesses are re-verified by direct overlap;
  `NoPlacement` needs the shape shrunk by `KNIFE_EDGE_METRES` (plus the
  rotation margin) to fit nowhere, so a fit by contact refuses. Never drop a
  margin or return `NoPlacement` from an undecided interval. Only the scope's
  own floor is searched (merged scopes must share it, and then no support
  is answered); other supports refuse. Obstacles are clipped to the
  request's `effective_band`. A frame-offset domain is a `Window`: witnesses
  from the box as computed and filtered by the contract's `contains_frame`,
  absence proven against the box grown by `KNIFE_EDGE_METRES`; never prove
  absence against the ungrown box. Witness candidates include slab
  midpoints, so a free region with a hole always offers interior points.
  Placement evidence cites the scope's source. Clearance tests each obstacle's solid
  against the volume's prism shrunk by `CONTACT_TOLERANCE_M`: a triangle
  meeting it (band-clipped, then plan separating axes) or the centre inside
  (winding number) obstructs. Never go back to testing height range and
  plan outline separately: an L-shaped body satisfies both from different
  parts. Obstacles whose box reaches the prism must be closed, consistently
  and outward wound, or the request refuses.
- `src/circulation.rs` answers `map_circulation` for the free-space
  service: the placement scene's free area, pieces from `erode_inner` less
  `dilate_outer` (proof of connection), possible pieces from `erode_outer`
  less `dilate_inner` for half the width less `KNIFE_EDGE_METRES` (proof of
  separation; never drop the margin, or a gap exactly as wide as the path
  snaps shut and reads as blocked), contacts by footprints grown with the
  matching one-sided dilation, and `axiolid_route::skeleton` per piece.
  The skeleton is called directly (axiolid-triangulate 0.3.1 carries the
  fix for axiolid/kernel#190); a piece whose skeleton the kernel refuses is
  unmapped, never guessed. Half widths are distances to the free area widened by the grid
  snapping. Door swings are not subtracted.
- `src/containment.rs` answers `assess_containment` for the free-space
  service: the overlay difference of the footprint's bounds (the exact
  rectangle, or a cylinder's inscribed and circumscribed 64-gons) less the
  scopes' projected triangles, so the part outside is measured directly
  rather than as a small difference of two large areas, which rounding could
  push past the area epsilon for a box flush with a wall. A scope without a
  mesh or tessellated refuses.
- `src/space.rs` implements `SpaceService`: seven independent space
  measurements over storey-assigned, role-tagged geometry.
- `src/envelope_membership.rs` derives envelope membership: an object bounds
  the envelope when its plan footprint meets a bounding space. The bounding
  spaces are the request's (the rule's selection), never host-declared; a
  bounding object without a mesh refuses the derivation. Declarations are
  plain `ObjectId` data, so this stays a geometry adapter.
- `src/plan_area.rs` implements `PlanAreaService`: footprints and footprint overlaps from the plan overlay. A tessellated mesh widens the area by `2·P·d + π·d²`; never report it as a point. A declared group (`with_group`) measures the union of its members; a member without a body, an unmeasured member or undecided membership refuses, never zero. An uncovered area brackets the grown cover between an inscribed and a circumscribed 16-gon (growth by `r − d` and `r + d` for a tessellated cover), handed to the overlay as fan triangles: its ring check refuses a vertex on the line through a non-adjacent edge. Never use the 0.3.0 overlay's `Region::dilate`, whose side of the true offset is unstated.
- `src/plan_span.rs` also answers `measure_rectangle` with the overlay's exact calipers (`minimum_area_rectangle`, axiolid-overlay ≥ 0.3.2): a tessellated footprint is `Unproven`, several least-area orientations `Tied`; the kernel's `error` widens centre and half extents and turns the axes by `asin(e / (L − e))`, except along the coordinate axes, where the extremes are recomputed exactly. That computation is `least_area_rectangle`, the crate's one rectangle: sections and the shelf layout call it too, never `minimum_area_rectangle` directly. `src/plan_area.rs` measures `measure_outside_bands` as the overlay of the subject with band hulls clipped to their shared stretch, the clip moved by `CUT_MARGIN` inwards (sure) and outwards (possible); a tessellated band member refuses.
- `src/coverage.rs` answers `measure_coverage` for the plan-area service: each effect bracketed between inner and outer regions (`Region::dilate_inner`/`dilate_outer` for grown effects; the exact visibility polygon cut to inscribed and circumscribed 64-gons; convex cells of the free region judged by the distance map with the 1-Lipschitz bound `D(g) ± ρ`, split up to `MAX_DEPTH`, undecided cells outer only; the reached cells are united by `union_soup` into a region). Effects are `Region`s, united and clipped with region operations; the trapezoids only cut the free region into convex cells. Tessellated subjects and blockers refuse; a tessellated, unmeasured or centreless source is an unmeasured effect, never an empty one.
- `src/sight.rs` implements `SightService` with the kernel's `line_of_sight`. Blockers apart from the box of the eye and the target are not handed over; tessellated targets in range and tessellated blockers that may matter refuse; an unmeasured blocker refuses. Never merge blockers into one mesh to decide more: occluders must stay attributable.
- `src/plan_span.rs` implements `PlanSpanService` over the plan-area service's footprints: longest diagonal and farthest span from convex-hull vertices (exact because distance is convex), centres from the overlay centroid. Tessellation widens a diagonal by `2d`, a farthest span by both deviations, a centre by `b·(R + d)/(A − b)`; a footprint no larger than its band has no bounded centre and refuses. A located centre is `Inside`/`Outside` only when farther from every footprint edge than its radius plus the chord deviation; otherwise `Undecided`. Recesses are the pockets between the outer ring and its convex hull, walked between consecutive ring vertices on the hull's boundary (vertices on a hull edge count, or a niche's mouth would be the whole wall); pockets under `RECESS_RESOLUTION` are grid rounding. Sections intersect the footprints and take their sides from `least_area_rectangle` (`Enclosing::sides`), which answers only for a unique orientation: a tied section refuses. Both refuse tessellated footprints: never bound a least-area rectangle's short side by a chord band, it is not monotone.
- `src/corridor_end.rs` (internal) answers `measure_corridor_ends` for the plan-span service with `axiolid_route::skeleton` (prune 1.5; spacing refined below an eighth of the narrowest width found, at most 50 000 samples). Collinear vertices are dropped first so a straight wall is one edge. A wall is `Decided` only when the end lies within its clearance plus two spacings of the kernel's wall ahead and rays within `SPREAD` (20°) of the path's direction all meet that wall first; never widen `SPREAD` or the slack to decide more walls without a proof. A tessellated space refuses (its edges are chords); a tessellated subject widens gap by `d` and facing by `2d`. Evidence for the ends is always approximate.
- `src/derived_relationships.rs` implements `DerivedRelationshipService`:
  element to containing (or nearest) space, opening to the spaces a probe
  first enters on each side, space to larger covering spaces. Spaces and
  openings are host-declared `ObjectId`s; bodiless openings are probed through
  a host-supplied void. Undecided cases (unmeasured or bodiless spaces, points
  on a boundary, ties, tessellations within reach) refuse the whole answer.
- `src/vertical_extent.rs` implements `VerticalExtentService`: bottom and top elevations of a mesh's used positions. A tessellation widens each by its chord deviation and is never exact, even at zero deviation. Directional extents project the same positions; only a coordinate axis projects exactly, any other direction widens by the dot product's rounding bound and is approximate.
- `src/walking_surface.rs` implements `WalkingSurfaceService`: ramp runs are connected planar upward faces flatter than 45° of an exact mesh; tessellated ramps, headroom subjects, landing ends and nearby tessellated obstacles refuse, and headroom is never exact. Sides and landing extents come only from `rectangle`: boundary edges (matched by exact coordinates) on the spanned rectangle's sides and the plan area equal to it, within a `slack` only for a tessellated tread (twice its chord deviation, and every side widened by it); never report a bounding box as a width. A landing is one connected level surface of one candidate (or a ramp's own, never a flight's treads) meeting the end's strip within `LANDING_REACH`; several are refused, never merged. A turning flight's landings and handrails refuse (`straight_direction`): its positions are arc lengths and its sides lie across each tread's own direction, so no single frame holds them. The clearance below leaves out the plan regions where the subject lies flush on the space's floor (convex `subtract`), never the whole face pair. Handrails (`src/walking_surface/handrail.rs`) must fill one rectangle along the walking direction with their upward faces, or the request refuses; heights come from the upper envelope of the rail's edge lines between breakpoints (ends and pairwise crossings), never from sampling. Tessellated rails are measured and widened by their chord deviation; lines that do not rise are evaluated without rounding, so a level extension rises by exactly zero. Warped runs refuse.
- `src/flight.rs` (internal) measures flights of a closed, outward, one-piece mesh, exact or tessellated, from `axiolid_inspect::detect_planes`: treads are upward planes whose certified deviation and corner spread stay within `LEVEL_TOLERANCE` of the coordinates plus twice the chord deviation, and every position of a tessellated flight widens by the chord deviation. Walking directions and lines come from the treads, never the placement. A turning flight's positions are floating-point constructions widened by `MARGIN` over the crossing's sine, so never exact; grazing crossings refuse. A tread's sides come from `rectangle` along its walking direction (the flight's, or square to its nosing on a turning flight); a winder tapers and fills none, so it has no sides and its flight no width. Riser closure: `Open` needs a face falling away from the lower tread's back line, `Closed` a rising face along all of it; the first riser is `Closed` only when coplanar faces cover the strip below the nosing to the base, otherwise `NotMeasured`, never `Open` (a set-back riser looks the same). Test fixtures build closed meshes with `stepped` (height field over plan cells) and `voxels` in `tests/walking_surface.rs`.
- `src/boundary_coverage.rs` implements `BoundaryCoverageService` over
  host-registered boundary surfaces keyed by space and boundary. Body faces
  group by oriented plane; a boundary triangle counts on the nearest face
  plane its corners all lie within the tolerance of, and a boundary with a
  triangle on none is `OffSurface`, reported and covering nothing. Per plane,
  faces and boundary parts are overlay `Region`s (union, intersection,
  difference; overlay 0.3.3 accepts its own outputs): uncovered is the faces
  less the boundaries' union, measured directly, covered the rest of the
  faces, overlap accumulated boundary by boundary. Only an axis plane all
  face points share projects exactly; others widen by rounding plus the
  overlay's grid snapping (`OVERLAY_SNAP`, #173), tessellated boundaries by
  their deviation, both as `2·P·w + π·w²` bands. Never drop those margins:
  the turned-box test fails without them. Curved, unmeasured or bodiless
  spaces and unmeasured boundaries refuse.
- `src/triangle_count.rs` implements `TriangleCountService`: the triangles of the registered mesh. Exact evidence only for a planar mesh; bodiless counts zero, unmeasured refuses.
- `src/facade_area.rs` implements `FacadeAreaService`: steep faces that look outside, classified at four samples each. A face held against another body or a host-declared space, or whose ray first meets itself or a space within `REACH`, is not facade; meeting nothing or another body is. A face whose samples disagree widens the interval, never a guess. Unmeasured bodies anywhere, bodiless declared spaces and tessellations within reach refuse.
- `src/walkable.rs` (internal) builds walkable plan domains for the two route
  services: floors (closed exact bodies with one horizontal underside), band
  footprints (what a body occupies inside an open headroom band: the boundary
  clipped to the band plus the winding section just above its bottom), portal
  frames, sides and corridors, mid-line chord bounds, and sweep proofs.
  **Never use `Region::erode` (0.3.0)**: it does not state its side. The
  module's own `erode` removes an enclosure of the boundary's disc sweep and
  only proposes paths; `sweep_inside` proves them with an outer enclosure
  grown by `MARGIN`, which must stay above the overlay's grid snapping.
  Width upper bounds come only from chords of a portal's mid-line.
  Plans are `Plan`: overlay output, handed back to the overlay as it is
  (settled output is a valid operand from axiolid-overlay 0.3.3) and
  validated and settled with a tight tolerance, since thin slivers fail at
  `ON_SURFACE`. `trapezoids` remains for convex pieces (an inner point,
  placement obstacles as rings). Test against the published crates, not a
  local kernel checkout.
- `src/walkability.rs` implements `WalkabilityService`: a region per surface
  (a hub) and per portal face (a landing). Every definite passage must end
  where the next begins, or a definite route could transit a surface without
  a proven sweep; never add a hub-like region whose passages are definite
  without a sweep from a shared point. A door's leaf and lining are unknown,
  so a crossing is definite only for a bodiless opening or a stated clear
  width (`with_clear_width`, or the request's
  `with_stated_clear_widths`; the narrower of the two counts, and a stated
  width bounds a void's crossing from above). Connector passages are never
  definite.
- `src/metric_routing.rs` implements `MetricRoutingService` on the origin's
  level. `Blocked` needs complete evidence (every declared surface and portal
  measured, no connector touching the level) and a separation of the free
  region less the chord-proven bands around narrow portals' mid-lines. The
  `axiolid-route` path is a lower bound only: it can cut across outside gaps
  between collinear boundary edges. `nearest_target` and `farthest_point`
  build one `distance_map` over the level, which then also holds the
  corridors of portals opening from it onto nothing else of it (exits to
  the outside); a target may stand in a portal. A target the service cannot
  place counts only by its straight line; a lower bound comes from the map
  only on a closed level with every target placed, and so does an
  unreachable verdict. Farthest points are for a point body only; never
  report the bracket for a body with a radius.
- `src/planar.rs` (internal) holds the plan-projection helpers shared by the
  services; `src/geometry.rs` holds the mesh store and triangle vocabulary.
- `src/linear_quantity.rs` implements `LinearQuantityService`: parallel shelf
  bands on the footprint less each requested door's clearance (its footprint
  grown by clearance plus plan gap, circumscribed for the lower bound, inscribed
  for the upper), along the footprint's and every door's least-area
  rectangle (`least_area_rectangle`; any orientation, tied or unproven, only
  proposes a layout), anchored at either wall; the longest layout counts. Doors are the
  request's (the rule's selection), never host-declared; a bodiless opening
  needs its void. Bands are measured exactly between breakpoints; keep the
  lower bound's grown cross-sections and shrunk runs, and the upper bound's
  reverse, or the interval stops holding the true length. Keep the 1 mm
  `CONSTRUCTION_TOLERANCE` above twice the margin: without it a band against a
  wall starts on the boundary and the lower bound loses every wall band.
- `src/contact.rs` implements the engine's `ContactService` over `axiolid-mesh`,
  `axiolid-measure` and `axiolid-overlay`. Hosts register a `TriMesh` per
  `ObjectId`; no IFC types appear anywhere in this crate.
- **Contact area is measured in plan**, by intersecting the projected triangle
  soups (`axiolid-overlay`). Do not go back to per-triangle counting: a triangle
  spanning the whole face counts entirely as soon as any part of it approaches,
  which reported a half-supported slab as fully supported.
- **Nearest separation is a 3D question** and stays on `closest_points_on_triangles`;
  plan overlap alone would count anything passing overhead as contact.
- The area clamp against the subject's own face area is load-bearing: independent
  counterparts are measured independently and can double-count a shared region.
- `src/proximity.rs` implements `ProximityService` for clash and distance
  checks: separation from `closest_points_on_triangles`, plan overlap from
  the oriented footprint overlay in `planar.rs`, and penetration witnessed by
  sampling points (including midpoints between an edge's crossings of the
  other surface, via `axiolid-ray-mesh`) against winding numbers. Only closed
  two-manifold meshes have an inside; a pair with one is measured, two open
  surfaces report no penetration.
  Fidelity comes from `AxiolidGeometry::with_tessellated_mesh`. A
  host-declared bodiless object is refused with `ProximityError::NoBody`,
  never `Unavailable`, so a comparison can tell "no body" from "unmeasured".
  `measure_region_distance` folds the same 2D closest points between a
  stated region's fan triangles and the counterpart's footprint, widened by
  the counterpart's chord deviation.
  `measure_distance` answers projections: horizontal distance folds 2D
  closest points over projected triangles (edge-on ones as segments), exact
  for non-convex footprints; vertical is the extent gap of footprint-related
  bodies (one-sided with a direction, whose side a tessellated end within the
  combined deviation leaves open); plan overlap uses the overlay. Overlap extents span witnessed
  intersection points (edge crossings both ways, inside vertices tried
  outermost first) below and the boxes' overlap above; the Hausdorff
  distance is bounded per triangle by the farthest vertex from one other
  triangle, split twice at most. Never report either as a point you did
  not bound. Intersection volumes come only from `axiolid-inspect`'s
  certified integrals (`intersection_volume`, `enclosed_volume`), for two
  closed solids; a kernel refusal leaves the volume `None`, never zero, and
  a tessellation widens it by `tube_volume`, never by nothing. A tessellated overlap is asserted
  only from a witness deeper than the deviations and denied only beyond the
  combined deviation; otherwise it stays open. Never decide it on the mesh.
- `src/face_distance.rs` answers `measure_face_distance` for the proximity
  service. Face classes come from outward normals (outward by the host's
  signed volume); a triangle within `CLASS_TOLERANCE` of 45° refuses the
  classes it might belong to. Only a body proven wholly inside (every vertex
  inside, surface clear of the host's) gets the exact triangle distance;
  otherwise the interval's lower bound spans the outside part (vertices not
  proven inside, surface crossings, nearby host vertices) face by face. The
  host must be an exact closed solid; a tessellated body widens both bounds
  and decides a vertex's side only beyond its deviation.
- **Every service honours fidelity.** Proximity reports approximate evidence.
  The services whose contracts only accept exact evidence -- contact,
  envelope, free space, guard, space -- refuse with their inexact-evidence
  error when a tessellation could change the answer: the subject itself, or a
  part whose enclosing extent (mesh box grown by its chord deviation) comes
  within the measurement's reach (`AxiolidGeometry::tessellated_near`). A
  curved part elsewhere in the model blocks nothing. Shelf length is an
  interval, so a curved space widens it by its chord deviation instead.
  `tests/fidelity.rs` pins each case and fails without the guards.
- Proximity queries go through an `axiolid-spatial` BVH per body; skips are
  exact (box gap never exceeds triangle gap), and the unit tests compare the
  indexed results with an exhaustive scan. Penetration ranks samples by
  indexed surface distance and runs the O(n) winding test deepest first,
  stopping at the first inside point. Keep that order: it is what makes a
  19k-triangle pair take 0.26 s instead of 91 s.
- `projected_polygons` winds every projected triangle counter-clockwise. A
  closed solid's top and bottom faces project with opposite windings and,
  under the non-zero fill every service uses, cancel to no footprint at all.
  Test fixtures with same-winding caps hid this; use closed, outward-oriented
  boxes when testing plan measurements.
- **Bodiless and unmeasured are different facts.** `with_no_body` says the
  object occupies no volume; `with_unmeasured` says it has a body the host
  could not mesh. Never let a service skip an unmeasured object as if it were
  bodiless: its extent is unknown, so a measurement it could affect refuses.
  Contact measures only the request's candidates and refuses while one of
  them is unmeasured or undescribed; space refuses while a declared role,
  storey member or requested cap element is; free space refuses it as an
  obstacle.
- A geometry set may hold several sources. Evidence about one object (contact,
  proximity, facade area, vertical and directional extent, triangle count, shelf
  length, clear height, derived relationships, a walkability passage, a
  reachable metric route's origin, a tread flight, sloped runs and the headroom
  above a subject) cites that object's source; only
  set-level measurements (a walkability snapshot, a blocked route's
  completeness) take the source given to their constructor.
- `src/lib.rs` keeps the source-scoping contracts and the in-memory conformance
  double. `UnavailableGeometryBackend` remains the explicit "no kernel linked"
  placeholder.
- This crate must remain independent of `axioval-ifc`; proprietary CAD sources are
  first-class inputs.
- Run `cargo test -p axioval-axiolid` plus strict clippy. `tests/contact.rs`
  measures real meshes through the published kernel, not a stub;
  `tests/conformance.rs` protects source scoping.
- Mutation-check decision logic with `scratch/mutate_contact.py` before changing
  how contact is measured.

## Pitfall

Depend only on what the registry publishes. The workspace pins `axiolid-*`
0.3.0, except `axiolid-overlay` 0.3.3 (`minimum_area_rectangle`, the
Minkowski and dilation family, settled `union_soup` output), `axiolid-route`
0.3.3 (`distance_map`, `farthest_point`, and `skeleton` behind circulation
maps and corridor ends, with `axiolid-triangulate` 0.3.1) and
`axiolid-inspect` 0.3.2 (volumes, `line_of_sight`, `detect_planes`). `mesh_distance` is published there, but certified exact-B-rep distance
(`boundary_distance` / `boundary_clearance`) exists only on the kernel's main
branch. Check the registry source, not the kernel checkout, before relying on
an API.

## Waiting on upstream

- axiolid/kernel#173: `overlay`/`Region` snap output to an integer grid, so
  plan areas here are off by ~1.5e-8 of the extent while reported exact.
- axiolid/kernel#174: publish certified `boundary_distance`/`boundary_clearance`,
  so curved parts can get exact clearances instead of tessellated estimates.
- One-sided disc morphology (`Region::erode_inner` and friends, #163) is
  published in axiolid-overlay 0.3.2, but walkability does not use it yet.
  With it, walkability could erode free regions by half the width and prove
  gaps inside surfaces blocking, and metric routing could report such routes
  blocked instead of refusing them.
- The single-route service still treats the kernel path as a lower bound or
  proposal only; the many-target queries of `axiolid-route` 0.3.2 use a point
  path as a witness only.
- `src/circulation.rs` builds on `axiolid_route::skeleton` (axiolid/kernel#139,
  axiolid-route 0.3.3) and needs axiolid-triangulate 0.3.1 with the fix for
  axiolid/kernel#190; it calls the skeleton directly, without a worker,
  timeout or retry spacings.
- axiolid/kernel discussion #175: winding numbers are O(n) per query; the
  deepest-first ordering in `proximity.rs` hides it in practice but not in
  the worst case.
