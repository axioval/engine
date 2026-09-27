# `axioval-axiolid`

Geometry evidence for any source, measured with the Axiolid kernel.

- `src/geometry.rs` holds `AxiolidGeometry`, the host-supplied mesh store shared
  by every service here. Doorway counts live here too: a mesh does not say which
  wall segments are openings, so the host declares them.
- `src/guard.rs` implements `GuardService`: barriers, landings and climbing
  aids around a walking surface's edge. Proximity is footprint-to-footprint,
  never vertex-to-vertex.
- `src/free_space.rs` implements `FreeSpaceService` for clearance and free
  area. `find_placement` refuses: its `NoPlacement` arm asserts an exhaustive
  search this adapter cannot perform. Clearance tests each obstacle's solid
  against the volume's prism shrunk by `CONTACT_TOLERANCE_M`: a triangle
  meeting it (band-clipped, then plan separating axes) or the centre inside
  (winding number) obstructs. Never go back to testing height range and
  plan outline separately: an L-shaped body satisfies both from different
  parts. Obstacles whose box reaches the prism must be closed, consistently
  and outward wound, or the request refuses.
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
- `src/plan_span.rs` also answers `measure_rectangle` with the overlay's exact calipers (`minimum_area_rectangle`, axiolid-overlay ≥ 0.3.2): a tessellated footprint is `Unproven`, several least-area orientations `Tied`; the kernel's `error` widens centre and half extents and turns the axes by `asin(e / (L − e))`, except along the coordinate axes, where the extremes are recomputed exactly. `src/plan_area.rs` measures `measure_outside_bands` as the overlay of the subject with band hulls clipped to their shared stretch, the clip moved by `CUT_MARGIN` inwards (sure) and outwards (possible); a tessellated band member refuses.
- `src/plan_span.rs` implements `PlanSpanService` over the plan-area service's footprints: longest diagonal and farthest span from convex-hull vertices (exact because distance is convex), centres from the overlay centroid. Tessellation widens a diagonal by `2d`, a farthest span by both deviations, a centre by `b·(R + d)/(A − b)`; a footprint no larger than its band has no bounded centre and refuses. A located centre is `Inside`/`Outside` only when farther from every footprint edge than its radius plus the chord deviation; otherwise `Undecided`.
- `src/derived_relationships.rs` implements `DerivedRelationshipService`:
  element to containing (or nearest) space, opening to the spaces a probe
  first enters on each side, space to larger covering spaces. Spaces and
  openings are host-declared `ObjectId`s; bodiless openings are probed through
  a host-supplied void. Undecided cases (unmeasured or bodiless spaces, points
  on a boundary, ties, tessellations within reach) refuse the whole answer.
- `src/vertical_extent.rs` implements `VerticalExtentService`: bottom and top elevations of a mesh's used positions. A tessellation widens each by its chord deviation and is never exact, even at zero deviation. Directional extents project the same positions; only a coordinate axis projects exactly, any other direction widens by the dot product's rounding bound and is approximate.
- `src/walking_surface.rs` implements `WalkingSurfaceService`: treads are upward level faces (corners within `LEVEL_TOLERANCE`, the rounding a placement leaves) of a closed, outward, exact, one-piece mesh; the walking direction comes from the treads' centres, never the placement; ramp runs are connected planar upward faces flatter than 45°. Tessellated subjects and nearby tessellated obstacles refuse, and headroom is never exact. Winders, open risers and warped runs refuse until plane detection (axiolid/kernel#131) and the #85 follow-ups land.
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
  Plans are `Plan`, never `Region`: overlay 0.3.0 rejects its own output as
  an operand (collinear non-adjacent edges count as self-intersection), so
  every operation cuts its operands into trapezoids first and validates
  with a tight tolerance. Test against the published 0.3.0, not a local
  kernel checkout.
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
  between collinear boundary edges.
- `src/planar.rs` (internal) holds the plan-projection helpers shared by the
  services; `src/geometry.rs` holds the mesh store and triangle vocabulary.
- `src/linear_quantity.rs` implements `LinearQuantityService`, measuring shelf
  running length from footprint, real ceiling height and doorways. It reports an
  upper bound, never an exact value, because the footprint bounds a
  non-rectangular room rather than describing it.
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
  `measure_distance` answers projections: horizontal distance folds 2D
  closest points over projected triangles (edge-on ones as segments), exact
  for non-convex footprints; vertical is the extent gap of footprint-related
  bodies (one-sided with a direction, whose side a tessellated end within the
  combined deviation leaves open); plan overlap uses the overlay. Overlap extents span witnessed
  intersection points (edge crossings both ways, inside vertices tried
  outermost first) below and the boxes' overlap above; the Hausdorff
  distance is bounded per triangle by the farthest vertex from one other
  triangle, split twice at most. Never report either as a point you did
  not bound, and never measure a volume without a certified boolean
  (axiolid/kernel#183). A tessellated overlap is asserted
  only from a witness deeper than the deviations and denied only beyond the
  combined deviation; otherwise it stays open. Never decide it on the mesh.
- **Every service honours fidelity.** Proximity reports approximate evidence.
  The services whose contracts only accept exact evidence -- contact,
  envelope, free space, guard, space -- refuse with their inexact-evidence
  error when a tessellation could change the answer: the subject itself, or a
  part whose enclosing extent (mesh box grown by its chord deviation) comes
  within the measurement's reach (`AxiolidGeometry::tessellated_near`). A
  curved part elsewhere in the model blocks nothing. Shelf length is an upper
  bound that only grows with the room, so it widens by the deviation instead.
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
0.3.0. `mesh_distance` is published there, but certified exact-B-rep distance
(`boundary_distance` / `boundary_clearance`) exists only on the kernel's main
branch. Check the registry source, not the kernel checkout, before relying on
an API.

## Waiting on upstream

- axiolid/kernel#173: `overlay`/`Region` snap output to an integer grid, so
  plan areas here are off by ~1.5e-8 of the extent while reported exact.
- axiolid/kernel#174: publish certified `boundary_distance`/`boundary_clearance`,
  so curved parts can get exact clearances instead of tessellated estimates.
- axiolid/kernel#163: one-sided disc morphology (`Region::erode_inner` and
  friends, axiolid-overlay 0.3.1, not yet published). With it, walkability can
  erode free regions by half the width and prove gaps inside surfaces
  blocking, and metric routing can report such routes blocked instead of
  refusing them.
- axiolid-overlay 0.3.0 validates operands with an area-scaled cross
  product against the linear tolerance and flags collinear non-adjacent
  edges anywhere on their lines; 0.3.1 checks the extent. Until it is
  published, `walkable.rs` re-cuts overlay output into trapezoids.
- `axiolid-route` 0.3.0 checks a visibility edge by proper crossings and its
  midpoint only, so an edge along two collinear boundary edges passes over
  the gap between them. Its path may leave the region; use it as a lower
  bound or a proposal, never as a witness or a disconnection proof.
- axiolid/kernel discussion #175: winding numbers are O(n) per query; the
  deepest-first ordering in `proximity.rs` hides it in practice but not in
  the worst case.
