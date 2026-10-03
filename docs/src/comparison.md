# Model comparison

A comparison sets two revisions of a model side by side, object by object.
The semantic facets are always compared; placement, geometry and coordinate
systems are compared when the request asks for them, each with a tolerance,
through typed host services only. It runs two ways:

- `axioval_rules::compare_sessions(base, revised, request)` compares two
  evidence sessions. It is a host entry point: `axioval compare` runs it over
  two IFC revisions, relationship kinds included (see
  [Command line](./cli.md#axioval-compare)), and
  projects the result into an ordinary `Report`, so everything that reads
  reports (the saved result, `axioval report`, the BCF sink) reads a
  comparison too.
- The `model-comparison` capability compares two sources of one session,
  named by their disciplines, as a rule beside other rules (see
  [The comparison as a rule](#the-comparison-as-a-rule)).

## Matching

Objects are matched by an ordered chain of matchers, never by `ObjectId`. A
local id is whatever the source file numbered an object, and every re-export
renumbers. `ComparisonRequest::new(scheme)` matches by one external identity
scheme; `ComparisonRequest::matching(matchers)` by several in turn, each
over the objects the ones before it left unmatched:

- `Matcher::Scheme(scheme)`: the same identity in an external identity
  scheme. The engine attaches no meaning to it: an IFC session supplies
  `GlobalId`s under the IFC adapter's scheme, and another source supplies
  its own.
- `Matcher::Property { base, revised }`: the same value of a property, read
  as `base` on the base revision and as `revised` on the revised one (a door
  number, for instance, when a re-export regenerates every `GlobalId`).
  Values are compared exactly; an absent, null or blank value is no key, and
  a measured interval is refused, since it identifies nothing exactly. A
  rule selecting storeys matches them with each other this way, by `Name`
  or `Elevation`.

Re-exported models often carry fresh identities. When both revisions are
sources of one session, so their bodies can be measured together, four
pairwise matchers fall back on the objects themselves, each only between
objects of the same kind:

- `Matcher::Geometry { tolerance_metres }`: the same body, the certified
  Hausdorff distance between the two surfaces within the tolerance (the
  same mapped geometry or boundary representation at the same place).
- `Matcher::Placement { tolerance }`: the same body placed alike: frame
  origins and axes within the tolerance, or both unplaced.
- `Matcher::Overlap { minimum_ratio }`: a certified shared volume of at
  least that share of the larger body's volume.
- `Matcher::Related { path, tolerance_metres }`: each reaches exactly one
  object along the relationship `path` (a door its opening), and the two
  reached objects are already matched or their surfaces coincide.

A pairwise match needs each object to be the other's one sure candidate,
with no undecided one: a candidate the measurement cannot decide (a
straddling interval, a refused measurement) or several sure ones leave
every object involved undecided, never guessed. An object whose body
cannot be measured leaves every object of its kind on the other side
undecided. Across two sessions a pairwise matcher leaves every object it
would judge undecided.

Each identity ends up in one of three states:

- **Added**: it is held only by the revised session.
- **Removed**: it is held only by the base session.
- **Matched**: it is held by both. The pair then carries its differences, any
  facets that could not be compared, and any undetermined measures. It is
  unchanged when all three lists are empty.

Matching fails closed. An identity held by several objects of one revision
matches nothing (see [Nothing is dropped](#nothing-is-dropped)). An identity
that cannot be read (a property resolver error) leaves its object
**undecided**, and with it every object of the other revision the same
matcher keyed but left unpaired, since it may be that object's partner; none
of them is matched further, reported added or removed. An object keyed by
some matcher but paired by none is added or removed, named by its first key;
one no matcher could key is unidentified.

## Semantic facets

- **Kind.**
- **Classifications.** When a session registers a `ClassificationService`, its
  assignments are used; otherwise the object's own classifications are.
  Suppose one session has the service and the other does not. One side then
  lists resolved chains and the other plain codes, so every difference would be
  an artefact, and the facet is reported unresolved.
- **Carried properties.** These are the properties on the object itself,
  keyed by set and name.
- **Requested properties.** `ComparisonRequest::with_property(set, name)`
  names a property to resolve through each session's
  `PropertyResolutionServiceHandle`. Sources such as the IFC adapter answer
  properties only on request rather than carrying them on the object. Exact
  absence counts as a value. A resolver error, or a session without a resolver,
  leaves the property unresolved.
- **Property sets.** `ComparisonRequest::with_property_set(set)` compares
  every property of a set, and `with_all_property_sets()` every property of
  every set, listed on both sides through the resolver's property
  enumeration. A property present on one side only appeared or disappeared,
  and a set present on one side only is one difference, `property set NAME
  added` or `removed`, not one per property.
  An enumeration the resolver refuses leaves that set unresolved; a property
  also named with `with_property` is compared through resolution alone.
- **Relationships.** Relationships an object carries (`Object.relationships`)
  name their targets by identity in the scheme, so a renumbered target is not
  a change. A target without a unique identity leaves that relationship
  unresolved. Sources that answer relationships on request, such as IFC, are
  compared per [relationship kind](#relationship-kinds) when the request asks
  (`ComparisonRequest::with_relationships()`).

Values are compared exactly, except that a value equals itself (NaN included)
and the two zeros are equal.

### Relationship kinds

`ComparisonRequest::with_relationships()` compares, for every matched pair,
the objects related to each side through each `RelationshipKind`:
`containment`, `aggregation`, `voids`, `fills`, `space-boundary`, `type`,
`group` and `connection`. A related object is any object at the other end of
an edge of the kind, whichever end the matched object holds: a door's
`containment` is its storey, a storey's the elements it contains. Each
revision lists every kind's edges among its objects once, through its
relationship-selection service (`RelationshipSelectionServiceHandle::edges`),
so the cost follows the number of edges, not the number of pairs. The IFC
adapter maps each kind onto one relationship type (see
[Adapters](./adapters.md)).

Related objects are mapped through the matching, never by `ObjectId`:

- a **matched** related object is named by its pair's identity, so the two
  sides' sets compare directly, and a difference is a change:
  `containment -[S1] +[S2]` for a door moved from storey `S1` to `S2`;
- an object the comparison does not match (outside the rule's selection) is
  named by its unique identity in the first scheme matched by, as carried
  relationship targets are;
- a related object that is itself **unmatched** (added or removed) is never
  a change of the relationship: the revision may have regenerated the same
  object. It is listed as unmatched, and the kind is unresolved beyond the
  matched related objects;
- a related object whose match is **undecided** (unidentified, ambiguous or
  undecided itself) leaves the kind unresolved the same way, naming it.

A kind the service refuses (an unknown kind, a malformed relationship, an
absent required end) or a session without a relationship-selection service
leaves that kind unresolved for every matched object, never compared as
empty. An unchanged model reports no relationship differences.

## Spatial facets and tolerances

A `ComparisonTolerance` holds a length in metres and an angle in radians, both
finite and non-negative. Each spatial facet is opt-in with its own tolerance:

| Request | Facet | Service | Measures |
|---|---|---|---|
| `with_placement` | `placement` | `ObjectFrameServiceHandle` | `origin`: distance between frame origins; `orientation`: rotation between axis triples |
| `with_geometry` | `geometry` | `ProximityServiceHandle` | `bounds`: largest shift of any face of the axis-aligned bounds |
| `with_mesh_geometry` | `geometry` | `ProximityServiceHandle` | `mesh`: certified two-sided Hausdorff distance between the two surfaces; `boundary`: the same between the two exact boundaries, where both bodies have one |
| `with_coordinate_systems` | `coordinate-system` | `CoordinateSystemServiceHandle` | `world-origin`, `world-orientation`, `true-north`, `map-offset`, `map-rotation`, `map-scale` |

A session without the service a requested facet needs leaves that facet
unresolved for every matched object (or source), never compared as equal.

### Three outcomes, never rounded

Every measure is an interval the true difference is known to lie in:

- **changed** when the whole interval lies above the tolerance;
- **unchanged** when the whole interval lies within it;
- **undetermined** otherwise. An undetermined measure is neither a change nor
  agreement: it is listed on the pair and reported not evaluated.

Placement frames and coordinate systems are stated, exact evidence, so their
intervals are points and never undetermined. The rotation between two frames
is computed from the difference of their axis matrices (`2·asin(‖A−B‖/2√2)`),
which stays accurate for the small angles a tolerance is about.

### Placement

A frame is the object's placement as its source states it (see
[typed host services](./services.md)). Both unplaced is no difference; placed
on one side only is a `placement` difference; any other refusal (an
unsupported placement, an unreadable unit) leaves the facet unresolved.

The placement's differences tell the kind of move apart: `origin` is a
move, `orientation` a rotation, and `mirroring` a mirroring. A frame is
right-handed by contract, so no rotation mirrors; a mirroring is the sign of
the determinant of the transform placing the body, which the body facts
state as `axioval:body.Mirrored` (see [IR](./ir.md)). An object without body
facts is not mirrored; a refused or missing fact leaves `mirroring`
unresolved.

### Geometry

Geometry compares the extent of each object's measured body. An exact mesh is
the body, so its bounds are exact. A tessellated mesh approximates curved
faces within its chord deviation, which the comparison reads as a two-sided
bound: a tessellation's vertices lie on the surface and every surface point
lies within the deviation of the mesh. Each true bound therefore lies within
the deviation of the measured one, and the shift of the true bounds lies
within the measured shift widened by the sum of both deviations. A
tessellated body compared with a tolerance below that sum can be undetermined
even when nothing moved; that is the evidence, not a defect.

Both bodiless (`ProximityError::NoBody`, such as a storey) is no difference;
a body on one side only is a `geometry` difference; an unmeasured body leaves
the facet unresolved.

Bounds see a move, a resize and a reshaping that changes the extent. A
reshaping inside unchanged bounds (a hole cut, an opening moved along its
wall, a profile changed within its envelope) is not seen by them; the mesh
mode sees it.

#### Mesh

`ComparisonRequest::with_mesh_geometry(tolerance)` (`GeometryMode::Mesh`)
compares the two surfaces themselves: the certified two-sided Hausdorff
distance between the base body and the revised body, in world coordinates,
judged against the length tolerance as measure `mesh`. The revised session's
proximity service hands out its body's surface (`body_surface`), and the
base session's service measures its own body against it
(`measure_surface_distance`), so the two revisions may be two sessions. The
distance is the farthest any point of either surface lies from the other: an
opening moved 0.5 m along a wall leaves the old opening's far reveal 0.5 m
inside the new opening, so the wall has changed though its bounds have not,
and a re-export of the same shape is zero apart up to rounding, unchanged.
The interval is refined to a tenth of the tolerance where the kernel can,
and judged like every measure: one straddling the tolerance is undetermined
and reported not evaluated.

A changed `mesh` finding carries its witness as evidence,
`comparison:witness:mesh:base(x,y,z)->revised(x,y,z)` (or the other way
round): the point of one body that strays farthest found, which is at least
the lower bound from the other body, and its nearest point found on that
body, in metres, citing the source of the body the first lies on. The
witness is also the measurement's `witness` (`Witness`).

Only exact surfaces are certified. A tessellation bounds how far its true
surface lies from the mesh, not how far the mesh lies from the true surface,
so a body on either side registered as a tessellation, without an exact
boundary on both sides, leaves the `geometry` facet unresolved (`mesh` not
compared), never unchanged. Bodiless and unmeasured bodies are judged as in
the bounds mode.

#### Exact boundaries

Where both revisions of a body have an exact boundary registered beside
their meshes (`BodySurface::exact_boundary`, an `ExactBoundaryHandle` the
engine never reads), the base session's service measures between the two
boundaries instead of the meshes, whatever the meshes' fidelity, so a round
column or any other curved body is compared too. The evidence says so
(`SurfaceDistanceEvidence::basis`, `SurfaceBasis::ExactBoundary`) and the
distance is judged as measure `boundary`: `geometry changed: geometry
boundary differs by …`, with the witness as
`comparison:witness:boundary:…` and both boundaries cited as exact
evidence, `comparison:exact-boundary:<object>` on each side's source. A
service that cannot read the counterpart's boundary, or whose kernel
refuses the pair, measures the meshes as above. So does a boundary the
Axiolid service built from operands its kernel moved within a tolerance
(an opening flush with a turned wall's face): it is not exactly the
model's surface. A wall less its openings or clipped by its roof that the
kernel built deciding nothing within its tolerance (axiolid/kernel#236) is
the exact boolean of its operands and is compared between its boundaries,
the interval widened by the rounding the boolean merges points within
(2^-40 of its operands' largest coordinate): a wall less a door and a
window moved 5 mm measures 5 mm. A body of several items is compared by
the boundary of their union, which the kernel forms only for items apart,
touching without a shared patch or sharing a face on an axis plane of the
body's frame; any other layout measures the meshes.

An identical re-export and a moved copy match face to face, a translate
whatever coordinates its faces are trimmed in (axiolid/kernel#227), and
close to the accuracy asked: a round column moved by 1 mm measures 1 mm
to within a micrometre, changed against a 0.5 mm tolerance and with meshes
alone not compared at all. A distance between exact boundaries is always
sound, but it may come back wider than asked: two boundaries the kernel
cannot match face to face (a turned or reshaped copy) close only at first
order, and the Axiolid service stops each direction after 4096 splits
(`BOUNDARY_HAUSDORFF_SPLITS`), so one pair never costs more than a few
seconds. Since the interval is sound however wide, it is judged like every
measure: changed when all of it lies above the tolerance, unchanged when
none of it does, otherwise undetermined and not evaluated; its width never
decides. A square column turned by 0.01 rad, whose corners move 2 mm,
comes back between about 2 mm and 12.6 mm: changed against a 1 mm
tolerance, undetermined against 5 mm.

### Coordinate systems

Coordinate systems belong to sources, not objects. The comparison pairs the
base source with the revised one when each session holds exactly one;
otherwise it pairs the sources that declare the same
[discipline](./cli.md#several-models-and-disciplines), one per side. A source
left without a pair is reported not evaluated.

For each pair it compares what the sources state: the world frame's origin
and orientation, the angle between true-north directions, and the map
conversion's target reference system (by name), offset, rotation and scale.
A statement made on one side only is a difference. The map offset is compared
in metres only when both map units are known exactly; otherwise equal
statements agree and different ones are unresolved. The scale is compared
exactly.

### Timestamps

`ComparisonRequest::with_timestamps()` compares each pair of sources' header
timestamps, the `timestamp` source metadata (for IFC `FILE_NAME.time_stamp`).
A revised source written surely before its base is a `timestamp` finding of
severity error, whatever the rule's severity: the two may have been swapped.
A timestamp without a UTC offset may lie in any zone, so it stands for every
instant fourteen hours either side; two that may be in either order, a
timestamp not read, missing, stated twice or not a date-time leave the facet
unresolved.

## Nothing is dropped

- An object no matcher can key is **unidentified**: it cannot be matched,
  and it may be the one that changed.
- An identity claimed by several objects of one session is **ambiguous**. This
  can happen across sources, for example with a federated copy. The identity
  matches nothing on the other side, and the object it leaves unmatched there
  is reported with the ambiguity.
- An object whose identity cannot be read, or that may match such an object,
  is **undecided** (`ModelComparison::undecided`).

`is_identical()` is true only when every identity matched and every compared
facet agreed, with nothing unidentified, ambiguous, undecided, unresolved,
undetermined or unpaired.

## Reports

`ModelComparison::report(rule_id, severity)` projects the comparison into a
`Report`, so it can travel through any sink, BCF included. Each entry's rule
id is `rule_id` and a suffix naming what it is about:

| Rule id | Entry |
|---|---|
| `RULE.added` | finding on the revised object |
| `RULE.removed` | finding on the base object |
| `RULE.<facet>` | finding per changed facet on the revised object, its base object in `related`; the message lists that facet's differences |
| `RULE.coordinate-system` | finding scoped to the revised source when its coordinate system changed |
| `RULE.<facet>` | not evaluated: a facet not compared, or an undetermined measure |
| `RULE.identity` | not evaluated: an unidentified, ambiguous or undecided object |

Findings are sorted by rule id, scope and message, and not-evaluated outcomes
by their full content, so the same two sessions give the same report.

## The comparison as a rule

`axioval:capability.model-comparison` compares two models of one run, so a
comparison travels in a ruleset beside other rules and its findings in the
same report. The models are sources of the session, named by the discipline
each declares: `axioval check --model a.ifc:base --model b.ifc:revised`
with `base: base` and `revised: revised`. Exactly one source must declare
each discipline; none (with some declaring nothing), none at all, or several
leave the rule not evaluated, and `base` equal to `revised` is an invalid
declaration.

| Parameter | Kind | Meaning |
|---|---|---|
| `base`, `revised` | `string` | Required. The disciplines of the base and the revised model. |
| `identity_scheme` | `string` | Match by this external identity scheme (`ifc-globalid` for IFC). |
| `identity_property` | `propertyReference` | Match by this property's value, such as a door number. |
| `revised_identity_property` | `propertyReference` | The property read on the revised model instead, when it states the identity elsewhere. Needs `identity_property`. |
| `match_by` | `stringList` | The order of the matchers: `identity` (the scheme) and `property`. By default the scheme, then the property, whichever are declared. At least one matcher is required. |
| `properties` | `table` | Rows of `property_set` (optional) and `property`: properties compared through resolution. |
| `property_sets` | `table` | Rows of `property_set`: sets compared whole through enumeration. |
| `all_property_sets` | `boolean` | Compare every set whole. |
| `match_by` values `geometry`, `placement`, `overlap`, `related` | | The pairwise matchers, named in `match_by` only. |
| `match_length_tolerance` | `number` | Metres, for `geometry`, `placement` and `related`; default 0.001. |
| `match_angle_tolerance` | `number` | Degrees, for `placement`; default 0.01. |
| `minimum_overlap_ratio` | `number` | Required with `overlap`: above 0, at most 1. |
| `match_path` | `stringList` | Required with `related`: the relationship steps, as a traversal `path` names them, such as `IfcRelFillsElement:forward`. |
| `revised_selector` | `selector` | Restricts the revised model instead of the rule's selector. |
| `compare_placement`, `compare_geometry`, `compare_coordinate_systems` | `boolean` | The spatial facets, as above. |
| `compare_timestamps` | `boolean` | The header timestamps, as above. |
| `compare_relationships` | `boolean` | The related objects per [relationship kind](#relationship-kinds). |
| `length_tolerance` | `number` | Metres; default 0. |
| `angle_tolerance` | `number` | Degrees; default 0. |
| `geometry` | `string` | Compares geometry in this mode: `bounds` (as `compare_geometry`, within `length_tolerance`) or `mesh` (the certified surface distance, see [Mesh](#mesh)). Contradicting `compare_geometry: false` is an invalid declaration. |
| `tolerance_metres` | `number` | Required with `geometry: mesh`, and only with it: the largest surface distance that counts as unchanged. |

`geometry: mesh` with `tolerance_metres: 0.01` reports a wall whose opening
moved 0.5 m as `geometry changed: geometry mesh differs by …`, its witness
points as evidence, and leaves a re-exported identical wall unchanged.

The rule's selector restricts the objects compared, on both sides. An object
the selector cannot decide is compared all the same; a change involving only
such objects (an added or removed one, or a pair of two) is not evaluated,
never reported, while a pair with a selected object is reported. Findings
carry the rule's own id and severity and name what they are about in the
message: `added (MATCHER:IDENTITY)` on the revised object,
`removed (MATCHER:IDENTITY)` on the base object, and `<facet> changed: …`
on the revised object relating its base object. Not-evaluated outcomes are
those of the [report](#reports) under the rule's id.

## Open

- **Tessellated mesh difference.** The mesh mode certifies exact surfaces
  only: a curved body without an exact boundary on both sides (an ellipse,
  a revolution, a sweep, a boolean) stays unresolved in that mode.
- **Turned boundaries.** Boundaries the kernel cannot match face to face
  (a turned or reshaped copy) close only at first order, so a change close
  to the tolerance may come back straddling it at the split cap (not
  evaluated). Moved copies close exactly.
