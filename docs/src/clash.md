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
    coincide;
  - the **intersection volume** (`IntersectionVolume`): the volume the two
    closed bodies share, with each body's own volume.

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

- **Volumes are certified.** The shared volume and both bodies' volumes
  are `VolumeInterval`s in cubic metres, sure to contain the true values,
  attached with `with_intersection_volume`. The shared volume never
  exceeds either body's; open surfaces enclose none, so a pair without a
  penetration measurement cannot carry one; bodies apart at the surface
  and not holding one another share none; and a body said to lie inside
  the other shares its whole volume with it. `ratio_of_smaller()` bounds
  the shared volume's share of the smaller body, rounded outward. A
  service that does not measure volumes leaves them `None`.

### Extents along the bodies' own axes

`measure_overlap_along(request)` answers what world axes cannot: how far the
intersection reaches along stated directions, such as each body's placement
axes. An `OverlapAlongRequest` names the two bodies and one to six unit
directions; `OverlapAlongEvidence` holds one `LengthInterval` per direction,
in request order, witnessed below and proven above like `OverlapExtents`,
and exact exactly when the geometry is. The default method refuses with
`UnsupportedProjection`, and the handle refuses evidence for another
request, so a service that does not measure them fails closed.

### Distance to a class of faces

`measure_face_distance(request)` answers what cover and protrusion checks
ask: how far a body lies from one class of another body's (the host's)
faces. A `FaceDistanceRequest` names the body, the host and a `FaceClass`:

| Class | Host faces whose outward normal |
|---|---|
| `top` | points upwards within 45° of vertical (z component at least √½) |
| `bottom` | points downwards within 45° of vertical |
| `side` | lies between: walls' faces and ends |
| `any` | every face |

The answer is a signed distance. With `F` the selected faces, each point of
the body counts `+d(p, F)` when it lies in the host (boundary included) and
`-d(p, F)` when it lies outside; the body's distance is the least of them.
Positive is a cover (the whole body inside, that far from the faces),
negative a protrusion (part of the body outside, the farthest of it that far
from the faces), zero a body reaching the faces. It is a
`SignedDistanceInterval` sure to hold the true value, a point only when the
measurement proves one. The default method refuses with
`FaceDistanceError::Unsupported`, so a service that does not measure face
distances fails closed.

### Distances in a projection

`measure_distance(request)` answers a narrower question than
`measure_proximity`: the distance between two bodies in the request's
`ProximityProjection`, as a `ProjectedDistanceEvidence` interval.

| Projection | Distance |
|---|---|
| `Minimum3d` | shortest distance between the surfaces in space |
| `Horizontal` | plan distance between the footprints; zero when they meet |
| `Vertical { footprint_offset_metres, direction, surfaces }` | gap between the vertical extents (bottom to top) of bodies above one another, one-sided with a direction; with `surfaces` `Between`, from the subject's top or bottom to the counterpart's top, bottom or nearest surface over the subject's footprint |
| `PlanOverlap` | zero when the footprints overlap with positive area |

`measure_region_distance(request)` measures from a stated region instead of
a body: the plan distance from a `ConvexPlanRegion` (a convex polygon in
canonical metres, anticlockwise) to the counterpart's footprint, zero when
they meet, as a `RegionDistanceEvidence` interval bound to its
`RegionDistanceRequest`. It is exact exactly when the counterpart is, and
its evidence cites the counterpart's source. The trait's default refuses
with `UnsupportedProjection`. `ConvexPlanRegion::separation` measures two
regions against each other without geometry: their distance apart, or minus
their depth of overlap. Capabilities use regions for door swings, bracketing
each sector between an inscribed and a circumscribed polygon. The Axiolid
adapter folds 2D closest points between the region's fan triangles and the
counterpart's projected triangles, as for `Horizontal`, and widens a
tessellated counterpart by its chord deviation.

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
- **Extents along stated directions** reuse the same witnesses: each
  crossing and inside vertex is projected onto the direction, and the upper
  bound is the overlap of the two bodies' own ranges along it (the
  intersection lies in both). Along a coordinate axis a projection reads one
  coordinate exactly; along any other every projection widens by a bound on
  the dot product's rounding. Two open surfaces share no volume and are
  refused.
- **Hausdorff distance** is bounded below by the farthest any vertex lies
  from the other surface. Above, it is bounded per triangle: the distance to
  one triangle is convex, so the farthest point of a triangle from it is a
  vertex, and the least over the other body's triangles of that farthest
  vertex distance bounds the whole triangle. A triangle whose bound exceeds
  the largest distance already known is split in four, twice at most.
  Identical meshes come out exactly zero; a tessellation widens both bounds
  by the combined deviation.

- **Intersection volume** comes from `axiolid-inspect`'s certified volume
  integrals (`intersection_volume`, `enclosed_volume`): no boolean mesh is
  built, and every value is an interval sure to hold the true one. Only two
  closed solids have one. Bodies apart at the surface and not holding one
  another share exactly nothing without integrating. A mesh the kernel
  refuses (open, self-intersecting, enclosing no volume) leaves the volume
  unmeasured, not the whole measurement. A tessellated body's true surface
  lies within its chord deviation of the mesh, so its volumes widen by an
  upper bound on the volume within that band: per triangle
  `2·d·A + (π/2)·P·d² + (4/3)·π·d³` (Steiner's formula for a flat convex set
  of area `A` and perimeter `P`).
- **Face distance** reads each host triangle's class from its outward
  normal, the outward side taken from the host's signed volume so either
  winding reads alike. A body wholly inside the host (every vertex inside
  by winding number, its surface clear of the host's) has its distance
  exactly: the least triangle-to-triangle distance to the selected faces,
  exact for planar meshes. Otherwise the distance is bounded: above by each
  vertex clear of the host surface, signed by its side, by zero where the
  body reaches the faces and by the body's distance to them; below by minus
  the farthest the part outside the host can lie from the faces. That part
  lies in the hull of the body's vertices not proven inside, the points
  where the two surfaces cross and the host's vertices near the body, and
  the distance to one face is convex, so its farthest vertex bounds it; the
  least of those over the faces bounds every point. The bound is exact when
  the outside part lies over one face triangle and may be wider over
  several. The host must be an exact closed solid (a tessellation's chords
  do not state its faces' normals); a triangle within rounding of 45°
  refuses a class it might or might not belong to; a tessellated body
  widens both bounds by its chord deviation.

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
| `volume_tolerance_cubic_metres` | number, optional | an intersection must share more volume; default zero |
| `report_duplicates` | boolean, optional | report duplicates; default true |
| `report_containment` | boolean, optional | report bodies inside others; default true |
| `report_intersections` | boolean, optional | report intersections; default true |
| `exclude_paths` | string list, optional | relationship paths; pairs reaching a shared target are skipped |
| `exclude_target_property` | property, optional | reached targets also meet when they state the same value of it |
| `exclude_same_layer` | boolean, optional | skip pairs of one model sharing a presentation layer; default false |
| `group_by`, `per_storey`, `storey_path`, `group_property`, `group_tolerance_metres` | optional | group reported pairs into issues, see [Grouping findings](#grouping-findings) |
| `severity_by_class`, `grade_by`, `severity_grades`, `duplicate_quantities` | optional | severities by class and size, and what duplicates are compared by, see [Severities and duplicates](#severities-and-duplicates) |
| `tolerance_cases` | table, optional | intersections excused along the elements' own axes, see [Tolerance cases](#tolerance-cases) |

Each pair falls into the first class that holds:

1. **Duplicate**: the Hausdorff distance between the surfaces is within
   `duplicate_tolerance_metres`.
2. **Containment**: one body lies wholly inside the other.
3. **Intersection**: a witnessed penetration deeper than the penetration
   tolerance, whose intersection reaches further than the horizontal
   tolerance along both x and y (the narrower plan axis decides) and further
   than the vertical tolerance in z, whose certified volume exceeds the
   volume tolerance, and that no [tolerance case](#tolerance-cases)
   excuses. A zero tolerance asks nothing of its axis or of the
   volume. A duct sunk 5 mm into a slab is wide in plan but 5 mm high, so a
   10 mm vertical tolerance lets it pass; a volume tolerance lets small
   overlaps at joints pass however they are shaped.
4. **Clearance**: a separation below the clearance, when no class above
   holds.

A class switched off is not reported, and its pairs are not reported as
anything else: a duplicate is not also an intersection. Surfaces that meet
with no penetration measurement are reported as not evaluated. Axis
extents are taken along the world axes, so an intersection with a wall at an
angle reaches further along x and y than its depth into the wall; the
penetration tolerance still bounds that depth.

**Judging intervals.** A class holds when the whole measured interval says
so and fails when none of it does. When a Hausdorff distance, an extent or
the shared volume straddles its tolerance, the pair is judged both ways: it is reported when
both readings are findings (with the finding that holds either way) and
passes when both pass; otherwise it is not evaluated. A service that
measures no Hausdorff distance therefore leaves touching pairs open while
duplicates are reported, and an unmeasured volume (an open surface, a mesh
the kernel refuses) leaves an intersection open under a volume tolerance.

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

Federated models split one system into a system object per model, so no
target is ever shared between them. With `exclude_target_property`, the
targets reached from the two members also meet when they state the same
value of that property: a duct in the ventilation model and a pipe in the
plumbing model, each assigned to a system named `SUP-01`, are excluded with
the path `IfcRelAssignsToGroup:backward` and the property
`axioval:attributes.Name`. Only reached targets are compared, never the
members themselves: two walls of one name are not one system. An absent,
null or blank value matches nothing; a value that cannot be read leaves
the exclusion undecided when a target on the other side could match it.
The property needs an exclusion path; alone it is an invalid declaration.

`exclude_same_layer` skips pairs of one model whose
`axioval:presentation.Layer` lists share a name; an object on no layer
shares none. Layers are named per model, so the same name in two models is
no shared layer and never excludes a pair. An exclusion is decided
before measuring, so an excluded pair costs no narrow phase. One that cannot
be decided (a relationship the source refuses, a source recording no layers)
never hides a pair and never reports one: a pair that would be reported is
not evaluated instead, and one that passes stays passed.

### Tolerance cases

Axis tolerances are measured along the world axes, so they misjudge an
element at an angle: a slab edge sunk 10 mm into a wall standing at 30°
reaches metres along x and y. `tolerance_cases` excuses such intersections
along the elements' own placement axes. Each row names a case, two
component filters and a tolerance:

| Column | Kind | Meaning |
|---|---|---|
| `case` | string, required | `horizontal_orthogonal`, `vertical_orthogonal`, `horizontal_protrusion` or `vertical_protrusion` |
| `first_selector`, `second_selector` | selector | the first and the second element of the pair, either way round; blank accepts any |
| `tolerance_metres` | number, required | the extent the case accepts; not negative |

| Case | The intersection's extent along |
|---|---|
| `horizontal_orthogonal` | the second element's plan axes (its placement's right and forward), the lesser |
| `vertical_orthogonal` | the second element's up axis |
| `horizontal_protrusion` | the first element's plan axes, the lesser |
| `vertical_protrusion` | the first element's up axis |

An orthogonal case measures how far the first element reaches into the
second across the second's own axes (the slab edge through the wall's
thickness); a protrusion how far the first element sticks out along its
own. The axes are the placement frames the object-frame service states
(`ObjectFrameServiceHandle`), and the extents come from
`measure_overlap_along`, one request per pair for both bodies' axes. A case
is asked only for a pair that would otherwise be an intersection. The slab
in the 30° wall, with `first_selector` the slabs, `second_selector` the
walls and a 20 mm `horizontal_orthogonal` case, passes; without the case it
is a hard clash.

A case excuses a pair when its filters surely match and the whole extent
lies within the tolerance, and does not when a filter surely fails or the
whole extent lies beyond. Anything else is open: a filter the selection
cannot decide, a straddling extent, a missing object-frame service or
frame, a measurement the proximity service refuses. An open case never
hides and never reports the intersection; the pair is not evaluated, and
says which case is open. A finding that a case did not excuse carries the
frames and the measurement as evidence.

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
| `exclude_target_property` | property, optional | as for `clash`, over `system_path` and `exclude_paths` |
| `exclude_same_layer` | boolean, optional | skip pairs of one model sharing a presentation layer; default false |
| `group_by`, `per_storey`, `storey_path`, `group_property`, `group_tolerance_metres` | optional | as for `clash`; a group never spans two cells |
| `severity_by_class`, `grade_by`, `severity_grades`, `duplicate_quantities` | optional | as for `clash`; a cell's `severity` wins over the class's |
| `tolerance_cases` | table, optional | as for `clash`, for every cell |

Each cell keys both sides of the pair: `subject_*` and `counterpart_*`.

| Column | Kind | Meaning |
|---|---|---|
| `*_discipline` | pattern | the discipline the object's source plays |
| `*_selector` | selector | a selector the object must match, such as an entity type |
| `*_key_1` … `*_key_3` | pattern | the value of the property `key_<n>` names |
| `penetration_tolerance_metres` | number, required | as for `clash` |
| `clearance_metres`, `duplicate_tolerance_metres`, `horizontal_tolerance_metres`, `vertical_tolerance_metres`, `volume_tolerance_cubic_metres` | number | as for `clash` |
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

### Severities and duplicates

A clash rule's findings need not share the rule's severity. Both `clash`
and `clash-matrix` take:

| Parameter | Type | Meaning |
|---|---|---|
| `severity_by_class` | table, optional | rows of `class` (`duplicate`, `containment`, `intersection`, `clearance`) and `severity`; each class once |
| `grade_by` | string, optional | `smallest_extent` or `volume`: what intersections are graded by; required with `severity_grades` |
| `severity_grades` | table, optional | rows of `above` (metres, or cubic metres for `volume`; not negative, each once) and `severity` |
| `duplicate_quantities` | table, optional | rows of `property` and an optional `property_set`: quantities duplicates are compared by |

A finding's severity is, in order: its grade, for an intersection whose
measure exceeds a grade's `above` (the highest such grade); a clash matrix
cell's own `severity`; its class's in `severity_by_class`; the rule's. The
smallest extent is the least of the intersection's x, y and z extents, the
volume the certified volume the bodies share. With grades above 25 mm
(`warning`) and 200 mm (`error`), and `info` for intersections, a 10 mm
sliver is `info` and a 300 mm intersection an `error`. The message ends
with the grade and the measure (`, graded error by its smallest extent of
0.3000 m`).

Measures are intervals. One straddling a grade's bound, or an unmeasured
one, takes the most severe severity it may reach, and says so: a clash is
never reported milder than it may be.

A duplicate's finding also says what the copies differ in: their types,
their measured volumes when the two intervals are apart, and each quantity
`duplicate_quantities` names, read from the source (`; the copies differ
in type (IFCWALL and IFCSLAB), Qto_WallBaseQuantities.NetSideArea (12.5 and
12)`), or what they agree in. A quantity read on one copy and absent on the
other differs. One that cannot be read, or a volume the measurement cannot
separate, is named as unknown (`; whether they differ in … is unknown`),
never as the same. The quantities read are evidence of the finding.

### Grouping findings

`clash` and `clash-matrix` report one finding per pair, so a duct through
forty identical walls is forty findings about one problem. `group_by` turns
the pairs sharing a key into one finding:

| `group_by` | Pairs grouped together |
|---|---|
| `subject` | every pair of one subject |
| `type_pair` | every pair between the same two object types, either way round |
| `similar` | pairs of one class (duplicate, containment, intersection, clearance) between the same two object types, whose intersection extents round to the same multiples of `group_tolerance_metres` |

| Parameter | Type | Meaning |
|---|---|---|
| `group_by` | string, optional | `subject`, `type_pair` or `similar`; without it every pair is its own finding |
| `group_tolerance_metres` | number | the rounding step of `similar` extents; required for `similar`, refused otherwise |
| `group_property` | property, optional | `similar` pairs must also agree on its value on both sides |
| `per_storey` | boolean, optional | also key groups by storey; default false |
| `storey_path` | string | the relationship path from an object to its storey; required with `per_storey` |

`similar` compares the narrower and the wider plan extent and the height of
the intersection, so a wall along x and one along y alike are similar. With
`per_storey`, a pair's storeys are those `storey_path` reaches from its two
members together (in IFC, `IfcRelContainedInSpatialStructure:backward`), so
the same duct through walls on two storeys is two issues. A clash matrix
never groups pairs judged by different cells. Any grouping parameter without
`group_by`, or `per_storey` without a path, is an invalid declaration.

A group of several pairs is one finding on the object most of them involve
(the first in identity order on a tie), relating every other object, at the
most severe severity among its pairs, carrying each pair's evidence in turn.
Its message counts the pairs, names the types and storeys, and lists each
pair's own message after its subject, such as `5 similar intersection
clashes of duct with wall on storey-1: [duct] hard clash with wall-1: …; …`.
A group of one pair is that pair's own finding.

Grouping arranges findings and never decides one. A pair whose key cannot be
read is reported on its own and says why (`(not grouped: …)`): a storey walk
the source refuses, an unreadable `group_property`, intersection extents that
were not measured or straddle a rounding step. Not-evaluated pairs are
reported per subject, as without grouping.

### Containment and cover

`axioval:capability.containment` checks that the rule's selected inner
elements (columns, reinforcement, fixings) lie in `counterparts` (walls,
slabs, concrete bodies), keep their cover to the outer element's faces, and
are held in the declared numbers.

| Parameter | Type | Meaning |
|---|---|---|
| `counterparts` | selector | the outer elements |
| `minimum_volume_ratio` | number, required | the share of the smaller body the two must share; above zero, at most one |
| `combine_adjacent` | boolean, optional | also take outer elements whose surfaces meet together; default false |
| `cover` | table, optional | bands on the distance to a class of faces, columns below |
| `minimum_count`, `maximum_count` | integer, optional | how many inner elements each outer element holds |
| `report_orphans` | boolean, optional | report an inner element that lies in no outer element; default false |

| Column | Kind | Meaning |
|---|---|---|
| `faces` | string, required | `top`, `side`, `bottom` or `any` |
| `side` | string | `inside` (the default): the cover the body keeps inside the faces; `outside`: how far it reaches past them |
| `minimum_metres`, `maximum_metres` | number | the band; at least one |

A rule must declare a cover band, a count or `report_orphans`; one
`(faces, side)` pair per row.

**Contained.** An inner element lies in an outer one when their certified
shared volume is at least `minimum_volume_ratio` of the smaller body's
volume. The ratio is an interval: contained when its lower bound reaches
the ratio, not contained when its upper bound falls short, undecided
otherwise. Certified intervals rarely prove an exact ratio of one, so ask
for slightly less (0.99) where "wholly inside" is meant. With
`combine_adjacent`, an inner element in no single outer element may lie in
several whose surfaces meet, taken together: the column at a wall junction.
The combined shared volume is bounded by the sum of the shares less what
the outer elements share with one another (never below the largest share),
and the combined body likewise. An adjacency the measurement cannot decide
is read both ways: a combination holds when it holds of surely adjacent
members and fails only when it fails however the undecided ones are read.

**Cover.** For each outer element an inner one surely lies in, each band
judges the face distance of the inner body to that class of the outer
element's faces: an `inside` band the signed distance itself, an `outside`
band its negation. A finding needs the whole interval beyond a bound; one
straddling a bound is not evaluated. The faces of a combination are not one
body's (their junction would count as a face), so an inner element held only
by a combination has its cover reported not evaluated; so does its cover to
an outer element it may or may not lie in.

**Counts.** Each outer element counts the inner elements that surely lie
in it (alone, or as a member of a combination it shares volume with) and
those that may: an undecided containment, and every inner element whose
extent could not be read. A minimum fails when even the possible ones fall
short, a maximum when the sure ones exceed it; anything between is not
evaluated. The finding is the outer element's and names the inner elements
it surely holds.

**Orphans.** With `report_orphans`, an inner element lying in no outer
element, alone or combined, is a finding, but only when every containment
was decided and every outer element measured; otherwise it is not
evaluated.

In IFC, "columns inside walls with at least 40 mm side cover" is a rule
over `IfcColumn` with `counterparts` selecting `IfcWall`,
`minimum_volume_ratio` 0.99 and one `cover` row `side`, minimum 0.04.

### Distance

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
| `subject_extent`, `counterpart_extent` | string, optional | `body` (default) or `leaf_swing` (also `door_swing`): what each side is measured by |
| `subject_surface`, `counterpart_surface` | string, optional | `vertical` only, declared together: `top` or `bottom` of the subject to `top`, `bottom` or `nearest` of the counterpart |
| `elevation_overlap` | string, optional | `any` (default) or `overlapping`: `horizontal` only relates counterparts at the subject's heights |
| `elevation_offset_metres` | number, optional | with `overlapping`, the height gap a counterpart must stay under (zero: the heights overlap) |
| `relationship`, `direction`, `follow_chain`, `path`, `skip_absent_relationship_ends` | optional | the container traversal |
| `container_selector` | selector, optional | with a traversal, only reached objects it picks are containers |

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

**Surfaces.** The plain vertical distance is the gap between two extents.
`subject_surface` and `counterpart_surface` choose the surfaces instead: the
subject's `top` or `bottom` level, and the counterpart's `top` or `bottom`
level (the difference of the two levels in the direction, for bodies whose
footprints are related) or its `nearest` surface, the part of it directly
over or under the subject's footprint nearest to the subject's level. The
six pairs a check commonly chooses among are all spelled this way: a
sprinkler's top to the nearest surface above is the sloped slab's underside
right above it, not the slab's lowest point elsewhere. `nearest` takes no
`footprint_offset_metres`. A tessellated nearest surface has an upper bound
only from a witness well inside the footprint, so a shortfall may be left
not evaluated.

**Heights.** A `horizontal` distance ignores heights: a counterpart one
storey up can be close in plan. With `elevation_overlap` `overlapping` only
counterparts whose vertical extent overlaps the subject's count, or with
`elevation_offset_metres` those whose height gap is below it (a band around
the subject's heights). A counterpart whose extent cannot be read, or whose
gap straddles the offset, is unknown; the subject's own unreadable extent
leaves it not evaluated. It needs the vertical-extent service.

**Scoping.** With a traversal declared, only counterparts sharing a container
with the subject are measured: the objects the traversal reaches from each,
through declared relationships or derived ones such as
`axioval:derived.contained-in-space`. An object reaching no container shares
none. A subject whose containers cannot be decided is not evaluated; a
counterpart whose containers cannot be decided is undecided. With
`container_selector` only the reached objects it picks are containers, so
sprinklers reaching both a fire zone and a lighting zone are scoped to the
fire zone alone: two sharing only the lighting zone are not paired. A
reached object the selector cannot decide may be a shared container, so the
pair is undecided unless a surely picked one is shared.

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

**Door and window swings.** With `subject_extent` or `counterpart_extent`
`leaf_swing` (`door_swing` is the same extent under its older name), that
side is measured by its swing footprint instead of its body: the plan
sectors its side-hinged leaves sweep (a door's leaves, a window's
casements), and the rectangle each tilting window panel sweeps out to its
height along its opening direction, as the object-frame service states
them (`ObjectFrameService::leaves`). Only `horizontal` is
allowed, since a footprint is a plan shape; anything else is an invalid
declaration. Each sector is bracketed between an inscribed and a
circumscribed convex polygon (64 per quarter turn, a radial gap under
0.12 mm per metre of leaf), so the distance is the interval from the
circumscribed polygons' distance to the inscribed ones'. A swing measured
against a body goes through `measure_region_distance`; two swings are
measured against each other without geometry. A door or window whose
leaves neither swing nor tilt (sliding, rolling, fixed) sweeps nothing and
has no distance to anything. One whose leaves cannot be read, or an object
that is neither a door nor a window, is not evaluated as a subject and
unknown as a counterpart. A window panel tilting in a plane that is not
vertical (a skylight) is not evaluated. The broad phase is the
plan gap between the footprint's box and the counterpart's box grown by its
chord deviation, which bounds the distance from below. "No column within
0.5 m of a door's swing" is `none_closer_than` with `subject_extent`
`leaf_swing`; "no door swing within 0.5 m of another" declares both sides,
and "no casement opening over a walkway" selects the windows.

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
