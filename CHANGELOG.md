# Changelog

All notable changes are documented here. This project follows Semantic Versioning and Keep a Changelog.

## [Unreleased]

### Added

- **Clash tolerance cases along the elements' own axes.** (#135)
  `ProximityService::measure_overlap_along` answers the extents of two
  bodies' intersection along stated directions (`OverlapAlongRequest`,
  `OverlapAlongEvidence`), refused by default; the Axiolid adapter
  measures them from the same witnesses as the world-axis extents, the
  upper bound the overlap of the bodies' own ranges, widening every
  projection off a coordinate axis by its rounding. `clash` and
  `clash-matrix` take `tolerance_cases`: rows of a case
  (`horizontal_orthogonal`, `vertical_orthogonal`,
  `horizontal_protrusion`, `vertical_protrusion`), two component filters
  and a tolerance, measured along the second element's or the first
  element's placement axes from the object-frame service. A slab edge sunk
  10 mm into a wall standing at 30° passes a 20 mm orthogonal case and is
  a hard clash without it. An undecided filter, frame or extent leaves the
  pair not evaluated. **Breaking:** definitions bound to `clash` or
  `clash-matrix` must declare the new optional parameter;
  `ProximityService` gains a defaulted method.
- **Clash severities by class and size.** (#97) `clash` and
  `clash-matrix` take `severity_by_class` (a severity per class:
  duplicate, containment, intersection, clearance) and grade
  intersections by their smallest extent or shared volume (`grade_by`,
  `severity_grades`): an intersection past a grade's `above` takes the
  highest such grade's severity, so a 10 mm sliver can be graded below a
  300 mm intersection. A straddling or unmeasured measure takes the most
  severe grade it may reach, and the message says so. A matrix cell's own
  severity wins over the class's; a grade wins over both. A duplicate's
  finding names what the copies differ in: their types, their measured
  volumes, and the quantities `duplicate_quantities` names, an unreadable
  one named as unknown. **Breaking:** definitions bound to `clash` or
  `clash-matrix` must declare the four new optional parameters, and every
  duplicate finding's message gains the comparison.
- **Grouped clash findings.** (#96) `clash` and `clash-matrix` take
  `group_by`: `subject` (every pair of one subject), `type_pair` (every
  pair of the same two object types) or `similar` (pairs of one class and
  the same two types, and values of `group_property`, whose intersection
  extents round alike to `group_tolerance_metres`), with `per_storey` and
  `storey_path` keeping storeys apart. Each group is one finding on the
  object most of its pairs involve, relating every other object, at its
  most severe severity, carrying each pair's evidence: a duct through
  five identical walls on one storey is one finding relating six objects
  with five evidence entries. A pair whose key cannot be read is reported
  on its own with the reason; a matrix never groups across cells. Without
  `group_by` the output is unchanged. **Breaking:** definitions bound to
  `clash` or `clash-matrix` must declare the five new optional
  parameters.
- **Clash exclusions across federated models.** (#136) `clash` and
  `clash-matrix` take `exclude_target_property`: the targets an exclusion
  path (or the matrix's `system_path`) reaches from the two members also
  meet when they state the same value of that property, so a duct and a
  pipe in two models, assigned to systems both named `SUP-01`, are one
  system. Only reached targets are compared, never the members; an
  unreadable value that could match leaves the pair not evaluated.
  `exclude_same_layer` now holds only within one model: the same layer
  name in two models no longer excludes a pair. **Breaking:** definitions
  bound to `clash` or `clash-matrix` must declare the new optional
  parameter.

- **Severity bands.** (#90) A rule instance may declare `severityBands`:
  ascending thresholds of the relative deviation `|value - bound| /
  |bound|`, each with a severity, the rule's own severity standing beyond
  the last. The runtime grades every finding a capability reports with its
  deviation (`CapabilityEvaluation::push_graded_finding`,
  `axioval_engine::Deviation`); a deviation interval straddling bands
  takes the most severe band it may reach and says so in its message.
  `plan-area`, `area-ratio`, `related-count`, `keyed-limit`,
  `table-allocation`, `space-distance`, `stair-geometry` and
  `ramp-geometry` report deviations. Compilation refuses bands on a
  capability that reports none (`RuleCapability::grades_deviation`) or
  bands that do not ascend. **Breaking:** `RuleInstance` gains
  `severity_bands` (omitted when empty, so packages are unchanged),
  `EngineError` gains `InvalidRefinement`, and
  `ExecutionPlan::refinement` exposes what a rule declares.
- **Object counts among the sources of a discipline.** (#104)
  `object-count` takes `disciplines`, a string list: only the sources
  playing one of them are counted, so a per-source duct count limited to
  `mep` over an architecture model and two MEP models reports only the MEP
  model without ducts. A source declaring no discipline is not evaluated
  per source, and its matching objects are undecided across sources; no
  source playing a listed discipline leaves the rule not evaluated.
  `SourceDisciplines::new` is public for capability tests. **Breaking:**
  definitions bound to `object-count` must declare the optional
  `disciplines` (`stringList`).
- **Several rulesets in one check.** (#103) `compile_rulesets` compiles
  several rulesets into one plan, each against its own definition packages
  exactly as `compile` compiles it, and qualifies every rule id by its
  ruleset's package id (`package-id/rule-id`), so two rulesets may both
  define `r1` and report both findings under distinct ids, grouped by
  package in rule id order. One ruleset keeps its ids. The CLI's `check`
  and `validate` take `--ruleset` repeatedly. Two rulesets sharing a
  package id are refused (`EngineError::DuplicateRuleSet`), as is an empty
  list (`EngineError::NoRuleSet`).
- **Disciplines from source metadata.** (#102) A `DisciplineMap` of
  ordered wildcard rules over a source metadata field
  (`EvidenceSession::with_discipline_map`) assigns a discipline to each
  source that declares none: the first rule matching one of the field's
  values wins, a declared discipline is never replaced, and a source whose
  field a rule reads was never stated keeps none. The session records each
  assignment (`DisciplineOrigin::Mapped`: rule and value), readable through
  `SourceDisciplines::origin`, and the `discipline` selector cites it as
  inexact evidence. The CLI takes repeatable `--discipline-map
  FIELD:PATTERN=DISCIPLINE` (`application:*Architecture*=architecture`)
  and lists every source with its discipline and its origin in the
  result's new `sources` field. The `like` wildcard translation moved to
  `axioval_engine::wildcard_regex`. **Breaking:** `SourceDisciplines` is
  a struct of disciplines and origins.
- **Empty and not-empty, classification patterns, source metadata.** (#101)
  Selectors gain the operators `isEmpty` (present but null, blank or a
  list of nothing else) and `isNotEmpty` (present with a value), with the
  meaning `property-requirements` gives `presence` `empty` and
  `not-empty`; an absent property is neither. A `classification` selector
  takes `codePattern`, an XML Schema pattern over the whole code
  (`Ss_25_.*`), and without `code` or `codePattern` selects any
  classification in its system. A new `source` selector compares a
  source's metadata (`fileName`, `application`, `schema`, `project`) as a
  property selector compares a value, so `application like
  "*Architecture*"` selects the objects of the models an architecture
  application wrote; a field never read is not evaluated, once per source.
  `EvidenceSession::with_source_metadata` states `SourceMetadata` field by
  field and the runtime installs it per run as `SourceMetadataIndex`. The
  IFC adapter reads the authoring applications (`IfcOwnerHistory`'s
  `OwningApplication`) and the project names; the CLI states each model's
  file name. **Breaking:** `Selector::Classification`'s `code` is an
  `Option` beside the new `code_pattern`; `Selector` gains `Source`,
  `ComparisonOperator` gains `IsEmpty` and `IsNotEmpty`, and
  `EvidenceSessionError` gains `ConflictingMetadata`.
- **Centre lines beside walls.** (#130) The capability
  `centre-line-distance` judges the distance from the centre line of a
  footprint's least-area rectangle (its `long` or `short` axis, or
  `against-wall`, from the wall the component stands against to its
  front) to the nearest `wall_selector` wall beside it, within `reach`:
  the nearer of the two sides (`nearest`) or each side (`both`), against
  `minimum` and `maximum`. Results are "too close", "too far" and "no
  wall nearby"; a straddling interval is not evaluated. It reads the
  walls through `PlanSpanService::measure_side_distances`.

- **A front against the wall.** (#128) `component-clearance` takes
  `front_axis` `against-wall`, with `wall_selector`, `wall_reach` and
  `wall_inset`: the sides are those of the footprint's least-area
  rectangle, the side whose nearest wall (from the side) is surely nearer
  than every other side's is the back, and the front faces away from it,
  so one rule serves fixtures whose placements are turned any way. A tie,
  or no wall within reach, is not evaluated (`front not decided`), never
  guessed; the derived front is cited as inexact evidence naming the
  wall. `PlanSpanService` gains `measure_side_distances`
  (`SideDistanceRequest`, `SideDistances`, `SideDistance`,
  `RectangleSide`, `SidePresence`): per side of the least-area rectangle,
  every candidate that may lie in the strip beside it, surely or possibly,
  with its least distance from the centre line as an interval; the
  default refuses. The Axiolid plan-span service answers it from the
  candidates' plan triangles, never exactly. **Breaking:** definitions
  bound to `component-clearance` must declare the optional parameters
  `wall_selector`, `wall_reach` and `wall_inset`.

- **Window leaves.** (#36) `ObjectFrameService::leaves` answers a window's
  panels as leaves, as it answers a door's: the IFC adapter derives them
  with `openbim-ifc`'s `window_operation` (openbimrs/ifc#170) from the
  window's placement, partitioning (IFC2X3 `IfcWindowStyle`), panel
  properties and lining offsets. A side-hung casement swings like a door
  leaf, in the plane of its bottom edge; each panel states its height,
  and a top-, bottom-hung or tilt-and-turn panel its tilt sector
  (`DoorLeaf::tilt`). `distance`'s swing extent is now `leaf_swing`,
  measuring a door's or window's swing footprint, a tilting panel's plan
  rectangle included; `door_swing` still works. **Breaking:**
  `LeafMotion` gains `TiltAndTurn`, `Tilt` and `Removable`, and
  `LeafPosition` gains `Bottom` and `Top`; `LeafMotion::is_hinged` holds
  for tilt-and-turn panels, and a sliding leaf may slide along its height.
- **Openings in free outlines.** (#65) The body set states an arbitrary
  profile's outline as `Profile.OutlineX` and `Profile.OutlineY`, lists of
  vertex coordinates in the profile's plane, and each void's as
  `Profile.Void<n>.OutlineX` and `.OutlineY`; the IFC adapter reads them
  through `ifc-geometry`'s `profile_outline` (openbimrs/ifc#166) from
  polylines and line-only indexed poly curves, and refuses them for an
  outline with an arc. `opening-zone` and `opening-area` read hosts,
  openings and supports of such outlines exactly: an opening's extent is
  its outline's reach, its area the outline's less its voids, and against
  a host of a free outline (a wall mitred at its end) an opening must lie
  inside the outline over the depth at which it passes through, its
  distance from the ends and edges measured to the outline there. Only an
  opening extruded straight through the host is placed there exactly;
  another can pass but never be found. An outline that is not one region,
  or states no vertices because an edge is curved, is not evaluated.
- **Openings beside supports, and gross against net wall area.** (#65)
  `opening-zone` finds each host's supports and connecting members by
  `support_path` (with IFC, `IfcRelConnectsElements:either`, which takes
  in `IfcRelConnectsPathElements`) and by contact through the proximity
  service (`support_gap`), among the `support_selector` objects, and
  requires each opening to keep `support_distance` from every support
  along the host and `support_clearance` from every footprint in its
  face. Supports are read from the body set: a straight extrusion of any
  bounded section, its extent along a face axis an inner and an outer
  interval, the same where its outline is exact. A finding needs the
  inner one, a pass the outer; a position in between, an unreadable
  support, or an undecided contact, relationship or selection that may
  come too close leaves the opening not evaluated. The new
  `opening-area` requires the openings a wall hosts to cover its stated
  gross side area less its net side area (`gross_area`, `net_area`, such
  as `Qto_WallBaseQuantities`) within `area_tolerance`, each counting the
  exact area of its section where it crosses the wall's middle plane. A
  wall with an opening it cannot place, openings that may overlap, or
  only one area stated is not evaluated; one stating neither is not
  checked. **Breaking:**
  definitions bound to `opening-zone` must declare the new optional
  parameters `support_path`, `support_selector`, `support_gap`,
  `support_distance` and `support_clearance`.
- **Openings in members and walls.** (#65) `opening-zone` places each
  opening in its host's face (`host_path`, `host_selector`; the face
  spanned by `length_axis` and `height_axis`, two of `extrusion`,
  `profile-x`, `profile-y`) and requires it to lie within the host, and
  optionally `end_distance` from the host's ends, `edge_distance` from its
  edges or (with `zone` `web`) its flanges, and `opening_spacing` clear of
  the other openings of the host. It reads both bodies from the reserved
  body set: an opening's extent is the exact reach of its rectangle,
  rounded rectangle, circle or ellipse swept along its extrusion; a host
  must be a straight extrusion of a bounded section. Spacing is exact
  between axis-aligned rectangles extruded through the host and otherwise
  only passes. Anything it cannot place soundly is not evaluated.
- **Allowed profiles.** (#64) `allowed-profile` requires each member's
  body to be one swept solid whose profile is a row of a `profiles` table:
  its family (`type`), its name where the row states one (`HEA*`), and
  every dimension the row states (`width`, `depth`, `web_thickness`,
  `flange_thickness`, `thickness`, `wall_thickness`, `radius`, `girth`,
  `fillet_radius`, named alike across families) within the rule's or the
  row's `tolerance`. Findings tell wrong geometry (no body, several items,
  no swept profile), an arbitrary profile and a profile no row fits apart,
  and the last names the nearest row and each dimension it is off in. It
  reads the reserved body set and needs no geometry service. A refused
  dimension leaves the member not evaluated unless another row fits; a
  derived profile is not evaluated, a mirrored one is judged by its parent.
- **How a body is modelled.** (#40) The reserved `axioval:body` set
  (`BODY_SET`) states, from the body representation a source authors and
  never from a mesh, the number of geometric items (`Count`), their kinds
  (`Kinds`, and `Item<n>.Kind`: `extrusion`, `brep`, `csg`,
  `tessellation` and so on) and whether they are mapped (`Mapped`), and
  for each swept-area item its profile (`Profile.Type`, `Profile.Name`,
  the family's dimensions such as `Profile.OverallDepth` in metres, and
  its position), its placement (`Placement.Origin…`, `Placement.XAxis…`)
  and its path (`Extrusion.Depth`, `Extrusion.Direction…`,
  `Extrusion.Inclination`; `Revolution.…`). Names without `Item<n>.` read
  the only item and are a conflict on a body of several. Every property
  capability and selector reads them, so a column's section depth or a
  wall's extrusion direction is an ordinary property rule. The IFC adapter
  answers the set through `ifc-geometry`'s `body_description`
  (openbimrs/ifc#147), mapped items resolved, in the project's exact units
  and world coordinates, with a locator naming the representation, the
  mapped items followed, the item and the profile. A body it cannot
  describe exactly is refused for every name, never answered in part; an
  object without a body, an unset optional attribute and another family's
  parameter are exact absences.
- **IDS property facets named by pattern, and composite values.** (Closes
  #44) A rule naming its sets by pattern needs a match in every set the
  pattern matches that holds a property, as IDS requires:
  `property-value` with `property_set_pattern` reports each such set
  without a match, and an included `property-requirements` row naming its
  set reports the first one as a `missing property`. The staging IDS
  translator enumerates property facets whose set or name is an
  `xs:pattern` or an enumeration of several names (an escaped alternation;
  an enumeration narrowed by patterns is its matching names): a value
  becomes `property-value` with the pattern parameters, a required
  presence a `property-requirements` row with the pattern columns, and a
  prohibited one an excluded `not-empty` pattern row. Every property value
  is judged under `quantifier` `any`, or `all` for a range restriction,
  with `si_units`. Its corpus moves from ExactPass 127, SoundPass 42,
  CaughtFail 81, UnjudgedFail 26, NotEvaluated 31 to ExactPass 141,
  SoundPass 37, CaughtFail 88, UnjudgedFail 23, NotEvaluated 18, with 0
  mismatches. The name-pattern and name-enumeration gaps are gone;
  applicability property facets and prohibited facets with a value stay
  gaps. A matching set holding no property is invisible to enumeration, so
  a required facet does not fail on it where IDS would.

- **Property sets and properties named by pattern.** (Refs #44) The
  property service enumerates an object's properties:
  `PropertyResolutionService::enumerate` answers a
  `PropertyEnumerationRequest` (a `NameMatch` for the set and for the
  property: `Any`, `Exact` or a whole-name `NamePattern`) with a
  `PropertyEnumeration`: every selected property, sorted and distinct,
  each with exact evidence, and exact evidence that there is no other, so
  an empty one proves absence. The trait's default refuses; reserved sets
  are never enumerated; the handle binds the answer to its request and
  federation routes it by source. The IFC adapter enumerates through
  `ifc-properties`' `exact_properties_where` (openbimrs/ifc#78), with the
  traversal and refusals of `exact_property`; an unselected complex
  property no longer refuses an answer about its neighbours. Rules name
  sets and properties by XML Schema pattern, matched against the names the
  source states and never bound as concepts: the new `propertyPattern`
  selector compares every matching property under `matched` (`any` or
  `all`), no match being no match; `property-value` takes
  `property_pattern` and `property_set_pattern` instead of `property`,
  each matching property meeting the constraints and one having to match
  unless optional; `property-requirements` reads wildcards in its
  `property_set` and `property` cells and XML Schema patterns in the new
  `property_set_pattern` and `property_pattern` columns, checks an included
  statement against every match and an excluded one against none (so a
  forbidden pattern is decided), and answers set-only rows by whether the
  set holds a property. A required property whose named set holds no
  property is reported as `missing property set`, and a forbidden set
  present as `forbidden property set present`; a source that cannot
  enumerate leaves pattern and set rows not evaluated per object instead
  of once per rule.
  **Breaking:** `Selector` has the variant `PropertyPattern`;
  `PropertyEnumeration`, `PropertyEnumerationRequest`, `NameMatch` and
  `NamePattern` are new, and `axioval-engine` depends on `regex`.
  Definitions bound to `property-value` must declare `property` as
  optional and the optional `property_set_pattern` and `property_pattern`
  (`string`); definitions bound to `property-requirements` must declare the
  optional columns `property_set_pattern` and `property_pattern`
  (`string`). A required property missing together with its whole set now
  reads `missing property set: <set> is absent` where the source can
  enumerate. The MCS schema does not yet declare `propertyPattern`.

- **Enumerated, list, bounded and table property values.** (Refs #44)
  `PropertyValue` gains `Bounded { lower, upper, set_point }` (a range, each
  part an optional scalar, at least one stated, an unstated bound leaving
  the range open) and `Table` (rows of a `defining` and a `defined` value,
  `PropertyTableRow`). `PropertyValue::stated_values` lists the scalars a
  list, bounded value or table states, and `is_scalar` tells a scalar from
  null and the composites; the property handle refuses a range without a
  part or of mixed kinds, an empty table, and a non-scalar part or cell.
  The IFC adapter maps the values `ifc-properties` 0.4.1 reads
  (openbimrs/ifc#150): an `IfcPropertyEnumeratedValue` is its selected item,
  a list of them when several are selected and null when none is; an
  `IfcPropertyListValue` a list; an `IfcPropertyBoundedValue` a bounded
  value; an `IfcPropertyTableValue` a table, each column in its own unit.
  Every scalar is read as a single value of its declared type (measures in
  SI through the unit the kind states), and the declared type is reported
  when every scalar shares it. A predefined set's enumeration attribute is
  its constant as text, typed by its enumeration, and an unset optional
  attribute is null with its declared type. `IfcPropertyReferenceValue`,
  entity-valued attributes and complex properties stay refused.
  `property-value` takes `quantifier` (`any` or `all`), which judges a
  list, bounded value or table by its stated values (a range under `all`
  also by its open ends, which fail every bound on their side), and
  `si_units`, which reads numeric literals compared with a quantity in the
  coherent SI unit of its dimension; without them such values stay not
  evaluated. Property selectors quantify over a bounded value's or a
  table's stated values as over a list's, and `property-requirements`
  judges them the same way, a range open on a required bound's side being
  a `wrong value`. `property-value` also reads an empty list as missing.
  **Breaking:** `PropertyValue` has two more variants; definitions bound to
  `property-value` must declare the optional parameters `quantifier`
  (`string`) and `si_units` (`boolean`). Previously refused IFC values
  (enumerated, list, bounded and table values, enumeration and unset
  predefined attributes) now resolve.

- **Turning flights' landings and handrails.** (Closes #85) A turning
  flight's landing is placed along its end tread's direction, square to
  that tread's nosing, in plan positions rather than arc lengths, and
  `landing_at_least_walking_width` compares it with that tread's width; an
  end on a winder stays refused. Its handrails are measured in its
  straight parts: `HandrailEvidence::try_in_parts` takes `StretchPart`s
  (direction and sides) and each `RailMeasurement` is measured `in_part`;
  sides come from the rail's part, the extension beyond an end only from a
  rail along the part holding it, and a side's pieces are ordered part by
  part. The Axiolid adapter gives consecutive treads whose nosings are
  parallel within their uncertainty one frame, assigns each rail to the one
  part it runs straight along on one side, and measures its height above
  the nosings' ends on that side; around a turn the pitch line is bounded
  by the neighbouring treads' elevations. A rail at an angle to every part,
  over the middle, or along a tessellated turning flight whose parts turn
  with its chords refuses. **Breaking:**
  `HandrailEvidence::bottom_extension` and `top_extension` return an
  `Option`; `WalkingEnd::FlightBottom` and `FlightTop` and
  `WalkingStretch::Flight` cover turning flights.
- **Handrails in pieces.** (Refs #85) The rails along one side of a flight
  or run are the pieces of one handrail: `HandrailEvidence::side_rail`
  puts them in order from the bottom (each starting and ending decidably
  further along than the one before, otherwise refused with their names)
  and `HandrailEvidence::gap` bounds the plan gap between two pieces.
  `stair-geometry` and `ramp-geometry` take the extension from the first
  piece at the bottom and the last at the top, check heights per piece, and
  gain `handrail_gap_maximum`: a gap between consecutive pieces larger than
  allowed is a finding relating both. Pieces that cannot be put in order
  leave the side's extension and gaps not evaluated; a rail reaching over
  the middle that falls short may be a piece, so it is not evaluated; an
  undecided rail may continue a short extension or bridge a gap.
  **Breaking:** definitions bound to `stair-geometry` or `ramp-geometry`
  must declare the new optional parameter `handrail_gap_maximum`.
- **Doors on and over stair landings.** (Refs #85) `stair-geometry` gains
  `landing_doors`, `landing_door_height` and `landing_door_swing`, as
  ramps have them: no selected door may stand in the column over the
  landing at either end of a flight (its base at the bottom, its top at
  the top), nor with `landing_door_swing` swing over it, judged as for a
  ramp's landings. **Breaking:** definitions bound to `stair-geometry` must
  declare the three new optional parameters.
- **Doors swinging over ramp landings.** (Refs #85) `ramp-geometry` gains
  `landing_door_swing`: no `landing_doors` door may swing over a landing at
  a run's end. A hinged leaf's sector overlapping the landing's rectangle
  in plan, for a door reaching into the column over it, is a finding; a
  door surely above or below the column is skipped, and unknown leaves, an
  unknown height or a swing that only may overlap leave the landing not
  evaluated. **Breaking:** definitions bound to `ramp-geometry` must
  declare the new optional parameter `landing_door_swing`.
- **Escape routes: passages from the walked route, and multiplied travel
  split per section.** (Closes #79) Metric routing gains two default
  methods. `NearestTargetRequest::with_avoided` asks for the nearest target
  by walks keeping out of named objects, answered only by a backend whose
  `avoids_objects()` is true (the handle refuses it otherwise); its lower
  bound beyond the plain walk's upper bound, or an unreachable verdict,
  proves every shortest walk enters one of them, ties included.
  `trace_path` (`PathTraceRequest`, `PathTrace`) measures how much of a
  polyline lies over each object's plan footprint, as an interval, the
  handle checking one answer per object and none longer than the path.
  The Axiolid backend answers both: avoided bodies join the obstacles, and
  a trace cuts each segment at the footprint's boundary (a tessellation's
  plan box bounds only from above). `escape-route` with the new
  `walked_passages` derives each space's passages from the walks out of
  its doors: surely a passage the walk round which is longer than the
  plain walk from every door, perhaps any passage not ruled off every
  shortest walk by plan distances, so a passage carries its sure loads at
  least and its possible ones at most, and is found too narrow only with
  one space surely relying on it. Declared passages stay. From a door, the
  multiplied travel is bounded from above by the answer's own walk traced
  over the sections (plain upper bound plus, per section, its length times
  its factor less one) where that is tighter than the largest factor; the
  farthest point keeps the old bound.
  **Fixed:** the Axiolid nearest-target lower bound on a level that is not
  closed (an unmeasured surface or portal, a connector leaving it) is the
  straight line, not the map's distance, which may miss a shortcut.
  **Breaking:** definitions bound to `escape-route` must declare the new
  optional parameter `walked_passages`.
- **Exit doors open in the direction of escape.** (Refs #79)
  `escape-route` gains `exit_door_direction`: every exit door `exit_path`
  reaches must open out of the checked space. Its leaves come from the
  object-frame service and the side the space lies on from the free-space
  service's containment probes, as for `door-swing`. An exit door swinging
  into the space is a finding; one opening away or double-acting passes; an
  exit that is no door is skipped; a door without a hinged leaf, unknown
  leaves or a space beside neither side leave the space not evaluated.
  **Breaking:** definitions bound to `escape-route` must declare the new
  optional parameter `exit_door_direction`.
- **Door swing.** (Closes #82, Closes #74) `door-swing` requires each
  selected door to swing into (`swing_into`) or not into
  (`swing_not_into`) the spaces `space_path` reaches from it, from its
  leaves as the object-frame service states them. Which side of the door a
  space lies on is asked of the free-space service at two small probes per
  hinged leaf, on its swing side and behind it; a double-acting leaf swings
  into both sides, a space neither probe lies in decides nothing, and a
  door without a hinged leaf is not evaluated. It maps the opening
  direction relative to the space type (a WC door opens outward) and the
  door swing model check. No breaking change: the capability is new.
- **Door clear widths from the lining and leaves, and door clearances by
  swing side.** (Refs #82) `keyed-limit`'s `clear-width` gains a step
  between the stated width and the rule's deduction:
  with `clear_width_from_leaves`, the overall width less the lining on
  both jambs and every hinged leaf's thickness, as the door's leaves state
  them (with IFC, `LiningThickness` and `PanelDepth`), cited as the inexact
  `…:step=lining-and-leaves`. It moves on when the source states no leaves
  or thicknesses or a leaf does not swing. `component-clearance` takes
  `front_axis` `swing` (the side a door's hinged leaves open into) and
  `-swing`, and `align` `handle` and `hinge` (flush with the edge of a
  single hinged leaf's handle or hinge), so the clear areas in front of,
  behind and beside a door are rules; a double-acting, sliding or
  multi-leaf door leaves what it cannot place not evaluated. A threshold is
  the lining's `ThresholdThickness`, read as any property.
  **Breaking:** definitions bound to `keyed-limit` must declare the new
  optional parameter `clear_width_from_leaves`.
- **Distances to door swings.** (Closes #60) `distance` gains
  `subject_extent` and `counterpart_extent`: `door_swing` measures that side
  by the floor sectors its doors' hinged leaves sweep instead of its body,
  in plan (`projection` `horizontal` only). Each sector is bracketed between
  an inscribed and a circumscribed polygon, so the distance is an interval;
  a door without a hinged leaf sweeps nothing, and one whose leaves cannot
  be read is not evaluated or unknown. `ProximityService` gains
  `measure_region_distance` (refusing by default): the plan distance from a
  `ConvexPlanRegion` to an object's footprint as a `RegionDistanceEvidence`,
  answered by the Axiolid adapter from the same 2D closest points as
  `Horizontal`. `ConvexPlanRegion::separation` measures two regions without
  geometry. **Breaking:** definitions bound to `distance` must declare the
  two new optional parameters.
- **Door leaves.** (Refs #36) `ObjectFrameService::leaves` (refusing
  by default with `DoorLeavesError::Unsupported`, forwarded by federation)
  answers a door's `DoorLeaves`: the operation type as the source names
  it, the overall width, the lining thickness where stated, and per
  `DoorLeaf` its closed position, width direction, opening direction and
  up axis (orthonormal, possibly mirrored), thickness where stated, motion
  (`Swing`, `DoubleSwing`, `Slide`, `RollUp`, `Fixed`), and for a hinged
  leaf its `HingeSide` seen from above along the opening direction and its
  `SwingSector` (a quarter disc, a half disc when double-acting;
  `plan_bounds` brackets its footprint between two convex polygons). The
  IFC adapter derives them with `openbim-ifc` 0.7.0's `door_operation`
  (openbimrs/ifc#148; `geometry-select` and `properties` only, no codec or
  geometry kernel) from the placement, `OperationType` and
  `IfcDoorPanelProperties`, and reads `LiningThickness` and each panel's
  `PanelDepth` in metres. A door turned over (`Axis` (0,0,-1)) hangs its
  hinge on the other side. Missing inputs are `NotStated`, refused or
  contradictory operations `Refused`, never defaulted. Window leaves are
  not read: no published crate derives them. No breaking change: the
  trait method has a default.
- **Winders, turning flights, open risers and tessellated flights.**
  (Refs #85) `WalkingSurfaceService::measure_tread_flight` takes a
  `TreadFlightRequest` saying where a turning flight's walking line runs
  (`WalkingLinePlacement`: its centre line, or a distance from the side it
  turns towards), and a `TreadFlight` climbs along a `WalkingLine`:
  `Straight` (a direction) or `Turning` (a plan polyline with a vertex on
  every tread, positions being arc lengths along it). A `Tread` may carry
  its nosing (`PlanSegment`) and whether the riser below it is closed
  (`RiserClosure`); the contract derives winder angles
  (`TreadFlight::winder_angles`) and every riser's closure
  (`riser_closures`). A turning flight's tread states its sides across its
  own direction, square to its nosing; a winder tapers and states none, so
  a flight with winders has no width. `AxiolidWalkingSurfaceService` now
  finds treads with plane detection (`axiolid-inspect` 0.3.2,
  `detect_planes`, axiolid/kernel#131): it measures winders and turning
  flights along a walking line through each tread (along the nosing of a
  tread with parallel edges, along the bisector of a winder's), open
  risers (the tread below ending in a face falling away), and tessellated
  flights with every position widened by the chord deviation (a
  tessellated tread's rectangle is proven within twice it). A turning
  flight's landings and handrails are refused, since they are placed along
  one direction; its headroom above and below is measured as any flight's.
  `stair-geometry` gains `walking_line_offset`, `winder_angle_maximum` (a
  plane angle) and `forbid_open_risers`; its `width_minimum` and
  `width_maximum` leave a flight with winders not evaluated, as they do its
  landing and handrail checks on a turning flight.
  **Breaking:** `measure_tread_flight` takes a `TreadFlightRequest` instead
  of an `ObjectId`; `TreadFlight::try_new` takes the request and a
  `WalkingLine` instead of an `ObjectId` and a direction, and
  `TreadFlight::direction` is replaced by `walking_line`; definitions bound
  to `stair-geometry` must declare the three new optional parameters
  `walking_line_offset`, `winder_angle_maximum` and `forbid_open_risers`;
  the workspace requires `axiolid-inspect` 0.3.2.
- **Openings at corridor ends.** `corridor-end-openings` finds windows (any
  opening `opening_selector` picks among those `opening_path` reaches, such
  as `axioval:derived.adjacent-space:backward`) in the wall a selected
  corridor ends at: within `wall_depth` (0.5 m by default) of the end wall
  and facing more than `facing` (0.1 m) of it, so a window in a side wall
  beside the corner is not one. `PlanSpanService::measure_corridor_ends`
  (refusing by default) answers a `CorridorEndRequest` with `CorridorEnds`:
  each `CorridorEnd`'s point, certified clearance and `EndWall`, either
  `Decided` (the boundary segment and a `WallContact` per subject, with the
  plan gap to the segment and the length faced as `PlanLength` intervals)
  or `Undecided` with its reason. The ends come from an approximate
  skeleton, so their evidence is never exact; an undecided wall or a
  straddling gap or facing leaves the opening not evaluated. The Axiolid
  adapter builds the skeleton with `axiolid-route` 0.3.3, decides a wall
  only when the path stops at it and rays within 20° of the path's
  direction all meet it, and refuses a tessellated space. No breaking change: the trait method
  has a default and the capability is new. (#86)
- **Local circulation within rooms.** `local-circulation` requires, within
  each selected space, a path `width_metres` wide from the entrances
  (`access_path`, such as `axioval:derived.adjacent-space`, with
  `door_selector` or `opening_selector`) to the components
  (`component_selector`, placed in spaces by `space_path`), free up to
  `clear_height_metres` of what `obstacles` occupy. A component the path
  cannot reach is a finding (`no entrance of … reaches it on a path … wide`);
  with `component_mode` `link` the components must be linked with one
  another instead. Every skeleton end of the path must offer a free area
  `end_width_metres` by `end_length_metres` within `end_reach_metres`,
  unless its branch is shorter than `short_end_metres` or its free width
  narrower than `narrow_end_metres`; the `passing_*` parameters require
  passing spaces along the path from each entrance to each component, as
  on accessible routes. `tolerance_metres` (default 0.05 m) says how close
  the path must pass an entrance or component. Door swings are not
  subtracted: sources state none. Three-valued throughout. It rests on a
  new free-space measurement, `FreeSpaceService::map_circulation`
  (`CirculationRequest`, `CirculationMap`; the default refuses): pieces of
  the free area eroded by half the width from inside, which prove a path,
  possible pieces from outside, which prove there is none, which pieces
  come near each entrance and component, and the skeleton of the pieces
  with proven half widths. The Axiolid adapter answers it with one-sided
  disc morphology and `axiolid_route::skeleton`; the workspace now requires
  `axiolid-route` 0.3.3 and resolves `axiolid-triangulate` and
  `axiolid-predicates` 0.3.1. `passing_spaces` judges any polyline in one
  space. (#78)
- **Space-boundary coverage.** (Refs #74) `BoundaryCoverageService`
  answers a `BoundaryCoverageRequest` (a space and a plane tolerance) with
  a request-bound `BoundaryCoverage`: the space body's surface area, the
  covered, uncovered and overlapping areas as `SurfaceAreaInterval`s,
  every declared boundary as a `MeasuredBoundary` (`OnSurface` with its
  area, or `OffSurface`) and the overlapping pairs (`BoundaryOverlap`).
  The contract derives the covered share; evidence is exact only when
  every interval is a point, and the method refuses by default.
  `AxiolidBoundaryCoverageService` groups the body's faces by plane,
  assigns boundary triangles to the face plane they lie on within the
  tolerance, and bounds the areas with overlay `Region` union,
  intersection and difference; a plane off the coordinate axes widens by
  its projection's rounding and the overlay's grid snapping, a
  tessellated boundary by its chord deviation, and a curved body or an
  unreadable boundary refuses. The capability `space-boundary-coverage`
  judges `minimum_covered_share`, `maximum_uncovered_area` and
  `maximum_overlap_area` (with `plane_tolerance`) per space, and always
  reports a boundary lying on no face of the body. With `--geometry` the
  CLI lowers each `IfcRelSpaceBoundary` connection surface
  (`IfcCurveBoundedPlane`, `IfcFaceSurface`, `IfcAdvancedFace`,
  `IfcFaceBasedSurfaceModel`) in its space's frame and meshes it.

- **Stair and ramp handrails, ramp end spaces and landing doors.** (Refs
  #85) `WalkingSurfaceService::measure_handrails` answers a
  `HandrailRequest` (subject, `WalkingStretch`, the rails a rule selects,
  how far outside the sides and above the pitch line a rail may lie, and
  an extension length) with `HandrailEvidence`: the pitch line's ends and
  the walking surface's sides, and per rail a `RailMeasurement` of its
  positions along and across, the least and greatest height of its top
  above the pitch line (a flight's nosing line, a run's surface) and how
  much its top rises over the extension beyond each end. The contract
  derives each extension and the `RailSide` a rail runs along; the
  method refuses by default. `AxiolidWalkingSurfaceService` requires a
  rail's upward faces to fill one rectangle along the walking direction
  (any other rail refuses), bounds its top by the upper envelope of its
  edge lines with a numerical margin, and widens a tessellated rail by
  its chord deviation. `stair-geometry` and `ramp-geometry` take
  `handrail_objects`, `handrail_reach_across`, `handrail_reach_above`,
  `handrail_height_minimum`, `handrail_height_maximum`,
  `handrail_extension_minimum` (the rail reaching that far beyond each
  end, level there) and `handrail_sides` (`one` or `both`, with
  `handrail_both_sides_above_width` for both on wider flights and runs).
  `ramp-geometry` also takes `landings_required` (a landing at both ends
  of every run), `end_space_depth`, `end_space_width`, `end_space_height`
  and `end_space_obstacles` (a free box in front of the lowest and beyond
  the highest run, through the free-space service), and `landing_doors`
  with `landing_door_height` (no door standing on a landing at a run's
  end). A stair near a ramp is a `distance` rule in `nearest` mode.
  **Breaking:** definitions bound to `stair-geometry` must declare the
  eight new optional handrail parameters, those bound to `ramp-geometry`
  those eight and `landings_required`, `end_space_depth`,
  `end_space_width`, `end_space_height`, `end_space_obstacles`,
  `landing_doors` and `landing_door_height`.
- **Escape routes: multiplied sections and passage widths.**
  `escape-route` takes a `sections` table (`objects`, `factor` of at least
  one, optional `shared_by` and `label`): a metre walked on those objects
  counts `factor` times, and with `shared_by` only where at least that many
  checked spaces reach the section along `section_path`. Metric routing
  answers the plain walk, so the multiplied travel is bracketed between the
  plain walk's lower bound and its upper bound times the largest factor of
  a section the walk may cross; a section whose horizontal distance from
  the start surely exceeds that bound is left out. Travel within the
  maximum only unmultiplied is not evaluated, never a pass. With
  `passage_selector`, passages (reached along `passage_path`, or the space
  itself) must be as wide as the new `widths` column `passage_width` for
  the summed load of every checked space relying on them: a stated
  `passage_width_property` decides both ways, the shorter side of the
  least-area rectangle enclosing the footprint only a failure, and a space
  of unknown load leaves the passage not evaluated. Exit door opening
  direction stays open (openbimrs/ifc#148). **Breaking:** definitions bound
  to `escape-route` must declare the new optional parameters `sections`,
  `section_path`, `passage_path`, `passage_selector` and
  `passage_width_property`, and the new `widths` column `passage_width`.
  (#79)

- **Stair and ramp widths, landings and headroom below.** (Refs #85) A
  `Tread` and a `SlopedRun` may carry the positions of their sides across
  the walking direction (`with_sides`, along `across`), stated only where
  the surface fills that rectangle; `TreadFlight::width` is the narrowest
  tread's. `WalkingSurfaceService::measure_landing` answers a
  `LandingRequest` (subject, `WalkingEnd`, the candidates a rule selects)
  with `LandingEvidence`: the direction leaving the end, its arrival line
  (the first riser, the last riser, a run's end) and the `Landing` a
  candidate's level surface carries there, with its far side and sides
  when it is a rectangle, so depth and width are derived as sure intervals.
  `measure_clearance_below` answers a `ClearanceBelowRequest` with the
  least height of the subject's underside above the floors of the spaces a
  rule selects, leaving out where it rests on them. Both default to a
  refusal. `AxiolidWalkingSurfaceService` proves rectangles from the
  boundary edges of the faces and their area, finds one landing surface
  per end within `LANDING_REACH` (several are refused, never merged; a
  ramp may carry its own landings, a flight's top tread is no landing) and
  measures the clearance below with a numerical margin, never exactly.
  `stair-geometry` and `ramp-geometry` take `width_minimum` and
  `width_maximum` (the flight's or each run's width), `landing_objects`
  with `landing_depth_minimum`, `landing_width_minimum` and
  `landing_at_least_walking_width` (the landing at least as deep and wide
  as the flight or run), and `minimum_headroom_below` with
  `headroom_below_spaces`; `stair-geometry` also takes `landings_required`
  (a selected slab or landing meets both ends). A landing that fills no
  rectangle, or an end the selection leaves undecided, is not evaluated.
  **Breaking:** definitions bound to `stair-geometry` must declare the nine
  new optional parameters, those bound to `ramp-geometry` the eight.
- **Surface transparency.** `axioval:presentation.Transparency`
  (`axioval_ir::PRESENTATION_TRANSPARENCY`) lists every distinct
  transparency of an object's styled body surfaces, ascending, from `0.0`
  (opaque) to `1.0`, so a selector can leave see-through objects out of a
  view's blockers: `not` `Transparency` `greaterThanOrEquals 0.5` with
  `quantifier: all` keeps opaque and unstyled objects and drops glazing.
  The IFC adapter reads it through `ifc-style` 0.3.0: an item's own
  `IfcStyledItem`, else its `IfcPresentationLayerWithStyle`, is
  authoritative; mapped representations are followed; an item with no
  surface style of its own is drawn with the styles of its object's
  material (`IfcMaterialDefinitionRepresentation`). An unset
  `Transparency` is opaque, as the schema states. An object without a
  styled surface is exactly absent; a surface style without shading, and
  two styled items on one item, are refused. (#71)
- **Footprints with their own axes.** `PlanSpanService::measure_rectangle`
  answers the rectangle of least area enclosing a footprint as a
  `PlanRectangle`: centre and radius, two unit axes and how far they may be
  turned, and the half extents as intervals. `RectangleOrientation` says
  whether the axes are the footprint's own (`Unique`), one of several
  least-area orientations (`Tied`) or unproven on a tessellation;
  `width_and_length` answers only for a unique orientation, `long_axis`
  also needs one side surely longer (a square has none), and
  `long_axis_angle` bounds the acute angle between two long axes. The
  default refuses. The Axiolid service measures it with the overlay's exact
  rotating calipers (axiolid-overlay 0.3.2, now the workspace's minimum),
  widened by the kernel's rounding bound, and exactly from the extreme
  coordinates when the rectangle lies along the coordinate axes.
  `PlanAreaService::measure_outside_bands` measures a footprint's area
  outside a set of `PlanBand`s, each the convex hull of two footprints cut
  to the stretch along a direction that both reach; the default refuses,
  and the Axiolid service brackets each computed cut inwards and outwards
  and refuses a band bounded by a tessellated footprint. (#87, #61)
- **Parking bays.** `parking-bay` checks each selected bay along its own
  axes, never its bounding box: `min_width`, `max_width`, `min_length`,
  `max_length`, `min_height` and `max_height`; with `aisles`,
  `orientation` (`parallel`, `perpendicular` or `angled`) and
  `angle_tolerance`, its long axis against an aisle within `aisle_reach`;
  with `obstacles`, `obstruction_reach`, `end_obstructions` and
  `side_obstructions` (`none`, `one`, `both`), how many ends and sides
  objects within reach obstruct, from their extents along the bay's axes.
  A bay turned 45 degrees whose box is long enough but which is itself too
  short is found. A bay without a unique orientation, or a square one
  where its ends matter, is not evaluated. (#87)
- **Wall spacing.** `wall-spacing` judges the parallel walls or beams a
  storey reaches along `member_path`: pairs whose long axes lie within
  `angle_tolerance` of parallel and that face each other must stand at
  least `minimum` apart in plan, and with `maximum`, `footprints`,
  `footprint_path` and `uncovered_above`, the bands between pairs at most
  `maximum` apart must cover each footprint object up to that area. Sure
  and possible pairs bound the uncovered area from above and below. (#61)
- **Axis-compatible counterparts.** `counterpart-coverage` takes an
  optional `axis_tolerance`: only counterparts whose long axis lies within
  it of parallel to the element's count, one surely at another angle is
  left out, and one whose angle or axes are undecided is a possible cover
  only. **Breaking:** definitions bound to `counterpart-coverage` must
  declare the new optional parameter `axis_tolerance`. (#63)
- **Containment and cover.** The new capability `containment` checks that
  the selected inner elements (columns, reinforcement) lie in
  `counterparts` (walls, concrete bodies): their certified shared volume is
  at least `minimum_volume_ratio` of the smaller body's, judged on the
  interval, and with `combine_adjacent` outer elements whose surfaces meet
  count together, the combined volume bounded by inclusion and exclusion. A
  `cover` table bounds the signed distance from the inner body to one class
  of the outer element's faces (`top`, `side`, `bottom`, `any`) from
  `inside` (a cover) or `outside` (a protrusion) between a minimum and a
  maximum; `minimum_count` and `maximum_count` bound the inner elements per
  outer element, and `report_orphans` reports one lying in none. Undecided
  containments, straddling intervals and counts the undecided elements
  could change are not evaluated. The proximity contract gains
  `measure_face_distance` (`FaceDistanceRequest`, `FaceClass`,
  `FaceDistanceEvidence`, `FaceDistanceError`), a signed distance positive
  inside the host and negative outside, refused by default; the Axiolid
  adapter answers it exactly for a body wholly inside an exact planar host
  and with sound bounds otherwise, refusing a tessellated or open host. (#59)
- **Intersection volume and a volume tolerance.** `ProximityEvidence`
  carries an optional `IntersectionVolume`: the certified volume two closed
  bodies share and each body's own, as `VolumeInterval`s
  (`with_intersection_volume`, `intersection_volume`,
  `ratio_of_smaller`). The Axiolid adapter measures it with
  `axiolid-inspect`'s certified volume integrals, exactly zero for bodies
  apart, unmeasured for open or refused meshes, and widened for a
  tessellation by the volume within its chord deviation. `clash` and the
  `clash-matrix` cells take `volume_tolerance_cubic_metres`: an
  intersection counts only when its shared volume exceeds it, a straddling
  or unmeasured volume leaving the pair not evaluated. **Breaking:**
  definitions bound to `clash` must declare the new optional parameter, and
  `clash-matrix` definitions the new optional `cells` column. (#57)

- **Escape routes.** The new capability `escape-route` checks each space
  against the first row of a `uses` table whose `spaces` selector picks it:
  `maximum_travel` from the farthest point of its walkable area
  (`route_start: farthest-point`, the default) or from each of its doors
  (`door`, through `door_path` and `door_selector`) to the nearest exit,
  the number of `exits`, and, with `area_per_occupant`, exit widths for
  the occupant load (footprint over area per occupant, rounded up) from a
  `widths` table (`occupants`, `width` per exit, optional `total_width`).
  Exits are the `exit_selector` objects `exit_path` reaches, as for
  `exit-separation`. Widths are stated clear widths
  (`clear_width_property`); without one, an exit whose whole footprint is
  narrower than required is a finding and anything else is not evaluated.
  Travel follows a point's walking line (`walking_height`,
  `walking_step`) and is bounded through the sure exits from above and
  every possible exit from below; part of a space that reaches no exit is
  a finding. Stair and shared-section multipliers, passage widths and exit
  door opening direction (door leaves, openbimrs/ifc#148) are not checked.
  (#79)
- **Nearest targets and farthest points in metric routing.**
  `MetricRoutingService` gains `nearest_target` (`NearestTargetRequest`,
  `NearestTargetOutcome`: the distance from one point to the nearest of
  several targets, or complete evidence that none is reachable) and
  `farthest_point` (`FarthestPointRequest`, `FarthestPointOutcome`: a
  certified interval on the largest distance from any point of a region to
  its nearest target, with a witness point and convergence against a
  tolerance, or a point of the region no target reaches). Both default to a
  refusal; the handle binds every answer to its request.
  `AxiolidMetricRoutingService` answers them with one `axiolid-route`
  distance map per query: targets may stand in a portal, so exit doors
  (also to the outside) are targets; the upper bound holds on any level and
  the lower bound and unreachable verdicts need a closed one with every
  target placed; farthest points are for a point body only.
  `space-distance` now walks with two nearest-target queries (sure and
  possible destinations) instead of one route per pair, so a body sweep is
  proven to one destination only. **Breaking:** `MetricRoutingError` gains
  `NoTargets`, `InvalidTolerance` and `InconsistentResponse`; the workspace
  requires `axiolid-route` 0.3.2. (#79)
- **Floating clearance volumes and size modes.** `component-clearance`
  takes `slide_from` and `slide_to`: the volume floats and is free when it
  is free at some offset between the two across the side. It is decided by
  the placement search in the spaces `space_path` reaches, with a
  frame-offset domain anchored on the component where `align` and
  `lateral_offset` put the volume and the obstacle band from its base up by
  its height; a witness for the volume grown by its position interval
  passes, a proof that the shrunk volume fits at no offset is a finding
  relating the spaces, anything else is not evaluated. `size_mode`
  (`minimum`, the default, `maximum` or `fixed`) and `size_tolerance`
  bound the size: a minimum is checked less the tolerance, a maximum is
  exceeded when the volume one tolerance larger in any one dimension is
  free, fixed or floating. The largest fitting volume is not measured.
  **Breaking:** definitions bound to `component-clearance` must declare the
  new optional parameters `slide_from`, `slide_to`, `size_mode` and
  `size_tolerance`. (#83)
- **Passing spaces on accessible routes.** `accessible-route` takes
  `passing_width_metres`, `passing_length_metres`,
  `passing_spacing_metres` and `passing_reach_metres`: every proven route
  must offer a free box that size, `clear_height_metres` high, at most every
  `passing_spacing_metres` along it, its ends counting as passing spaces.
  The route is the metric-routing polyline between the start's and the
  destination's representative points; it is searched in tiles no longer
  than half the spacing, segment by segment, with frame-offset placement
  domains anchored on the segments in the route spaces it crosses. A
  witness in every interior tile passes, a run of tiles proven empty longer
  than the spacing is a finding relating the start, and anything else is
  not evaluated, as is a destination without the metric-routing, plan-span,
  vertical-extent or free-space service. **Breaking:** definitions bound
  to `accessible-route` must declare the four new optional parameters.
  (#77)
- **Free floor space options.** `free-floor-circle` and
  `free-floor-rectangle` take an `obstacles` selector (default: every other
  object, as before), an elevation band `band_from_metres` /
  `band_to_metres` above the floor in which obstacles count (default: the
  floor up by `height_metres`), and a `merge_path` whose spaces are searched
  together with the selected one, such as a derived
  `overlapping-group-space` or a stated grouping. An obstacle the selection
  cannot decide can only keep a proof of absence open: the proof is asked
  again without it and stands only if it still holds. Merged spaces are
  never obstacles, are related to the finding, and make the request
  unconstrained instead of supported by one space. `PlacementRequest`
  gains `with_band` (`ElevationBand`) and `with_merged_scopes`; a
  frame-offset anchor may be grounded on another object than the scope,
  such as a door, and `FrameOffsetPlacement::contains_frame` is public. The
  Axiolid placement search clips obstacles to the band, searches the union
  of merged scopes on one floor, and answers frame-offset domains: the
  configuration space intersected with the box of admitted offsets, exact
  for fixed orientations, with absence proven against the box grown by
  1 µm. Its witness search now also tries slab midpoints, so a free region
  around a column is no longer refused. A free corridor width is left out:
  deciding whether an eroded region connects two sides is not sound yet.
  **Breaking:** definitions bound to either capability must declare the
  new optional parameters `obstacles`, `band_from_metres`,
  `band_to_metres` and `merge_path`; `FreeSpaceError` gains
  `InvalidElevationBand` and `MergedScopeConflict`; a frame-offset anchor
  grounded on another object is no longer refused as
  `PlacementScopeMismatch`. (#84)
- **Shelf capacity measures a real layout.** `shelf-capacity` no longer
  bounds shelving by the footprint's bounding box with a lower bound of zero,
  which no room could pass. The Axiolid adapter lays parallel bands
  `shelf_depth_metres` deep across the footprint, a band against a wall, an
  aisle `horizontal_spacing_metres` wide, two bands back to back, and so on,
  each band served by the aisle along one side; a band carries shelving where
  its whole depth lies on the floor outside every door clearance and its
  aisle on the floor. A door's swing is unknown, so its clearance is every
  point within `door_clearance_metres` of its footprint, measured from the
  room's side of the wall. Bands run along both axes of the footprint's and
  every door's least-area rectangle, anchored at either wall, and the
  longest layout counts, stacked in whole tiers up to the lower of the top
  elevation and the clear height. The interval holds the layout's length on
  the true geometry, so a compliant room passes: a 6 × 4 m room with one 1 m
  door holds 84 m in the documented arrangement. A space whose clear height
  lies below `top_elevation_metres` is a new finding, `space too low for the
  shelving`. Doors and openings are the rule's selection: `access_path`
  (required), `door_selector`, `opening_selector` and `space_selector` read
  them as `space-connection` does (`axioval:derived.adjacent-space`, or a
  stated relationship), and they travel in the request. Findings take the
  rule's severity and relate the doors. **Breaking:** definitions bound to
  `shelf-capacity` must declare the four new parameters, `access_path`
  required; `horizontal_spacing_metres` is now the aisle between bands, not
  the pitch of uprights, and `door_clearance_metres` the clearance's reach,
  not wall length per doorway. `AxiolidGeometry::with_doorways` and
  `doorway_count` are removed, and the CLI no longer counts doorways from
  space boundaries; `LinearQuantityRequest` gains `with_doors` and `doors`,
  `LinearQuantityEvidence` `with_clear_height` and `clear_height`, and
  `AxiolidLinearQuantityService` takes opening voids. The bands follow the
  same least-area rectangles `measure_rectangle` answers; any orientation,
  tied or unproven, only proposes a layout. (#55)
- **Building envelope: recesses and light wells.** `recess-width` requires
  every recess of a footprint, a pocket between its outer boundary and its
  convex hull, to be as wide at its mouth as the first row of a
  `requirements` table keyed by depth demands (`minimum_depth_metres`,
  `maximum_depth_metres`, `minimum_width_metres`,
  `minimum_width_per_depth`). The convex hull is the reference because it is
  orientation-free and each of its pockets is a genuine indentation; a
  minimum-area rectangle would also report the corners of a trapezoidal room.
  `light-well` judges the spaces `member_path` reaches from each well: no
  vertical gap above `gap_tolerance_metres`, a shared plan section (the
  intersection of their footprints) that is not empty, and that section's
  area and width, the short side of its least-area rectangle, against the
  first row of `requirements` whose `maximum_height_metres` the well's
  height does not exceed. `PlanSpanService` gains `measure_recesses`
  (`PlanRecesses`, `PlanRecess`) and `measure_section` (`PlanSection`), both
  refused by default; the Axiolid adapter measures them on exact footprints
  and refuses tessellated ones. A section's sides come from the same
  least-area rectangle as `PlanRectangle` and only for a unique orientation:
  a tied section is refused and its well not evaluated. Adjacency to a wall declared external is a
  `distance` rule (`counterparts` the walls with `IsExternal` true,
  `nearest`, `horizontal`, `maximum_metres`), documented with an end-to-end
  test; it needs no capability of its own. (#67)
- **Component visibility.** `component-visibility` requires targets within
  a `radius` of an eye `eye_height` above each component's base, over its
  footprint centre, to be in view past the `blockers` selection: at least
  `minimum` of them (`mode: at-least`, default 1) or none (`mode: none`).
  The new line-of-sight contract (`SightService`, `SightServiceHandle`,
  `SightRequest`, `SightEvidence`, `SightOutcome`) answers `Visible` with a
  witness point, `Hidden` with the occluders, which must be requested
  blockers, or `Undecided`, and bounds the distance to the target so a
  target surely beyond the range is not looked at. The Axiolid adapter
  (`AxiolidSightService`) answers with the kernel's certified
  `line_of_sight`: a target covered only where two separate blockers meet
  stays undecided, and tessellated targets in range or blockers that may
  matter refuse. Undecided targets, straddling distances and undecided
  blockers decide only what they cannot change. The CLI registers the
  service with `--geometry`. See-through blockers are left out by the
  `blockers` selection, through `axioval:presentation.Transparency`. (#71)
- **Effective coverage.** `effective-coverage` requires the union of the
  `sources`' effect areas, clipped to each element's footprint, to cover at
  least `minimum_ratio` of it. An effect reaches `range` as `mode` says:
  the source's footprint grown (`grown`), grown but only for sources
  touching the element within `touch_tolerance` (`touching`), the points
  within that travel distance of the source's centre (`travel`), or the
  points its centre sees (`visible`), travel and sight going round the
  `blockers`' footprints. With `capacity_property` and
  `capacity_multiplier`, the summed property of the sources reaching the
  element times the multiplier must also reach its area. The plan-area
  contract gains `measure_coverage` (`CoverageRequest`, `Participant`,
  `EffectReach`, `CoverageEvidence`, `EffectMeets`; the default refuses): the
  covered area is an interval from the certain sources' inner bounds to
  every source's outer bound, whole when an effect cannot be measured, and
  each source's effect is reported as meeting the footprint surely,
  possibly, not or unmeasured. The Axiolid adapter dilates footprints with a
  stated side (`Region::dilate_inner`, `dilate_outer`), cuts the exact
  visibility polygon to discs inscribed in and circumscribing the range, and
  judges convex cells of the free region by the kernel's travel-distance
  map. Uncertain sources raise only the upper bound, uncertain blockers
  narrow only the lower. Effects propagating into connected spaces and an
  area taken from a property wait. (#72)
- **Above or below in vertical distances.** `distance` takes a
  `vertical_direction` (`either`, the default, `above` or `below`) with the
  `vertical` projection, so a rule can require a sprinkler at most 0.5 m
  below the ceiling (`nearest`, `maximum_metres` 0.5, `above`) or nothing
  above within 2 m (`none_closer_than`, `above`). A counterpart is above
  unless it lies lower at both ends of its vertical extent, below unless it
  lies higher at both; the distance is the one-sided gap, zero when the
  extents overlap. `ProximityProjection::Vertical` carries a
  `VerticalDirection`; `projected_candidate_pairs` prunes each direction by
  its one-sided box gap and drops counterparts whose box lies wholly on the
  other side, keeping a pair when either orientation the groups allow
  qualifies. The Axiolid adapter measures directions exactly on planar
  meshes and leaves the side open where a tessellated end lies within the
  combined chord deviation of the subject's. **Breaking:** definitions bound
  to `distance` must declare the new optional parameter
  `vertical_direction`; `ProximityProjection::Vertical` gains the
  `direction` field, so code constructing or matching it must name it. Door
  swing footprints as distance sources wait on door leaves
  (openbimrs/ifc#148). (#60)
- **Report tables.** A report carries named tables of measured values
  beside its findings (`Report::tables`, `axioval_ir::ReportTable`): per
  rule, typed columns (`quantity` with its SI dimension, `number`, `text`)
  and one row per scope, keyed like findings (`object_id`, `source`, or
  neither for the project), with `exact`, `interval` or `unknown` numbers
  and text. Tables are ordered by rule and name, rows by scope; names,
  row widths and values are validated when built and when read. The
  field is omitted when empty, so a report without tables serializes byte
  for byte as before. Capabilities add tables with
  `CapabilityEvaluation::push_table`; the runtime binds each to its
  compiled rule and fails the run (`EngineError::DuplicateReportTable`)
  when a rule reports one name twice. `level-spacing` reports `levels`
  (`elevation`, `height`) and, with `space_selector`, `spaces` (`level`,
  `height`, `level_height`); `plan-area` reports `areas` (`plan_area` or
  `facade_area`); `area-ratio` reports `ratios` (`numerator_area`,
  `denominator_area`, `ratio`). They report every measured value, passing
  or not, and need no new parameter. `axioval report` summarizes each
  table as a group (rows, columns, example objects) and lists its rows
  with `--section tables`, filtered by `--rule` and `--object`; BCF export
  ignores tables. Space height against its storey's height stays with
  `level-spacing` (`space_selector`), now visible per space in `spaces`.
  **Breaking:** `Report` gains the public field `tables`, so a struct
  literal must set it; `EngineError` gains a variant; a report containing
  tables needs a reader of this version, since older readers reject the
  unknown field. (#56)

- **Accessible route.** `accessible-route` requires each selected
  destination to be reachable from a start point through the route spaces
  for a mobility profile (`width_metres`, `clear_height_metres`), through
  the selected portals and the lifts, ramps and stairs selected by kind.
  Portals must be at least `door_width_metres` wide, ramps and stairs
  `ramp_width_metres` and `stair_width_metres`, read from
  `clear_width_property` where the model states them and from the
  geometry's upper bound otherwise; `forbid_stairs` (the default) makes a
  room reached by stairs only a finding. A proven route passes, a proven
  block is a finding that relates the blocking doors, stairs or rooms, and
  anything else is not evaluated. Stated door widths go into the
  walkability request, so a model that states them lets the geometry prove
  a door passable. (#77)
- **Rule judgement of walkable passages.**
  `WalkabilitySnapshot::route_between_admitting` routes with a
  `PassageAdmission` (`Admitted`, `Undecided`, `Refused`) per passage on
  top of its width, and `blocking_passages` returns the cut of passages
  that blocks an unreachable route. `WalkabilityRequest::with_stated_clear_widths`
  carries clear widths a rule reads from its source; `AxiolidWalkabilityService`
  treats them as host-stated ones (the narrower wins) and bounds even an
  opening's crossing by them. `WalkabilityError` gains
  `InvalidStatedClearWidth`; matching it exhaustively is a breaking change.
  (#77)
- **Door clear widths in `keyed-limit`.** The new `quantity` `clear-width`
  limits a door's clear width per door type: the length
  `quantity_property` states, else, when that is exactly absent, the length
  the new parameter `overall_width` states (with IFC,
  `axioval:attributes.OverallWidth`) less the new `width_deduction`, a
  non-negative length the rule states for frame and lining. The deduction
  is the rule author's declared approximation: the finding says so and cites
  the inexact evidence entry
  `axioval:derived.clear-width:<door>:step=overall-width-less-deduction;deduction=<metres>`.
  A stated value that is null, not a length or not positive, an absent
  overall width and a deduction leaving no width are not evaluated, never
  replaced by the next step. Widths are read as the decimals they display,
  so a 1 m door less 0.1 m meets a 0.9 m minimum. The capability model's
  new "Doors" section maps every door-accessibility sub-check to a rule:
  clear width, threshold height and glazing ratio as stated values, the
  distance between doors through `distance` (`none_closer_than`,
  `horizontal`), and the clear areas beside the leaf and the opening
  direction as not yet decidable (door leaves, openbimrs/ifc#148; lining
  and panel properties, openbimrs/ifc#149; rectangles fixed to the door,
  #18). **Breaking:** definitions bound to `keyed-limit` must declare the
  new optional parameters `overall_width` and `width_deduction`. (#82)
- **Distances and connections between spaces.** Two capabilities.
  `space-connection` checks each space against the rows of a `connections`
  table whose `from` selector picks it: direct access to a space `to` picks
  `allowed`, `required` or `forbidden`, through `any` door or opening, only
  `doors` or only `openings`, and a direct exit to the outside likewise.
  `space-distance` checks the nearest destination a row of its `distances`
  table names (`to`, optionally on the same storey and with direct access)
  against a `minimum` and a `maximum`, in a straight line between the
  footprints' centroids or walking the metric route between them. Doors
  (`door_selector`) and openings (`opening_selector`) reach their spaces
  through `access_path`: with `axioval:derived.adjacent-space` two spaces
  must lie on opposite faces and a face entering no space is the outside;
  a stated relationship such as `IfcRelSpaceBoundary` connects spaces but
  cannot show an exit. A walk starts at a space's centroid on its floor and
  refuses a space whose centroid lies outside its footprint. Everything is
  three-valued: an element of undecided type or unreadable spaces, a refused
  route or an undecided destination leaves only what it could change not
  evaluated; a blocked route is no destination. Each pair is routed on its
  own until a many-target search exists (axiolid/kernel#186). (#80)
- **Footprint centres.** `PlanSpanService::measure_centre` (default: refuse)
  returns a `PlanCentre`: the centroid the `centres` span measures from, the
  radius the true one lies within, and a `CentrePlacement` (`Inside`,
  `Outside`, `Undecided`); the handle refuses a centre of another object.
  `AxiolidPlanSpanService` places it against the measured footprint, off
  every edge by more than its radius plus the chord deviation. (#80)
- **Free space around components.** The new capability
  `component-clearance` requires a fixed box (`width`, `depth`) or cylinder
  (`radius`), `height` high, on a stated `side` (`front`, `back`, `left`,
  `right`, optionally `both_sides`) of each selected component, placed in
  its placement frame. The rule states which axis is the front
  (`front_axis`: `forward`, `-forward`, `right`, `-right`, or `stated` for a
  front the source states); none is inferred. The volume starts at the
  component's outermost point on that side plus `offset`, is centred on it
  or flush with an edge (`align`, `lateral_offset`), and rises from the
  floor of the spaces `space_path` reaches or the component's bottom or top
  (`height_reference`, `vertical_offset`). `obstacles` less
  `allowed_intruders` may obstruct it; `protrusion` lets them reach that far
  in through any plan side; `within_space` also requires its plan inside the
  (merged) spaces. Measured positions are intervals: the union of every
  position must be clear, or their common part obstructed, otherwise the
  side is not evaluated; undecided obstacles can only obstruct. A sliding
  (floating) volume waits for the placement search. (#83)
- **Clearance containment.** `FreeSpaceService::assess_containment` answers
  a `ContainmentRequest` (a clearance frame and shape and the scopes it must
  lie in) with `ContainmentOutcome::Inside` or `Outside`, exact and bound to
  the request; the default implementation refuses. `AxiolidFreeSpaceService`
  implements it from the overlay difference of the footprint's bounds less
  the scopes' footprints, refusing a cylinder whose band straddles a scope
  boundary and a scope without a mesh or tessellated. The clearance frame's
  origin is documented as the centre of the volume's base. (#83)
- **Walkability and metric routing from geometry.** `axioval-axiolid` now
  implements both contracts, and `--geometry` registers them.
  `AxiolidWalkabilityService` builds a region per selected surface (its floor
  footprint minus the obstacles' parts inside the headroom band) and two per
  selected entrance, one per face. A portal crossing's width is at most the
  longest free interval of its mid-line (and a stated clear width), so a door
  narrower than the route width makes the route `Unreachable` with complete
  evidence; a crossing is definite only when a sweep of the body from
  landing to landing is proven inside the free region with exact booleans
  and the leaf and lining admit it (a bodiless opening, or a door whose clear
  width the host states). `AxiolidMetricRoutingService` routes on the
  origin's level over host-declared surfaces, portals and connectors, every
  other body obstructing between the maximum step and the clear height: a
  proven sweep gives `Reachable` with its length as the upper bound and the
  exact point shortest path (narrow portals cut) as the lower bound; a level
  that the points or narrow portals separate, with no vertical connector
  touching it, is `Blocked`. A gap narrower than the body inside a room is
  refused rather than reported blocked until one-sided erosion is published
  (axiolid-overlay 0.3.1). The CLI declares every `IfcSpace` a surface,
  every `IfcDoor` and opening element a portal and every stair, ramp and
  transport element a connector, and states no clear widths. Passage and
  route evidence cites the measured object's own source, so it stays right
  with several models; both services are bound to every snapshot. New
  dependency:
  `axiolid-route` 0.3.0. (#76)
- **Typed vertical connectors.** `VerticalConnector` pairs an object with a
  `VerticalConnectorKind` (`Lift`, `Ramp`, `Stair`);
  `WalkabilityRequest::with_connectors` selects them and
  `VerifiedWalkablePassage::with_connector` marks a climb, which the snapshot
  accepts only for a requested connector of the same kind.
  `WalkabilitySnapshot::route_between_avoiding` routes without the forbidden
  kinds, so a stairs-only connection is `Unreachable` once stairs are
  forbidden. `WalkabilityError` gains `ConflictingConnector`,
  `PortalConnectorPassage`, `ForbiddenConnectorPassage` and
  `Unavailable(String)` (a backend refusal); matching it exhaustively is a
  breaking change. (#76)
- **Clash matrix.** The new capability `clash-matrix` judges each candidate
  pair with the tolerance profile and severity of one row of its `cells`
  table. A cell keys both sides of the pair (`subject_*`, `counterpart_*`)
  by a pattern over the discipline the object's source plays, patterns over
  the text properties `key_1` to `key_3` name, and a selector (an entity
  class), and gives `clash`'s tolerances and class switches as columns, with
  an optional `severity` and `label`. The single most specific cell applies:
  the one keying the most categories, then the one with the most literal
  pattern characters; cells cover a pair either way round unless
  `symmetric` is false. Tied cells or an unreadable category leave the pair
  not evaluated; a pair no cell covers is ignored or, with
  `report_unmatched`, reported. Same-system exclusion is on by default and
  needs `system_path` (or `exclude_same_system: false`); same-layer
  exclusion is off by default; `exclude_paths` works as for `clash`. The pair
  judgement, measurement and exclusions are shared with `clash`, so a cell
  means exactly what a `clash` rule with its values means. (#58)
- **Clash classes, axis tolerances and pair exclusions.** `clash` now puts
  each pair in one class: a **duplicate** (the two surfaces lie within
  `duplicate_tolerance_metres` of each other, default zero), one body
  **inside** the other, or an **intersection**, which counts only when its
  extent exceeds `horizontal_tolerance_metres` along both plan axes and
  `vertical_tolerance_metres` in height. `report_duplicates`,
  `report_containment` and `report_intersections` switch each class off; a
  switched-off class is not reported as another. `exclude_paths` skips pairs
  whose objects reach a shared target, or each other, through a relationship
  path (`IfcRelAssignsToGroup:backward` for the same system,
  `IfcRelAggregates:backward` for the same parent element, a port path for
  connected elements), and `exclude_same_layer` pairs on a shared
  presentation layer. Every comparison is on an interval: a straddling
  tolerance or an exclusion that cannot be decided leaves a pair not
  evaluated unless it is a finding either way. The contract gains
  `OverlapExtents` (the intersection's extent along x, y and z as intervals)
  and the Hausdorff distance between two surfaces on `ProximityEvidence`
  (`with_overlap_extents`, `with_hausdorff`); `AxiolidProximityService`
  measures both, witnessing the intersection's vertices from edge crossings
  and inside vertices and bounding the Hausdorff distance per triangle. The
  intersection volume, and a volume tolerance, wait on a certified mesh
  boolean (axiolid/kernel#183). **Breaking:** definitions bound to `clash`
  must declare the eight new optional parameters; a service that measures
  no Hausdorff distance leaves touching pairs not evaluated while duplicates
  are reported. (#57)
- **Counterpart coverage.** The new capability `counterpart-coverage` checks
  that each selected element is covered by the objects `counterparts` picks,
  typically another discipline's elements (a `discipline` selector with an
  entity type), in plan and in height: the share of its footprint outside
  the union of the counterparts' footprints grown by the horizontal
  tolerance, and the share of its vertical extent outside the vertical
  extents, grown by the vertical tolerance, of the counterparts overlapping
  it in plan. Coverage declares one `tolerance`, conformity
  `horizontal_tolerance` and `vertical_tolerance`; a negative tolerance
  switches its check off. `info_above`, `warning_above` and `error_above`
  grade the uncovered share into severities. Shares are intervals: one
  straddling the lowest threshold is not evaluated, and undecided or
  unmeasurable counterparts leave only a pass standing. Counterparts are not
  filtered by axis yet. (#63)
- **Uncovered areas and vertical differences.** `PlanAreaService` gains
  `measure_uncovered_area`: an object's footprint outside the union of a
  cover's footprints grown by a stated length, refused by default.
  `AxiolidPlanAreaService` measures it exactly for planar meshes where the
  growth is zero or the grown corners lie outside the element, and otherwise
  brackets the grown disc between an inscribed and a circumscribed 16-gon and
  widens by the chord deviations. `VerticalExtent` gains `height_metres` and
  `uncovered_height`, the height of an extent outside a set of grown extents
  as sure bounds.
- **Stair and ramp geometry.** (Refs #85) A new `WalkingSurfaceService`
  (`WalkingSurfaceServiceHandle`) measures a straight stair flight's base,
  top and treads (elevation, nosing and back edge along the direction it
  climbs), a ramp's sloped runs, and the headroom above a walking surface to
  the obstacles a request names. Risers, goings, nosings, rises, run lengths
  and slopes are derived in the contract as intervals sure to hold the exact
  value. `AxiolidWalkingSurfaceService` measures exact, closed, outward
  meshes: treads are upward level faces, the walking direction comes from
  the treads' centres, runs are planar upward faces flatter than 45°;
  tessellations, winders and turning flights, flights in several pieces and
  warped runs are refused. The CLI registers it with `--geometry`. Two
  capabilities judge it: `stair-geometry` (riser, going, `2r + g`, nosing
  ranges, riser count, flight rise, riser and going uniformity, headroom)
  and `ramp-geometry` (`slope_limits` rows of maximum slope with optional
  maximum run length and rise, equal slopes, headroom). Winders, open
  risers, headroom under a flight, landing sizes, clear width, handrails and
  the slab connection remain open.
- **IDS translation of every facet a capability decides exactly.** (#50)
  The staging IDS importer (`staging/ids`) now translates entity predefined
  types (resolved as IDS resolves them: type object first, user-defined
  element, process and object types, `NOTDEFINED` deferring), attribute,
  classification, material and part-of facets in the applicability and as
  requirements, entity requirements for other classes, class patterns,
  prohibited properties and attributes without a value
  (`property-requirements` rows excluding `not-empty`, one per enumerated
  name), prohibited material, part-of and attribute facets
  (`selector-conformance` with the negated selector), classification
  requirements through the new `classification` capability (a system
  alone, patterns, optional and prohibited), material values against
  `axioval:material.Names`, `totalDigits` and
  `fractionDigits`, and specification cardinality (`object-count` per
  source, so a required specification reports a model without applicable
  objects and a prohibited one a model with any). What no capability
  decides exactly stays a reported gap: applicability property facets,
  property name patterns (openbimrs/ifc#78) and enumerations outside a
  prohibited facet, prohibited property values, material values
  restricted by several facets at once, applicability classifications
  without a value or given as patterns, part-of without a relation, through
  voids and fills, or to a type object, and requirements on a prohibited
  specification. The buildingSMART corpus runs without a mismatch (126
  exact passes, 42 sound passes, 80 caught fails). See the new "IDS
  import" page.
- **`classification` requirements.** The `classification` capability
  requires what a `classification` selector cannot state: `systems` and
  `codes` as literals, `system_patterns` and `code_patterns` as XML Schema
  patterns, a system alone, `optional` (holds for an object without any
  classification, must be met by one with any) and `prohibited`. It reads
  the classification service and decides as the selector does: one
  assignment meets the system and the code together, a code matches the
  assigned item or any ancestor, and an assignment without a stated system
  is not evaluated when it could decide the verdict.
- **Chained relationship steps.** A `path` step ending in `+`, such as
  `IfcRelAggregates:backward+`, is taken one or more times, in `related`
  selectors and every capability that takes a `path`.
- **Digit limits in `property-value`.** `total_digits` and `fraction_digits`
  bound a number's digits as XML Schema counts them; a decimal is counted on
  its shortest round-trip decimal. Text, booleans and dates are an invalid
  declaration. **Breaking:** definitions bound to `property-value` must
  declare the two new optional parameters.
- `axioval_rules::translate_xsd_pattern` exposes the XML Schema pattern
  translation behind `property-value` for `matches` selectors.

- **Exit separation.** The new capability `exit-separation` requires each
  selected space's exits, reached through `exit_path` (such as
  `axioval:derived.adjacent-space:backward`) and filtered by
  `exit_selector`, to lie at least `fraction` (one half by default) of the
  space's longest plan diagonal apart, or `flagged_fraction` when the boolean
  `flag`, read on the space or through `flag_path` (its storey, its
  building), is true: a third when sprinklered. `separation` measures between
  closest points (the proximity service's `horizontal` distance), `centres`
  or `farthest` points; `pairs` `any` (the default) needs one pair far
  enough apart, `all` every pair; `minimum_exits` makes too few exits a
  finding. Lengths are intervals: a straddling pair is not evaluated, an
  unknown flag widens the requirement to both fractions, and an exit the
  selector cannot decide leaves a verdict it could change not evaluated.
  (#81)
- **Plan spans.** A new `PlanSpanService` (`PlanSpanServiceHandle`) measures
  an object's longest plan diagonal and the centre-to-centre and
  farthest-point distances between two footprints as `PlanLength`
  intervals. `AxiolidPlanSpanService` measures them from convex-hull vertices
  and the overlay centroid of the plan-area footprints, exactly for planar
  meshes and within a derived bound for tessellated ones; the CLI registers
  it with `--geometry`.
- **Model quality checks.** (#74) Three capabilities, mapped per sub-check
  in the capability docs' "Model quality" section:
  - `body-extent` measures each object's body along one of its own
    placement axes (`axis` `right`, `forward` or `up`) and compares it with a
    stated length (`target_property`, within a `tolerance`) or a
    `minimum`/`maximum`. With `forward` and the material set's
    `TotalThickness` it checks a wall's layer thickness against its body.
    The extent is the whole body's depth along the axis, so it is the
    thickness only of a straight wall; intervals straddling the target are
    not evaluated, an absent target is a finding.
  - `triangle-count` limits the triangles of each object's mesh. The count
    is of the mesh the host produced, not a source fact; a tessellation of
    curved faces is counted with approximate evidence and its finding says
    the count depends on the tessellation.
  - `same-container` requires an object to lie in the same nearest
    containers as every counterpart a path reaches from it: a door or window
    on another storey than its host wall. Undecided containers or
    counterparts are not evaluated.

  The vertical-extent seam gains `DirectionalExtent`
  (`measure_directional_extent`, refused by default), which the Axiolid
  service answers exactly along coordinate axes and as a rounding-widened
  interval along any other direction. A new `TriangleCountService`
  (`TriangleCountServiceHandle`) is implemented by
  `AxiolidTriangleCountService` and registered by the CLI with
  `--geometry`. Space-boundary coverage stays open: no service measures the
  boundaries' connection geometry. Door swing direction waits on door
  leaves (openbimrs/ifc#148).
- **Model comparison on the command line, with placement, geometry and
  coordinate-system facets.** `axioval compare --base A.ifc --revised B.ifc`
  compares two revisions of one model, matched by `GlobalId`, on kind,
  classifications, relationships, each `--property SET.NAME`, placement and
  coordinate system, and with `--geometry` each object's measured bounds.
  Lengths and angles are judged against `--length-tolerance` (default
  0.005 m) and `--angle-tolerance` (default 0.01°); a tessellated difference
  straddling the tolerance is undetermined, never rounded. The result has
  the shape of a check's, so `--report`, `--summary`, `--bcf` and
  `axioval report` work unchanged, plus a `comparison` field listing every
  added, removed, changed and incomplete identity with its differences per
  facet, the counts, unidentified and ambiguous objects, and the coordinate
  systems. Exit status as for `check`: 0 identical, 3 differences, 4
  incomplete. Two files with one name become the sources `name@base` and
  `name@revised`. In `axioval-rules`, `ComparisonRequest` gains
  `with_placement`, `with_geometry` and `with_coordinate_systems`, each
  with a `ComparisonTolerance`: placement compares object frames (origin
  distance, axis rotation), geometry compares bounds widened by both chord
  deviations, and coordinate systems compare each pair of sources (the only
  one per side, else one per declared discipline). A comparison is a host
  entry point, not a registered capability, since it needs two sessions.
  A certified mesh difference (two-sided Hausdorff distance) waits on
  axiolid/kernel#148; until then geometry compares bounds only.
  **Breaking:** `ObjectChange::Matched` gains `undetermined`, `Unresolved`
  names its `Facet` and `subject`, `Difference` gains `Measured` and
  `Stated`, `ComparisonError` gains `InvalidTolerance`, and
  `ModelComparison::report` gives each entry a rule id per facet
  (`RULE.added`, `RULE.removed`, `RULE.property`, `RULE.placement`, ...,
  `RULE.identity`) and one finding per changed facet. (#73)
- **Source coordinate systems.** A new `CoordinateSystemService`
  (`CoordinateSystemServiceHandle`) states per source its world frame
  (`CoordinateFrame`, in metres), true north and map conversion
  (`MapConversion`: target system, offset, rotation, scale, and the map unit
  when known exactly), each `None` when unstated. The IFC session registers
  it from the model context's `WorldCoordinateSystem` and `TrueNorth` and an
  IFC4 `IfcMapConversion`; ambiguous statements are refused. Federated
  sessions route it by source.
- **Bodiless is not unmeasured in proximity.** `ProximityError::NoBody`
  reports an object declared to occupy no material; Axiolid proximity
  returns it for host-declared bodiless objects instead of `Unavailable`.
  Clash and distance treat it as before. **Breaking:** `ProximityError`
  gains a variant.

- **Sill heights in `keyed-limit`.** The new `quantity` `sill-height`
  limits each object's bottom elevation above the floor of every object the
  new parameter `floor_path` reaches from it, through the vertical-extent
  service: with `floor_path` and `key_1_path` set to
  `axioval:derived.adjacent-space`, one table bounds window sills per space
  type. Each reached floor is judged on its own, so a window between spaces
  with different floor elevations is found when it is too high above either;
  the finding names and relates that space. Sill heights are intervals that
  always hold the exact difference; one straddling a bound, an unmeasurable
  floor (unless another floor fails) or window, and a path reaching nothing
  are not evaluated. Windows at corridor ends are not decided yet: that needs
  a corridor axis no service provides. **Breaking:** definitions bound to
  `keyed-limit` must declare the new optional parameter `floor_path`. (#86)
- **Manual checks on an empty selection.** `manual-issue` raises its check
  once for the project when the selection decidedly picks nothing, instead
  of raising nothing.
- **Object frames.** A new `ObjectFrameService` (`ObjectFrameServiceHandle`)
  supplies an object's placement frame as a `MetricFrame` grounded on the
  object, in canonical metres with right-handed right, forward and up axes,
  and an `ObjectFront` that is `Stated` only where the source states one. The
  handle refuses uncovered sources and frames of another object; evidence
  must be exact and reviewable. The IFC session registers it without
  geometry: frames compose the `IfcLocalPlacement` chain through
  `ifc-geometry`'s placement resolver and convert the origin through the
  exact project length unit. IFC states no front, so every IFC frame reports
  `NotStated`; unplaced objects are `NotPlaced`, and `IfcGridPlacement` and
  unresolvable length units are refused. Door leaves are still to come. (#36)
- **Several models in one check, with a discipline per source.**
  `axioval check` takes `--model` repeatedly; each file is one source of one
  session, under source-qualified identities, and `--model PATH:DISCIPLINE`
  declares the discipline it plays (`arch.ifc:architecture`). With
  `--geometry` every model is meshed into one geometry set bound to all
  snapshots, so a clash between two files is an ordinary pair. Two models
  with one file name are refused, and an invalid discipline is a usage
  error. Summaries over several documents name objects `arch.ifc/#42`, and
  `report --object` now accepts that form. `EvidenceSession::federate`
  combines sessions over disjoint sources and routes each semantic service
  (properties, relationships, type hierarchy, classifications, integrity,
  object frames) to the member owning the request's source, refusing an
  uncovered source
  and a member holding a service it cannot route (`UnfederableService`).
  `EvidenceSession::with_discipline` declares a source's `Discipline` (a
  lowercase token, new in `axioval-ir`), kept beside the snapshot rather
  than in its identity; the runtime installs the declarations per run as
  `SourceDisciplines`. The new `discipline` selector
  (`{"kind": "discipline", "value": "structure"}`) selects every object of
  the sources playing that discipline; an object whose source declares none
  is not evaluated, once per rule and source, never a non-match. Axiolid
  contact, facade-area, vertical-extent, shelf-length, clear-height and
  derived-relationship evidence now cites the measured object's source.
  **Breaking:** `Selector` and `IrError` gain the `Discipline` and
  `InvalidDiscipline` variants, `EvidenceSessionError` gains `UnknownSource`,
  `DuplicateDiscipline` and `UnfederableService`;
  `TypeHierarchyServiceHandle` is no longer a tuple struct;
  `AxiolidContactService::new`, `AxiolidDerivedRelationshipService::new`,
  `AxiolidFacadeAreaService::new`, `AxiolidLinearQuantityService::new` and
  `AxiolidVerticalExtentService::new` no longer take a `SourceId`; the CLI's
  `--model` value is parsed as `PATH[:DISCIPLINE]`. The MCS package schema
  needs the matching `discipline` selector. (#28)

- **Derived light-opening area in `area-ratio`.** With
  `numerator_derivation` `light-area`, each numerator member's area is its
  light-transmitting area from the first step that produces one: the area
  `numerator_property` states, else the most specific `light_area_table` row
  keyed on a `type` name pattern (`light_type`, optionally via
  `light_type_path`) and the member's `overall_width` × `overall_height`
  (within `light_size_tolerance`), else width × height less the frame
  allowance 2·(W+H)·`frame_width`. A step is skipped only when its input is
  exactly absent, never on a wrong-typed value, an unknown type name or tied
  rows; a member no step gives an area leaves its anchor not evaluated. Each
  area carries an `axioval:derived.light-area:<member>:step=<step>` evidence
  entry and findings count the areas per step. A stated light area larger
  than the member's overall area is a finding against the member (its anchor
  is not evaluated), and `empty_numerator_finding` reports an anchor that
  reaches no numerator object, such as a space with no window, instead of a
  ratio of 0. `measure: facade` with `light-area` is an invalid
  declaration. **Breaking:** definitions bound to `area-ratio` must declare
  the new optional parameters `numerator_derivation`,
  `empty_numerator_finding`, `overall_width`, `overall_height`,
  `light_area_table`, `light_type`, `light_type_path`,
  `light_size_tolerance` and `frame_width`. (#54)
- **Envelope bounding spaces come from the rule.** `external-wall-validation`
  takes `derivations`, a list of `all-spaces` and `gross-area-groups`, and
  runs each on its own in one rule; every finding and not-evaluated outcome
  names its derivation. `all-spaces` is bounded by the objects
  `bounding_selector` selects, `gross-area-groups` by the members of the
  groups `gross_area_group_selector` selects, reached along
  `gross_area_group_path` (with IFC, `IfcRelAssignsToGroup:forward`). The
  resolved set travels in the `EnvelopeMembershipRequest` and the Axiolid
  adapter derives around exactly those objects. A derivation without its
  bounding input is an invalid declaration; an undecided selection, an empty
  one, groups with no member or a refused relationship answer leave that
  derivation not evaluated while the other still runs. A bounding object
  without a mesh (bodiless, unmeasured or undescribed) makes the derivation
  unavailable instead of being skipped, and a bounding object's own
  declaration no longer takes part in the comparison. The CLI registers the
  envelope service with `--geometry` alone. **Breaking:** the
  `envelope_derivation` parameter is replaced by `derivations`, and
  definitions must declare the three new optional parameters;
  `EnvelopeMembershipRequest::new` takes the bounding objects, the request is
  no longer `Copy` and `EnvelopeMembershipEvidence::request` returns a
  reference; `AxiolidEnvelopeMembershipService::with_space` and
  `with_gross_area_space` are removed; the CLI's `--envelope-zone` is
  removed (a usage error, status 2), since a rule states the same zone with
  `gross_area_group_selector` and `gross_area_group_path`.
- **Distance modes, projections and scoping.** `distance` takes a `mode`:
  `nearest` (the default, as before), `none_closer_than` (no counterpart
  closer than `minimum_metres`) or `at_least` (at least `count`
  counterparts within `maximum_metres`, and no nearer than
  `minimum_metres` when declared, so N within a range). A `projection`
  measures the distance in space (`minimum_3d`, the default), in plan
  between footprints (`horizontal`), between the vertical extents of bodies
  above one another (`vertical`, related when their footprints overlap or,
  with `footprint_offset_metres`, come closer than the offset), or as
  overlapping footprints (`plan_overlap`). The traversal parameters
  (`relationship` or `path`, as elsewhere) scope counterparts to those
  sharing a container with the subject, declared or derived
  (`axioval:derived.contained-in-space`). Every distance is judged as an
  interval: undecided counterparts (a straddling interval, an unmeasured
  distance or extent, an undecided container) count as unknown, and a
  verdict is given only when they cannot change it. `ProximityRequest`
  carries a `ProximityProjection` (`ProximityRequest::projected`),
  `ProximityService::measure_distance` returns a
  `ProjectedDistanceEvidence` interval (exact evidence is a point;
  infinite bounds say the bodies are unrelated in the projection), and
  `projected_candidate_pairs` keeps the broad phase complete in each
  projection. The Axiolid adapter measures horizontal distance exactly over
  the projected triangles, non-convex footprints and edge-on sheets
  included, and widens every projection on tessellated geometry by the
  chord deviation; it asserts a tessellated plan overlap only from a
  witness point deeper than the deviations, and leaves it open otherwise.
  Door-swing footprints as sources wait on object frames (#36).
  **Breaking:** definitions bound to `distance` must declare the new
  optional parameters (`mode`, `count`, `projection`,
  `footprint_offset_metres` and the traversal parameters); `distance`
  judges tessellated measurements by their interval, so one straddling a
  bound is now not evaluated rather than decided on the mesh value;
  `ProximityError` has the new variant `UnsupportedProjection`,
  `ProximityRequest` a projection field, and `ProximityEvidence::try_new`
  refuses a request in any projection but `minimum_3d`.
- **Date and date-time values.** `PropertyValue::Date` is a calendar day
  (`{"type": "date", "value": "2026-09-27"}`) and `PropertyValue::DateTime`
  an instant with the UTC offset it was stated in
  (`{"type": "dateTime", "value": "2026-09-27T10:00:00+02:00"}`), both
  validated ISO 8601 (`axioval_ir::temporal`, no new dependency). A
  date-time without an offset is not representable. Packages state `date`
  and `dateTime` literals (`ParameterValue`, `ParameterKind`,
  `PropertyValueKind` and `ParameterType` variants); one that is no real
  day, or a date-time without an offset, is refused when the package is
  read. Dates compare by day and date-times as instants whatever their
  offsets; a date-time compares with a date only when the rule states
  `precision` `day`, which reads it as the calendar day it states in its own
  offset. `property-predicate` takes `date` and `date_time` targets and
  `precision`, `property-comparison` `target_date`, `target_date_time` and
  `precision`, `property-value` casts literals to dates and takes
  `precision`, and property selectors take `date` and `dateTime` values with
  an optional `precision: day` (lists under a `quantifier` too); `between`
  ranges and table columns do not take dates yet. Without day precision a
  date-time against a date is not evaluated, never guessed. `unique-value` and
  `consistent-value` treat one instant in two offsets as one value. The IFC
  adapter reads `IfcDate` as a date, `IfcDateTime` with an offset and
  `IfcTimeStamp` (UTC) as date-times, in property sets and attributes; an
  `IfcDateTime` without an offset is refused as incomplete, and text that is
  not the type's ISO 8601 form is an invalid value. **Breaking:**
  `PropertyValue`, `ParameterValue`, `ParameterKind`, `PropertyValueKind`
  and `ParameterType` have new variants; `Selector::Property` has the new
  field `precision` (omitted when unset, so existing packages read and write
  unchanged); definitions bound to `property-predicate`,
  `property-comparison` and `property-value` must declare the new optional
  parameters; IFC date and time properties, previously text or integers,
  are now dates, so a text or integer rule over them is not evaluated or
  fails as a type mismatch; `PropertyResolutionError::InvalidValue` now
  reads "property value is invalid for its type".
- **Property requirement tables.** The new capability
  `property-requirements` checks each selected object against every row of
  a `requirements` table that applies to it (`applies_to`, a selector such
  as one exact class or a class with its subtypes). A row names a property
  by `property_set` and `property`, marks it `required`, `optional` or
  `forbidden`, and may constrain its value with `value_like` (a wildcard
  pattern), `one_of` (`|`-separated values) and a `minimum`/`maximum` range
  in `unit`, optionally divided `per` the object's measured plan area or
  its stated area (`area_property`) or volume (`volume_property`). Each
  failing row is a finding whose message names its result: missing
  property, missing value, forbidden property present, forbidden value or
  wrong value. Property set and property names are resolved exactly: the
  property service cannot list an object's properties, so a row with a
  wildcard name or a set without a property is reported not evaluated for
  the rule while the other rows are checked.
- **Filtered requirement templates in `property-requirements`.** A row may
  now carry a `state` instead of a `requirement`: its statement must hold
  (`include`), must not hold (`exclude`) or is skipped (`ignore`, also on a
  `requirement` row, which then is neither checked nor refused). The
  statement is a `presence` (`defined`, `undefined`, `empty`, `not-empty`)
  or value conditions, which gain `one_of_like` (`|`-separated wildcard
  patterns), `contains` (a substring of text, an element of a list) and
  `decimals` (round a ranged value in the row's unit before bounding it).
  The rule's selector is the template's element filter and `applies_to` its
  class. With the new `group_by_value`, findings of one row, result and
  value found are one finding naming the count and relating the objects;
  with the new `category_property`, findings start with the object's
  category in brackets, as in `property-comparison`, and groups split by it.
  Rows without the new columns behave as before. **Breaking:** the
  `requirements` table has the new optional columns `state`, `presence`,
  `one_of_like`, `contains` and `decimals`, and `requirement` is now
  optional; the capability has the new optional parameters `group_by_value`
  and `category_property`. A definition bound to `property-requirements`
  must declare them.
- **Keyed limit tables.** The new capability `keyed-limit` checks a
  quantity of each selected object against the single applicable row of a
  `limits` table: a fire compartment's area limit by its building's fire
  class, its use class, and whether its storey is sprinklered. Up to four
  keys `key_1` … `key_4` are property references, each read from the
  object or, with `key_<n>_path`, from the objects a relationship path
  reaches from it; rows key them with text patterns, and the most specific
  matching row applies. `quantity` is `plan-area` (the measured footprint,
  three-valued as in `plan-area`) or `property` (`quantity_property`, in SI
  units), bounded by the row's `minimum` and `maximum`. No matching row is a
  "no limit defined" finding; a row without bounds sets no limit; a key
  that is missing or disagrees across reached objects, when a row testing
  it could apply, and rows tied for most specific are not evaluated.

- **Doors, windows and openings connect the spaces their wall calls for.**
  The new capability `opening-spaces` requires each selected element to
  relate to two spaces, one on each side, when its host wall is internal,
  and to one space with the other side outside when it is external. The host
  is reached through `host_path` (fills and voids) among `host_selector`
  objects, and its exposure is the boolean `external_property` (such as
  `IsExternal`) resolved through the property service; an undeclared,
  null or non-boolean value leaves the element not evaluated. The spaces are
  reached through `space_path`, a stated relationship such as
  `IfcRelSpaceBoundary` or the derived `axioval:derived.adjacent-space`,
  whose recorded sides must be opposite: two spaces on the same face are a
  finding. A source in which no host wall is declared external is reported
  against the source. The engine exports `AdjacentSide` and
  `adjacent_side`, which read the side an adjacency evidence locator
  records, and documents that locator form as the provider contract.
- **Storey metrics.** `level-spacing` measures the highest level from
  geometry with `content_path` (and optionally `content_selector`): the
  highest top of the contents the path reaches, through the vertical-extent
  service, less the level's elevation, judged as an interval. With
  `space_selector`, `space_path` and `space_tolerance`, each level's spaces
  must be as high as the level. A new facade-area service
  (`FacadeAreaService`, `FacadeArea`, `FacadeAreaServiceHandle`) measures an
  object's outward-facing surface, with an Axiolid implementation
  (`AxiolidFacadeAreaService`) that the CLI registers with every `IfcSpace` as
  the interior; `area-ratio` and `plan-area` take `measure: facade` to judge
  facade areas instead of footprints, which gives the facade area per storey
  and the window-to-wall ratio per storey and per building. Net-to-gross and
  empty-area ratios, storeys without elements and compartment-to-group areas
  are compositions of existing capabilities, documented under "Storey
  metrics". A tabular report beside the findings is not part of this change.
  **Breaking:** the `level-spacing`, `area-ratio` and `plan-area` signatures
  gain optional parameters, so definitions bound to them must declare them.

- **Selecting objects by related objects.** The new selector kind
  `related` follows a relationship `path` from each candidate (steps
  `Relationship` or `Relationship:direction`, as in the `path` parameter)
  and tests the reached objects against a nested `selector` under a
  `quantifier`: `any` (the default, omitted when serialized), `all` (at
  least one reached, and every one matches) or `none`. Fire-wall doors are
  doors whose wall, reached through `IfcRelFillsElement` then
  `IfcRelVoidsElement` backwards, states `Compartmentation` true. A refused
  relationship answer or a reached object the nested selector cannot decide
  leaves the candidate not evaluated unless the others settle the verdict.
  **Breaking:** `Selector` has the new variant `Related`, beside the new
  enum `RelatedQuantifier`; existing packages serialize unchanged.
- **Table allocation.** The new capability `table-allocation` assigns each
  selected object to exactly one row of a `rows` table, by the first or the
  most specific matching row (`mode`), and checks each row per anchor
  (`anchor_selector` and the traversal parameters, such as per storey), per
  source or across the project: exactly `count` objects, and a summed plan
  area within `area` ± `area_tolerance` square metres, measured or stated
  by `area_property`. Rows key on text patterns over up to three properties
  the rule declares as `key_1` to `key_3`. Objects no row matches and rows
  that matched nothing are findings; ties, undecided keys and straddling
  areas are not evaluated.
- **Group composition.** The new capability `group-composition` requires
  each selected group to hold the members a `requirements` table lists:
  entries keyed by text patterns over up to three member properties
  (`key_1` to `key_3`, as in `table-allocation`), each taking `count`
  members. Members are reached through the (required) traversal parameters
  and restricted by `member_selector`. A member may fit several entries but
  fills one place: members are allocated by a maximum bipartite matching,
  not in declared order. Per group, entries every maximum allocation leaves
  short are findings, jointly when competing entries could each be the short
  one; members beyond the places they fit, and members fitting no entry, are
  surplus findings. With `group_key`, rows with a `group` pattern apply only
  to matching groups, and a group no such row matches is a finding. With
  `ungrouped_selector`, objects no group reaches are findings. An undecided
  membership, member key or group key leaves the group not evaluated.
- **Every presentation layer of an object, and models without layers.**
  `axioval:presentation.Layer` now lists all distinct layers of an object,
  sorted by name, as the new `PropertyValue::List` (scalar elements only;
  the resolver rejects a null or nested element). Property selectors take a
  `quantifier`: `any` or `all` elements must satisfy the operator, a scalar
  counts as a list of one, and `all` never holds for an empty list. "Every
  layer is agreed" is `oneOf` with `quantifier: all`, "at least one layer is
  agreed" the same with `any`. A source that records a kind of fact for no
  object answers `PropertyResolutionError::NotRecorded`, reported as the new
  `NotEvaluatedReason::NotRecorded` once per rule and source like an unbound
  concept: the IFC adapter answers so for a model without any
  `IfcPresentationLayerAssignment`, so a layer rule over it is not
  applicable rather than a pass or a "no value" finding per object.
  **Breaking:** an object on several layers is no longer a conflict; its
  layer is a list, which a selector without `quantifier` leaves not
  evaluated, so existing layer rules must add one. `Selector::Property` has
  the new field `quantifier` (omitted from serialized packages when unset),
  and `PropertyValue`, `PropertyResolutionError` and `NotEvaluatedReason`
  each have a new variant. `property-value` does not evaluate a list.
- **Plan area ranges.** The new capability `plan-area` requires each
  selected object's measured footprint to lie within `minimum` and
  `maximum` square metres (a space area range, a fire compartment's area
  limit), or, with `member_selector`, the summed footprints of the members
  each anchor reaches through the declared traversal (the space area of each
  storey). Tessellated areas are intervals, and one straddling a bound is not
  evaluated; an object without a body, or an anchor with such a member, is
  not evaluated. Undecided members can only add area, so only an excess
  over the maximum stands.
- **Table-valued rule parameters.** A `table` parameter declares named
  `columns`, each with a kind (`string`, `textPattern`, `number`,
  `quantity`, `integer`, `boolean`, `selector` or `reference`) and whether
  it is required; its value is a list of rows mapping column IDs to cells of
  those kinds. Capabilities declare the columns with
  `ParameterType::Table(&[TableColumn])`, and the binder requires the
  definition to declare the same columns and every row to fit them: an
  unknown column, a cell of another kind, a missing required cell or a
  malformed text pattern fails compilation with
  `EngineError::InvalidTableRow`. `axioval-rules` gains a shared row
  matcher (first match, most specific match, all matches) that fails closed
  on undecided rows and reports ties. **Breaking:** `ParameterKind`,
  `ParameterValue`, `ParameterType` and `EngineError` have new variants, and
  `ParameterDefinition` has a `columns` field; it is omitted when empty, so
  existing packages read and write unchanged.
- **Relationships derived from geometry.** Checks that relate components to
  the space they stand in, doors and openings to the spaces they connect, or
  rooms to a larger group space no longer need the model to state those
  relationships. `DerivedRelationshipServiceHandle` answers the same
  relationship-selection requests for three identities,
  `axioval:derived.contained-in-space`, `axioval:derived.adjacent-space` and
  `axioval:derived.overlapping-group-space`, with their tolerances as
  `;key=value` parameters and the derivation named in every evidence locator.
  `EvidenceSession::with_derived_relationships` routes those identities to it
  and every other one to the semantic service, so every capability taking a
  `relationship` or `path` (`related-count`, `property-comparison`'s
  `same_space`, and the rest) uses them unchanged.
  `AxiolidDerivedRelationshipService` derives them: containment by winding
  number with a nearest-space fallback, per-side adjacency by a probe swept
  from each face of the opening (an external door relates to one space and
  records the outside side), and footprint-overlap grouping. Unmeasured or
  bodiless spaces, points on a boundary, ties and tessellated geometry near
  the decision refuse. `axioval check --geometry` declares every `IfcSpace`,
  `IfcDoor`, `IfcWindow` and `IfcOpeningElement`, meshing opening voids for
  the derivation alone.
- **Property comparison along paths, within spaces and buildings.**
  `property-comparison` takes a relationship `path` in `related` mode, and
  new `same_space` and `same_building` modes compare with the objects that
  share a nearest `container_selector` object, climbed to through the
  relationship service along the declared steps. New operators, named as in
  property selectors: `like` (whole-value wildcards `*` and `?`, `\`
  escapes), `matches` (a regular expression anchored to the whole value),
  `contains` over a text list, `is_defined` and `is_undefined`, and
  `between` for an inclusive range, which gives `count` and `sum` a minimum
  and maximum in one rule. `case_sensitive` relaxes text comparison, and
  `category_property` prefixes each finding with a property value of the
  checked object. The geometric same-space variant is not included. Invalid
  declarations now say what is wrong. **Breaking:** a definition bound to
  `property-comparison` must declare the new optional parameters `path`,
  `container_selector`, `case_sensitive`, `minimum_number`,
  `maximum_number`, `minimum_quantity`, `maximum_quantity` and
  `category_property`.
- **A zone's footprint is the union of its members.** A bodiless group, such
  as an `IfcZone` modelling a fire compartment, measured a zero plan
  footprint, so `plan-coverage` and area limits against it could never pass.
  `AxiolidGeometry::with_group` declares a group's members and
  `with_undecided_group` a membership the host could not decide;
  `AxiolidPlanAreaService` then measures the group as the union of its
  members' footprints, nested groups included. A member without a body, an
  unmeasured or undescribed member, an empty or self-containing group and an
  undecided membership make the footprint unavailable, never zero; a
  tessellated member makes it approximate. `axioval check --geometry` reads
  every `IfcGroup`'s members from `IfcRelAssignsToGroup` and declares them.
- **Materials.** The reserved `axioval:material` set (`MATERIAL_SET`) names
  the material an object is made of: its `Kind`, `Name` and `Category`, a
  layer set's `TotalThickness`, the member `Count`, and numbered members
  such as `Layer1.Material` and `Layer1.Thickness`, and `Names`, a list of
  every name and category the material, its members and their materials go
  by (`MATERIAL_NAMES`), which a selector with `quantifier: any` matches
  without enumerating members. Every property capability and selector
  reads them. The IFC adapter answers them from
  `IfcRelAssociatesMaterial` through `ifc-material`, on the object or
  else its type object: single materials, layer sets (directly or through
  a usage), constituent sets, profile sets and material lists, thicknesses
  in metres with exact provenance; a unit the file does not resolve refuses
  that measure alone. An object without material is an exact absence; two
  assignments conflict.
- **Property selectors on par with `property-predicate`.** Selectors take
  the operators `like` (wildcards `*` and `?`, `\` escapes), `contains`,
  `oneOf` and `noneOf` (a string list), and the options `caseSensitive`
  (default `true`) and `trim` (default `false`) for text comparisons.
  Quantities compare in SI with a quantity of the same dimension, so
  `2400 mm` selects a length of `2.4 m`. `Selector::property` builds a
  selector with the default options. **Breaking:** `Selector::Property`
  has the two new fields; they are omitted from serialized packages when
  default, so existing packages read and write unchanged. `matches` now
  matches the whole value, as in `property-predicate`: `EI\d+` no longer
  selects `EI30-T1` (write `EI\d+.*`). A value of another type than the
  selector compares (text against a number, a quantity against a unit-less
  number or another dimension) makes the object not evaluated instead of
  silently dropping it from the selection, `notEquals` included. A value
  that does not fit its operator is an invalid declaration.
- **Findings about a source or the whole project.** A finding or
  not-evaluated outcome now has a `Scope`: one object, one source, or the
  project. "The model has no building" is a finding against the source
  instead of nothing, which read as a pass. Scoped findings carry exact
  evidence and related objects like object findings; the runtime orders
  project, then sources, then objects. `CapabilityEvaluation` gains
  `push_source_not_evaluated`, and an unbound concept, already reported once
  per source, is now scoped to that source. **Breaking:** `Finding::object_id`
  and `NotEvaluated::object_id` are replaced by `scope`, with `object_id()`
  accessors; `Finding::new` and `with_evidence` build one. The serialized
  form of object findings and rule-level outcomes is unchanged, but a report
  with scoped entries (a finding without `object_id`, or with `source`) needs
  a reader of this version.
- **Existence and cardinality checks.** `object-count` requires the rule's
  selection to hold between `minimum` and `maximum` objects (at least one
  by default) per source, or in the project with `across_sources`, and
  reports an empty selection as "no object matches the selection" against
  that source or the project.
- **Model-level BCF topics and CLI entries.** A source or project finding
  becomes a BCF topic without a viewpoint or component, whose GUID does not
  depend on the source's file name; object and rule-level topics keep their
  GUIDs. `axioval report` lists such an entry with `scope` in place of
  `object`, and `--object` accepts a source document name.
- **Relative counts by group, for small counts and at table edges.**
  `relative-count` takes `group_property` to count per property value (a
  location code, say) instead of per anchor, within one source unless
  `across_sources`; a group with required objects and no provided object is
  reported as present only in the required set. In ratio mode,
  `small_required_below` and `small_provided` state the requirement for
  small nonzero required counts explicitly. Below a table's first row the
  anchor or group is now skipped; it was extrapolated from zero with the
  increments.
- **Numeric tolerance and rounding.** `unique-value`, `property-comparison`
  and `property-predicate` take optional `tolerance` (absolute),
  `relative_tolerance` (a fraction of the larger magnitude) or `decimals`
  (round half away from zero, as the value is displayed) for numbers and
  quantities, quantities in SI units. Within the tolerance two values are
  equal, boundary included, and only beyond it greater or less. Rounding
  groups `unique-value` keys into classes; a tolerance is not transitive, so
  `unique-value` judges it pair by pair and each finding names exactly the
  objects within the tolerance of its own value. Findings state the
  tolerance used. **Breaking:** a definition bound to one of these
  capabilities must declare the three new optional parameters.
- **Slab stack spacing.** `slab-stack-spacing` pairs slabs whose footprints
  overlap by at least a declared share of the smaller one, orders each stack
  by top elevation, and checks consecutive top-to-top, bottom-to-bottom and
  top-to-underside distances against optional bands, optionally requiring
  them equal within a tolerance. Elevations come from the new source-neutral
  `VerticalExtentService` (bottom and top as intervals with evidence),
  implemented by `AxiolidVerticalExtentService` and registered by
  `axioval check --geometry`. A tessellated slab's elevations are never
  exact; a distance, overlap or order its intervals leave open is not
  evaluated.
- **Numbering consistency.** `numbering-consistency` reads a number from
  each value through a pattern with one captured group and requires the
  numbers of one scope (a source, or a storey reached through a relationship)
  to share a leading prefix and, optionally, to leave no gaps. A value the
  pattern does not number is not evaluated, never passed.
- **Slab-contact scope.** `slab-contact` takes an optional `counterparts`
  selector, and `ContactRequest` now carries the resolved candidates
  (`ContactRequest::new` takes them; a breaking change). The Axiolid adapter
  measures only those and refuses only when one of them is unmeasured or
  undescribed, not when any object anywhere is. Evidence naming an object
  outside the candidates is refused (`ContactError::UnrequestedCandidate`).
  `skip_top_storey` and `skip_bottom_storey` leave out subjects on the
  highest or lowest `storey_selector` object, ordered by the `Elevation`
  attribute; an unknown elevation or storey assignment is not evaluated.

- **Unbound concepts are reported once.** A package concept the source's
  declared vocabulary cannot express is now `NotEvaluatedReason::UnboundConcept`
  (it was `InvalidDeclaration`) and is reported once per rule, source and
  cause, with the number of affected objects and a few examples, instead of
  once per object. A 2,040-object IFC2X3 model checked against an IFC4-only
  package now yields one outcome and one BCF topic, not 2,040.
- **Geometry for `axioval check`.** `--geometry` meshes the model's net
  bodies (openings subtracted) with `ifc-geometry` and registers the Axiolid
  contact, free-space, space and proximity services, so clash, distance,
  contact, space and free-space rules run on real IFC files. Each body is
  exact (planar faces), tessellated (within 1 mm), without body, or
  unmeasured. Unmeasured bodies are listed in the result's new `geometry`
  field and make the measurements they could affect not evaluated. On a real
  6 MiB model, 951 bodies mesh exactly, 20 as tessellations and none fail.
- **Host-built services.** `EvidenceSession::with_host_service` registers a
  service the host built from the session's own source, bound to the
  snapshots the host names and checked like any other binding.
- **Bodiless and unmeasured objects.** `AxiolidGeometry::with_no_body` and
  `with_unmeasured` let a host say that an object occupies no volume, or that
  its body exists but could not be meshed. Free-space checks skip bodiless
  candidates instead of refusing them. Contact and space measurements refuse
  while an unmeasured object could change them, where they previously
  measured as if it were not there.
- **Bounded views of a check result.** `axioval check --summary` prints one
  line per rule, not-evaluated reason and integrity code, with counts, the
  most frequent message, example objects and the next command to run.
  `axioval report <result.json>` reads a saved result back as that summary,
  or with `--rule`, `--code`, `--object` or `--section` as a paged listing.
  Results now carry an `objects` index (kind and GlobalId of every object the
  report names). On a real model the summary is 712 bytes against 271 KB of
  JSON, so an agent can read the shape first and fetch entries on demand.
- **Declared property types.** `Property::data_type` carries the value's type
  as the source declares it, and the IFC session reports it (`IFCLABEL`,
  `IFCBOOLEAN`, ...). Unreported is `None` and serializes as before. This is a
  breaking change for code that builds `Property` with a struct literal.
- **`axioval:capability.property-data-type`.** A required, non-empty
  property whose declared type equals `data_type`. Another type is a finding;
  an unreported type is not evaluated. Measure-typed IFC values
  (`IFCLENGTHMEASURE`, ...) stay not evaluated while the adapter refuses them
  pending unit handling.
- **`axioval:capability.property-value`.** Values, XML Schema patterns,
  numeric bounds and lengths, written as lexical strings and cast to the
  resolved value's kind; decimals compare with the IDS tolerance, bounds
  exactly. An `optional` rule passes absent and `null` properties. Anything
  that cannot be applied to a value is not evaluated.
- **More IFC property types.** The IFC session carries every value whose
  defined type is `STRING`-, `INTEGER`- or `NUMBER`-based (`IfcDate`,
  `IfcDuration`, `IfcTimeStamp`, `IfcCountMeasure`, ...) with its declared type,
  instead of refusing it. Unit-bearing real measures are still refused.
- **`axioval check`.** Runs a ruleset over an IFC2X3 or IFC4 model and writes
  the report and the model's integrity issues as JSON, and optionally a BCF
  2.1 archive (`--bcf`). Exit status separates a clean pass (0), findings (3)
  and an incomplete check (4), so a check that could not evaluate something
  never exits 0. `SOURCE_DATE_EPOCH` makes BCF output reproducible.
- **Clash and distance checks.** New capabilities
  `axioval:capability.clash` (hard clashes beyond a penetration tolerance,
  containment, optional clearance) and `axioval:capability.distance` (the
  nearest counterpart within a minimum and/or maximum). They are built on a new
  engine contract, `ProximityService` / `ProximityServiceHandle`, and a
  complete sweep-and-prune broad phase, `candidate_pairs`. A pair the broad
  phase drops is proven farther apart than the margin, tessellation deviation
  included. `AxiolidProximityService` measures separation with
  `closest_points_on_triangles`, plan overlap with `axiolid-overlay`, and
  penetration by testing sampled points against winding numbers. A pair with
  one closed solid is measured, including a sheet crossing a wall; only two
  open surfaces report no penetration. Curved parts
  registered with `AxiolidGeometry::with_tessellated_mesh` produce approximate
  evidence, which is never marked exact.
- **Model comparison.** `compare_sessions` diffs two evidence sessions matched
  by an external identity scheme, such as IFC GlobalIds. It covers kind,
  classifications, carried and requested properties, and relationships named
  by target identity. Unidentified objects, ambiguous identities and facets
  that cannot be read are reported, never dropped. `ModelComparison::report`
  projects the result into a `Report` for any sink.

- **BCF export.** New crate `axioval-bcf` (facade feature `bcf`) writes a
  report as a BCF 2.1 archive through `openbim-bcf` 0.3: one topic per finding
  and per not-evaluated outcome, viewpoints selecting objects by GlobalId, and
  topic GUIDs that survive re-export of the model. It depends on `axioval-ir`
  only. See the *Report sinks* page.
- **Attribute sets.** Two reserved property sets,
  `axioval:attributes` (`ATTRIBUTE_SET`) and `axioval:type-attributes`
  (`TYPE_ATTRIBUTE_SET`), read an object's own attributes and those of its
  type object through the property resolver. They bind to themselves in
  every source, so packages reference them without declaring a concept. The
  IFC adapter answers them from the entity and from its
  `IfcRelDefinesByType` type object: a space's number (`Name`), name
  (`LongName`) and type name. Measures such as a storey's `Elevation` are
  refused until units are converted.
- **Quantity sets.** With `ifc-properties` 0.4.0, `IfcElementQuantity`
  members resolve like properties and convert to SI:
  `Qto_SpaceBaseQuantities.NetFloorArea` is an area quantity. Complex
  quantities, and a property set and a quantity set sharing a name, are
  refused, never answered as absent.
- **Measured values in SI.** `ifc-properties` 0.3.0 resolves a measure's
  effective unit exactly. The IFC adapter converts measure-typed property
  values and attributes to SI quantities, or refuses when no unit applies.
  `QuantityDimension` gains `PlaneAngle` and `Other { exponents }`, which is a
  breaking change for exhaustive matches.
- **Presentation layers.** The reserved `axioval:presentation` set's `Layer`
  is the presentation layer of an object's shape. The IFC adapter reads it
  through representations, items and mapped representations.
- **Plan areas.** `PlanAreaService` measures footprints and footprint
  overlaps as intervals; `AxiolidPlanAreaService` implements it. The
  capabilities `area-ratio` and `plan-coverage` judge them, and
  `level-spacing` checks storey heights from elevations.
- **Relationship paths and relative-count tables.** Relationship-scoped
  capabilities accept a `path` of relationship steps. `relative-count`
  gains a table mode.
- **Semantic capabilities.** These read exact properties, classifications
  and relationships and need no geometry:
  - `selector-conformance` checks agreed value lists.
  - `unique-value` checks identifiers per source or per related scope.
  - `consistent-value` checks that objects sharing a key share a value.
  - `related-count` and `relative-count` check absolute and relative counts
    of related objects.
  - `name-sequence` checks consecutive numbering in a declared order.
  - `manual-issue` records checks owed by hand.
- **`property-predicate`** compares text (`equal`, `not_equal`, `contains`,
  whole-value regex `matches`), text lists (`one_of`, `none_of`), numbers,
  booleans and presence (`is_defined`, `is_undefined`), with optional case
  folding. `value` became optional beside the new target parameters, so a
  definition declaring it required no longer matches; integer rules behave
  as before. Findings now name the actual
  value.
- **`property-comparison`** accepts constant targets (`target_number`,
  `target_text`, `target_texts`, `target_boolean`), `one_of`/`none_of`, and
  the `count` and `sum` quantifiers. `target_property` and
  `compared_property` are now optional parameters: a definition package
  declaring them required no longer matches the signature.

- **External identities.** `ExternalId` (scheme plus value) lets an object
  carry aliases beside its source-qualified `ObjectId`, read with
  `Object::external_id`. `Project::new` rejects an object with two ids in one
  scheme and two objects of one source sharing an id. This is a breaking change
  for code that builds `Object` with a struct literal; serialized objects
  without aliases keep their shape.
- **IFC GlobalId aliases.** The IFC session attaches each object's
  `IfcRoot.GlobalId` in the `ifc-globalid` scheme (`IFC_GLOBAL_ID`). An unset,
  malformed or out-of-range GlobalId (`identity.invalid-global-id`) and one
  claimed by several `IfcRoot` instances (`identity.duplicate-global-id`) are
  integrity warnings, and no object carries them. On three real exports
  (3,578 objects), every object carries its alias and no warning is raised.
- **Classification service.** `ClassificationService` /
  `ClassificationServiceHandle` let a source state which classifications an
  object carries, each as a system plus its code chain from the assigned item
  to the root. The IFC session registers one backed by
  `ifc-classification` 0.2.1: direct assignments and those inherited from the
  object's type, read per release (IFC2X3 `ItemReference`, IFC4
  `Identification`). An assignment whose system the file does not state, such
  as an IFC2X3 notation or a reference with no `ReferencedSource`, makes a
  system-qualified selector not evaluated rather than a mismatch.
- **Cardinality warnings.** The IFC integrity scan reports
  `spatial.contained-twice` (an element named by two
  `IfcRelContainedInSpatialStructure`) and `zone.member-not-spatial` (an
  `IfcZone` grouping something other than zones, spaces and spatial zones),
  both from `ifc-systems` 0.2.1. On one real IFC2X3 model the scan reports
  29 elements contained twice, matching an independent count of the file.
- **IFC2X3 support.** `import_ifc_session` accepts IFC2X3 TC1 as well as IFC4
  ADD2 TC1. The header's release is decided once (`IfcRelease`) and every
  service reads that release's schema: property resolution
  (`ifc-properties` 0.2.1), entity inheritance, relationship end slots and
  integrity. The snapshot declares `IFC2X3_TYPE_SYSTEM`, so a package binds
  on IFC2X3 data only through IFC2X3 external names. IFC4X3, several
  releases, or none are still refused. On six real models (one IFC4, five
  IFC2X3), 4,378 relationship answers match `ifc-spatial`'s independent
  reader. Of 22,392 real property assignments, every one is either resolved
  or refused for a stated reason; the 636 refusals are measure-typed values
  (`IFCAREAMEASURE`, `IFCENERGYMEASURE`, ...) the adapter does not yet map.
- **IFC2X3 materials.** (Closes #38) `ifc-material` 0.3.0 reads every
  material slot in the release the file declares (openbimrs/ifc#77), so the
  IFC adapter answers `axioval:material` for IFC2X3 files as for IFC4
  instead of refusing them: an `IfcMaterialLayerSetUsage`, a layer set, a
  single material and a material list. An attribute IFC2X3 does not
  declare (a material's category, a layer's name and category) is absent,
  since the file cannot state it. The IFC2X3 transparency fallback to a
  material's styles now answers as well.
- **Predefined property sets.** With `ifc-properties` 0.4.1
  (openbimrs/ifc#149), attributes of `IfcDoorLiningProperties`,
  `IfcDoorPanelProperties`, their window counterparts and every other
  predefined set resolve like properties, found by the set's `Name` or its
  entity name, measures in SI: a door's `Lining.LiningDepth` is a length.
  An unset, enumerated or referencing attribute is still refused, as are
  the enumerated, list, bounded, table and reference values `ifc-properties`
  now reads (openbimrs/ifc#150).

### Fixed

- **A clearance volume could be obstructed by a body that never entered
  it.** `AxiolidFreeSpaceService::assess_clearance` named an obstacle as a
  blocker when its height range overlapped the volume's band and its plan
  outline overlapped the volume's footprint, tested separately, so an
  L-shaped body with a low foot under the volume and a tall column beside
  it, or a table whose top overhangs the volume above its band, was
  reported `Obstructed` with exact evidence. An obstacle now obstructs only
  when its solid shares interior with the volume: one of its triangles
  meets the volume shrunk by 1 µm (the triangle's part inside the height
  band, compared with the footprint in plan), or the volume's centre lies
  inside it. Contact with a face stays clear. A cylinder keeps its
  inscribed and circumscribed 64-gons, now as prisms. An obstacle whose
  mesh box reaches the volume must be a closed, consistently and outward
  wound surface, or the request is refused: an open or inward-facing mesh
  bounds no solid to decide either way. `component-clearance` inherits the
  fix. (#88)
- **Breaking. A model with no objects could not be checked, and would have
  passed an existence rule.** `EvidenceSession::try_new` refused a snapshot
  whose source contributes no object, so an IFC file holding only
  presentation data failed with "snapshot source is not present in the
  project". Such a snapshot is now an empty source of the session, also as a
  federation member. `EvidenceSessionError::UnexpectedSource` is removed:
  an empty source is legitimate input, and a snapshot without objects cannot
  be told apart from one. `MissingSource` and `DuplicateSource` are kept.
  The runtime installs the run's sources per run as `SessionSources`
  (replacing any host copy, like `SourceDisciplines`), listing every
  snapshot's source, or the sources a bare project's objects name.
  `object-count` and `table-allocation` (without `anchor_selector`) form one
  scope per listed source, so an empty source reports "no object matches the
  selection in source …" and each row it cannot meet, instead of never
  being judged. Other capabilities judge only sources holding a selected
  object, so an empty source is to them what a source without a match is.
- **A derived relationship was unusable as a `path` step.** A step was split
  at its first colon, so `axioval:derived.adjacent-space` read as the
  relationship `axioval` with the direction `derived.adjacent-space` and was
  an invalid declaration. A derived step now ends only at a trailing
  `:forward`, `:backward` or `:either`.
- **Tessellated parts were measured as exact.** Axiolid contact, envelope,
  free-space, guard and space evidence is exact by contract, but it was
  reported for curved parts registered as tessellations. Each service now
  refuses with its inexact-evidence error when a tessellation could change
  its answer, and only then. Shelf length widens its upper bound by the chord
  deviation instead.
- **Closed solids lost their footprint in plan.** The Axiolid free-space,
  space, envelope-membership and guard services, and the new proximity service,
  projected triangles without orienting them. A closed, outward-oriented
  body's top and bottom faces project with opposite windings, and the non-zero
  overlay cancelled them to no area. Free area read zero, coincident spaces were
  not duplicates, a wall overlapping a space was not on the envelope, and a
  closed deck had no edges to guard. Existing tests used same-winding caps and
  never saw it; each service now has a closed-body regression test.
- **A quantity was reported as an exact absence.** `ifc-properties` resolves
  `IfcPropertySet` members only and skips quantity sets
  (`IfcElementQuantity`) and predefined property sets
  (`IfcDoorLiningProperties`, ...), yet the IFC property service turned its
  "absent" into complete absence evidence. A rule requiring `Foo` in a
  quantity set `Foo_Bar` therefore reported a wall as missing a quantity it
  carried. Absence is now refused as incomplete (not evaluated) when the
  requested set is one of those definitions, or, for an unqualified request,
  when one of them has a member of that name. The check is model-wide, so it
  can only turn an absence into not evaluated. Found by running translated
  buildingSMART IDS test cases.
  `ifc-properties` 0.4.0 resolves quantity sets itself (openbimrs/ifc#66),
  so this local guard is gone: see *Quantity sets* above.
- **Classification selectors silently passed over sources.** A
  `classification` selector read the project's inline classification list,
  which no production adapter fills. Over an IFC model every classification
  rule selected nothing and returned an empty report. It now asks the
  classification service, and a session without one reports each object as
  not evaluated (`MissingService`). **Breaking:** hosts that relied on inline
  `Object::classifications` must register a `ClassificationService`.
- **Package concepts were never bound to source data, so a checker could
  report a false pass.** A ruleset names canonical concepts
  (`axioval:fire.ifc4.wall`); the engine compared them verbatim with source
  kinds (`IFCWALL`) and property names (`Reference`). Nothing matched, the
  selector selected nothing, and the report came back empty: zero findings,
  zero not-evaluated. The compiler now builds a concept catalog, and every
  entity-type selector and property reference is translated per source through
  an external name in the type system that source's snapshot declares. A
  concept that cannot bind, binds ambiguously, or names a source with no
  declared type system is reported as not evaluated, never skipped.
- `includeSubtypes` was ignored: an `IfcWallStandardCase` was never selected
  by a wall rule. Subtype membership now comes from a
  `TypeHierarchyServiceHandle`; without one, a different kind is not evaluated
  rather than treated as a non-member.
- The package contract drifted from MCS `0.1.0` normalized output: localized
  package names, source catalogs, citations, requirements, target-group
  applicability, parameter citations and explanatory images were rejected, so
  every published Axioval package failed to load. The IR now accepts the
  complete contract, and the compatibility fixture is regenerated from the
  current MCS `minimal` example.

### Added

- `axioval-ifc` registers an exact `RelationshipSelectionService` over the
  model's objectified relationships (containment, aggregation, voids, fills,
  space boundaries, type and group assignment, and any other IFC4
  relationship with object ends). Relationship identities are IFC4 entity
  names; supertypes include every subtype. Checked against `ifc-spatial`'s
  independent spatial tree on a real IFC4 model: 323 containment answers, all
  equal.
  IFC2X3 sources are refused until upstream exact property resolution
  supports them (openbimrs/ifc#48).
- `RelationshipSelectionService::source_snapshots`, so a relationship service
  can be bound to an `EvidenceSession`.
- `AbsentEndPolicy` and `RelationshipSelectionRequest::with_absent_ends`: a
  relationship instance that leaves a schema-required end empty refuses the
  answer by default; `Skip` answers from the existing edges and cites each
  skipped instance. `property-comparison` exposes it as the optional
  `skip_absent_relationship_ends` parameter (default `false`). On the IFC4
  reference model, 98 virtual space boundaries without a
  `RelatedBuildingElement` previously made every space-boundary query refuse.
- `SourceIntegrityService` / `SourceIntegrityServiceHandle`, `IntegrityIssue`
  and `IntegritySeverity`: a channel for source irregularities that are
  neither findings nor not-evaluated outcomes. `axioval-ifc` reports absent
  required relationship ends as warnings (`ABSENT_REQUIRED_END`) and malformed
  relationships as errors (`MALFORMED_RELATIONSHIP`).
- `ConceptCatalog`, `ConceptBindings` and `BindingError`; the compiler rejects
  references to concepts no loaded definition package declares.
- `SourceSnapshot::with_type_system`, and `TypeHierarchyService` /
  `TypeHierarchyServiceHandle`.
- Target-group applicability: one group compiles as its selector; several
  groups are carried in `ExecutionPlan::deferred` and reported as not
  evaluated, because a one-selector capability cannot evaluate them.
- `axioval-ifc` declares the IFC4 type system (`IFC4_TYPE_SYSTEM`) and
  registers IFC4 entity inheritance from the bundled normative schema. An
  entity the schema does not declare is an error, not a non-member.

### Changed

- The workspace requires `ifc-geometry` 0.4.1 and `openbim-ifc` 0.7.2.
  Space-boundary coverage lowers each connection surface in the space
  body's frame from `ifc-geometry`'s `product_representation_frame`
  (openbimrs/ifc#164) instead of rebuilding the context's world
  coordinate system and moving the mesh afterwards: a curve-bounded plane
  under a non-identity frame now keeps its boundaries in the plane's
  parameters (openbimrs/ifc#163). Measurements are unchanged. (Refs #74)
- **Breaking.** The workspace requires `ifc-geometry` 0.4.0, `ifc-material`
  0.3.0, `ifc-properties` 0.4.1 and `ifc-spatial` 0.2.2. The CLI reads a
  space boundary's `ConnectionGeometry` through `ifc-spatial`'s
  `SpaceBoundary::connection_geometry` (openbimrs/ifc#156) and lowers it
  with `ifc-geometry`'s `lower_connection_surface`, so a face surface
  (`IfcFaceSurface`, `IfcAdvancedFace`) is measured for space-boundary
  coverage instead of leaving its space not evaluated (openbimrs/ifc#155).
  Compiled bodies change with the new geometry release: a B-rep's voids
  are meshed as cavities, every solid of a multi-solid B-rep is meshed,
  and an `IfcBlock` sits at its `Position` corner rather than half its
  size away. (Refs #74)
- The workspace requires `axiolid-overlay` 0.3.3, which settles
  `union_soup` output and accepts holes touching their outer ring, so
  every overlay output is a valid operand again (axiolid/kernel#191).
  Coverage now builds its effects as regions (a travel effect is the
  union of its reached cells) and unites and clips them with region
  operations instead of handing the overlay convex pieces, and the
  walkable plans of walkability and metric routing go back to the overlay
  as they are instead of re-cut into trapezoids. Measurements, bounds and
  refusals are unchanged.
- Dependencies: `axiolid-mesh-compile` 0.3.4 (curve-bounded planes,
  axiolid/kernel#192) as a direct dependency of the CLI, and
  `axiolid-overlay` 0.3.3.

- **Breaking.** Semantic capability results are closer to what a reviewer
  acts on. `manual-issue` raises one finding per rule, against the first
  selected object and relating the others, instead of one per object.
  `selector-conformance` separates an object with no value for any consulted
  property from one with values the agreed list does not know, and reports
  each unknown combination of values once, relating every object holding it;
  `message` is now followed by those values. `consistent-value` groups
  objects without a key together and reports a missing key only when those
  objects disagree, instead of always. `name-sequence` reports a number
  below `first` as "is below the start" and no longer treats it as the
  member below the next one, so it no longer causes an order finding there.
- **Breaking.** `RuleInstance::applicability` is `RuleApplicability`, and
  `PackageMetadata::name`/`description` are `LocalizedText`.
- **Breaking.** `Runtime::run` over a bare `Project` declares no type systems,
  so a compiled package's concepts bind to nothing there; run an
  `EvidenceSession` to evaluate packages.

### Removed

- **Breaking.** `axioval-spec` drops the rule plans that registered
  capabilities now express, together with their `CheckSemantics` variants:
  `RelativeCountPlanSpec` (`rule::aggregate`, now `relative-count`),
  `PropertyComparisonPlanSpec` (`rule::comparison`, now
  `property-comparison`), `ManualIssuePlanSpec` (`rule::manual_issue`, now
  `manual-issue`), `LayerAgreementPlanSpec` (`rule::layer_agreement`, now
  `selector-conformance` over presentation layers),
  `ModelArchitecturePlanSpec` (`rule::model_architecture`, now one rule per
  sub-check), `BuildingStoreyPlanSpec` (`rule::building_storey`, now the
  storey-metric compositions), and from `rule::model`
  `RequiredComponentsPlanSpec` (`object-count`),
  `SpacesInDerivedGroupsPlanSpec` and `SpaceGroupContainmentPlanSpec`
  (`group-composition`), `FireCompartmentAreaPlanSpec` (`keyed-limit`) and
  `StoreyNameSequencePlanSpec` (`name-sequence`), with their row and enum
  types. Nothing in the engine consumed them. `ModelSelectionSpec` remains.
  The "Retired rule plans" table on the capability migration page maps each
  plan's fields to capability parameters. (#41)
- **Breaking.** `axioval-spec` drops `ComponentContainmentPlanSpec` (the
  `rule::containment` module with `HostSurface`, `SurfaceSide` and
  `DimensionBandSpec`) and its `CheckSemantics::ComponentContainment`
  variant, now expressed by `containment`; the migration page maps its
  fields. (#59)

## [0.2.0] - 2026-09-24

A minor release rather than 0.1.19: the dependency upgrade changes a public
signature. Under Cargo's 0.x rules `cargo update` would pull a patch release
into every `axioval = "0.1"` build, breaking any consumer that builds its own
meshes with `axiolid-mesh` 0.1.

### Changed

- **Breaking.** `AxiolidGeometry::with_mesh` and `AxiolidGeometry::mesh` take
  and return `axiolid_mesh::TriMesh` from `axiolid-mesh` 0.3. A consumer that
  builds meshes with `axiolid-mesh` 0.1 gets a type mismatch; move both
  `axioval` and `axiolid-mesh` together. Verified: a 0.1 mesh fails to compile
  against this release, and the same scenario built on 0.3 produces output
  identical to 0.1.18.
- Axiolid dependencies move from 0.1 to 0.3 (`axiolid-core`, `-mesh`,
  `-measure`, `-overlay`) and IFC dependencies from 0.1 to 0.2
  (`ifc-model` 0.2.2, `ifc-schema` 0.2.2, `ifc-step` 0.2.1,
  `ifc-properties` 0.2.0). No other Axioval API changes; `axioval-ifc`
  exposes no upstream types. The IFC crates bring the STEP parser
  `openbim-step` with them, transitively, from 0.4.0 to 0.5.1.
- IFC error enums upstream are now `#[non_exhaustive]`. Unrecognised property
  errors, including the new `AuthoringInvalid`, resolve to
  `PropertyResolutionError::Unavailable`, so a new upstream failure mode can
  never pass as evidence.
- `CC0-1.0` is allowed in `deny.toml`. It reaches the tree through
  `tiny-keccak` <- `const-random` <- `ahash`, now used by both upstreams.
  Entity iteration order is unaffected: `ifc-model` iterates an explicit
  insertion-order vector, not its hash map.

## [0.1.18] - 2026-09-07

### Added

- `AxiolidGuardService` measures guard edges: what stands above a walking
  surface's exposed edge (barriers), what lies below it (landings), and what
  sits beside a barrier low enough to climb it. Edge coverage is reported as a
  parameterised interval so a policy can union overlapping protection.
- `AxiolidFreeSpaceService` measures clearance and free floor area. `Clear` is
  returned only when every named obstacle was measured and none intersects, so
  the completeness claim is earned rather than assumed.
- `SpaceService::measure_boundary_gaps` is now implemented. Unioning the
  triangle soup collapses interior edges, leaving the real perimeter to walk;
  previously this aspect reported `Unavailable`.
- Internal `planar::boundary_rings`, `ring_segments` and `ring_perimeter`,
  shared by the services that walk a footprint edge.

### Changed

- Guard proximity is measured between plan footprints, not between vertices.
  Two boxes whose faces are 50 mm apart have corner vertices a metre apart, so
  vertex distance put a touching climbing aid outside a 0.5 m search radius.
- An object can be both a barrier and a climbing aid for a taller barrier. The
  climbable pass previously excluded everything already classed as a barrier,
  which hid the classic defeat case of a low parapet beside a tall railing.

### Not implemented

- `FreeSpaceService::find_placement` returns `Unavailable`. `NoPlacement`
  asserts an exhaustive search proving the shape fits nowhere; a sampled sweep
  can only fail to find a witness, which is a weaker claim. Returning it would
  launder "did not find" into "does not exist".


### Changed

- The retired `axioval-openbim` name now carries a deprecation notice on
  crates.io, published as `0.2.0`. It contains no functionality and points at
  `axioval-ifc`.

  Released as `0.2.0` rather than `0.1.12` deliberately: under Cargo's semver
  rules a `0.1.12` would be selected automatically by anyone depending on
  `"0.1"`, turning a routine update into an empty crate. Verified against the
  live registry -- a `"0.1"` dependency still resolves to `0.1.11`.

  `0.1.11` is intentionally not yanked. Yanking would break existing lockfiles
  while explaining nothing about where the code went.


## [0.1.17] - 2026-09-07

### Added

- `AxiolidSpaceService` measures all seven space aspects from supplied geometry:
  duplicates, clear height, boundary gaps, overlaps, cap coverage, storey
  residuals and support counts. Each aspect fails independently, so an
  unmeasurable clear height no longer hides a measurable overlap.
- `AxiolidGeometry` gained storey assignment and element roles (space, slab,
  roof), declared by the host rather than inferred from a mesh.

### Removed

- The cap-coverage clamp `covered.min(whole)`. Coverage is the intersection of
  the merged cap elements with the space footprint, so it is bounded by that
  footprint by construction; the clamp could never change a result.


## [0.1.16] - 2026-09-07

### Added

- `AxiolidEnvelopeMembershipService` derives building-envelope membership from
  geometry: an object bounds the envelope when its plan footprint meets a
  declared bounding space. Both `EnvelopeDerivation` variants are served from
  separate declared space sets, so a model can be checked against all spaces or
  against gross-area groups only.

### Changed

- Plan-projection helpers moved to an internal `planar` module shared by the
  services, and the mesh/triangle vocabulary moved to `geometry`. No public API
  changed; `AxiolidGeometry` gains `with_space` and `with_gross_area_space`.

### Removed

- A redundant empty-envelope guard in the envelope derivation. An empty
  bounding set always produced empty bounding geometry, which the following
  guard already rejected with the same error, so the branch could never be
  observed to matter. Proven by a mutant that no test could kill until the
  duplicate was removed.

## [0.1.15] - 2026-09-07

### Added

- `AxiolidGeometry` is `Clone`, so one geometry set can be registered with
  several services. Found by an application registering contact and
  linear-quantity services over the same model: without it every consumer has
  to rebuild the store, which is how two services end up measuring different
  geometry.

## [0.1.14] - 2026-09-07

### Added

- `axioval-axiolid` implements `LinearQuantityService`, measuring shelf running
  length from an object's footprint, ceiling height and declared doorways. The
  `shelf-capacity` capability now runs against real geometry.
- `AxiolidGeometry::with_doorways` records how many openings interrupt an
  object's perimeter. Openings are a semantic fact a mesh does not carry, so
  the host declares them rather than the adapter inferring them.

### Changed

- The mesh store moved from `contact` to a shared `geometry` module and is now
  `AxiolidGeometry`: it was never contact-specific, and a second service needs
  the same lookup. `AxiolidContactGeometry` is renamed accordingly.
- Shelf length is reported as an upper-bounded interval rather than an exact
  value, because it derives from a bounding footprint. The capability already
  fails closed on an interval that straddles its minimum.

## [0.1.13] - 2026-09-07

### Added

- `axioval-axiolid` implements the engine's `ContactService` over the published
  Axiolid kernel (`axiolid-mesh`, `axiolid-measure`, `axiolid-overlay`). An
  application can now measure real contact from meshes using published crates
  only -- previously every geometry service had to be supplied by the host.
  Hosts register a `TriMesh` per `ObjectId`, so proprietary CAD sources use this
  without any IFC dependency.

  Contact area is measured in plan by intersecting projected geometry, and the
  nearest separation stays a 3D measurement.


## [0.1.12] - 2026-09-07

### Changed

- **Renamed `axioval-openbim` to `axioval-ifc`**, and the facade's `openbim`
  feature to `ifc`. The crate adapts IFC specifically -- it never covered the
  rest of the OpenBIM ecosystem -- so it was named for a dependency family
  rather than its content. Migration: replace the dependency name, the
  `openbim` feature with `ifc`, and `axioval::openbim` with `axioval::ifc`.
  `axioval-openbim` is discontinued at 0.1.11 and will not receive updates.
- Crates are now grouped by role: `contracts/`, `engine/`, `sources/`,
  `facade/`, and `apps/`. Published crate names are unchanged apart from the
  rename above, so this is a source-tree change only. A new geometry backend
  or source format is a sibling directory under `sources/` and needs no change
  to the engine or contracts.

### Fixed

- The architecture gate discovers crates recursively by declared package name
  rather than by a fixed `crates/*` glob, and fails closed when discovery
  returns nothing. The previous glob would have silently passed every
  neutrality check under the nested layout.

## [0.1.11] - 2026-09-07

### Fixed

- A barrier is treated as present on an edge only when it runs along more than
  half of it, matching the native gate. A stub of railing beside a long open
  edge now reports `missing_barrier` rather than `hole_in_barrier`: the first
  tells a reviewer the railing was never built, the second sends them looking
  for a gap to close.

## [0.1.10] - 2026-09-07

### Fixed

- Horizontal guard reports every distinct defect on a surface instead of only
  the worst one. Edges are sample points along a boundary, so a slab with a
  short rail on one side and no rail on another has two separate problems and
  a reviewer must see both. Findings are grouped by surface and defect, and
  the elements responsible are merged and sorted across the edges that share
  a defect.

## [0.1.9] - 2026-09-07

### Changed

- The horizontal-guard capability names which fall-protection defect an edge has
  instead of reporting one generic verdict. Nine diagnoses are distinguished,
  ordered worst-first, and a surface reports its worst defect rather than one
  finding per edge. Findings carry the barrier, landing or climbable object
  responsible.

### Fixed

- A barrier that is only tall enough when measured from its curb is reported as
  `barrier_too_low_due_to_curb` rather than the generic `barrier_too_low`. The
  curb reason was previously unreachable.
- Landing shortfalls name their own cause (`landing_too_low`,
  `landings_too_small`, `landing_too_far_away`) instead of collapsing into
  `insufficient_landings`.

## [0.1.8] - 2026-09-07

### Fixed

- Space-validation findings carry their related objects: the body a space
  overlaps, the elements along an uncovered boundary run, and the elements
  covering a cap. Only duplicate findings did before, so the rest said
  something was wrong without saying where.

## [0.1.7] - 2026-09-07

### Fixed

- `CapCoverage` normalises negative zero. A geometry kernel can return `-0.0`
  for an empty intersection; it compares equal to zero and so passed every
  guard, but rendered as `-0.0`, reporting an uncovered cap as
  "bottom cap only -0.0% covered".

### Added

- `EnvelopeDerivation` derives `Hash`, so it can key a `HashMap` in an adapter.

## [0.1.6] - 2026-09-07

### Added

- `Finding::related` carries the other objects that participate in a finding --
  the slab a wall rests on, the spaces a body duplicates. A finding a reviewer
  cannot act on is a finding that gets ignored: "insufficient contact" is only
  useful alongside what the face fails to rest on. `Finding::with_related`
  sorts, deduplicates, and drops the subject, so ordering never depends on the
  order an adapter walked the model.
- Slab contact reports the touching surfaces; space validation reports
  coincident space bodies and the body a space overlaps.

### Compatibility

- `related` is `#[serde(default)]` and omitted when empty, so existing payloads
  deserialize unchanged.

## [0.1.5] - 2026-09-07

### Added

- `ContactService` and the `axioval:capability.slab-contact` capability: how
  much of a face rests on another element is measured; whether that suffices
  is decided by policy. The measurement is exact, so a ratio can no longer be
  rounded across a declared minimum.
- `EnvelopeMembershipService` and `axioval:capability.external-wall-validation`:
  declared and derived envelope sets are compared per element. Applicability
  is not carried as evidence, and one request names one derivation.
- `SpaceService` and `axioval:capability.space-validation`: duplicates, clear
  height, boundary gaps, overlaps, cap coverage and storey residuals are
  measured independently, so one unavailable aspect no longer sinks the rest.
- `GuardService` and `axioval:capability.horizontal-guard`: exposed edges and
  nearby barriers, landings and climbable objects are measured; search radii
  travel with the request. Edge coverage unions intervals rather than summing
  them, so overlapping rails cannot fake a guarded edge.

## [0.1.4] - 2026-09-07

### Fixed

- Published crates no longer name a downstream vendor or internal repository
  path. `0.1.3` shipped 36 such references across 21 files, including every
  crate README, and is yanked; `0.1.0`-`0.1.2` carry the same leak.
- Doc comments that explained the neutral model by reference to one vendor now
  attribute it to "a source format", matching the source-neutrality these
  crates claim. The reasoning is retained, not deleted.

### Changed

- `check_package_contents.py` reads every shipped text member of every `.crate`
  and rejects forbidden vendor terms, so the check runs against what a registry
  would actually receive.

## [0.1.3] - 2026-09-07

### Added

- Source-neutral `LinearQuantityService`: adapters report a measured
  `LinearInterval` plus supporting evidence, never a verdict (ADR 0004).
- `ShelfCapacity` capability compares a measured shelf run against a declared
  minimum. A measurement spanning the minimum is reported as incomplete
  evidence rather than a violation, because it has not been shown to fail.

### Changed

- `reviewable_exact_evidence` now has a single owner in `services`, so every
  evidence service applies the same admission test.

## [0.1.2] - 2026-09-01

### Added

- Immutable `EvidenceSession` snapshots bind each project source to revision,
  fingerprint, optional schema, and a session-authoritative service registry;
  registration rejects unbound or stale service snapshot identities.
- Production IFC4 STEP import in `axioval-openbim`, with exact direct-property
  presence/absence, occurrence/type provenance, and fingerprint-bound evidence.

### Changed

- Direct-property failures now preserve incomplete and conflicting source states;
  exact evidence from a different source is rejected.
- Integer property predicates now support evidence-preserving `not_equal` directly.
- Relationship property completeness remains deliberately unavailable and
  fail-closed in production IFC sessions.
- Cargo-deny narrowly ignores RUSTSEC-2025-0141 for `ifc-schema`'s bundled
  `bincode 2.0.1` decoder; the advisory is project discontinuation, not a vulnerability.

## [0.1.1] - 2026-09-01

### Added

- Source-neutral `axioval:capability.property-required`, with exact absence/null/blank findings and fail-closed property-resolution errors.

## [0.1.0] - 2026-08-31

### Added

- Initial source-neutral engine workspace and architecture contract.
- Fail-closed runtime registry-drift and duplicate package rejection.
- Dependency-policy, canonical snapshot identity, and workspace package-verification gates.
- Exact source-neutral connectivity graphs with deterministic width-constrained traversal.
- Backend-neutral metric-routing requests, bounded shortest-distance evidence, and request-bound blocked verdicts.
- Backend-neutral free-area, directional-clearance, and constrained placement-search evidence contracts.
- Complete walkable-region snapshots with deterministic three-valued width-constrained routing.
- Explicit deterministic report outcomes for missing services, backend outages, incomplete/invalid evidence, and resource limits.
- Exact grounded free-floor circle and rectangle capabilities backed by the source-neutral free-space service.
- Exact property-to-property comparison capability slice with independent candidate selectors, request-bound relationship selection, target factors, `each` / `at_least_one`, and evidence-backed missing-information behavior.
- Exact request-bound property resolution for both present values and conclusive absence evidence, including cross-object substitution rejection, finite numeric validation, and fail-closed property selector evaluation.
- Source-neutral canonical SI quantity dimensions, boxed selector-valued parameters, and exact relationship-selection requests bound to a checked object and complete candidate universe.

### Fixed

- Placement offset bounds no longer admit tolerance-expanded witnesses; supported found placements now require exact frame-bound whole-base support evidence.

[Unreleased]: https://github.com/axioval/engine/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/axioval/engine/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/axioval/engine/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/axioval/engine/releases/tag/v0.1.0
