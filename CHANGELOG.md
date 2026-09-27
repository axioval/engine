# Changelog

All notable changes are documented here. This project follows Semantic Versioning and Keep a Changelog.

## [Unreleased]

### Added

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
  such as `Layer1.Material` and `Layer1.Thickness`. Every property
  capability and selector reads them. The IFC adapter answers them from
  `IfcRelAssociatesMaterial` through `ifc-material` 0.2.0, on the object or
  else its type object: single materials, layer sets (directly or through
  a usage), constituent sets, profile sets and material lists, thicknesses
  in metres with exact provenance. An object without material is an exact
  absence; two assignments conflict. IFC2X3 materials are refused until
  `ifc-material` binds to the file's release (openbimrs/ifc#77).
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
  quantities, predefined-set attributes, and a property set and a quantity
  set sharing a name are refused, never answered as absent.
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

### Fixed

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
