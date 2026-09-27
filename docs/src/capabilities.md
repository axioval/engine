# Capability model

A capability is trusted executable policy registered by a host application.

Each descriptor declares:

- stable capability ID and version;
- execution scope;
- accepted selector shapes;
- typed parameter signature and defaults;
- required semantic/evidence capabilities;
- result and exactness contract.

Compilation rejects unknown capabilities, duplicate registrations, missing/extra parameters, invalid values and unsatisfied static contracts.

Execution is also fallible. `Runtime::run` rejects a plan if its runtime registry no longer contains every compiled capability; registry drift can never turn a rule into an implicit pass.

At runtime, missing evidence does not become a pass. `CapabilityEvaluation` carries conclusive findings separately from object- or rule-level not-evaluated outcomes. The runtime binds every such outcome to the compiled `RuleId`, sorts it deterministically, and exposes it through `Report::not_evaluated()`. Reasons distinguish an invalid declaration, an unbound concept, a missing service, backend outage, incomplete evidence, invalid evidence, and resource exhaustion. An unbound concept (the package names something the source's vocabulary cannot express) is reported once per rule and source rather than once per object; see [Concept binding](./concept-binding.md).

A capability may return findings and not-evaluated outcomes together when only part of its selected universe was computable. Consumers must not interpret an empty findings list as a pass while `not_evaluated` is non-empty.

## Built-ins

`axioval-rules` contains reusable, vendor-neutral implementations. Vendor identity, proprietary format handling, localized vendor text and oracle-only ordering remain adapters in the legacy runtime.

`axioval:capability.property-exists`, `axioval:capability.property-required`, `axioval:capability.property-value-equals`, and `axioval:capability.property-predicate` resolve values through `PropertyResolutionServiceHandle`. `property-predicate` takes exactly one target: `value` (integer), `number`, `quantity`, `text`, `texts` or `boolean`. A `quantity` is written with a unit and compared in SI; it compares only with a quantity of the same dimension. With a numeric target it accepts `equal`, `not_equal`, `greater_than`, `greater_or_equal`, `less_than` and `less_or_equal`. With `text` it accepts `equal`, `not_equal`, `contains` and `matches`, a regular expression that must match the whole value. With `texts` it accepts `one_of` and `none_of`, and with `boolean` it accepts `equal` and `not_equal`. `is_defined` and `is_undefined` take no target; a blank or null value counts as undefined. Text comparisons are case-sensitive unless `case_sensitive` is `false`. A numeric target may be compared under a `tolerance`, `relative_tolerance` or `decimals` (see [Numeric tolerance](#numeric-tolerance)). A comparison presupposes a value, so an exactly absent property fails every operator except `is_undefined`. A quantity is never compared with a unit-less target and is reported as not evaluated. Failed predicates retain exact source evidence and name the actual value. A present value must be bound to the complete request—including its source-qualified object identity—and carry exact reviewable evidence. A missing value is conclusive only when the provider returns exact request-bound `CompletePropertyAbsenceEvidence`; service absence, partial extraction, cross-object substitution, mismatched responses, or inexact provenance remain not evaluated. Property selectors use the same resolver, so incomplete applicability data cannot silently skip an object. `property-exists` checks exact presence and intentionally does not reinterpret a present typed value. `property-required` applies the stronger required-value contract: exact absence, `null`, or blank text emits an evidence-backed finding, while booleans, integers, finite numbers/quantities, and nonblank text satisfy it. `property-value-equals` accepts a typed `property` reference and boolean `expected` value; a non-boolean resolution is invalid evidence rather than a pass or violation.

`axioval:capability.property-data-type` takes a `property` reference and a `data_type` string. It applies the `property-required` contract and additionally requires the value's type as the source declares it (`Property::data_type`, for IFC `IFCLABEL`, `IFCBOOLEAN`, ...) to equal `data_type`, compared ASCII case-insensitively. A different declared type is a finding. A present value whose type the source does not report is not evaluated, never taken to match. It is what an IDS property facet with a `dataType` and no value translates to.

`axioval:capability.property-value` checks a value against lexical constraints cast to the kind of the value the source resolved: `values` (any of), `patterns` (XML Schema regular expressions matching the whole value), `min_inclusive`/`max_inclusive`/`min_exclusive`/`max_exclusive`, `length`/`min_length`/`max_length`, and optionally `data_type`. Text compares exactly and case-sensitively and alone takes patterns and lengths; booleans accept `true`/`1` and `false`/`0`; integers take integer literals; decimals take `xs:double` literals and are equal within `|x - v| <= |v|·1e-6 + 1e-6` (the IDS tolerance, boundaries included), while bounds compare exactly. Without `optional`, absence, `null` and blank text are violations; with it, an absent or `null` property passes and any present value is checked. A literal that cannot be cast, a constraint the value's kind does not take, or a pattern that cannot be translated exactly (class subtraction, `\i`/`\c`, block escapes) makes the object not evaluated (`InvalidDeclaration`); a quantity is not evaluated until units are handled.

`axioval:capability.property-comparison` currently covers exact property-to-property targets with an independent selector-valued candidate scope, checked/shared/related modes, target-side factors, and `each` or `at_least_one` quantifiers. It compares booleans, strings, exact integers, finite decimals, and canonical quantities with matching dimensions. Missing properties emit evidence-backed missing-information findings; incompatible types or unavailable evidence remain not evaluated. Numbers may be compared under a tolerance (see [Numeric tolerance](#numeric-tolerance)). The target is exactly one of:

- `target_property`, a property of the checked object;
- a constant: `target_number`, `target_quantity`, `target_text` or `target_boolean`;
- `target_texts`, a list that only `one_of` and `none_of` take.

Two more quantifiers compare a single number with the target:

- `count` compares the number of candidates, zero included, and needs no `compared_property`.
- `sum` compares the total of the candidates' compared values. The values must all be integers, all numbers, or all quantities of one dimension; otherwise the checked object is not evaluated. A candidate without the value gets its missing-property finding, and no verdict is drawn from the partial total.

`axioval:capability.free-floor-circle` and `axioval:capability.free-floor-rectangle` check whether each selected spatial scope can contain an exact supported vertical shape. Circle parameters are `diameter_metres` and `height_metres`; rectangle parameters are `width_metres`, `length_metres`, and `height_metres`, all in canonical metres, and an `orientation`. The rectangle `orientation` must be `any` (every rotation counts). A rule without it, or with an orientation that no service can yet ground in a frame (such as `fixed-to-room` or `fixed-to-door`), is reported not evaluated as an invalid declaration, because a rectangle placement is only meaningful once it says which rotations it allows. Each request covers every other project object as a candidate obstacle and requires exact whole-base support on the selected scope with zero hidden gap. A complete exact no-placement proof emits the shape-specific `NO_FREE_FLOOR_SPACE_*` finding; missing services, backend outages, or invalid/incomplete evidence emit not-evaluated outcomes instead.

`axioval:capability.slab-contact` requires at least `minimum_contact_ratio` of each selected subject's face to rest on something on its `contact_side` (`above` or `below`), measured through `ContactServiceHandle` with the `maximum_gap_metres`, `maximum_intersection_metres` and `minimum_polygon_area_square_metres` tolerances. What the face may rest on is the optional `counterparts` selector; without it every other project object is a candidate. The capability resolves the selector and sends the candidates in the `ContactRequest`, and the adapter measures exactly those: an object outside them neither supports the face nor blocks the measurement, while an unmeasured candidate makes the subject not evaluated. A counterpart the selector cannot decide leaves a shortfall not evaluated, since it might support the face; a pass stands, because more candidates can only add contact. `skip_top_storey` and `skip_bottom_storey` leave out subjects on the highest or lowest storey of their source. Storeys are the `storey_selector` objects, ordered by their `Elevation` attribute (a length), and a subject's storey is the one the declared traversal (`relationship` or `path`, as below) reaches from it. Skipping needs both. Every storey needs a known elevation and every subject exactly one storey; otherwise the subjects concerned are not evaluated rather than guessed at. A shortfall is graded by how far the ratio falls below the minimum, no contact at all by the distance to the nearest candidate.

`axioval:capability.clash` and `axioval:capability.distance` check selected subjects against a selector-valued `counterparts` group through `ProximityServiceHandle`, after the engine's complete broad-phase candidate search. A hard clash is a witnessed penetration beyond `penetration_tolerance_metres`, or containment. A clearance clash is a separation below the optional `clearance_metres`. Distance bounds the nearest counterpart with `minimum_metres` and/or `maximum_metres`. Findings on tessellated geometry carry inexact evidence and say so. See [Clash, interference and distance](./clash.md).

`axioval:capability.horizontal-guard` checks that the exposed edges of the selected walking surfaces are guarded, through `GuardServiceHandle`: by barriers tall and close enough, or by a short fall onto a landing wide enough to stand on, and not defeated by a climbable object beside the barrier. The rule's selection is the walking-surface profile. The optional selectors `barrier_selector`, `landing_selector` and `climbable_selector` name the objects that may play each role; their resolved sets travel in the `GuardSearch`, and only their members are counted for that role, so a cupboard along an edge is not taken for a railing. An absent selector leaves its role open to any nearby body. An object a role selector cannot decide makes the rule not evaluated, since it might be the barrier that guards an edge. A definition bound to this capability must declare all three selectors, as optional `selector` parameters.

`axioval:capability.space-validation` checks each selected space through `SpaceServiceHandle`, one measurement per sub-check, so a sub-check the adapter cannot measure is not evaluated without sinking the others. It takes `required_height_metres`, `uncovered_segment_length_metres`, `check_top_cap`, `check_bottom_cap`, `check_unallocated_area` and `maximum_unallocated_area_square_metres`, and three optional parameters:

- `tolerance_metres` (default 0.005): a space is too low only when it misses `required_height_metres` by more than this, and an overlap no thicker than this is contact, not an intersection.
- `top_cap_elements` and `bottom_cap_elements`: selectors naming the elements that may cap a space from above or below. The selection travels in the `CapRequest`, and the service considers exactly those elements. Without a selector the host's declared slabs (and, for the top, roofs) are used. A selector that selects nothing skips that cap; one that leaves objects undecided makes that cap not evaluated for every space, since an undecided element may be the covering one.

Every finding message starts with its sub-check's category code, so results can be grouped by problem as well as by space:

| Code | Sub-check |
|---|---|
| `duplicate_space` | Another space has the same body. |
| `insufficient_height` | Clear height below the requirement, beyond the tolerance. |
| `uncovered_boundary` | Boundary runs of at least `uncovered_segment_length_metres` that no element covers. |
| `contained_body` | The space contains, or lies inside, another body. |
| `intersecting_space` / `intersecting_component` | The space intersects another space or a component. |
| `uncovered_top_cap` / `uncovered_bottom_cap` | A cap less than 98 % covered; under 1 % is an error, up to 15 % a warning, otherwise information. |
| `unallocated_area` | Storey floor area belonging to no space, reported against the storey. |

### Numeric tolerance

`property-predicate`, `property-comparison` and `unique-value` take three optional parameters that relax exact numeric comparison. They apply to integers, decimals and quantities; a quantity is compared in its canonical SI unit, so a tolerance on a length is in metres. Text and booleans are never affected, and a tolerance declared with a text, text-list or boolean constant target is an invalid declaration. A definition bound to one of these capabilities must declare all three, as optional parameters (`number`, `number`, `integer`).

- `tolerance` (a non-negative number) and `relative_tolerance` (a fraction at least 0 and below 1), alone or together: two values `a` and `b` are equal when `|a − b| ≤ tolerance + relative_tolerance · max(|a|, |b|)`, the boundary included. Values are read as the decimals they display, so a few units in the last binary place are allowed: `1.0` and `1.1` are within `0.1`.
- `decimals` (0 to 15): both values are rounded half away from zero to that many decimal places of their shortest decimal form, so `2.345` rounds to `2.35` as displayed, and then compared exactly. It cannot be combined with a tolerance.

Under either, two equal values are neither greater nor less than each other: `greater_than 10` with `tolerance 0.5` fails for `10.5`. `property-comparison` applies the tolerance between the compared value and the target after its `factor`, and to `count` and `sum` as well. Every finding judged under a tolerance states it, for example `(within tolerance 0.1)` or `(rounded to 2 decimal(s))`.

Rounding is transitive, so `unique-value` groups values that round alike. A tolerance is not: `1.0` and `1.2` are each within `0.1` of `1.1` but not of each other. `unique-value` therefore judges a tolerance pair by pair: an object is a duplicate of every other object in its scope whose value is within the tolerance of its own, and its finding names exactly those objects. In the example, `1.1` is reported naming the other two, and `1.0` and `1.2` each naming `1.1` only. A quantity is compared only with quantities of the same dimension; an integer and a decimal are compared with each other.

### Semantic capabilities

These capabilities judge exact properties, classifications and relationships only, so they need no geometry. Relationship parameters (`relationship`, `direction`, `follow_chain`, `skip_absent_relationship_ends`) mean the same as in `property-comparison`. With the IFC adapter, a relationship is an IFC relationship entity name such as `IfcRelAggregates`. An object whose selection or value cannot be decided is not evaluated, never passed.

| Capability | Checks |
|---|---|
| `selector-conformance` | Each selected object satisfies the `requirement` selector. An agreed list is an `anyOf` of `allOf` rows. An object none of whose consulted properties has a value gets its own "no value" finding. Other failing objects are grouped by their combination of values: one finding per unknown combination, against the first object, naming the values and relating the others. `message` replaces the default text and is followed by the values. Every finding cites the property facts consulted. |
| `unique-value` | `property` does not repeat among the selected objects of one source, or of the project with `across_sources`, or among objects reaching the same related objects through `relationship` (for example one storey). Text is trimmed and compared ignoring case unless `trim` or `case_sensitive` says otherwise. A missing value is a finding unless `require_value` is `false`. Numbers may be compared under a `tolerance`, `relative_tolerance` or `decimals` (see [Numeric tolerance](#numeric-tolerance)). |
| `consistent-value` | Objects of one kind that share a `key` value share their `value` too. An absent value is a value of its own. Each member of a disagreeing group is reported, naming the others. Objects without a key form one group of their own, so a missing key is reported only when those objects disagree. |
| `related-count` | Each anchor has between `minimum` and `maximum` related objects that `related_selector` picks. Without `relationship`, the anchor's whole source is counted. Undecided related objects are counted as unknown, and the anchor is judged only when they cannot change the verdict. |
| `relative-count` | At each anchor, or in each group of objects sharing a `group_property` value, `provided / provided_unit` stands in `operator` (`equal`, `not_equal`, `greater`, `at_least`, `less`, `at_most`) to `required / required_unit`, computed in exact integers. |
| `name-sequence` | The members of each anchor, ordered by the numeric `order` property, carry whole-number names from `first` stepping by `increment`. A name is a number only when it is exactly one. A number below `first` and a number out of order are separate findings; like a name that is not a number, a number below `first` does not interrupt the sequence. A member without an order value leaves the anchor not evaluated. |
| `numbering-consistency` | The numbers `pattern` (an XML Schema pattern over the whole value with exactly one group of digits) reads from `property` agree within each scope, formed as in `unique-value`. With `prefix_length`, their first digits match: objects departing from the predominant prefix are reported, and every object when none predominates. With `gap_free`, the distinct numbers step by one, and the objects above each gap are reported. At least one check must be declared. A value that is missing or does not match is not evaluated, and a gap an unreadable object could fill is not evaluated rather than reported. |
| `manual-issue` | Raises the declared `title`, `category` and `description` once per rule, for checks owed by hand: the finding is against the first selected object and relates the others. An empty selection raises nothing. |

| `level-spacing` | Each level's height, the rise of its `order` length to the next level up, lies within `minimum` and `maximum` and, with `consistent`, matches the prevailing height within `tolerance`. The highest level is not evaluated unless `ignore_highest`, since its height needs geometry; `ignore_lowest` leaves out a basement. |

Deliberate differences from other checkers:

- `name-sequence` orders members only by the declared `order` property. When a member has no order value (a storey without `Elevation`), the anchor stays not evaluated rather than falling back to placement height: the engine exposes no exact source-neutral service for an object's placement height, and a storey has no body whose extent could stand in for it.
- `property-required` reports each property in its own rule, so an object missing several identity fields gets one finding per field. Combining them into one finding per object would add a parameter every `property-required` definition must then declare; grouping findings per object is left to the report consumer.
- `manual-issue` raises nothing for an empty selection. A model-level finding for that case needs findings that are not reported against an object, which the report contract does not have yet.

Most names and numbers these rules read are attributes of the object, not properties; see [attribute sets](./ir.md#attribute-sets). Quantities are written with a unit: `m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`. `²`, `³` and `°` are accepted too.

A traversal is either one `relationship` (with `direction` and `follow_chain`) or a `path` of steps walked in order, each `Relationship` or `Relationship:direction`. For example, `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` goes from a wall to the doors and windows filling its openings.

`relative-count` also has a table mode. `table` lists rows `R:P`, meaning "from R required objects on, at least P provided". The row with the largest R not above the required count applies. Beyond the last row, `additional_required` / `additional_provided` add P for every further R. Below the first row the table sets no requirement: the anchor or group is skipped, not extrapolated from zero. A table of increments alone applies them from zero. Parameters have no table type, so rows are text, and a malformed row is a declaration error.

In ratio mode, `small_required_below` n and `small_provided` k, declared together, replace the ratio for small counts: a required count from 1 up to but excluding n is judged as `provided operator k`. "With fewer than four workplaces, at least one washbasin" is n = 4, k = 1 with `at_least`; "below ten workplaces nothing is required" is n = 10, k = 0. A required count of zero is always judged by the ratio. Neither applies in table mode.

With `group_property`, `relative-count` groups instead of walking from anchors. The rule's selection is then the set of objects counted, and each object `provided_selector` or `required_selector` picks is counted in the group of its `group_property` value (a location code, say), within one source unless `across_sources`. Text is trimmed and compared ignoring case unless `case_sensitive`. Relationship parameters do not apply, and `across_sources` and `case_sensitive` apply only here. A group with required objects and no provided object is reported as present only in the required set, whatever the ratio, small-count case or table would say. A group's finding is raised against its lowest required object (its lowest provided object when it has none) and relates every other member. A counted object with no value (absent, null or blank) is a finding of its own; one whose value cannot be read is not evaluated and leaves every group of its scope not evaluated, since it could belong to any of them. A group with an object whose membership is undecided is not evaluated.

### Plan-area capabilities

These judge `PlanAreaService` measurements, so they need a geometry adapter.

| Capability | Checks |
|---|---|
| `area-ratio` | At each anchor, the summed footprints of `numerator_selector` over those of `denominator_selector` (or the anchor's own footprint) lie within `minimum` and `maximum`. `numerator_property` / `denominator_property` take a population's areas from an area-quantity property instead, e.g. glazing areas. |
| `plan-coverage` | Each subject's footprint lies within one `candidate_selector` object by at least `minimum_ratio`, e.g. a space within a fire compartment. |

### Slab stacks

`slab-stack-spacing` judges `VerticalExtentService` elevations together with `PlanAreaService` footprints, so it needs a geometry adapter. Unlike `level-spacing`, which reads storey elevations, it measures the slabs' own surfaces.

| Capability | Checks |
|---|---|
| `slab-stack-spacing` | Two selected slabs stack when their footprints overlap by at least `minimum_overlap_ratio` of the smaller footprint. Each slab is checked against the next slab up it stacks with, ordered by top elevation: `top_to_top`, `bottom_to_bottom` and `top_to_bottom` (the clear gap to the next slab's underside) each lie within their optional `<measure>_minimum` and `<measure>_maximum`. `consistent` lists the measures that must equal the prevailing one of the stack within `tolerance` (1 mm by default), as in `level-spacing`. |

Elevations and footprints are intervals, and a tessellated slab's are never points. A slab is judged only on the whole interval: a distance straddling a bound, an overlap ratio straddling the minimum, or two tops the intervals cannot order are not evaluated. An object whose selection is undecided still counts as a possible next slab up, and one whose extent cannot be measured leaves every slab not evaluated, since it could sit in any stack. A slab with nothing stacked above it has nothing to check. When a proximity service is registered, its enclosing boxes skip overlap measurements between slabs that cannot meet in plan.

## Adding a capability

1. Define or reuse canonical schema concepts and parameters.
2. Add failing contract and behavior tests.
3. Implement policy only; put source interpretation in an adapter.
4. Declare all evidence requirements.
5. Add deterministic and missing-evidence tests.
6. Register in the built-in registry.
7. Record legacy parity and cutover in the migration ledger when applicable.
