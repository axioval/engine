# Clash, interference and distance

Clash and distance checks are split along the ADR 0004 seam. The engine owns
the contracts and the broad-phase candidate search. A geometry adapter measures
pairs. The capabilities in `axioval-rules` decide what a measurement means
against declared tolerances.

## Broad phase: candidate search

`candidate_pairs(subjects, counterparts, margin_metres)` returns every
subject/counterpart pair whose boxes lie within the margin, in identity order.
It uses sweep-and-prune along x, and its cost is near-linear in the object
count plus the number of pairs it reports.

The search is **complete**, so a pair it does not report is proven farther
apart than the margin. Two properties make that true:

- It works on `ObjectBounds::enclosing()`: the mesh extent grown by the
  object's chord deviation. A tessellated cylinder's true surface bulges past
  its chords, so the mesh box alone could discard a real clash.
- It discards pairs by the Euclidean gap between boxes. That gap never
  exceeds the gap between the bodies inside the boxes.

Distances in a projection need their own pruning, because no projected
distance is bounded below by the box gap in space: two bodies on different
storeys can be close in plan. `projected_candidate_pairs` prunes each
projection by the gap that bounds it:

| Projection | Discarded when |
|---|---|
| `minimum_3d` | the Euclidean box gap exceeds the margin |
| `horizontal` | the plan box gap exceeds the margin |
| `plan_overlap` | the boxes do not meet in plan |
| `vertical` | the plan box gap exceeds the footprint offset, or the vertical box gap exceeds the margin |

Each is proven against the exhaustive search in its tests.

An object may be in both groups, which is how a group is checked against
itself. It is never paired with itself, and each unordered pair is reported
once, with the lesser identity as subject whenever that orientation is allowed.
If one object is supplied twice with different extents, the search is refused.

## Narrow phase: `ProximityService`

A `ProximityService` answers two questions:

- `bounds(object)` gives an object's axis-aligned extent and its
  `GeometryFidelity`.
- `measure_proximity(request)` gives a `ProximityEvidence` for one pair:
  - the **separation** between the surfaces (zero when they meet);
  - the **plan overlap** area of the two footprints;
  - the witnessed **penetration** depth;
  - an optional **containment**, when one body lies wholly inside the other.

The service measures and never decides. Two limits of the measurement are
explicit in the contract:

- **Zero separation is ambiguous.** A slab resting on a wall and a pipe
  through it both have surfaces that meet. Penetration tells them apart. It is
  the depth of the deepest point the adapter found inside the other body: a
  lower bound on the true depth, never an overestimate. Only a closed solid
  has an inside. A sheet entering a wall is measured against the wall, since a
  surface has no volume of its own. Two open surfaces report `None` rather
  than zero.
- **Tessellation is approximate.** A mesh registered as a tessellation of
  curved faces carries a chord deviation. The evidence for a pair combines the
  deviations of both bodies, sets `Evidence::exact` to `false`, and offers
  `separation_interval_metres()`: the range the true separation can lie in.
  `ProximityEvidence::try_new` refuses evidence whose exactness disagrees with
  the fidelity, so a tessellation cannot be presented as fact.

### Distances in a projection

`measure_distance(request)` answers a narrower question than
`measure_proximity`: the distance between two bodies in the request's
`ProximityProjection`, as a `ProjectedDistanceEvidence` interval.

| Projection | Distance |
|---|---|
| `Minimum3d` | shortest distance between the surfaces in space |
| `Horizontal` | plan distance between the footprints; zero when they meet |
| `Vertical { footprint_offset_metres }` | gap between the vertical extents (bottom to top) of bodies above one another |
| `PlanOverlap` | zero when the footprints overlap with positive area |

`Vertical` and `PlanOverlap` relate only some pairs. Vertical bodies are
related when their footprints overlap with positive area, or, with a positive
offset, when the counterpart's footprint comes closer than the offset to the
subject's: the subject's footprint grown by the offset. The vertical distance
is the gap between the two bodies' whole vertical extents, not their extents
over the overlap. An unrelated pair has no distance, reported as an infinite
interval.

Exact evidence is a point, related or not. A tessellation widens each
distance by the combined chord deviation and may leave the relation open: its
interval then reaches infinity. `measure_proximity` refuses a projected
request, and the default `measure_distance` answers `Minimum3d` from the full
measurement and refuses every other projection with `UnsupportedProjection`,
so a service that does not measure projections fails closed.

## Axiolid measurement

`AxiolidProximityService` builds every measurement from published Axiolid
primitives:

- **Separation** folds `closest_points_on_triangles` over triangle pairs. It
  skips pairs whose triangle boxes are already farther apart than the best
  distance found, a skip that loses nothing.
- **Plan overlap** intersects the projected, uniformly oriented triangle soups
  with `axiolid-overlay`.
- **Penetration** samples points of each body and tests them against the
  other's winding number. The samples are vertices, edge and face centres, the
  body's centre, and the midpoints between the points where each edge crosses
  the other surface (found with `axiolid-ray-mesh`). A pipe through a wall has
  no vertex inside the wall, but the midpoint of each long edge's crossings
  lies half a wall deep. An exact duplicate is found through its centre.

Both bodies are indexed with an `axiolid-spatial` bounding-volume hierarchy,
so each query touches only nearby triangles. Samples are ranked by their
distance to the other surface and tested deepest first; the first one inside is
the deepest witness. On two 18,816-triangle columns, a full measurement takes
0.26 s, and separation alone 0.8 ms against 140 s for the exhaustive scan.

Projected distances reuse the same pieces:

- **Horizontal** distance folds 2D closest points over the projected
  triangles through the same indexed search. A footprint is the union of its
  projected triangles, and the distance between two unions is the least
  distance between their parts, so a non-convex footprint (an L-shaped room,
  the notch of a stair) is measured exactly without a polygon
  boundary-distance primitive. A triangle standing edge-on projects to a
  segment, so an open vertical sheet has a footprint too.
- **Vertical** distance is the gap between the meshes' vertical extents. Plan
  overlap decides whether exact bodies are related, the plan distance whether
  they come within a footprint offset.
- **Tessellated** footprints may lie anywhere within their chord deviation of
  the mesh footprint. Overlap is asserted only from a witness point inside the
  meshes' overlap farther than the deviations from its boundary (the overlap's
  centroid and the centroids of its fan triangles are tried), and denied only
  when the plan distance exceeds the combined deviation. Anything between is
  reported open. A certified boundary distance between curved footprints
  would narrow this and waits on axiolid/kernel#174.

Points are tested only against closed two-manifold meshes. A pair needs at
least one of them to report a penetration. Hosts declare curved parts with
`AxiolidGeometry::with_tessellated_mesh(object, mesh, chord_deviation_metres)`.
A mesh registered with `with_mesh` asserts planar faces.

The other Axiolid services only produce exact evidence, so they refuse when a
tessellation could change their answer. That is the case when the subject is
tessellated, or when a tessellated part's enclosing box comes within the
measurement's reach: the contact gap, the guard radius, or the clearance
volume. A curved part elsewhere in the model does not block anything. Shelf
length is an upper bound, so it widens by the deviation instead of refusing.

## Capabilities

`axioval:capability.clash` checks the rule's selected subjects against a
`counterparts` selector. It takes these parameters:

| Parameter | Type | Meaning |
|---|---|---|
| `counterparts` | selector | the objects checked against |
| `penetration_tolerance_metres` | number, required | overlap accepted at joints |
| `clearance_metres` | number, optional | minimum separation; positive |

A **hard clash** is a witnessed penetration deeper than the tolerance, or one
body wholly inside another. A **clearance clash** is a separation below the
clearance, when there is no hard clash. Surfaces that meet with no penetration
measurement are reported as not evaluated.

`axioval:capability.distance` requires each subject's counterparts to keep a
declared distance. It takes these parameters:

| Parameter | Type | Meaning |
|---|---|---|
| `counterparts` | selector | the objects measured against |
| `mode` | string, optional | `nearest` (default), `none_closer_than` or `at_least` |
| `minimum_metres` | number, optional | lower bound |
| `maximum_metres` | number, optional | upper bound |
| `count` | integer, optional | how many counterparts `at_least` requires |
| `projection` | string, optional | `minimum_3d` (default), `horizontal`, `vertical`, `plan_overlap` |
| `footprint_offset_metres` | number, optional | grows the subject's footprint for `vertical` |
| `relationship`, `direction`, `follow_chain`, `path`, `skip_absent_relationship_ends` | optional | the container traversal |

The modes:

- `nearest`: the nearest counterpart lies within the bounds. At least one
  bound must be declared, and the minimum must not exceed the maximum. That is
  no counterpart closer than the minimum and at least one within the maximum.
- `none_closer_than`: no counterpart lies closer than a positive
  `minimum_metres`; a maximum is refused. The finding names every
  counterpart that is certainly too close.
- `at_least`: at least `count` counterparts lie within `maximum_metres`,
  and no nearer than `minimum_metres` when one is declared: N within a
  range.

With `plan_overlap`, a distance is zero or none, so `none_closer_than` with
any positive minimum forbids overlapping footprints and `at_least` with any
maximum requires them.

**Scoping.** With a traversal declared, only counterparts sharing a container
with the subject are measured: the objects the traversal reaches from each,
through declared relationships or derived ones such as
`axioval:derived.contained-in-space`. An object reaching no container shares
none. A subject whose containers cannot be decided is not evaluated; a
counterpart whose containers cannot be decided is undecided.

**Judging intervals.** A counterpart counts within a range only when its whole
interval lies inside, and breaks a minimum only when its whole interval lies
below. A counterpart whose interval straddles a bound, whose distance or
extent could not be measured, or whose container is undecided is **unknown**.
A verdict is given only when the unknowns cannot change it: a count is met by
certain counterparts alone, and a shortfall stands only when even counting
every unknown falls short. A certain violation stands whatever else is
unknown. When nothing lies within a maximum, that is a finding: the broad
phase is complete, so a pair it does not report is farther apart than the
margin.

Door-swing footprints as distance sources wait on object frames (#36).

Both capabilities fail closed:

- A missing service or an invalid declaration refuses every subject.
- An object whose extent cannot be read is reported not evaluated, because
  pairs involving it were never checked.
- A failed pair measurement leaves its subject not evaluated.
- For `distance`, an unmeasured counterpart leaves a subject not evaluated
  whenever that counterpart could change the verdict: it might break a minimum
  or complete an unmet count. A count already met by measured counterparts
  stays met.
- A projection the service does not measure leaves every subject not
  evaluated.

A finding names its counterpart in `related` and carries the measurement's
evidence. A finding measured on tessellated geometry says so in its message,
and its evidence is not exact.
