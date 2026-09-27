# Model comparison

`axioval_rules::compare_sessions(base, revised, request)` compares two
evidence sessions object by object. The semantic facets are always compared;
placement, geometry and coordinate systems are compared when the request asks
for them, each with a tolerance, through typed host services only.

`axioval compare` runs it over two IFC revisions (see
[Command line](./cli.md#axioval-compare)).

## A host entry point, not a capability

A capability evaluates one rule over one session. A comparison needs two
sessions, each with its own snapshots and services, so it is not a registered
capability and has no rule definition to bind: it is a host entry point, like
`axioval compare`. Its result is projected into an ordinary `Report`, so
everything that reads reports (the saved result, `axioval report`, the BCF
sink) reads a comparison too.

## Matching

Objects are matched by an external identity scheme, never by `ObjectId`. A
local id is whatever the source file numbered an object, and every re-export
renumbers. `ComparisonRequest::new(scheme)` names the scheme. The engine
attaches no meaning to it: an IFC session supplies `GlobalId`s under the IFC
adapter's scheme, and another source supplies its own.

Each identity ends up in one of three states:

- **Added**: it is held only by the revised session.
- **Removed**: it is held only by the base session.
- **Matched**: it is held by both. The pair then carries its differences, any
  facets that could not be compared, and any undetermined measures. It is
  unchanged when all three lists are empty.

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
  properties only on request, so there is no way to list what they hold. Exact
  absence counts as a value. A resolver error, or a session without a resolver,
  leaves the property unresolved.
- **Relationships.** Targets are named by their identity in the scheme, so a
  renumbered target is not a change. A target without a unique identity leaves
  that relationship unresolved.

Values are compared exactly, except that a value equals itself (NaN included)
and the two zeros are equal.

## Spatial facets and tolerances

A `ComparisonTolerance` holds a length in metres and an angle in radians, both
finite and non-negative. Each spatial facet is opt-in with its own tolerance:

| Request | Facet | Service | Measures |
|---|---|---|---|
| `with_placement` | `placement` | `ObjectFrameServiceHandle` | `origin`: distance between frame origins; `orientation`: rotation between axis triples |
| `with_geometry` | `geometry` | `ProximityServiceHandle` | `bounds`: largest shift of any face of the axis-aligned bounds |
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
reshaping inside unchanged bounds (a hole cut, a profile changed within its
envelope) is not seen. That needs a certified two-sided distance between the
two surfaces, a Hausdorff distance, which the geometry kernel does not yet
provide (axiolid/kernel#148). The engine does not approximate one.

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

## Nothing is dropped

- An object without an identity in the scheme is **unidentified**: it cannot
  be matched, and it may be the one that changed.
- An identity claimed by several objects of one session is **ambiguous**. This
  can happen across sources, for example with a federated copy. The identity
  matches nothing on the other side, and the object it leaves unmatched there
  is reported with the ambiguity.

`is_identical()` is true only when every identity matched and every compared
facet agreed, with nothing unidentified, ambiguous, unresolved, undetermined
or unpaired.

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
| `RULE.identity` | not evaluated: an unidentified object or an ambiguous identity |

Findings are sorted by rule id, scope and message, and not-evaluated outcomes
by their full content, so the same two sessions give the same report.

## Open

- **Mesh difference.** A certified two-sided Hausdorff distance between two
  revisions of a body is axiolid/kernel#148. Until it is published, geometry
  compares bounds only.
- **Listing properties.** The IFC adapter cannot enumerate an object's
  properties (openbimrs/ifc#78), so only named properties are compared on IFC
  sessions.
- **Relationships of IFC sessions.** IFC relationships are answered on
  request through the relationship-selection service, not carried on the
  object, so the relationship facet compares nothing for them yet.
