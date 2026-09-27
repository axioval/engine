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
| `vertical`, `above` | as `vertical`, with the gap from the subject's top up to the counterpart's bottom; or the counterpart's box lies wholly below the subject's |
| `vertical`, `below` | the mirror image of `above` |

A directed search keeps a pair when either orientation the groups allow
qualifies: an object in both groups is reported once, and the capability
measures the pair from each of its ends. Each rule is proven against the
exhaustive search in its tests, disjoint and overlapping groups alike.

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
  - an optional **containment**, when one body lies wholly inside the other;
  - the **overlap extents**: how far the intersection reaches along x, y
    and z (`OverlapExtents`), with `horizontal()` the narrower plan axis and
    `vertical()` the z extent;
  - the **Hausdorff distance** between the two surfaces: the farthest any
    point of either surface lies from the other, zero exactly when they
    coincide.

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
- **Shape comparisons are intervals.** Overlap extents and the Hausdorff
  distance are `LengthInterval`s the true values lie in, attached with
  `with_overlap_extents` and `with_hausdorff`. They are not points even for
  exact geometry: an extent's lower bound is witnessed and its upper bound
  proven. Extents need a penetration measurement (two open surfaces share no
  volume), and bodies apart at the surface that do not contain one another
  report an empty intersection. A Hausdorff distance below the separation is
  refused. Both are optional: a service that does not measure them leaves
  the questions they answer open.

The intersection's **volume** is not measured. A sound volume bound needs a
certified mesh boolean, which waits on axiolid/kernel#183; there is no
volume tolerance until then.

### Distances in a projection

`measure_distance(request)` answers a narrower question than
`measure_proximity`: the distance between two bodies in the request's
`ProximityProjection`, as a `ProjectedDistanceEvidence` interval.

| Projection | Distance |
|---|---|
| `Minimum3d` | shortest distance between the surfaces in space |
| `Horizontal` | plan distance between the footprints; zero when they meet |
| `Vertical { footprint_offset_metres, direction }` | gap between the vertical extents (bottom to top) of bodies above one another, one-sided with a direction |
| `PlanOverlap` | zero when the footprints overlap with positive area |

`Vertical` and `PlanOverlap` relate only some pairs. Vertical bodies are
related when their footprints overlap with positive area, or, with a positive
offset, when the counterpart's footprint comes closer than the offset to the
subject's: the subject's footprint grown by the offset. The vertical distance
is the gap between the two bodies' whole vertical extents, not their extents
over the overlap. An unrelated pair has no distance, reported as an infinite
interval.

A `VerticalDirection` narrows the vertical projection to one side, comparing
the two extents end by end:

| Direction | Related when the counterpart | Distance |
|---|---|---|
| `Either` | lies anywhere | gap between the extents |
| `Above` | is not lower at both ends (top below the subject's top and bottom below its bottom) | subject's top up to the counterpart's bottom |
| `Below` | is not higher at both ends | subject's bottom down to the counterpart's top |

Both distances are zero when the extents overlap. A riser passing a
sprinkler is therefore above and below it at zero; a pendant reaching down
past the sprinkler's top is above it, at zero, and not below. Every
counterpart is above or below, so `Either` is the lesser of the two
distances. Seen from the counterpart, above is below.

Exact evidence is a point, related or not. A tessellation widens each
distance by the combined chord deviation and may leave the relation open: its
interval then reaches infinity. A direction is open in the same way when
either end of the counterpart lies within the combined deviation of the
subject's and no end is decided beyond it. `measure_proximity` refuses a projected
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

- **Overlap extents** span points witnessed in both bodies: every point
  where an edge of either body crosses the other's surface, and the
  vertices of either that lie inside the other. For polyhedra those include
  every vertex of the intersection, so the lower bound is the true extent
  wherever the crossings are found; the upper bound is the overlap of the
  bodies' boxes. Inside vertices are tried outermost first on each side of
  each axis, and a side stops at the first vertex inside or the first that
  would not widen the box, so the winding test runs only where it can move
  the answer. A tessellation lowers the witnessed extent by twice the
  combined deviation and grows each box by its own deviation.
- **Hausdorff distance** is bounded below by the farthest any vertex lies
  from the other surface. Above, it is bounded per triangle: the distance to
  one triangle is convex, so the farthest point of a triangle from it is a
  vertex, and the least over the other body's triangles of that farthest
  vertex distance bounds the whole triangle. A triangle whose bound exceeds
  the largest distance already known is split in four, twice at most.
  Identical meshes come out exactly zero; a tessellation widens both bounds
  by the combined deviation.

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
- **Vertical** distance is the gap between the meshes' vertical extents,
  one-sided with a direction. Plan overlap decides whether exact bodies are
  related, the plan distance whether they come within a footprint offset,
  and the mesh ends which side a counterpart lies on. For a tessellation an
  end difference within the combined deviation decides no side.
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
| `duplicate_tolerance_metres` | number, optional | surfaces this close are duplicates; default zero |
| `horizontal_tolerance_metres` | number, optional | an intersection must reach further along both plan axes; default zero |
| `vertical_tolerance_metres` | number, optional | an intersection must reach further in height; default zero |
| `report_duplicates` | boolean, optional | report duplicates; default true |
| `report_containment` | boolean, optional | report bodies inside others; default true |
| `report_intersections` | boolean, optional | report intersections; default true |
| `exclude_paths` | string list, optional | relationship paths; pairs reaching a shared target are skipped |
| `exclude_same_layer` | boolean, optional | skip pairs sharing a presentation layer; default false |

Each pair falls into the first class that holds:

1. **Duplicate**: the Hausdorff distance between the surfaces is within
   `duplicate_tolerance_metres`.
2. **Containment**: one body lies wholly inside the other.
3. **Intersection**: a witnessed penetration deeper than the penetration
   tolerance, whose intersection reaches further than the horizontal
   tolerance along both x and y (the narrower plan axis decides) and further
   than the vertical tolerance in z. A zero axis tolerance asks nothing of
   that axis. A duct sunk 5 mm into a slab is wide in plan but 5 mm high, so
   a 10 mm vertical tolerance lets it pass.
4. **Clearance**: a separation below the clearance, when no class above
   holds.

A class switched off is not reported, and its pairs are not reported as
anything else: a duplicate is not also an intersection. Surfaces that meet
with no penetration measurement are reported as not evaluated. Axis
extents are taken along the world axes, so an intersection with a wall at an
angle reaches further along x and y than its depth into the wall; the
penetration tolerance still bounds that depth.

**Judging intervals.** A class holds when the whole measured interval says
so and fails when none of it does. When a Hausdorff distance or an extent
straddles its tolerance, the pair is judged both ways: it is reported when
both readings are findings (with the finding that holds either way) and
passes when both pass; otherwise it is not evaluated. A service that
measures no Hausdorff distance therefore leaves touching pairs open while
duplicates are reported.

**Exclusions.** Each `exclude_paths` entry is a relationship path, its
steps separated by spaces and written like the `path` of the distance
traversal (`Relationship` or `Relationship:direction`). A pair is skipped
when the objects reached from its two members through the same path meet,
counting each member as reaching itself, so a path from one member to the
other excludes it too. In IFC:

| Exclusion | Path |
|---|---|
| same system | `IfcRelAssignsToGroup:backward` |
| same parent element | `IfcRelAggregates:backward` |
| connected ports | `IfcRelConnectsPortToElement:backward IfcRelConnectsPorts:either IfcRelConnectsPortToElement:forward` |

`exclude_same_layer` skips pairs whose `axioval:presentation.Layer` lists
share a name; an object on no layer shares none. An exclusion is decided
before measuring, so an excluded pair costs no narrow phase. One that cannot
be decided (a relationship the source refuses, a source recording no layers)
never hides a pair and never reports one: a pair that would be reported is
not evaluated instead, and one that passes stays passed.

The intersection volume and a volume tolerance wait on
axiolid/kernel#183.

### Clash matrix

`axioval:capability.clash-matrix` gives each pair of categories its own
tolerance profile and severity. Pairs are proposed as for `clash` (the rule's
subjects against `counterparts`), within the widest clearance any cell
declares. It is a capability of its own rather than a mode of `clash`,
because a matrix moves every tolerance into the table: a `clash` rule's
flat parameters would all be meaningless beside it. The pair judgement is
shared: a cell means exactly what a `clash` rule with the same values means,
classes, switches, interval handling and exclusions included.

| Parameter | Type | Meaning |
|---|---|---|
| `counterparts` | selector | the objects checked against |
| `cells` | table, required | one row per cell, columns below |
| `key_1` … `key_3` | property, optional | the text properties the `*_key_<n>` columns match |
| `case_sensitive` | boolean, optional | whether patterns match case; default true |
| `symmetric` | boolean, optional | a cell covers a pair either way round; default true |
| `report_unmatched` | boolean, optional | report a pair no cell covers; default false |
| `exclude_same_system` | boolean, optional | skip pairs in one system; default true |
| `system_path` | string, optional | the relationship path from an object to its system |
| `exclude_paths` | string list, optional | further exclusion paths, as for `clash` |
| `exclude_same_layer` | boolean, optional | skip pairs sharing a presentation layer; default false |

Each cell keys both sides of the pair: `subject_*` and `counterpart_*`.

| Column | Kind | Meaning |
|---|---|---|
| `*_discipline` | pattern | the discipline the object's source plays |
| `*_selector` | selector | a selector the object must match, such as an entity type |
| `*_key_1` … `*_key_3` | pattern | the value of the property `key_<n>` names |
| `penetration_tolerance_metres` | number, required | as for `clash` |
| `clearance_metres`, `duplicate_tolerance_metres`, `horizontal_tolerance_metres`, `vertical_tolerance_metres` | number | as for `clash` |
| `report_duplicates`, `report_containment`, `report_intersections` | boolean | as for `clash`; default true |
| `severity` | string | `error`, `warning` or `info`; default the rule's |
| `label` | string | names the cell in findings |

Patterns are whole-value wildcards (`*`, `?`), read as `like` reads them. A
blank column accepts any object; an absent property matches no pattern. A
cell for `architecture` against `mep` and one for `architecture` against
`structure`, each with its own penetration tolerance, check both pairs in
one run:

```json
"cells": { "type": "table", "value": [
  { "subject_discipline": { "type": "string", "value": "architecture" },
    "counterpart_discipline": { "type": "string", "value": "structure" },
    "penetration_tolerance_metres": { "type": "number", "value": 0.01 },
    "severity": { "type": "string", "value": "error" } },
  { "subject_discipline": { "type": "string", "value": "architecture" },
    "counterpart_discipline": { "type": "string", "value": "mep" },
    "penetration_tolerance_metres": { "type": "number", "value": 0.05 },
    "severity": { "type": "string", "value": "warning" } } ] }
```

**Choosing the cell.** Each pair is judged with its single most specific
cell, through `support::table`'s row matching. A cell keying more categories
is more specific, whatever its patterns; among cells keying as many, the one
with more literal pattern characters is. With `symmetric` a cell covers a
pair when it matches either way round, at the more specific orientation.
The broad phase reports a pair of one group once, with the lesser identity
as subject, so an ordered matrix (`symmetric: false`) is meaningful only
between disjoint groups.

- **Tie.** Cells tied for most specific leave the pair not evaluated
  (`invalid-declaration`), naming the cells; declaration order never breaks
  the tie.
- **Unknown category.** A category that cannot be read (a property the
  source cannot answer, a selector it cannot decide, a source declaring no
  discipline) leaves the pair not evaluated when a cell testing it could
  apply. A source declaring no discipline is reported once per source.
- **Unmatched.** A pair no cell covers is ignored, or, with
  `report_unmatched`, a finding naming both objects' categories. It is not
  measured either way.
- **Switched off.** A cell with every class off and no clearance covers its
  pairs without checking them: they are neither measured nor unmatched.

**Exclusions** apply to the whole matrix and are decided before a cell is
chosen. Same-system exclusion is on unless switched off, and then needs
`system_path` (in IFC, `IfcRelAssignsToGroup:backward`): the engine names no
source's relationships, so a matrix without the path and without
`exclude_same_system: false` is an invalid declaration. Same-layer
exclusion is off unless switched on.

A finding carries its cell's severity, ends with the cell it was judged by
(``(clash matrix cell 0 `architecture x structure`)``), and carries the
evidence of the category values read to choose it.

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
| `vertical_direction` | string, optional | `either` (default), `above` or `below`: where a counterpart must lie for `vertical` |
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

With `vertical`, `vertical_direction` counts only counterparts above the
subject or only those below it, as the vertical projection defines them.
"A sprinkler at most 0.5 m below the ceiling" is `nearest` with
`maximum_metres` 0.5 and `above` (the ceiling lies above the sprinkler);
"nothing above within 2 m" is `none_closer_than` with `minimum_metres` 2
and `above`. A direction or an offset with another projection is an
invalid declaration.

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

Door-swing footprints as distance sources wait on door leaves in the IFC
source (upstream openbimrs/ifc#148) and on object frames (#36).

These capabilities fail closed:

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
