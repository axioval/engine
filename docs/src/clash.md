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
  lower bound on the true depth, never an overestimate. An open surface has no
  inside, so it reports `None` rather than zero.
- **Tessellation is approximate.** A mesh registered as a tessellation of
  curved faces carries a chord deviation. The evidence for a pair combines the
  deviations of both bodies, sets `Evidence::exact` to `false`, and offers
  `separation_interval_metres()`: the range the true separation can lie in.
  `ProximityEvidence::try_new` refuses evidence whose exactness disagrees with
  the fidelity, so a tessellation cannot be presented as fact.

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

Penetration and containment are measured only between closed two-manifold
meshes. Hosts declare curved parts with
`AxiolidGeometry::with_tessellated_mesh(object, mesh, chord_deviation_metres)`.
A mesh registered with `with_mesh` asserts planar faces.

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

`axioval:capability.distance` requires each subject's nearest counterpart to
lie within `minimum_metres` and/or `maximum_metres`. At least one bound must be
declared, and the minimum must not exceed the maximum. When nothing lies within
the maximum, that is a finding. The broad phase proves it, because a pair it
does not report is farther apart than the margin.

Both capabilities fail closed:

- A missing service or an invalid declaration refuses every subject.
- An object whose extent cannot be read is reported not evaluated, because
  pairs involving it were never checked.
- A failed pair measurement leaves its subject not evaluated.
- For `distance`, an unmeasured counterpart leaves a subject not evaluated
  whenever that counterpart could change the verdict: it might break a minimum
  or meet an unmet maximum. A maximum already met by a measured counterpart
  stays met.

A finding names its counterpart in `related` and carries the measurement's
evidence. A finding measured on tessellated geometry says so in its message,
and its evidence is not exact.
