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

At runtime, missing evidence does not become a pass. `CapabilityEvaluation` carries conclusive findings separately from not-evaluated outcomes. Either can be about one object, one source or the whole project (see [Scope](./ir.md#scope)): `push_finding` takes a `Finding` of any scope, and `push_object_not_evaluated`, `push_source_not_evaluated` and `push_not_evaluated` (rule level) record outcomes at each scope. The runtime binds every such outcome to the compiled `RuleId`, sorts it deterministically, and exposes it through `Report::not_evaluated()`. Reasons distinguish an invalid declaration, an unbound concept, a missing service, backend outage, incomplete evidence, invalid evidence, and resource exhaustion. An unbound concept (the package names something the source's vocabulary cannot express) is reported once per rule and source rather than once per object; see [Concept binding](./concept-binding.md). So is an unrecorded fact (`NotRecorded`): a source that records a kind of fact for no object at all, such as a model with no presentation layers, makes every rule consulting it not applicable to that source, rather than a pass or a violation per object.

A capability may return findings and not-evaluated outcomes together when only part of its selected universe was computable. Consumers must not interpret an empty findings list as a pass while `not_evaluated` is non-empty.

## Computed parameters

A parameter may depend on the object it judges: a minimum distance that depends on the fire-resistance class, an area limit per use. A parameter a capability declares *per object* may be given as an [expression](./expressions.md) of its kind instead of a literal, `{"type": "expression", "value": {…}}`, evaluated for each checked object; a per-object table may hold expression cells in its scalar columns.

```json
{"minimum_metres": {"type": "expression", "value": {"kind": "if",
   "branches": [{"when": {"kind": "compare", "operator": "equals",
                          "left": {"kind": "property", "propertySet": "Pset_DoorCommon", "property": "FireRating"},
                          "right": {"kind": "literal", "value": {"type": "string", "value": "EI30"}}},
                 "then": {"kind": "literal", "value": {"type": "number", "value": 1.0}}}],
   "else": {"kind": "literal", "value": {"type": "number", "value": 0.5}}}}}
```

- The compiler checks the expression's type against the parameter's kind: a truth for a `boolean`, an integer for an `integer`, a plain number for a `number`, a quantity of any unit for a `quantity` (a plain number is refused), text or an enumeration value for a `string` or an `enum`, and a date or a date-time; a table cell against its column's kind. An expression may read the rule's other literal parameters.
- Every other parameter is constant for the rule (selectors, switches that change what is measured, keys, counts over a population) and refuses an expression with `EngineError::InvalidExpression` ("constant for the rule").
- For each selected object the expressions are evaluated to literals; objects whose literals agree are judged together, as one rule over exactly them, so the capability itself reads literals only. An expression that cannot be decided for an object, or computes an interval rather than one value, leaves that object not evaluated with the reason; a `null` result is a parameter not given (an optional bound then does not apply; a required one leaves the object not evaluated).

These parameters are computed per object today:

| Capability | Parameters |
| --- | --- |
| `property-predicate` | `value`, `number`, `quantity`, `text`, `boolean`, `date`, `date_time` |
| `distance` | `minimum_metres`, `maximum_metres` |
| `keyed-limit` | the `limits` table (`minimum`, `maximum` and the key cells) |
| `stair-geometry` | `minimum_risers`, `maximum_risers`, `maximum_rise`, `riser_tolerance`, `going_tolerance`, `winder_angle_minimum`, `winder_angle_maximum`, `slope_tolerance`, `end_landing_depth_minimum`, `end_landing_width_minimum` |

A capability opts a parameter in with `ParameterDescriptor::per_object` and starts its `evaluate` with the shared per-object dispatch (`object_parameters::per_object` in `axioval-rules`).

## Table parameters

Many checks are naturally a table: rows of patterns and limits, such as a minimum area per space type or the components required per class. A `table` parameter carries such a table in one rule instead of one rule per row.

The definition declares the table's columns, each with an `id`, a localized `name`, a `kind` and whether it is `required` (the default). A column kind is one of `string`, `textPattern`, `number`, `quantity` (with a `unitDimension`), `integer`, `boolean`, `selector`, `reference`, `date` and `dateTime`. A table value is a list of rows; each row maps column IDs to cells written as the scalar value of the column's kind (a `textPattern` cell is a `string` value):

```json
{"type": "table", "value": [
  {"space_type": {"type": "string", "value": "Office*"},
   "minimum_area": {"type": "quantity", "value": 10, "unit": "m2"}}
]}
```

A capability declares the same columns in its descriptor, `ParameterType::Table(&[TableColumn::required("space_type", ColumnKind::TextPattern), ...])`. The binder requires the definition's columns to equal the descriptor's by ID, kind and requirement, in any order, and rejects `columns` on any other parameter kind and `allowedValues` on a table. Every row, bound or defaulted, must then fit: a cell in an unknown column, a cell of another kind, a missing required cell, or a text pattern ending in an unpaired backslash fails compilation with `InvalidTableRow`, naming the parameter, the zero-based row and the column. Concepts named in selector cells are checked like any other. Rows keep their declared order; an empty table is valid.

A text pattern reads like the `like` operator of property selectors: it matches the whole value, `*` stands for any run of characters, `?` for exactly one, and a backslash makes the next character literal. Its specificity is its number of literal characters, so `Office` is more specific than `Off*`, which is more specific than `*`.

Capabilities in `axioval-rules` read rows through the shared parameter reader and match them with one helper rather than each reimplementing row semantics. It selects the first matching row, the single most specific one, or all matching rows, and fails closed: a row that cannot be decided (a key value unknown) makes the outcome undecided whenever it could change it, and two rows tied for most specific are reported as ambiguous, never broken by declaration order. Keys of one row combine by conjunction, adding their specificities.

Existing packages are unaffected: `columns` is omitted from serialized definitions that declare none.

### Tables from data files

Requirement tables such as room programmes and door schedules are often kept in spreadsheets. A `table` parameter may name such a file in the package instead of listing its rows, as a rule binding or a definition's default:

```json
{"type": "tableFile", "path": "tables/rooms.csv",
 "sha256": "<lowercase hex SHA-256 of the file>",
 "columns": [
   {"id": "key_1", "header": "type", "kind": "textPattern"},
   {"id": "area", "header": "min_area", "kind": "number"}
 ]}
```

The file is a UTF-8 CSV file (RFC 4180: commas, optional double quotes with `""` for a quote, CRLF or LF; a leading byte order mark is dropped), or one sheet of an `.xlsx` workbook named by `sheet`. `path` is relative to the package root, `/`-separated, without empty, `.` or `..` segments, and must stay inside the package. `sha256` pins the file's exact bytes, so a package (and its `.mcs` inventory) stays deterministic: another file is refused. `columns` declares every column of the file: the table column `id` it fills, its `kind`, the file's `header` naming it (the `id` when omitted) and, for a `quantity` column, the `unit` every cell is stated in. A `selector` column cannot come from a file.

A host loads every table file before binding (`axioval_engine::load_table_files` for a ruleset, `load_definition_table_files` for a definition package, from a `PackageFiles` such as `PackageDirectory`; the CLI reads them beside each package file). Loading fails closed with `TableFileError` on an unsafe path, an unreadable file, another digest, a header naming an undeclared column or a column twice, a declared column missing from the header, a row wider than the header, and any cell that is not of its column's kind: a `number` or `quantity` must be a finite decimal number, an `integer` a whole number, a `boolean` `true` or `false`, a `date` or `dateTime` an ISO 8601 literal. The first non-blank row is the header; blank rows are skipped and an empty cell leaves its column out of the row. A workbook cell is read as written: a string as its text, a number as its literal, a boolean as `true` or `false`; a formula or an error cell is refused rather than read from its cached result, and nothing in the file is ever executed.

The binder then checks the declared columns against the parameter's table: a column that is not one of the table's, a column of another kind and a required column left undeclared fail compilation with `InvalidTableFile`, and so does a reference that was never loaded. The rows are then bound exactly as the same rows written inline, so a capability cannot tell them apart and every row check above applies; at run time a row that matches nothing still decides nothing, never a default.

## Built-ins

`axioval-rules` contains reusable, vendor-neutral implementations. Vendor identity, proprietary format handling, localized vendor text and oracle-only ordering remain adapters in the legacy runtime.

Each built-in's outside contract (its id, every parameter's name, kind, requirement, per-object and expression-text flags and table columns, whether it grades deviations and whether it takes authored parameters) is pinned by `crates/engine/rules/tests/golden/descriptors.json`. Packages and export profiles address a built-in by this contract, so a capability rebuilt on shared measured values and expressions keeps it unchanged.

`axioval:capability.property-exists`, `axioval:capability.property-required`, `axioval:capability.property-value-equals`, and `axioval:capability.property-predicate` resolve values through `PropertyResolutionServiceHandle`. `property-predicate` takes exactly one target: `value` (integer), `number`, `quantity`, `text`, `texts`, `boolean`, `date` or `date_time`. A `quantity` is written with a unit and compared in SI; it compares only with a quantity of the same dimension. With a numeric target it accepts `equal`, `not_equal`, `greater_than`, `greater_or_equal`, `less_than` and `less_or_equal`. With `text` it accepts `equal`, `not_equal`, `contains` and `matches`, a regular expression that must match the whole value. With `texts` it accepts `one_of` and `none_of`, and with `boolean` it accepts `equal` and `not_equal`. A `date` or `date_time` target accepts the ordered operators and compares chronologically (see [Dates and date-times](#dates-and-date-times)); `precision` applies to it alone. `is_defined` and `is_undefined` take no target; a blank or null value counts as undefined. Text comparisons are case-sensitive unless `case_sensitive` is `false`. A numeric target may be compared under a `tolerance`, `relative_tolerance` or `decimals` (see [Numeric tolerance](#numeric-tolerance)). A comparison presupposes a value, so an exactly absent property fails every operator except `is_undefined`. A quantity is never compared with a unit-less target and is reported as not evaluated. Failed predicates retain exact source evidence and name the actual value. `property-predicate` runs as a template: its stated property read by the expression evaluator, judged by the generic comparison judge through the one comparison every rule uses (see [Capability templates](./templates.md)). A present value must be bound to the complete request—including its source-qualified object identity—and carry exact reviewable evidence. A missing value is conclusive only when the provider returns exact request-bound `CompletePropertyAbsenceEvidence`; service absence, partial extraction, cross-object substitution, mismatched responses, or inexact provenance remain not evaluated. Property selectors use the same resolver, so incomplete applicability data cannot silently skip an object. `property-exists` checks exact presence and intentionally does not reinterpret a present typed value. `property-required` applies the stronger required-value contract: exact absence, `null`, or blank text emits an evidence-backed finding, while booleans, integers, finite numbers/quantities, and nonblank text satisfy it. `property-value-equals` accepts a typed `property` reference and boolean `expected` value; a non-boolean resolution is invalid evidence rather than a pass or violation.

`axioval:capability.property-data-type` takes a `property` reference and a `data_type` string. It applies the `property-required` contract and additionally requires the value's type as the source declares it (`Property::data_type`, for IFC `IFCLABEL`, `IFCBOOLEAN`, ...) to equal `data_type`, compared ASCII case-insensitively. A different declared type is a finding. A present value whose type the source does not report is not evaluated, never taken to match. A property whose value the source cannot read but whose type it declares exactly (`PropertyResolutionError::UnreadableValue`, see [services](./services.md)) is judged by that type alone: it is present and not empty, so another type is a finding and the same type passes. A table whose columns declare different types (`Property::column_types`) has the type of either column; no column of `data_type` is a finding. A complex property (`PropertyValue::Complex`) is present and of no type, so it is a finding. It is what an IDS property facet with a `dataType` and no value translates to.

`axioval:capability.property-value` checks a value against lexical constraints cast to the kind of the value the source resolved: `values` (any of), `patterns` (XML Schema regular expressions matching the whole value), `min_inclusive`/`max_inclusive`/`min_exclusive`/`max_exclusive`, `length`/`min_length`/`max_length`, `total_digits`/`fraction_digits`, and optionally `data_type`; on a table whose columns declare different types, only the cells of the columns declaring `data_type` are judged, and no such column is a finding. Text compares exactly and case-sensitively and alone takes patterns and lengths; booleans accept `true`/`1` and `false`/`0`; integers take integer literals; decimals take `xs:double` literals and are equal within `|x - v| <= |v|·1e-6 + 1e-6` (the IDS tolerance, boundaries included), while bounds compare exactly. `total_digits` and `fraction_digits` take a number only and count as XML Schema does, leading zeros of the whole part and trailing zeros of the fraction excluded; a decimal is counted on the shortest decimal that reads back as the same double, the form a model states it in (`0.1`, not its binary expansion). A date takes `xs:date` literals and a date-time `xs:dateTime` literals with a UTC offset, for `values` and bounds alike; with `precision` `day` either value takes either literal, compared by the day it states (see [Dates and date-times](#dates-and-date-times)). `precision` on any other value is an invalid declaration. Without `optional`, absence, `null`, blank text and an empty list are violations; with it, an absent or `null` property passes and any present value is checked. A property whose value the source cannot read but whose type it declares exactly (`UnreadableValue`) fails a `data_type` it does not declare and is otherwise not evaluated when any value constraint is given; `property-exists` and `property-required` count it as present and not empty. A literal that cannot be cast, a constraint the value's kind does not take, or a pattern that cannot be translated exactly (class subtraction, `\i`/`\c`, block escapes) makes the object not evaluated (`InvalidDeclaration`). A quantity is compared only with `si_units: true`, which reads every numeric literal in the coherent SI unit of the quantity's dimension (metres, square metres, kilograms, radians, ...), the unit it is stated in; otherwise it is not evaluated. A list, a bounded value or a table is judged by its stated values under `quantifier` (`any` or `all`), and is not evaluated without one: `any` holds when one stated value meets every constraint, `all` when each does and there is at least one. A range holds every value between its bounds, so under `all` a bounded value open on one side fails every bound on that side: `[1 m, open above]` is not within `max_inclusive` 6. A scalar under a quantifier is judged as itself, so one rule serves an enumerated value that selects one item or several. `property_pattern` with an optional `property_set_pattern` names the properties instead of `property`, by XML Schema patterns over the source's own names: every matching property, enumerated exactly, must meet the constraints, one must match unless `optional` (with `property_set_pattern`, one in every matching set that holds a property, as IDS requires), and each failing property and each set without a match is its own finding; declaring both `property` and `property_pattern`, or a set pattern alone, is an invalid declaration. A definition bound to `property-value` declares `property` as optional, and `property_set_pattern`, `property_pattern`, `quantifier` (`string`) and `si_units` (`boolean`) beside its other optional parameters.

`axioval:capability.property-comparison` compares a property of candidate objects, picked by the `compared_selector`, with a target for each checked object. It compares booleans, strings, exact integers, finite decimals, canonical quantities with matching dimensions, and dates and date-times (see [Dates and date-times](#dates-and-date-times)); a `factor` other than 1 or a tolerance does not apply to a date, and `precision` applies to nothing else. Missing properties emit evidence-backed missing-information findings; incompatible types or unavailable evidence remain not evaluated. Numbers may be compared under a tolerance (see [Numeric tolerance](#numeric-tolerance)).

`component_mode` says which objects are candidates:

- `checked`: the checked object itself.
- `shared`: the members of a group the checked object belongs to, through one `relationship`.
- `related`: the objects a traversal (`relationship`, or a `path` of steps, as below) reaches from the checked object. `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` compares a wall with the doors and windows filling its openings.
- `same_space` and `same_building`: the other objects that share a nearest container with the checked object. Containers are the `container_selector` objects (spaces, or buildings). An object's nearest containers are found by climbing the declared steps (`relationship` and `direction`, or every step of a `path`) in any order and any number of times, stopping at each container reached, so a building is found from a door through its storey, and a space nested in another is the nearer one. Climbing is always transitive, so `follow_chain` does not apply. An object that reaches no container shares none. A container the selector cannot decide leaves every checked object not evaluated, since it may be the one two objects share. For the geometric variant, the space an element lies in by its position rather than by a stated containment, declare `relationship: axioval:derived.contained-in-space` with `direction: forward` (see [relationships derived from geometry](#relationships-derived-from-geometry)). Across federated models, `container_relationship: axioval:derived.same-level` with a `level_property` matches containers of different sources as one level (see [Levels across models](#levels-across-models)): a candidate on the architecture model's storey then shares the MEP model's storey at the same elevation.

The target is exactly one of:

- `target_property`, a property of the checked object;
- a constant: `target_number`, `target_quantity`, `target_text`, `target_boolean`, `target_date` or `target_date_time`;
- `target_texts`, a list of texts or patterns, which only `one_of`, `none_of`, `contains`, `like` and `matches` take, and which is satisfied by any one entry;
- a range, which only `between` takes: one minimum (`minimum_number`, `minimum_quantity`, `minimum_date`, `minimum_date_time` or `minimum_property`, a property of the checked object) and one maximum (`maximum_number`, `maximum_quantity`, `maximum_date`, `maximum_date_time` or `maximum_property`). Two constant bounds must be of one kind (numbers, quantities of one dimension, dates, or date-times), the minimum not above the maximum; bounds of mixed kinds are an invalid declaration. A bound read from a property is cited, and an absent one is a missing-information finding. A date window compares as in [Dates and date-times](#dates-and-date-times): a value outside either bound is outside the range, and one whose order against a bound is undetermined (a zoned date within 14 hours of an unzoned bound) is otherwise not evaluated;
- none, for `is_defined` and `is_undefined`.

The operators are `equals`, `not_equals`, `greater`, `greater_or_equal`, `less`, `less_or_equal`, `between` (inclusive at both ends, with the tolerance applied at each), the text operators `contains`, `one_of`, `none_of`, `like` and `matches`, and the presence operators `is_defined` and `is_undefined`. The text operators mean what they mean in property selectors. `like` compares the whole value with a wildcard pattern: `*` stands for any run of characters, none included, `?` for exactly one, and a backslash makes the next character literal (`\*`, `\?`, `\\`). `matches` compares the whole value with a regular expression, anchored at both ends, so `EI\d+` does not match `EI30-T1`. A declared pattern that does not compile is an invalid declaration; one read from `target_property` that does not compile leaves the object not evaluated. Text comparisons respect case unless `case_sensitive` is `false`. `is_undefined` holds for an absent property, null or blank text, `is_defined` for anything else; neither reports a missing property as missing information.

Two more quantifiers compare a single number with the target:

- `count` compares the number of candidates, zero included, and needs no `compared_property`.
- `sum` compares the total of the candidates' compared values. The values must all be integers, all numbers, or all quantities of one dimension; otherwise the checked object is not evaluated. A candidate without the value gets its missing-property finding, and no verdict is drawn from the partial total.

`between` gives a count or a sum a minimum and a maximum in one rule. Text and presence operators do not apply to a count or sum.

With `category_property`, every finding of a checked object starts with that object's value of the property in brackets, such as `[F90] `, and cites it, so findings can be grouped by fire rating, discipline or similar. An absent, null or blank value adds no category. The property is read only for objects with findings; when it cannot be read, the object is not evaluated rather than reported uncategorised.

A definition bound to `property-comparison` must declare every parameter above, including the optional `path`, `container_selector`, `case_sensitive`, `minimum_number`, `maximum_number`, `minimum_quantity`, `maximum_quantity`, `minimum_date`, `maximum_date`, `minimum_date_time`, `maximum_date_time`, `minimum_property`, `maximum_property`, `target_date`, `target_date_time`, `precision` and `category_property`.

`axioval:capability.free-floor-circle` and `axioval:capability.free-floor-rectangle` check whether each selected spatial scope can contain an exact supported vertical shape. Circle parameters are `diameter_metres` and `height_metres`; rectangle parameters are `width_metres`, `length_metres`, and `height_metres`, all in canonical metres, and an `orientation`. The rectangle `orientation` must be `any` (every rotation counts). A rule without it, or with an orientation that no service can yet ground in a frame (such as `fixed-to-room` or `fixed-to-door`), is reported not evaluated as an invalid declaration, because a rectangle placement is only meaningful once it says which rotations it allows. Both take eleven optional parameters, which a definition bound to either must declare:

- `obstacles` (selector) chooses what blocks the floor. Without it every other project object is a candidate obstacle. An object the selection cannot decide is sent as a candidate, so a found placement stands; a proof that none exists is asked again without the undecided objects and becomes a finding only if it still holds, otherwise the space is not evaluated.
- `band_from_metres` and `band_to_metres` (numbers) set the elevation band, relative to the scope's floor, in which obstacles count; only the part of an obstacle's solid inside the band blocks. Without them the band runs from the floor up by `height_metres`; a missing end defaults to the floor or that height. A band starting below the floor or ending at or below its start is an invalid declaration. A band ending at 0.67 m, for example, leaves the knee room under a wash basin free.
- `subtract_door_swings` (selector) picks the doors whose leaves' swing is an obstacle too: each selected door's hinged leaves, as the object-frame service states them, sweep sectors of floor that are sent with the request (see [Door swings as obstacles](./free-space.md#door-swings-as-obstacles)); a sliding door sweeps nothing. A found placement keeps clear of every swing's circumscribed polygon, and a proof that none exists holds against the inscribed ones. A door the selection cannot decide is sent like an undecided obstacle; a door whose leaves cannot be read (or without the object-frame service) may cover any placement, so a found one leaves the space not evaluated, while a proof of absence stands.
- `entrance_path_width` (number) requires the shape to be reachable from the space's entrances by a path this wide: it must meet the free area, eroded by half the width, that comes within half the width plus `entrance_tolerance_metres` (default 0.05 m) of an entrance (see [Free space](./free-space.md#placement-search)). The entrances are the doors and openings `access_path` relates to the space (and to the merged spaces), picked by `door_selector` and `opening_selector` among the spaces `space_selector` picks, as in [`local-circulation`](#local-circulation); `entrance_path_width` and `access_path` go together. A placement reached from a surely selected entrance passes; a proof that no placement is reached from any entrance, sure or possible, is a finding, worded `…: the shape fits only where no path 1.2 m wide from an entrance reaches it` when the shape fits unreached, and relating the entrances. A turning circle behind a bed that leaves 0.8 m beside it is a finding for a 1.2 m path.
- `merge_path` (a relationship path, as in `related` selectors) reaches the spaces searched together with the selected one, such as `axioval:derived.overlapping-group-space:backward` from a group space to the spaces it covers, or a stated grouping. The union of their footprints is searched; merged spaces are never obstacles, must stand on the same floor, and are related to the finding. A path the source cannot answer leaves the space not evaluated.

Without merged spaces each request requires exact whole-base support on the selected scope with zero hidden gap; with them the search is bounded by the union of the footprints instead, since no single space supports a base across them. A complete exact no-placement proof emits the shape-specific `NO_FREE_FLOOR_SPACE_*` finding; missing services, backend outages, or invalid/incomplete evidence emit not-evaluated outcomes instead.

A free corridor width is not a free-floor option. Whether a path of a width runs from a space's entrances to its components is [`local-circulation`](#local-circulation), which states its ends (the entrances and components) and bounds the eroded free region from both sides; across spaces, corridor widths stay with [metric routing](./metric-routing.md) and `accessible-route`. A rectangle fixed to a door or fixture (a frame-offset domain) is answered by the Axiolid placement search (see [Free space](./free-space.md#the-axiolid-placement-search)), but no free-floor `orientation` grounds one yet.

`axioval:capability.slab-contact` requires at least `minimum_contact_ratio` of each selected subject's face to rest on something on its `contact_side` (`above` or `below`), measured through `ContactServiceHandle` with the `maximum_gap_metres`, `maximum_intersection_metres` and `minimum_polygon_area_square_metres` tolerances. What the face may rest on is the optional `counterparts` selector; without it every other project object is a candidate. The capability resolves the selector and sends the candidates in the `ContactRequest`, and the adapter measures exactly those: an object outside them neither supports the face nor blocks the measurement, while an unmeasured candidate makes the subject not evaluated. A counterpart the selector cannot decide leaves a shortfall not evaluated, since it might support the face; a pass stands, because more candidates can only add contact. `skip_top_storey` and `skip_bottom_storey` leave out subjects on the highest or lowest storey of their source. Storeys are the `storey_selector` objects, ordered by their `Elevation` attribute (a length), and a subject's storey is the one the declared traversal (`relationship` or `path`, as below) reaches from it. Skipping needs both. Every storey needs a known elevation and every subject exactly one storey; otherwise the subjects concerned are not evaluated rather than guessed at. A shortfall is graded by how far the ratio falls below the minimum, no contact at all by the distance to the nearest candidate.

`axioval:capability.clash` and `axioval:capability.distance` check selected subjects against a selector-valued `counterparts` group through `ProximityServiceHandle`, after the engine's complete broad-phase candidate search. Clash puts each pair in one class: a duplicate (surfaces within `duplicate_tolerance_metres` of each other), one body inside the other, or an intersection (a witnessed penetration beyond `penetration_tolerance_metres` whose extent exceeds `horizontal_tolerance_metres` along both plan axes and `vertical_tolerance_metres` in height, and whose certified shared volume exceeds `volume_tolerance_cubic_metres`). `report_duplicates`, `report_containment` and `report_intersections` switch the classes off. A clearance clash is a separation below the optional `clearance_metres`. `exclude_paths` (relationship paths, steps separated by spaces) leaves out pairs that reach a shared target, or, with `exclude_target_property`, targets stating the same value of that property (one system split across federated models); `exclude_same_layer` leaves out pairs of one model sharing a presentation layer. `group_by` (`subject`, `type_pair` or `similar`, with `group_tolerance_metres` and `group_property`) groups reported pairs into one finding each, relating every object involved and carrying each pair's evidence, and `per_storey` with `storey_path` keeps storeys apart. `severity_by_class` gives each class its own severity and `severity_grades` grades intersections by their smallest extent or shared volume (`grade_by`), a straddling measure taking the most severe grade it may reach; a duplicate's finding names what the copies differ in, types, volumes and the `duplicate_quantities`. `tolerance_cases` excuses intersections measured along the elements' own placement axes (orthogonal and protrusion cases, horizontal and vertical, each with two component filters and a tolerance), through the object-frame service and the proximity service's `measure_overlap_along`; an undecided case leaves the pair not evaluated. A definition bound to `clash` must declare all twenty-three parameters. Distance takes a `mode`: `nearest` (the default) bounds the nearest counterpart with `minimum_metres` and/or `maximum_metres`, `none_closer_than` keeps every counterpart at least `minimum_metres` away, and `at_least` requires `count` counterparts within `maximum_metres` (and no nearer than an optional `minimum_metres`). A `projection` measures in space (`minimum_3d`, the default), in plan (`horizontal`), between bodies above one another (`vertical`, with an optional `footprint_offset_metres` and a `vertical_direction` of `either`, the default, `above` or `below`), or as overlap in plan (`plan_overlap`). `subject_surface` and `counterpart_surface` measure a `vertical` distance between chosen surfaces (`top`, `bottom`, and for the counterpart `nearest`, the surface directly over the subject's footprint), and `elevation_overlap` `overlapping` (with `elevation_offset_metres`) relates only counterparts at the subject's heights in `horizontal`. The traversal parameters (`relationship` or `path`) scope counterparts to those sharing a container with the subject, and `container_selector` to containers of one kind. Findings report how far the named distance misses its bound, so rules may grade them with severity bands. Distances are intervals; a counterpart that could fall either side of a bound counts as unknown, and a verdict is given only when the unknowns cannot change it. `subject_extent` and `counterpart_extent` `leaf_swing` (or its older name `door_swing`) measure a side by the plan area its doors' leaves and windows' panels sweep instead of its body, in plan. A definition bound to `distance` must declare `mode`, `count`, `projection`, `footprint_offset_metres`, `vertical_direction`, `subject_extent`, `counterpart_extent`, `subject_surface`, `counterpart_surface`, `elevation_overlap`, `elevation_offset_metres`, `container_selector` and the traversal parameters as optional. Findings on tessellated geometry carry inexact evidence and say so. See [Clash, interference and distance](./clash.md).

`axioval:capability.containment` checks that the selected inner elements lie in `counterparts`: their certified shared volume is at least `minimum_volume_ratio` of the smaller body's, judged on the interval, with `combine_adjacent` also taking outer elements whose surfaces meet together. A `cover` table bounds, per row, the signed distance from the inner body to one class of the outer element's faces (`faces`: `top`, `side`, `bottom` or `any`), from `inside` (a cover) or `outside` (a protrusion), between `minimum_metres` and `maximum_metres`. `minimum_count` and `maximum_count` bound the inner elements each outer element holds, and `report_orphans` reports an inner element lying in none. Undecided containments, straddling intervals and counts the undecided elements could change are not evaluated. A definition bound to it must declare all seven parameters. See [Containment and cover](./clash.md#containment-and-cover).

`axioval:capability.clash-matrix` judges each pair with the tolerance profile and severity of one cell of a `cells` table. A cell keys both sides of the pair (`subject_*`, `counterpart_*`) by discipline pattern, by patterns over the text properties `key_1` to `key_3` name, and by a selector, and gives the `clash` tolerances and switches as columns, with an optional `severity` and `label`. The single most specific cell applies, either way round unless `symmetric` is false; tied cells or an unreadable category leave the pair not evaluated, and a pair no cell covers is ignored or, with `report_unmatched`, reported. Same-system exclusion (`exclude_same_system`, through `system_path`, matching systems by `exclude_target_property` too when declared) is on by default and same-layer exclusion off. Grouping, severities and duplicate comparisons are `clash`'s; grouping stays within one cell, and a cell's severity wins over its class's. A definition bound to it must declare all twenty-three parameters. See [Clash matrix](./clash.md#clash-matrix).

`axioval:capability.horizontal-guard` checks that the exposed edges of the selected walking surfaces are guarded, through `GuardServiceHandle`: by barriers tall and close enough, or by a short fall onto a landing wide enough to stand on, and not defeated by a climbable object beside the barrier. The rule's selection is the walking-surface profile. The optional selectors `barrier_selector`, `landing_selector` and `climbable_selector` name the objects that may play each role; their resolved sets travel in the `GuardSearch`, and only their members are counted for that role, so a cupboard along an edge is not taken for a railing. An absent selector leaves its role open to any nearby body. An object a role selector cannot decide makes the rule not evaluated, since it might be the barrier that guards an edge. A definition bound to this capability must declare all three selectors, as optional `selector` parameters.

`axioval:capability.external-wall-validation` compares the objects a model declares external with the objects on the envelope geometry derives, through `EnvelopeMembershipServiceHandle`, and reports each selected object that disagrees against itself. Which objects bound the envelope is the rule's choice, carried in every `EnvelopeMembershipRequest`; the service derives around exactly those, and the host declares none of its own.

| Parameter | Kind | Meaning |
|---|---|---|
| `derivations` | `stringList` | Required. `all-spaces`, `gross-area-groups`, or both. Each is measured and reported on its own; every finding and not-evaluated outcome names its derivation. An unknown or repeated name is an invalid declaration. |
| `bounding_selector` | `selector` | The objects the `all-spaces` envelope is derived around, such as every `IfcSpace`, or the spaces a `related` selector finds in one zone. Required by `all-spaces`. |
| `gross_area_group_selector` | `selector` | The gross-area groups, such as `IfcZone`s with a given name. Required by `gross-area-groups`, together with the path. |
| `gross_area_group_path` | `stringList` | Steps from each group to its members, as in `path`; with IFC, `IfcRelAssignsToGroup:forward`. The members of every selected group, together, bound the `gross-area-groups` envelope. |

A derivation whose bounding input is not declared is an invalid declaration of the whole rule, never measured around nothing. A bounding or group selector that cannot decide an object leaves that derivation not evaluated, since the undecided object might bound the envelope; so does a selector that selects nothing, groups with no member, or a refused relationship answer along the path. The other derivation is still measured. A bounding object is the envelope's inside and takes no part in the comparison. An object the model declares neither external nor internal, or whose body could not be measured, is not evaluated, never read as internal. A definition bound to this capability must declare all four parameters, the last three as optional.

When no selected object of a source is declared external, the model declares no envelope at all: that is one `error` finding against the source (`no selected object is declared external`), in place of one "not declared external" finding per object on the envelope. A model whose every wall states `IsExternal = false` gets exactly that one finding. A selected object stating neither, or whose body could not be measured, might be the external one, so such a source is not evaluated instead.

With both derivations measured, an object one derivation places on the envelope and the other does not is reported against itself (`on the gross-area-groups envelope but not on the all-spaces envelope`), whatever it declares: the two envelopes disagree about it. The comparison reads each derivation's full membership (`EnvelopeMembershipEvidence::on_envelope`), including objects that declare nothing; an object bounding either derivation is not compared.

`axioval:capability.space-validation` checks each selected space through `SpaceServiceHandle`, one measurement per sub-check, so a sub-check the adapter cannot measure is not evaluated without sinking the others. A sub-check refused because objects that could change it are unmeasured (`SpaceError::Unmeasured`) is not evaluated as incomplete evidence, and its message names the sub-check and the first five of those objects, then counts the rest. It takes `required_height_metres`, `uncovered_segment_length_metres`, `check_top_cap`, `check_bottom_cap`, `check_unallocated_area` and `maximum_unallocated_area_square_metres`, and six optional parameters:

- `maximum_unallocated_share` (between 0 and 1): with `check_unallocated_area`, the largest share of each storey's gross floor area (the plan area of its storey-assigned elements) that its unallocated regions may cover together; 5 % outside every space fails `0.03`. It is judged beside the absolute maximum, each its own finding. A storey whose gross area the service does not state, or states as zero, is not evaluated: its share is undefined.

- `tolerance_metres` (default 0.005): a space is too low only when it misses `required_height_metres` by more than this, and an overlap no thicker than this is contact, not an intersection.
- `top_cap_elements` and `bottom_cap_elements`: selectors naming the elements that may cap a space from above or below. The selection travels in the `CapRequest`, and the service considers exactly those elements. Without a selector the host's declared slabs (and, for the top, roofs) are used. A selector that selects nothing skips that cap; one that leaves objects undecided makes that cap not evaluated for every space, since an undecided element may be the covering one.
- `boundary_elements` and `intersection_elements`: selectors naming the elements that bound a space (cover its boundary) and that a space must not intersect. The selection travels in the `BoundaryRequest` and the `OverlapRequest`, and the service considers exactly those elements. Without a selector every body that is not a space bounds a space, and every other body (spaces included) is checked for intersection. As with the caps, a selector that selects nothing skips its sub-check, and an undecided one makes it not evaluated for every space. A space enclosed only by furniture has an uncovered boundary when `boundary_elements` selects walls, and none once it selects furniture too.

With `check_unallocated_area`, the service reports each connected region of storey floor that no space covers (`SpaceService::measure_unallocated_regions`), and each region is judged on its own against `maximum_unallocated_area_square_metres`: two 0.5 m² shafts and a 20 m² hole on one storey under a 1 m² allowance give one finding, for the hole. The finding is against the storey, names the region's area and relates the bodies around it; its deviation is the region's excess over the allowance, so `severityBands` can grade it (a band `below` 1 for up to twice the allowance, `below` 4 for up to five times).

Every finding message starts with its sub-check's category code, so results can be grouped by problem as well as by space:

| Code | Sub-check |
|---|---|
| `duplicate_space` | Another space has the same body. |
| `insufficient_height` | Clear height below the requirement, beyond the tolerance. |
| `uncovered_boundary` | Boundary runs of at least `uncovered_segment_length_metres` that no element covers. |
| `contained_body` | The space contains, or lies inside, another body. |
| `intersecting_space` / `intersecting_component` | The space intersects another space or a component. |
| `uncovered_top_cap` / `uncovered_bottom_cap` | A cap less than 98 % covered; under 1 % is an error, up to 15 % a warning, otherwise information. |
| `unallocated_area` | A connected region of storey floor belonging to no space and larger than the allowance, or a storey whose regions together cover a share of its gross area above `maximum_unallocated_share`, reported against the storey. |

### Entity types and resource objects

An `entityType` selector selects the objects of a class (`includeSubtypes` adds its subclasses; see [Subtypes](./concept-binding.md#subtypes)). It is also the only way to select [resource objects](./ir.md#resource-objects): instances the source does not make objects of, such as an IFC `IfcMaterial`, `IfcClassification` or `IfcRelConnectsPathElements`. A class selects either objects or resource objects, never both: a class whose instances are objects, and with `includeSubtypes` a class one of whose subclasses is (`IfcRoot`, `IfcObjectDefinition`), has no resource objects, so no rule written before resource objects existed selects anything new.

The population a rule selects from is every object of the project, then every resource object its applicability reaches:

- an `entityType` selector reaches its class's resource objects;
- `allOf` and `anyOf` reach what any operand reaches;
- a `ruleOutcome` selector reaches the resource objects the named rule judged (selected, failed or left undecided);
- nothing else reaches one: not `all`, not `not`, and not a `property`, `classification`, `related`, `discipline` or `source` selector alone.

A reached resource object is selected only when the whole selector matches it, as an object is. Selectors in other parameters (counterparts, obstacles, table cells) never enumerate resource objects. A source whose resource objects of a reached class cannot be listed is not evaluated as a whole (`IncompleteEvidence`), never an empty selection. `object-count` counts the same population, so a count of `IfcMaterial` counts materials. A capability that needs geometry or placement asks for it as for any object and is refused for a resource object.

Findings on resource objects are ordinary object findings; the report carries the resource objects it names (`Report::resources`). A resource object has no location (no storey or space, and nothing unresolved) and is never a BCF component.

### Property selectors

A `property` selector (in a rule's selection or any selector-valued parameter) resolves one property exactly, as `property-predicate` does, and compares it with its `value` under an `operator`:

| Operator | `value` | Selects when the resolved value |
|---|---|---|
| `exists` | none | is present, even `null` or blank. |
| `isEmpty` | none | is present but `null`, blank text, or a list of nothing else: the `empty` presence of `property-requirements`. An absent property is not empty. |
| `isNotEmpty` | none | is present with a value: the `not-empty` presence of `property-requirements`. |
| `equals`, `notEquals` | boolean, integer, number, quantity, string, enum, reference, date or dateTime | equals, or does not equal, the value. |
| `lessThan`, `lessThanOrEquals`, `greaterThan`, `greaterThanOrEquals` | integer, number, quantity, string, date or dateTime | orders against the value; strings by code point, dates chronologically. |
| `matches` | string | matches the regular expression as a whole: `EI\d+` selects `EI30`, not `EI30-T1`. |
| `like` | string | matches the wildcard pattern as a whole: `*` is any run of characters, `?` one character, and `\` makes the next character literal. |
| `contains` | string | contains the text. |
| `oneOf`, `noneOf` | string list | is, or is not, one of the texts. |

Integers and numbers compare with each other. A `quantity` value is written with a unit (`m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`) and compared in SI with a quantity of the same dimension, allowing the few units in the last place one unit conversion can introduce. Text comparisons are case-sensitive unless `caseSensitive` is `false`, which folds both sides (Unicode lowercase) and makes `matches` and `like` case-insensitive. `trim: true` drops surrounding whitespace from the resolved text first. Both options apply to text comparisons only. `precision: day` applies to a `date` or `dateTime` value only (see [Dates and date-times](#dates-and-date-times)).

A list value, such as the presentation layers of an object, is compared element by element under a `quantifier`: `any` selects when at least one element satisfies the operator, `all` when every element does. A bounded value and a table are compared the same way by their stated values (a bound, a set point, a table cell; see [the IR](./ir.md)).

A `propertyPattern` selector names the properties by XML Schema patterns instead of a concept: `propertyPattern` for the name and an optional `propertySetPattern` for the set, each matching the whole name the source states. It compares every matching property, enumerated exactly, as a `property` selector compares one (the same `operator`, `value`, `caseSensitive`, `trim`, `quantifier` and `precision`), and selects when at least one property matches and the comparison holds for `matched` of them (`any` or `all`). No matching property is no match, under `all` too; negate an `exists` pattern selector to select objects without any. A source that cannot enumerate, or a pattern that cannot be translated exactly, leaves the object not evaluated.

```json
{ "kind": "propertyPattern", "propertySetPattern": "Pset_.*Common", "propertyPattern": "FireRating",
  "matched": "all", "operator": "oneOf", "value": { "type": "stringList", "value": ["EI 60", "EI 90"] } }
``` `all` never holds vacuously, so an empty list satisfies neither. A scalar value under a quantifier counts as a list of one. "Every layer is agreed" is `oneOf` with `quantifier: all`, "at least one layer is agreed" the same with `any`, and "no layer is forbidden" `noneOf` with `all`. An element that cannot be compared (an integer against text) leaves the object not evaluated unless the other elements already decide it.

A selection that must leave out see-through objects, such as the objects that may block a view, reads `axioval:presentation.Transparency`: every distinct transparency of the object's styled surfaces, from `0.0` (opaque) to `1.0`. Keep an object unless all of its surfaces are at least half transparent:

```json
{ "kind": "not", "operand": {
  "kind": "property", "propertySet": "axioval:presentation", "property": "Transparency",
  "operator": "greaterThanOrEquals", "value": { "type": "number", "value": 0.5 },
  "quantifier": "all"
}}
```

Combine it with the element types that can block in an `allOf`.

As a visibility rule's `blockers`, glazing (`[0.7]`) drops out; a window whose frame is tinted `0.2` (`[0.2, 0.7]`) and an opaque wall (`[0.0]`) still block. An object without any styled surface has no value, so `all` does not hold and the `not` keeps it: an unstyled object blocks. The positive form, `lessThan 0.5` with `quantifier: any`, selects the same styled objects but would drop every unstyled one.

The selector fails closed:

- an exactly absent property, or a `null` value, matches every operator but `exists` as "no", as a comparison presupposes a value; `null` alone is also `isEmpty`;
- a value of another type than the operator compares (text against an integer, a quantity against a unit-less number or one of another dimension, a boolean with `contains`) leaves the object not evaluated (`InvalidEvidence`), never silently out of the selection; `notEquals` and `noneOf` are no exception;
- a list value compared without a `quantifier` is not evaluated (`InvalidEvidence`), except by `exists`, `isEmpty` and `isNotEmpty`, which judge it whole;
- a date-time compared with a `date` value without `precision: day` is not evaluated (`InvalidEvidence`), as is text compared with a date;
- a selector whose `value` does not fit its operator, an unknown unit, an invalid pattern, a text option on a non-text comparison, a `precision` on anything but a date comparison, or a `quantifier` or text option on `exists`, `isEmpty` or `isNotEmpty` is an invalid declaration.

### Dates and date-times

A date (`PropertyValue::Date`) is a calendar day, which may state the time zone it was stated in (`xs:date` allows `2022-01-01Z` and `2022-01-01+01:00`); a date-time (`PropertyValue::DateTime`) is an instant with the UTC offset it was stated in. Package literals are `{"type": "date", "value": "2026-09-27"}` and `{"type": "dateTime", "value": "2026-09-27T10:00:00+02:00"}`; one that is not a real day, or a date-time without an offset, is refused when the package is read. `property-predicate`, `property-value`, `property-comparison` and property selectors compare them the same way:

- a date with a date as XML Schema orders `xs:date` values: two unzoned dates by day; two zoned dates by the instant each day begins (`2022-01-02+12:00` equals `2022-01-01-12:00`); a zoned and an unzoned date only when they lie more than 14 hours apart, since the unzoned one may be in any zone from `-14:00` to `+14:00`. Otherwise the pair is incomparable: never equal (so `equals` fails and `notEquals` holds: `2022-01-01Z` is not `2022-01-01`), and an order (`less_than`, a bound) is not evaluated;
- a date-time with a date-time as instants, whatever their offsets: `10:00:00+02:00` equals `08:00:00Z`;
- a date-time with a date only when the rule states `precision` `day` (a `precision` parameter, or `precision: day` on a selector). Day precision reads every date-time as the calendar day it states in its own offset, not the UTC day: `2026-09-27T22:30:00-05:00` is on the 27th. Two date-times at day precision compare their stated days, and a zoned date reads as the day it states, its zone aside.

Without day precision a date-time neither precedes nor follows the day it falls on, so that pair is not evaluated instead of guessed. Parameter names stay snake_case like every capability parameter (`date_time`, `target_date_time`), while the value tags are `date` and `dateTime`. Property selectors support dates in full: `date`/`dateTime` values with `equals`, `notEquals` and the ordered operators, `precision: day`, and list values under a `quantifier`. Table parameters take `date` and `dateTime` columns, such as the date bounds of `property-requirements`. `between` in `property-comparison` takes date and date-time bounds, constant or read from properties. Dates never compare with text, numbers or quantities, and take no tolerance; `unique-value` and `consistent-value` treat two date-times as one value when they name the same instant, and two zoned dates when their days begin at the same instant.

### Related selectors

A `related` selector selects an object by the objects a relationship `path` reaches from it. The `path` is written as the `path` parameter of the relationship-scoped capabilities: steps `Relationship` or `Relationship:direction` (`forward`, the default, `backward` or `either`), each optionally followed by `+` to take it one or more times (`IfcRelAggregates:backward+` reaches every whole above a part) and naming several relationships with `|` (see the step grammar under [semantic capabilities](#semantic-capabilities)), walked one after another through `RelationshipSelectionServiceHandle`, never reaching the object itself. The reached objects are tested against the nested `selector` under a `quantifier`:

| `quantifier` | Selects when |
|---|---|
| `any` (default, omitted when serialized) | at least one reached object matches. |
| `all` | every reached object matches, and at least one is reached. |
| `none` | no reached object matches; also when none is reached. |

Fire-wall doors are doors whose wall states `Compartmentation` true; with IFC:

```json
{ "kind": "allOf", "operands": [
  { "kind": "entityType", "objectType": "…door", "includeSubtypes": true },
  { "kind": "related",
    "path": ["IfcRelFillsElement:backward", "IfcRelVoidsElement:backward"],
    "selector": { "kind": "property", "propertySet": "…wall-common",
                  "property": "…compartmentation", "operator": "equals",
                  "value": { "type": "boolean", "value": true } } } ] }
```

The other direction, walls holding at least one unrated door, is `IfcRelVoidsElement` then `IfcRelFillsElement` forward from the wall. A `related` selector fails closed as well: a relationship answer the service refuses (an unknown relationship, an unresolved relationship end, an unavailable backend) leaves the object not evaluated, since the objects it would reach are unknown. So does a reached object the nested selector cannot decide, unless the others already settle the verdict: one match settles `any` and `none`, one non-match settles `all`. An empty or malformed `path` is an invalid declaration. Relationship names are the source's own, as in capability parameters; the nested selector names concepts like any selector and is bound per source. In `selector-conformance`, the properties of related objects are not among the checked object's consulted properties.

### Expression selectors

An `expression` selector selects the objects for which an [expression](./expressions.md) holds: `{"kind": "expression", "expression": {…}}`. It lets applicability depend on computed and measured values, such as walls whose height exceeds twenty times their thickness:

```json
{"kind": "allOf", "operands": [
  {"kind": "entityType", "objectType": "Wall"},
  {"kind": "expression", "expression": {"kind": "compare", "operator": "greaterThan",
    "left": {"kind": "divide",
      "left": {"kind": "property", "propertySet": "axioval:measured", "property": "extent_z"},
      "right": {"kind": "property", "propertySet": "Pset_WallCommon", "property": "Width"}},
    "right": {"kind": "literal", "value": {"type": "number", "value": 20.0}}}}]}
```

It follows the selectors' three-valued rules: true selects, false and `null` do not, and an expression that cannot be decided leaves the object not evaluated in every rule using the selector, never skipped (a measured height straddling the ratio). It composes with `allOf`, `anyOf`, `not` and `related` like any selector, and reaches no resource object: as for a property selector, only an `entityType` names those. It reads properties, measured values and the ruleset's derived values, never a rule's parameters; the compiler type checks it wherever selectors appear (rule applicability, selector parameters and table cells, classification rows, grouping members and relation ends) and refuses one that is not a truth with `EngineError::InvalidExpression`. `selector-conformance` reports the properties it reads as consulted.

### Discipline selectors

A `discipline` selector selects the objects of the sources that play a discipline in the check, such as `architecture` or `structure`:

```json
{ "kind": "discipline", "value": "structure" }
```

IFC carries no discipline, so the host declares one per source (the CLI's `--model PATH:DISCIPLINE`, or `EvidenceSession::with_discipline`) or assigns one from what the source states (the CLI's `--discipline-map`, or `EvidenceSession::with_discipline_map`: the first rule whose wildcard pattern matches a value of a [source metadata](#source-selectors) field assigns its discipline to a source declaring none, and the session records the rule and the value as the discipline's origin, which the selector cites as inexact evidence); the engine keeps it next to the source's snapshot and installs it per run as `SourceDisciplines`, which any capability may read. The value is a lowercase token: 1 to 64 ASCII letters, digits, `-` or `_`, starting with a letter or digit. A package with any other value does not load. Names compare exactly and the engine attaches no vocabulary to them; a project agrees on its names as it agrees on its rules.

Every object of a source matches or none does. An object whose source declares no discipline is not evaluated, never a non-match, so a discipline-scoped rule cannot pass over a model nobody classified; the outcome is reported once per rule and source (`not-recorded`), not once per object. `clash-matrix` keys its cells by discipline patterns directly; a `clash` rule between two disciplines selects its subjects and its `counterparts` with one discipline each:

```json
"applicability": { "kind": "allOf", "operands": [
  { "kind": "entityType", "objectType": "…wall", "includeSubtypes": true },
  { "kind": "discipline", "value": "architecture" } ] },
"counterparts": { "type": "selector",
  "value": { "kind": "discipline", "value": "structure" } }
```

### Classification selectors

A `classification` selector selects the objects carrying a classification in `system`, as their source states it through the classification service (see [Adapters](./adapters.md#classifications)). It tests the code in one of three ways:

| Field | Selects an assignment in `system` whose code |
|---|---|
| `code` | is the code. |
| `codePattern` | matches the XML Schema pattern as a whole, as IDS writes classification patterns: `Ss_25_.*` selects `Ss_25_10` and `Ss_25_10_30`, not `Ss_20_05`. |
| neither | is anything: the system alone. |

```json
{ "kind": "classification", "system": "Uniclass", "codePattern": "Ss_25_.*" }
```

`includeDescendants` also tests the codes of the assignment's ancestors, so a parent's code selects its children; it needs a `code` or a `codePattern`. The pattern is translated as `propertyPattern` translates names, and the system and the code are met by one assignment together. An assignment whose system the source does not state leaves the object not evaluated unless another assignment already matches. `code` together with `codePattern`, `includeDescendants` without either, or a pattern that cannot be translated exactly (character-class subtraction, `\i`/`\c`, `\p{Is…}`) is an invalid declaration.

A group a ruleset's own grouping derives (a flat implied by the flat number its rooms state) is selected by a `derivedGroup` selector, `{"kind": "derivedGroup", "grouping": "flats"}`, and reached from its members through `axioval:derived.group;by=<grouping>` ([Derived groups](./derived.md#derived-groups)). A class a ruleset's own classification derives is selected by a `derivedClass` selector instead, with the same `includeDescendants` over the classification's declared class tree ([Derived properties](./derived.md#class-trees)).

### Source selectors

A `source` selector selects the objects of the sources whose metadata satisfies a comparison, such as the models an architecture application wrote:

```json
{ "kind": "source", "field": "application", "operator": "like",
  "value": { "type": "string", "value": "*Architecture*" }, "quantifier": "any" }
```

| `field` | Holds | With the IFC adapter and the CLI |
|---|---|---|
| `fileName` | the file name the host read the source from | the `--model` file's name |
| `application` | every application the source states wrote it | `ApplicationFullName` of each `IfcOwnerHistory.OwningApplication` |
| `schema` | the schema the source declares | `IFC2X3` or `IFC4`, from the snapshot |
| `project` | the name of every project the source describes | `IfcProject.Name` |
| `timestamp` | when the source states it was written, as written | `FILE_NAME.time_stamp` of the header |

The field is compared as a property selector compares a value, with the same `operator`, `value`, `caseSensitive`, `trim` and `quantifier`: one value as a scalar, several (a model written by two applications) as a list that needs a `quantifier`. Every object of a source matches or none does. A field the source states it lacks, such as the application of a file without owner histories, matches no operator, as an absent property does. A field that was never read, because the adapter does not read it or could not read it exactly (an owner history naming no `IfcApplication`), is not evaluated, reported once per rule and source (`not-recorded`), never a non-match. The engine installs the metadata per run as `SourceMetadataIndex`: adapters and hosts state it with `EvidenceSession::with_source_metadata`, field by field, and a field stated twice with other values is refused. Metadata is not part of a snapshot's identity. A selector whose `value` does not fit its operator is an invalid declaration, as for property selectors.

### Numeric tolerance

`property-predicate`, `property-comparison` and `unique-value` take three optional parameters that relax exact numeric comparison. They apply to integers, decimals and quantities; a quantity is compared in its canonical SI unit, so a tolerance on a length is in metres. Text and booleans are never affected, and a tolerance declared with a text, text-list or boolean constant target is an invalid declaration. A definition bound to one of these capabilities must declare all three, as optional parameters (`number`, `number`, `integer`).

- `tolerance` (a non-negative number) and `relative_tolerance` (a fraction at least 0 and below 1), alone or together: two values `a` and `b` are equal when `|a − b| ≤ tolerance + relative_tolerance · max(|a|, |b|)`, the boundary included. Values are read as the decimals they display, so a few units in the last binary place are allowed: `1.0` and `1.1` are within `0.1`.
- `decimals` (0 to 15): both values are rounded half away from zero to that many decimal places of their shortest decimal form, so `2.345` rounds to `2.35` as displayed, and then compared exactly. It cannot be combined with a tolerance.

Under either, two equal values are neither greater nor less than each other: `greater_than 10` with `tolerance 0.5` fails for `10.5`. `property-comparison` applies the tolerance between the compared value and the target after its `factor`, and to `count` and `sum` as well. Every finding judged under a tolerance states it, for example `(within tolerance 0.1)` or `(rounded to 2 decimal(s))`.

`consistent-value` takes its own `tolerance` or `tolerance_quantity` on the range of a group (see [Semantic capabilities](#semantic-capabilities)); it rounds nothing and has no relative tolerance.

Rounding is transitive, so `unique-value` groups values that round alike. A tolerance is not: `1.0` and `1.2` are each within `0.1` of `1.1` but not of each other. `unique-value` therefore judges a tolerance pair by pair: an object is a duplicate of every other object in its scope whose value is within the tolerance of its own, and its finding names exactly those objects. In the example, `1.1` is reported naming the other two, and `1.0` and `1.2` each naming `1.1` only. A quantity is compared only with quantities of the same dimension; an integer and a decimal are compared with each other.

### Expression requirements

`axioval:capability.expression` requires an [expression](./expressions.md) to hold for each selected object. Its `requirement` (kind `expression`) must be a truth; the compiler checks its structure, the concepts and measured values it reads, and its types and units, and refuses an ill-typed requirement with `EngineError::InvalidExpression` naming the path into it (`requirement.and[2].compare.left`).

- True passes. False is a finding naming the subexpression that failed (its `label`, or its path and kind: the first false operand of an `and`, the consequent of an `implies`) and every value read, with each value's interval and unit, citing the evidence of every read. A requirement that is `null` (a value it needs is stated absent, as a comparison with `null` is `null`) is a missing-information finding, `requirement cannot be confirmed: … is null`, never a pass.
- Not evaluated leaves the object not evaluated, with the reason and the subexpression's path: a property that cannot be read (with the resolver's own reason), a measured value straddling a bound (`incomplete_evidence`), or an `if` whose condition is undecided and whose branches disagree.

Two optional parameters word and grade the findings:

- `deviation` (kind `expression`, a plain number) is evaluated for each failing object and graded by the rule's `severityBands` as every grading capability's deviation is (see [Refining outcomes](./refinement.md)): `{"kind": "subtract", "left": slope, "right": 6}` with `severityBands: [{"below": 1.0, "severity": "warning"}]` grades a slope up to 1 % over its limit a warning and anything further the rule's own severity. A measured deviation is an interval and takes the most severe band it may reach; a negative deviation counts as zero, and one that cannot be computed leaves the rule's severity.
- `message` (a `string`) replaces the finding's message: each `{label}` is the value of the subexpression of the requirement or the deviation carrying that `label`, with its unit and interval (`the ramp slopes {slope} %, {excess} % over 6 %` reads `the ramp slopes 6.5 %, 0.5 % over 6 %`). A label no evaluated subexpression carries stays as written.

A definition bound to `expression` may declare parameters of its own beside `requirement` (`RuleCapability::takes_authored_parameters`): scalar values (`string`, `boolean`, `integer`, `number`, `quantity`, `enum`, `date`, `dateTime`), `stringList`s and `table`s with the columns the definition declares. The compiler checks each value against its declared kind and each table row against its columns. The requirement reads them as `parameter` and `lookup` nodes: a lookup matches its keys against the table's key cells as `keyed-limit` matches rows (a text cell is a wildcard pattern over the key read as text, any other cell must equal the key, a blank cell accepts any key, the single most specific row applies) and reads its `column`. No matching row, or a blank result cell, is `null`; tied or undecided rows leave the object not evaluated.

A three-branch cover requirement in one rule:

```json
{"requirement": {"type": "expression", "value": {
  "kind": "compare", "operator": "greaterThanOrEquals", "label": "cover",
  "left": {"kind": "property", "propertySet": "Pset_Concrete", "property": "Cover"},
  "right": {"kind": "if", "branches": [
      {"when": {"kind": "compare", "operator": "equals",
                "left": {"kind": "property", "propertySet": "Pset_Concrete", "property": "ExposureClass"},
                "right": {"kind": "literal", "value": {"type": "string", "value": "XC4"}}},
       "then": {"kind": "literal", "value": {"type": "quantity", "value": 40.0, "unit": "mm"}}},
      {"when": {"kind": "compare", "operator": "equals",
                "left": {"kind": "property", "propertySet": "Pset_Concrete", "property": "ExposureClass"},
                "right": {"kind": "literal", "value": {"type": "string", "value": "XC3"}}},
       "then": {"kind": "literal", "value": {"type": "quantity", "value": 35.0, "unit": "mm"}}}],
    "else": {"kind": "literal", "value": {"type": "quantity", "value": 25.0, "unit": "mm"}}}}}}
```

A property's type comes from its declared `valueKind` (a quantity's dimension from its `unitDimension`: `length`, `area`, `volume`, `plane_angle` or a unit); one whose type is not declared is checked when read, and a mismatch then leaves the object not evaluated.

### Semantic capabilities

These capabilities judge exact properties, classifications and relationships only, so they need no geometry. Relationship parameters (`relationship`, `direction`, `follow_chain`, `skip_absent_relationship_ends`) mean the same as in `property-comparison`. With the IFC adapter, a relationship is an IFC relationship entity name such as `IfcRelAggregates`. An object whose selection or value cannot be decided is not evaluated, never passed.

| Capability | Checks |
|---|---|
| `selector-conformance` | Each selected object satisfies the `requirement` selector. An agreed list is an `anyOf` of `allOf` rows. An object none of whose consulted properties has a value gets its own "no value" finding. Other failing objects are grouped by their combination of values: one finding per unknown combination, against the first object, naming the values and relating the others. `message` replaces the default text and is followed by the values. Every finding cites the property facts consulted. A layer agreement ("walls of this type must be on one of these layers") selects by class and type in the rule's selection and requires `axioval:presentation.Layer` `oneOf` the agreed layers with `quantifier: all`; over a source without layers it is not evaluated once for that source. It runs as a template over a group decision ([Capability templates](./templates.md#selector-conformance)). |
| `classification` | Each selected object carries a classification meeting the requirement a `classification` selector cannot state: `systems` and `codes` as literals, `system_patterns` and `code_patterns` as XML Schema patterns (literals and patterns together must both hold), or a system alone. One assignment must meet the system and the code together, and a code matches the assigned item or any ancestor. An unclassified object fails unless `optional`; an optional classification holds for an object with none at all and must be met by one with any. `prohibited` inverts the verdict. An assignment without a stated system leaves the object not evaluated when it could decide the verdict; a missing classification service leaves every object not evaluated. Optional and prohibited together are an invalid declaration. |
| `unique-value` | `property` does not repeat among the selected objects of one source, or of the project with `across_sources`, or among objects reaching the same related objects through `relationship` (for example one storey). Text is trimmed and compared ignoring case unless `trim` or `case_sensitive` says otherwise. A missing value is a finding unless `require_value` is `false`. Numbers may be compared under a `tolerance`, `relative_tolerance` or `decimals` (see [Numeric tolerance](#numeric-tolerance)). It runs as a template over a group decision ([Capability templates](./templates.md#unique-value)). |
| `consistent-value` | Objects of one kind that share a `key` value share their `value` too. An absent value is a value of its own. Each member of a disagreeing group is reported, naming the others. Objects without a key form one group of their own, so a missing key is reported only when those objects disagree. With `tolerance` (a number, applied to numbers and to quantities in SI units) or `tolerance_quantity` (a quantity, applied only to quantities of its dimension), numeric values agree when the group's range, its greatest value less its least, is within the tolerance; a measured interval counts with its full width, and a range that may lie on either side of the tolerance leaves the group not evaluated. A group beyond it reports each member farther than the tolerance from the group's median (or, when none is, the members at the ends of its range), relating the others; a member whose distance straddles the tolerance is not evaluated. A value the tolerance does not apply to leaves its object not evaluated. It runs as a template over a group decision ([Capability templates](./templates.md#consistent-value)). |
| `object-count` | The rule's selection holds between `minimum` and `maximum` objects in each source, or in the whole project with `across_sources`. With neither bound it is an existence check: at least one. A violation is reported against the source or the project, not an object, so an empty selection reads "no object matches the selection" instead of passing. The finding relates the objects found and cites the facts that selected them. Undecided objects count as unknown: the scope is judged only when they cannot change the verdict, and is otherwise not evaluated together with each undecided object. Every source of the session is counted, including one that holds no objects: an empty model reports "no object matches the selection in source …" rather than passing because nothing was there to count. Per source, a run with no source at all is not evaluated. `disciplines` (a string list of [disciplines](#discipline-selectors)) counts only the sources playing one of them, so a per-source duct count limited to `mep` judges the MEP models and never the architecture model; a source declaring no discipline may or may not count, so per source it is not evaluated (once, `not-recorded`) and across sources its matching objects are undecided; no source playing a listed discipline leaves the rule not evaluated. An empty list or an invalid name is an invalid declaration. A definition bound to `object-count` must declare `disciplines` as an optional `stringList`. It runs as a template deciding per scope ([Capability templates](./templates.md#object-count)). |
| `related-count` | Each anchor has between `minimum` and `maximum` related objects that `related_selector` picks. Without `relationship`, the anchor's whole source is counted. With `same_ends`, a path as `path` takes it, a related object counts only when that path reaches the same set of objects from it as from the anchor (a swing door between the same spaces as a revolving door); one whose ends cannot be read counts as unknown, and an anchor whose ends cannot be read or reach nothing is not evaluated. Undecided related objects are counted as unknown, and the anchor is judged only when they cannot change the verdict. A definition bound to `related-count` must declare `same_ends` as optional. It runs as a template over widening members ([Capability templates](./templates.md#related-count)). |
| `relative-count` | At each anchor, or in each group of objects sharing a `group_property` value, `provided / provided_unit` stands in `operator` (`equal`, `not_equal`, `greater`, `at_least`, `less`, `at_most`) to `required / required_unit`, computed in exact integers. It runs as a template over a proportion ([Capability templates](./templates.md#relative-count)). |
| `name-sequence` | The members of each anchor, ordered by the numeric `order` property, carry whole-number names from `first` stepping by `increment`. A name is a number only when it is exactly one. A number below `first` and a number out of order are separate findings; like a name that is not a number, a number below `first` does not interrupt the sequence. A member without an order value leaves the anchor not evaluated, unless `order_fallback` is `placement_height`: then a member whose order value is absent or null is ordered by the height of its placement origin in metres, read from the object-frame service (`ObjectFrameServiceHandle`). |
| `numbering-consistency` | The numbers `pattern` (an XML Schema pattern over the whole value with exactly one group of digits) reads from `property` agree within each scope, formed as in `unique-value`. With `prefix_length`, their first digits match: objects departing from the predominant prefix are reported, and every object when none predominates. With `gap_free`, the distinct numbers step by one, and the objects above each gap are reported. At least one check must be declared. A value that is missing or does not match is not evaluated, and a gap an unreadable object could fill is not evaluated rather than reported. |
| `manual-issue` | Raises the declared `title`, `category` and `description` once per rule, for checks owed by hand: the finding is against the first selected object and relates the others. A selection that decidedly picks nothing raises the check once for the project, saying that no object matched. |

| `level-spacing` | Each level's height, the rise of its `order` length to the next level up, lies within `minimum` and `maximum` and, with `consistent`, matches the prevailing height within `tolerance`. The highest level is not evaluated unless `ignore_highest`, or unless `content_path` measures it from geometry (see [Storey metrics](#storey-metrics)); `ignore_lowest` leaves out a basement. With `space_selector`, each level's spaces must be as high as the level within `space_tolerance` (unless `space_height` is `false`), and with `space_elevation` (`bottom`, `top` or `both`) they must share their bottom or top elevation within `space_tolerance`. It runs as a template judging the levels one by one ([Capability templates](./templates.md#level-spacing)). |

Deliberate differences from other checkers:

- `name-sequence` orders members by the declared `order` property and, only with `order_fallback: placement_height`, a member without one (a storey without `Elevation`) by the world height of its placement origin, which the object-frame service composes exactly. The two are mixed in one order, so the fallback suits an order property stated in the same height system as the placements. A member with neither an order value nor a placement the service reads exactly leaves the anchor not evaluated, and without the service it is a missing service. A present order value that is not a number is never replaced by the placement.
- `property-required` reports each property in its own rule, so an object missing several identity fields gets one finding per field. Combining them into one finding per object would add a parameter every `property-required` definition must then declare; grouping findings per object is left to the report consumer.

Most names and numbers these rules read are attributes of the object, not properties; see [attribute sets](./ir.md#attribute-sets). Quantities are written with a unit: `m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`. `²`, `³` and `°` are accepted too.

A traversal is either one `relationship` (with `direction` and `follow_chain`) or a `path` of steps walked in order, each `Relationship` or `Relationship:direction`. For example, `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` goes from a wall to the doors and windows filling its openings. A step ending in `+` is taken one or more times, reaching every object along the relationship's chain: `IfcRelAggregates:backward+` reaches every whole above an object (its assembly, storey, building and site), where `follow_chain` would apply to the whole traversal. The anchor is never among the objects reached, even around a cycle.

Every relationship path shares one step grammar (`PathSegment` in `axioval-engine`):

```text
step          = relationships [ ":" direction ] [ "+" ]
relationships = relationship { "|" relationship }
direction     = "forward" | "backward" | "either"
```

A step naming several relationships, separated by `|`, may take any of them; the one direction is written after the last and applies to all, and no relationship may be named twice. With `+` the chain mixes them, each hop taking any alternative: `IfcRelFillsElement|IfcRelVoidsElement:backward+` climbs from a door through the opening it fills to the wall the opening voids. Where such a chain changes relationship it passes through objects of the project; within one relationship it follows that relationship's chain as the source answers it. A derived identity keeps its own colon, since only a colon followed by a direction word ends the last alternative. The single `relationship` parameter names exactly one relationship; alternatives belong in a `path` step.

### Relationships derived from geometry

Every traversal parameter also accepts a derived relationship identity, answered from geometry rather than from what the model states: `axioval:derived.contained-in-space` (an element to the space containing it, or the nearest within `horizontal` and `vertical` tolerances), `axioval:derived.adjacent-space` (a door, window or opening to the spaces on each side of it), `axioval:derived.overlapping-group-space` (a space to the larger spaces covering at least `ratio` of its footprint) `axioval:derived.spans-level` (a space to every storey its height reaches at least `overlap` metres into, default 1, or half its own height into; storey areas walked `backward` count an atrium in every storey it spans), `axioval:derived.intersects` (an element to every object whose body shares volume with its own, both ways, such as a duct to the beam it passes through with no void modelled) and `axioval:derived.adjacent-across` (a wall or slab to the spaces within `tolerance` metres, default 0.05, of either of its faces along at least `overlap` metres, default 0.3, of it; the element's stated space boundaries win where it has any). Two spaces adjacent across a wall are the spaces on its opposite faces, as `across_side` reads them from the evidence; a room straddling the tolerance leaves its wall not evaluated. Tolerances follow the name, for example `axioval:derived.contained-in-space;horizontal=0.3;vertical=0.5`. Edges run from the element to the space, so counting components per space is `related-count` from each space `backward`. The service needs a geometry adapter; see [typed host services](./services.md). An undecided derivation leaves the anchor not evaluated, never counted as unrelated.

A derived identity holds a colon of its own, so in a `path` step only a trailing `:forward`, `:backward` or `:either` is read as the direction: `axioval:derived.adjacent-space;reach=1.5:forward` is one step.

`property-comparison` in `same_space` mode takes a derived containment as its container relationship: `relationship` `axioval:derived.contained-in-space` with `direction` `forward` and a space `container_selector` compares each component with the others standing in the same space, whether or not the model states where they stand.

### Openings and the spaces they connect

`opening-spaces` requires each selected door, window or opening to relate to the spaces its host wall calls for: two, one on each side, in an internal wall; one in an external wall, its other side outside. Which kinds are checked is the rule's selection, so openings, doors and windows are separate rules or one `anyOf`.

| Parameter | Kind | Meaning |
|---|---|---|
| `host_path` | `stringList` | Steps from the element to its host wall, as in `path`. With IFC, `IfcRelFillsElement:backward` then `IfcRelVoidsElement:backward` from a door or window, `IfcRelVoidsElement:backward` from an opening. |
| `host_selector` | `selector` | Which reached objects are host walls. |
| `external_property` | `propertyReference` | The host's boolean exposure, such as `IsExternal` in `Pset_WallCommon`, resolved through the property service. |
| `space_path` | `stringList` | Steps from the element to its spaces: a relationship the model states (with IFC, `IfcRelSpaceBoundary:backward`) or `axioval:derived.adjacent-space`. |
| `space_selector` | `selector` | Which reached objects count as spaces; every object by default. Optional. |

A stated relationship is judged by count: exactly two spaces for an internal host, exactly one for an external one. The derived adjacency is also judged by side, from its evidence: an internal host needs one space on each face and a different one on each, so two spaces on the same face are reported rather than counted as connected; an external host needs its one space on one face and the other face recorded outside. A space the evidence places on no side is invalid evidence. The derived adjacency must then be the only `space_path` step, forward, or the declaration is invalid, since the sides it records are the checked element's.

The element is not evaluated, never guessed internal, when its host does not declare the property (absent or null) or states a non-boolean, when hosts disagree, when no host is reached, and when a reached host or space is undecided and could change the count. A finding names the host and relates it and the spaces found.

Every source holding a selected element or a host wall is also checked as a whole: a source in which no host wall is declared external, including one with none at all, is a finding against the source, relating the walls examined. A wall that does not declare the property might be the external one, so the source is then not evaluated instead.

`relative-count` also has a table mode. `table` lists rows `R:P`, meaning "from R required objects on, at least P provided". The row with the largest R not above the required count applies. Beyond the last row, `additional_required` / `additional_provided` add P for every further R. Below the first row the table sets no requirement: the anchor or group is skipped, not extrapolated from zero. A table of increments alone applies them from zero. Parameters have no table type, so rows are text, and a malformed row is a declaration error.

In ratio mode, `small_required_below` n and `small_provided` k, declared together, replace the ratio for small counts: a required count from 1 up to but excluding n is judged as `provided operator k`. "With fewer than four workplaces, at least one washbasin" is n = 4, k = 1 with `at_least`; "below ten workplaces nothing is required" is n = 10, k = 0. A required count of zero is always judged by the ratio. Neither applies in table mode.

With `group_property`, `relative-count` groups instead of walking from anchors. The rule's selection is then the set of objects counted, and each object `provided_selector` or `required_selector` picks is counted in the group of its `group_property` value (a location code, say), within one source unless `across_sources`. Text is trimmed and compared ignoring case unless `case_sensitive`. Relationship parameters do not apply, and `across_sources` and `case_sensitive` apply only here. A group with required objects and no provided object is reported as present only in the required set, whatever the ratio, small-count case or table would say. A group's finding is raised against its lowest required object (its lowest provided object when it has none) and relates every other member. A counted object with no value (absent, null or blank) is a finding of its own; one whose value cannot be read is not evaluated and leaves every group of its scope not evaluated, since it could belong to any of them. A group with an object whose membership is undecided is not evaluated.

### Plan-area capabilities

These judge `PlanAreaService` measurements, so they need a geometry adapter.

| Capability | Checks |
|---|---|
| `area-ratio` | At each anchor, the summed footprints of `numerator_selector` over those of `denominator_selector` (or the anchor's own footprint) lie within `minimum` and `maximum`. `numerator_property` / `denominator_property` take a population's areas from an area-quantity property instead, e.g. glazing areas. `numerator_derivation` `light-area` derives each numerator member's light-transmitting area by a fallback (below). It runs as a [template](./templates.md): the measured `ratio_area` (or `light_area`) summed over each population, their ratio judged by the generic range judge. |
| `plan-coverage` | Each subject's footprint lies within one `candidate_selector` object by at least `minimum_ratio`, e.g. a space within a fire compartment. How much of an element the union of several counterparts leaves uncovered is `counterpart-coverage` (below). It runs as a [template](./templates.md): the measured `plan_coverage` search judged against the minimum. |
| `plan-area` | Each selected object's footprint lies within `minimum` and `maximum` square metres, e.g. a space of at least 8 m² or a fire compartment (a zone, measured as the union of its members) of at most 400 m². With `member_selector`, the summed footprints of the members each anchor reaches lie within the range instead, e.g. the space area of each storey. It runs as a [template](./templates.md): the measured `plan_area`, or its sum over the members, judged by the generic range judge. |

All bounds are inclusive, and `area-ratio` and `plan-area` need at least one of `minimum` and `maximum`. Areas are intervals: a tessellated body measures within a bound derived from its chord deviation. A verdict needs the whole interval on one side of a bound, so an area that straddles one is not evaluated rather than judged from a midpoint.

`area-ratio` and `plan-area` take `measure`: `footprint` (the default) or `facade`. `facade` measures each object's outward-facing surface through `FacadeAreaService` instead of its footprint (see [typed host services](./services.md)); a population with a declared area property still reads the property. A facade area may be zero, so `plan-area` does not treat an empty facade as a missing body. `area-ratio` also takes `numerator_measure` and `denominator_measure`, each `footprint` (the default) or `facade`, to measure the two sides differently: an external-wall ratio is the facade area of a storey's external walls over its gross footprint, and its findings read "facade area to plan area ratio is …". `measure` declared beside either is an invalid declaration, and so is a `facade` side with the `light-area` derivation.

`plan-area` reaches members as `related-count` does: through the declared traversal (`relationship` or `path`, as above; with IFC, `IfcRelAssignsToGroup` from a zone or `IfcRelAggregates` from a storey), or everywhere in the anchor's source without one; a traversal without `member_selector` is an invalid declaration. Footprints are summed, so members that overlap count twice; select members that tile the floor, such as spaces. An object with an empty footprint has no body and is not evaluated, and so is an anchor with such a member, since its sum is unknown. A member whose selection is undecided can only add area: a sum already above the maximum is still a finding, anything else is not evaluated. A finding relates the members summed. A definition bound to `plan-area` declares `minimum`, `maximum`, `member_selector` and the traversal parameters, all optional.

#### Light-opening area

A floor-to-window ratio sums the light-transmitting area of windows, doors or curtain walls per space, and a model rarely states that area for every opening. With `numerator_derivation` `light-area`, `area-ratio` takes each numerator member's area from the first of these steps that produces one:

1. the area-quantity property `numerator_property` states;
2. else the `light_area` of the most specific `light_area_table` row whose `width` and `height` equal the member's `overall_width` and `overall_height` (within `light_size_tolerance`, default exact) and whose `type` pattern matches the member's type name;
3. else overall width × height less the frame allowance 2·(W+H)·`frame_width`.

| Parameter | Kind | Meaning |
|---|---|---|
| `overall_width`, `overall_height` | `propertyReference` (required in this mode) | The member's overall size, a length each. With IFC, `axioval:attributes.OverallWidth` and `OverallHeight`. |
| `light_area_table` | `table` | Rows of `type` (`textPattern`, blank matches any type), `width`, `height` (length `quantity`) and `light_area` (area `quantity`), all but `type` required. |
| `light_type` | `propertyReference` | The type name rows match, required when a row has a `type`. |
| `light_type_path` | `stringList` | A relationship path to the objects `light_type` is read from, such as the type object; they must agree. |
| `light_size_tolerance` | `quantity` | A length within which a row's size matches; only with a table. |
| `frame_width` | `quantity` | The frame allowance's width, a length of at least zero. |
| `empty_numerator_finding` | `boolean` | Report an anchor that reaches no numerator object; any numerator mode. |

At least one of `light_area_table` and `frame_width` is required, and the mode's parameters without it are an invalid declaration. The step order is fixed, so a rule without a stated property or without a table simply leaves that step out.

A step is skipped only when its input is exactly absent: no stated property, or no row of the right size and type. A stated value that is not an area (null, text, a length), a size that is not a positive length, a type name that is absent, blank or not text when a row of the right size tests it, or rows that tie for most specific stop the chain, and the anchor is not evaluated rather than fall back. So is an anchor with a member for which no declared step produces an area, and one whose frame allowance leaves no light area. Every summed area carries an evidence entry `axioval:derived.light-area:<member>:step=<step>` naming its step (`stated`, `table;row=<n>` or `frame-allowance;frame_width=<metres>`), and a finding's message counts the areas each step produced.

Two further results:

- **Light area larger than the element**: a stated light area above the member's overall width × height is a finding against the member, relating the anchor, reported once however many anchors reach it; its anchor is not evaluated, since the value cannot be trusted. A stated area whose overall size is unknown is used, and the member is not evaluated for the comparison. Table rows are checked for this when the rule is read, and the frame allowance cannot exceed the element.
- **No opening**: with `empty_numerator_finding`, an anchor that reaches no numerator object at all (a space with no window) is the finding "no numerator object is reached …; the ratio is 0" instead of a ratio. Without it, such an anchor is judged as a ratio of 0.

The denominator is measured as `measure` says, but `measure: facade` together with `light-area` is an invalid declaration: a light area over facade areas is no defined ratio, and a window-to-wall ratio relates the windows' facade areas, not their light areas.

This is a numerator mode of `area-ratio` rather than a capability of its own: the traversal, the undecided-member rule, the denominator and the interval judgement are all `area-ratio`'s, and only where each numerator member's area comes from differs. A separate capability would have to repeat all of them, and capabilities cannot feed one another. A definition bound to `area-ratio` declares `numerator_selector` (required), `denominator_selector`, `minimum`, `maximum`, `numerator_property`, `denominator_property`, `measure`, `numerator_measure`, `denominator_measure`, `numerator_derivation`, `empty_numerator_finding`, the light-area parameters above and the traversal parameters.

### Property requirements

`property-requirements` checks each selected object against a `requirements` table: which properties it must, may or must not carry, and which values they may hold. One rule carries a whole requirement sheet.

| Column | Kind | Meaning |
|---|---|---|
| `applies_to` | `selector` | Objects the row applies to, such as `{"kind": "entityType", "objectType": "IfcWall", "includeSubtypes": false}` for one exact class or `true` for the class and its subtypes. Blank applies the row to every selected object. |
| `property_set` | `textPattern` | The property set's exact name, or a wildcard pattern (`Pset_*Common`). Blank asks for the property by name in any set. |
| `property` | `textPattern` | The property's exact name, or a wildcard pattern. Blank, with a set, asks whether the set holds any property. |
| `property_set_pattern` | `string` | The property set as an XML Schema pattern (`Pset_.*Common`), instead of `property_set`. |
| `property_pattern` | `string` | The property as an XML Schema pattern, instead of `property`. |
| `requirement` | `string` | `required`, `optional` or `forbidden`. A row declares `requirement` or `state`. |
| `state` | `string` | `include` (the row's statement must hold), `exclude` (it must not hold) or `ignore` (the row is skipped). On a `requirement` row only `include` (the row as written) and `ignore` are allowed. |
| `presence` | `string` | The statement of a `state` row about presence: `defined` (present, null included), `undefined` (exactly absent), `empty` (present but null, blank or an empty list) or `not-empty` (present with a value). It takes no value condition. |
| `value_like` | `textPattern` | A whole-value wildcard pattern the value must match. |
| `one_of` | `string` | Allowed values separated by `\|`; a backslash escapes the next character (`a\\|b` is one value). |
| `one_of_like` | `string` | Allowed whole-value wildcard patterns separated by `\|` (`Exist*\|New`); `\|` is a literal bar and `\*`, `\?` literal wildcards. |
| `contains` | `string` | Text the value contains: a substring of a text value, or an element of a list value (text, booleans and integers compared as text). |
| `minimum`, `maximum` | `number` | Numeric bounds, in `unit`; inclusive unless marked exclusive. |
| `minimum_exclusive`, `maximum_exclusive` | `boolean` | `true` makes the bound beside it exclude its own value, for either a numeric or a date bound: `minimum` 0 with `minimum_exclusive` is `> 0` and fails on 0. Only beside the bound it marks. |
| `minimum_date`, `maximum_date` | `date` | Date bounds for a date or date-time value, compared as in [Dates and date-times](#dates-and-date-times). |
| `minimum_date_time`, `maximum_date_time` | `dateTime` | Date-time bounds, instead of the date bound on the same side. |
| `precision` | `string` | `day`: a date-time compares with a date bound (and date-time bounds with a date) by the day it states. Only with a date bound. |
| `unit` | `string` | The bounds' unit, as for quantity parameters (`mm`, `m2`, `l`, `deg` …). Without it, the bounds compare with unit-free numbers only. |
| `per` | `string` | Divides the value before it is bounded: `measured-area` (the measured plan footprint through `PlanAreaService`, in m²), `measured-volume` (the certified volume of the closed body through `ProximityService::measure_body_volume`, in m³), `measured-face-area` (the body's largest plane face through `FacadeAreaService::measure_face_area`, in m²: a wall's side less its openings, a slab's top), `stated-area` (the area quantity `area_property` names) or `stated-volume` (`volume_property`, in m³). |
| `decimals` | `integer` | Rounds the value (or quotient), read in `unit`, half away from zero to 0–15 decimals before it is bounded, as the rule-level `decimals` of the comparison capabilities does: `299.6 mm` rounded to 0 decimals meets `minimum` 300. |

The rule-level `case_sensitive` (default `true`) applies to `value_like` and `one_of`. Every row that applies to an object is checked, through the shared row matcher with all rows selected: an `applies_to` selector that cannot be decided leaves the object not evaluated. Each failing row is one finding against the object, and its message begins with the result:

- `missing property set`: a `required` property is exactly absent and its named set holds no property at all, or a `required` set-only row's set holds none.
- `missing property`: a `required` property is exactly absent.
- `forbidden property set present`: a `forbidden` set-only row's set holds a property.
- `missing value`: a `required` property is present but null, blank or an empty list.
- `forbidden property present`: a `forbidden` row without a value condition, and the property is present with any value, null included.
- `forbidden value`: a `forbidden` row with a value condition, and the value meets it.
- `wrong value`: a `required` or `optional` value that does not meet the row's conditions.

An `optional` property may be absent, null or blank. Conditions must all hold. `value_like`, `one_of` and `one_of_like` compare text, booleans (`true`/`false`) and integers as text; any other value is not evaluated. A range compares integers and decimals when the row has no `unit`, and quantities of the unit's dimension in canonical SI units when it has one; any other pairing is not evaluated, never a pass. A list value must meet the conditions with every element, and a forbidden value is present when any element meets them; `contains` alone reads the list as a whole. A bounded value and a table are judged the same way by their stated values, and a bounded value open on a side a required range limits is a `wrong value`: its values run past the bound. A divided value is an interval when the measured footprint, volume or face is: a quotient straddling a bound, a divisor that is not positive, a body the service cannot measure (an open surface has no volume, a tessellated body no certified plane face), or a missing stated area or volume is not evaluated. A date range compares dates and date-times only; a number, text or a date-time against a date bound without `precision` `day` is not evaluated. A row bounds either numbers or dates, never both.

#### Filtered requirement templates

A filtered requirement template is a table of rows (class, property, operator, value, state) under one element filter. The filter is the rule's selector, the class is `applies_to`, and each row is a `state` row whose statement is a `presence` or value conditions:

| `state` | The statement … | Absent | Null or blank | A value |
|---|---|---|---|---|
| `include` with `presence` | must hold | `missing property` unless `undefined` | `missing value` for `not-empty`, `forbidden property present` for `undefined` | `forbidden property present` for `undefined`, `wrong value` for `empty` |
| `exclude` with `presence` | must not hold | `missing property` for `undefined` | `forbidden property present` for `defined`, `missing value` for `empty` | `forbidden property present` for `defined`, `forbidden value` for `not-empty` |
| `include` with conditions | must hold | `missing property` | `missing value` | `wrong value` unless every element meets the conditions |
| `exclude` with conditions | must not hold | passes | passes | `forbidden value` if an element meets the conditions |
| `ignore` | is skipped | — | — | — |

A `requirement` row is the same thing spelled differently: `required` is `include` (with `not-empty` when it has no condition), `forbidden` is `exclude` (with `defined` when it has no condition), and `optional` is `include` that lets a missing value pass. An ignored row is neither checked nor refused, even when its name is a pattern, and its `applies_to` is not evaluated. A `state` row with neither `presence` nor a value condition, an unknown `state` or `presence`, `presence` together with a value condition or with `requirement`, `exclude` on a `requirement` row, an empty `contains` or `one_of_like` value, `decimals` without bounds or outside 0–15, and a row with neither `requirement` nor `state` are invalid declarations.

#### Grouping and categories

With `group_by_value: true`, the findings of one row that share their result and the value found are one finding, such as ``wrong value: Pset_WallCommon.FireRating is `F30` on 3 objects; required one of `F60`, `F90` (requirement row 1)``. Its scope is the first of those objects in selection order and it relates the others; all objects missing the property form one group. Values group exactly, whatever `case_sensitive` says, so `F30` and `f30` are two findings. A grouped finding does not state per-object quotients of a divided range. Groups are reported after every object is checked, ordered by row, category, result and value.

With `category_property`, every finding of an object starts with that object's value of the property in brackets, such as `[Architecture] `, and cites it, as for `property-comparison`; grouped findings are grouped per category. An absent, null or blank value adds no category. The property is read only for objects with findings; when it cannot be read, the object is not evaluated rather than reported uncategorised.

A definition bound to `property-requirements` declares `requirements` with all twenty-four columns above (only the column set, kinds and optionality are compared) and the optional `case_sensitive`, `area_property`, `volume_property`, `group_by_value` and `category_property`.

Exact names are resolved one property at a time. A row whose set or property is a pattern (a wildcard, or an XML Schema pattern in the `_pattern` columns), or that names a set without a property, is checked against every matching property, enumerated exactly through the property service (see [Typed host services](./services.md)); a pattern matches the whole name the source states and names no concept, while an exact name next to it binds as usual. An included statement must hold for every match and needs one to match (a row naming its set as well needs one in every set it names, including a set the object carries without any member, as IDS requires); an excluded one must hold for none, so a forbidden pattern is found as soon as one matching property is present (or, with `not-empty`, holds a value). The first failing property, by set and name, is reported under its own name, and a row with no match is reported under its name as written. A set is present when it holds a property; a property named in an empty set is a `missing property`, not a missing set; a set-only row states only presence (`required`, `optional`, `forbidden`, or a `state` row with `defined` or `undefined`). A source that cannot enumerate leaves such rows not evaluated per object, and reports an absent exactly named property as a `missing property` without looking at its set. A backslash-escaped `*` or `?` is part of an exact name; a cell in both a name column and its `_pattern` column is an invalid declaration. An unknown requirement, bounds in the wrong order, a `unit` or `per` without bounds, `stated-area` without `area_property`, an empty `one_of` value, or a value condition on a set-only row is an invalid declaration.

### Storey metrics

Storey checks are compositions of the capabilities above; no capability is specific to storeys. With IFC, storeys hang off the building through `IfcRelAggregates`, spaces off each storey through `IfcRelAggregates`, and elements through `IfcRelContainedInSpatialStructure`.

| Metric | Rule |
|---|---|
| Storey height | `level-spacing` from each building over its storeys, `order` the `Elevation` attribute, bounded by `minimum` / `maximum` or `consistent`. |
| Height of the highest storey | The same rule with `content_path: ["IfcRelContainedInSpatialStructure"]` (and optionally `content_selector`, e.g. walls): the highest top of the storey's contents, through `VerticalExtentService`, less its elevation. The height is an interval when a content is tessellated. A highest storey with no contents, an undecided content or an unmeasurable one is not evaluated. |
| Space height matches its storey | The same rule with `space_selector` (spaces), `space_path: ["IfcRelAggregates"]` and `space_tolerance`: each space's rise from bottom to top must lie within the tolerance of its storey's height, highest storey included when it is measured. A space is not evaluated when its extent is unknown or the difference straddles the tolerance. |
| Spaces of a storey share their floor or ceiling elevation | The same space parameters with `space_elevation: bottom` (or `top`, or `both`), and `space_height: false` to leave the height comparison out: each space's bottom elevation is compared with the prevailing one among the storey's spaces (the one most share, the lowest among equally common ones; only exact elevations decide it). A floor 0.3 m below the others is found under a 0.05 m tolerance; one 0.02 m off is not; one straddling the tolerance is not evaluated. |
| Facade area per storey | `plan-area` from each storey with `measure: facade`, `member_selector` the external walls (`IsExternal` true in `Pset_WallCommon`) and `relationship: IfcRelContainedInSpatialStructure`. |
| Window-to-wall ratio per storey | `area-ratio` from each storey with `measure: facade`, `numerator_selector` the windows and `denominator_selector` an `anyOf` of the external walls and the windows: walls are measured with their openings cut out, so gross wall area is the walls' facade plus the windows'. Select the windows on external walls with a `related` selector through `IfcRelFillsElement` then `IfcRelVoidsElement` backwards when internal windows exist. |
| Window-to-wall ratio per building | The same rule from each building, with `path: ["IfcRelAggregates:forward", "IfcRelContainedInSpatialStructure:forward"]`. |
| Net-to-gross ratio | `area-ratio` from each storey: spaces (their footprints, or `numerator_property` such as `Qto_SpaceBaseQuantities.NetFloorArea`) over the storey's own `denominator_property`, e.g. `Qto_BuildingStoreyBaseQuantities.GrossFloorArea`, through `IfcRelAggregates`. |
| Empty-area ratio | The share of each storey's gross area no space covers: `space-validation` with `check_unallocated_area` and `maximum_unallocated_share`. Void spaces the model states, selected by name, type or classification, are a different measure: the same `area-ratio` rule with them as the numerator. |
| External-wall ratio | `area-ratio` from each storey with `numerator_measure: facade` over the external walls and `denominator_measure: footprint` over the storey's slabs (or its own `denominator_property`), through `IfcRelContainedInSpatialStructure`. |
| Storeys with no elements or no external walls | `related-count` from each storey through `IfcRelContainedInSpatialStructure`, `minimum: 1`, with `related_selector` everything or the external walls. |
| Compartment area against its gross-area group | Where the model assigns the compartments to the group (`IfcRelAssignsToGroup`), `area-ratio` from the group over its compartments with `denominator_selector` omitted, so the denominator is the group's own footprint (the union of its members), bounded around 1, e.g. `minimum: 0.95` and `maximum: 1.05` for 5 %. Whether a compartment lies within a group at all is `plan-coverage`. |

Storey height is not a property a space can be compared with through `property-comparison`: it is measured, not stated, and it is an interval when a content is tessellated. The space-against-storey comparison therefore stays with `level-spacing`, which measures both and judges the difference as an interval; a stated storey height property, where a model has one, is compared with `property-comparison` like any other quantity.

#### Report tables

Every metric reports what it measured beside its findings, passing or not, as tables in the report (see [Report tables](./ir.md#tables)). Rows are keyed by the object they are about, so tables of several rules join on the storey into one row per storey.

| Capability | Table | Columns |
|---|---|---|
| `level-spacing` | `levels` | `elevation` (m), `height` (m; unknown for a level not measured) |
| `level-spacing` with `space_selector` | `spaces` | `level` (its id), `height` (m), `level_height` (m) |
| `plan-area` | `areas` | `plan_area` or, with `measure: facade`, `facade_area` (m²); a subject with undecided members has no row |
| `area-ratio` | `ratios` | `numerator_area` (m²), `denominator_area` (m²), `ratio` (unknown when the denominator may be zero) |
| `quantity-takeoff` | `takeoff` | grouped by its keys: `count`, then `<aggregate>_<name>` per column and aggregate (each part of a `property_set` or `profile` column), every column stating its exactness (see [Information takeoff](#information-takeoff)) |

Tables need no parameter: a definition's signature is unchanged. An anchor that could not be measured has no row, and its not-evaluated outcome says why.

### Table allocation

`table-allocation` assigns each selected object to exactly one row of its `rows` table and then checks every row's objects together: "two offices and one meeting room per storey", "an archive of 30 m² ± 1 m²". One rule per row cannot express this, because an object would count in every row it matches.

| Column | Kind | Meaning |
|---|---|---|
| `key_1` … `key_4` | `textPattern` | Pattern over the property the same-named rule parameter names. |
| `anchor` | `textPattern` | Pattern over the anchor's `anchor_key` value: the row applies only in the anchors it matches, such as `EG*` for the ground storeys. Blank applies the row in every anchor. |
| `label` | `string` | How findings name the row; otherwise by its number and patterns. |
| `count` | `integer` | Exactly this many objects are assigned to the row. |
| `area` | `number` | Their summed plan area, in square metres, or with `area_mode` `each` every object's own area. |
| `area_tolerance` | `number` | Allowed deviation from `area`, in square metres (0 by default). |
| `area_tolerance_ratio` | `number` | Allowed deviation as a share of `area` (`0.1` is ± 10 %), instead of `area_tolerance`. |

All columns are optional. The keyed properties are rule parameters, not cells: `key_1` might be `{"type": "propertyReference", "propertySet": "Pset_SpaceCommon", "property": "Reference"}` and `key_2` the name, and each row fills the pattern columns it tests. A row matches an object when every key cell it fills matches the object's value, compared case-sensitively unless `case_sensitive` is false; a row with no key cell matches everything, which makes a catch-all last row. A key value that is absent or null matches no pattern, not even `*`; one that is not text, or cannot be read, leaves the rows testing it undecided. A key cell whose parameter is not declared, a negative count, area or tolerance, a tolerance without an area or both tolerances in one row, an `anchor` cell without `anchor_key`, `anchor_key` without `anchor_selector` or without a row filling `anchor`, and an `area_mode` other than `sum` or `each` are invalid declarations.

With `anchor_key` (a property of the anchors, such as a storey's name), one rule states different rows per anchor: "two offices in `EG`, one in `OG`". In each anchor only the rows whose `anchor` cell matches its value (or is blank) apply, objects are assigned among them, and the anchor pattern's literal characters add to a row's specificity. An anchor whose key is not text or cannot be read is not evaluated; an object is reported extra or undecided once, however many anchors reach it.

With `area_mode` `each` (the default is `sum`), a row's area is a match condition beside its keys, as individual-area space programmes use it: an object fits the row only when its own area lies within `area` ± the tolerance, so a row of 12 m² ± 10 % takes an 11 m² space but not a 10 m² one, which matches another row or is an extra. An object whose area straddles the bounds, or cannot be measured, leaves the rows it might fit undecided; the summed-area check does not apply.

`mode` is `first` (the default), the first matching row in declared order, or `most_specific`, the matching row with the most literal pattern characters, the specificities of a row's keys adding up. Matching goes through the shared row matcher above, so it fails closed: an object whose row an undecided key could change, or whose most specific rows tie, is not evaluated.

Rows are judged per group. With `anchor_selector`, each anchor is a group of the objects it reaches through the traversal parameters (or all of its source's objects without them), such as every storey through `IfcRelAggregates`; its outcomes go against the anchor, and an object that no anchor reaches is not evaluated. Without anchors, each source of the session is a group, even one that holds no objects, or the whole project with `across_sources`, and outcomes go against the source or the project; a traversal without `anchor_selector` is an invalid declaration.

- An object no row matches is a finding of its own, the "extra", naming its key values.
- A row that matched no object in a group is a finding, unless its `count` is 0.
- A row's `count` must equal the number of objects assigned to it.
- The summed area must lie within `area` ± `area_tolerance`. Areas are measured footprints through `PlanAreaService`, as in `plan-area`, or the area quantity `area_property` names. Measured areas are intervals, and a sum straddling a bound is not evaluated; an object with an empty footprint has no body and leaves its row's area not evaluated.

Objects that may belong to a row without being decided (a tie, an undecided key, or an undecided selection) make its count and area not evaluated, unless the row already exceeds its count or area, since they could only add to it. A finding relates the objects assigned to the row.

### Group composition

`group-composition` requires each selected group to hold a multiset of members: "two bedrooms, one kitchen and one bathroom per apartment". `related-count` checks one entry at a time and counts a member for every entry it fits; this capability allocates members across entries.

The rule's selection are the groups. Each reaches its members through the traversal parameters, which are required: `relationship` (such as `IfcRelAssignsToGroup`, the derived `axioval:derived.overlapping-group-space` walked `backward`, from a group space to the spaces it covers, or a ruleset's derived groups, `axioval:derived.group;by=<grouping>` walked `backward` from groups a `derivedGroup` selector selects) or a `path`. `member_selector` restricts which reached objects are members, every one by default. The `requirements` table lists the member entries:

| Column | Kind | Meaning |
|---|---|---|
| `key_1`, `key_2`, `key_3` | `textPattern` | Pattern over the member property the same-named rule parameter names, as in `table-allocation`. |
| `group`, `group_2`, `group_3` | `textPattern` | Patterns over the group's `group_key_1` (or `group_key`), `group_key_2` and `group_key_3` properties, such as an apartment's type, name and number; the row applies only to groups matching every one it fills. |
| `label` | `string` | How findings name the entry; otherwise by its number and patterns. |
| `count` | `integer` | Required: how many members the entry takes. |

A member fits an entry when every key cell the entry fills matches its value (absent or null matches no pattern); a row with no key cell fits every member. A member may fit several entries but fills one place, so members are allocated to places by a **maximum bipartite matching**: as many places as possible are filled, whatever order members and rows come in. A bedroom that fits both `*room` and `Bed*` goes to `Bed*` when a living room can take only `*room`, where filling entries in declared order would strand the living room.

Maximum allocations need not be unique, so outcomes are reported only as far as all of them agree:

- An entry that every maximum allocation leaves short is a finding naming how many of its members it has and how many are missing. Entries that compete for the same members, when any of them could be the short one, are reported together with their joint shortfall ("row 1 `bedroom` and row 2 `sleeping room` together have 1 of 2 required member(s); 1 missing"), never blamed one by one.
- Members beyond the places they fit are a surplus, reported per set of full entries they compete for ("row 1 `bedroom` takes 2 member(s), but 3 fit; 1 surplus"). A member that fits no entry is a surplus finding of its own, naming its key values.

Findings go against the group and relate the members concerned. With group keys, one table carries every kind of group: a row filling group cells applies only to groups whose values of those keys it matches, and a row without any applies to every group. A group that no row with a group cell matches is a finding ("no requirement row matches the group"), and its members are not judged. `group_key` is the older name of `group_key_1`; declaring both, a group cell without its key, a group key with no row filling its cell, a key cell without its property, a missing or negative `count`, an empty table and a missing traversal are invalid declarations.

Nothing undecided is guessed: a reached object whose membership in `member_selector` is undecided, a member whose key cannot be read or is not text when a row tests it, or a group whose `group_key` cannot be read leaves the group not evaluated, and the member is reported once as not evaluated. A group whose members cannot be walked is not evaluated.

With `report_absent_groups: true`, each requirement row that no selected group matches is a finding against the project, "not in model: no group matches row 2 (Pset.Type like `B`)": an apartment type, a storey or any other required group the model lacks. A group whose selection or key cannot be read might be the missing one, so a row it could match is not evaluated instead. Without the switch, an unmatched row stays unused.

With `ungrouped_selector`, each object it picks that no selected group reaches is a finding, "in no group". It is not evaluated when a group that could not be walked, or whose own selection is undecided, might reach it, or when its selection by `ungrouped_selector` is undecided. Spaces that must belong to a group are thus checked in the same rule as the groups, over the same relationship; a separate `related-count` rule walking the relationship backwards from each space checks the same where one rule per concern is preferred.

### Keyed limits

`keyed-limit` looks a limit up in a table keyed by facts about each selected object, then checks one of its quantities against it. A fire compartment's area limit depends on the building's fire class, the compartment's use class and whether its storey is sprinklered; one rule carries the whole table.

Up to four keys, `key_1` (required) to `key_4`, are property references. A key is read from the object itself, or, with `key_<n>_path` (steps as in the `path` parameter), from the objects that path reaches from it: `IfcRelContainedInSpatialStructure:backward` then `IfcRelAggregates:backward` climbs from an element to its building. A key's value is matched as text: a string as stated, a boolean as `true` or `false`, an integer in decimal. It is unknown when it is absent, null, blank or of another type, when the path reaches no object, or when the reached objects state different values; a refused property or relationship answer leaves the object not evaluated outright.

The `limits` table has the optional columns `key_1` … `key_4` (text patterns, as in [Table parameters](#table-parameters); a blank cell accepts any value) and `minimum` and `maximum` (numbers). A row may key only the keys the rule declares. The single most specific matching row applies, through the shared row matcher:

- No row matches: a finding that no limit is defined for the object's keys.
- A row that tests an unknown key could apply: not evaluated. A row another key already rules out does not matter, so an unknown key no row can reach is harmless.
- Rows tie for most specific: not evaluated as an invalid declaration, naming the rows; never broken by order.
- The row has neither bound: no limit applies, and the object passes.

`quantity` names what the row limits: `plan-area`, `member-plan-area`, `property`, `sill-height`, `clear-width`, `clear-height`, `threshold-step`, `glazing-ratio` (the last five below) or `measured`. `measured` limits the registered measured value `measured_value` names, read from `axioval:measured` as an expression reads it (`measured_value: "door_clear_width;stated=Pset/ClearWidth;overall=Attributes/OverallWidth;deduction=0.1"`); the name is checked against the registry when the rule compiles. The door values (`door_clear_width`, `door_clear_height`, `sill_height`, `threshold_step`) are measured by the same steps as the built-in quantities, so a rule reading them judges alike, a length within a few units in the last place of a bound meeting it as the built-in quantities do. The door-type defaults table remains the built-in quantities'; `threshold_step` takes ramps by source kind (`ramps`, `ramp_reach`). `plan-area` is the object's measured footprint in square metres through `PlanAreaService` (a zone's is the union of its members, and so is a derived group's, such as a derived fire compartment's; see [Compartments](./derived.md#compartments)); like `plan-area`, a footprint straddling a bound, or an empty one, is not evaluated. `member-plan-area` sums the footprints of the members `member_selector` picks among the objects the object reaches through the traversal parameters (`relationship`, `path`, …; everywhere in its source without them), exactly as `plan-area` with `member_selector` does: a table of storey patterns and summed space areas (`EG*` → 300–400 m², `OG*` → 250–350 m²) judges every storey in one rule, and a storey no row matches is "no limit defined" instead of passing unchecked. A reached object whose membership is undecided can only add area, so only a sum already above the maximum stands with one; a finding relates the summed members. `member_selector` is required with `member-plan-area` and, like the traversal parameters, refused with any other quantity. `property` is the number or quantity `quantity_property` states, compared in canonical SI units; an absent or non-numeric value is not evaluated. Bounds are inclusive. Key patterns are case-sensitive unless `case_sensitive` is `false`. A finding names the row (zero-based) and the key values, relates the objects the keys were read from, and cites the key, relationship and measurement evidence.

`sill-height` limits a window's sill per space type: the object's bottom elevation above the floor of each object `floor_path` reaches from it, in metres, through `VerticalExtentService`. A floor is the reached object's own bottom, so `floor_path: ["axioval:derived.adjacent-space"]` measures a window against the spaces on each side of it, and `key_1_path` along the same path keys the table on those spaces' use. `floor_path` is required with `sill-height` and `threshold-step` and refused with any other quantity, as `quantity_property` is outside `property`, `clear-width`, `clear-height` and `glazing-ratio`; every quantity-specific parameter below is refused with a quantity it does not apply to.

- Each reached floor is judged on its own against the one row the keys select. A window between two spaces whose floors lie at different elevations is too high when it is too high above either: one failing floor is a finding naming that floor, relating its space and citing the window's and that floor's extents.
- Elevations are intervals, and a tessellated body's are never points. The sill height runs from the window's lowest possible bottom less the floor's highest possible bottom to the reverse, widened by one rounding step wherever the subtraction rounded, so it always holds the exact difference. A sill height straddling a bound is not evaluated.
- A floor that cannot be measured, or straddles a bound, leaves the window not evaluated unless another floor already fails it. A window that cannot be measured, or whose `floor_path` reaches nothing, is not evaluated.
- The keys must agree across the reached spaces, as for any key: a window between an office and a corridor, whose rows differ, is not evaluated rather than judged against either, unless the key is the pair key below.

**Pairs of space types.** Door and window tables are commonly keyed on the *pair* of space types an element connects, in either order. `pair_key` (`key_1` … `key_4`) names the key read that way: its `key_<n>_path` must be `axioval:derived.adjacent-space` alone, forward, since only the derived adjacency records which face each space lies on. The key is then read separately on each face of the object, giving an unordered pair: the spaces on one face must agree, as for any key, and a face whose probe entered no space (`:outside` in the adjacency evidence) is the reserved key `exterior`, so a door from a corridor to the outside is the pair `corridor` and `exterior`, and one reaching no space at all `exterior` and `exterior`. A row names one side in its `key_<n>` cell and the other in the `other_side` column; either may be blank or `*`, a wildcard, and the row applies whichever side is which.

- Rows are ranked first by how many sides they name (a pattern holding a literal character), then by the other keys' specificity: between two offices the row `office` / `office` wins over `office` / `*`.
- Equally specific rows that set the same bounds apply as one (the first is named); rows that disagree leave the object not evaluated as an invalid declaration, naming them. `office` / `*` and `corridor` / `*` are equally specific for a door between an office and a corridor; the table needs an `office` / `corridor` row.
- A side whose key is unknown (absent, disagreeing spaces, a face the evidence records both as a space and as the outside, or neither, a space on both faces or none) leaves the object not evaluated when a row that could apply tests it; the other side's known key may already rule every row out. A space stating `exterior` itself (in any case) is unknown, never taken for the outside.
- No matching row is the usual "no limit defined" finding, so a door to the outside with no `exterior` row is reported, not passed.

A finding names the pair as the faces record it (``Pset.Use (via axioval:derived.adjacent-space) `corridor` and `office` ``). Every quantity reads the row the pair selects, so a window between an office and a corridor is judged by its `office` / `corridor` sill-height row on each floor:

```json
{"limits": {"type": "table", "value": [
   {"key_1": {"type": "string", "value": "office"},
    "other_side": {"type": "string", "value": "corridor"},
    "minimum": {"type": "number", "value": 0.9}},
   {"key_1": {"type": "string", "value": "office"},
    "minimum": {"type": "number", "value": 1.0}},
   {"key_1": {"type": "string", "value": "corridor"},
    "other_side": {"type": "string", "value": "exterior"},
    "minimum": {"type": "number", "value": 1.2}}]},
 "quantity": {"type": "string", "value": "clear-width"},
 "quantity_property": {"type": "propertyReference", "propertySet": "…door-accessibility",
                       "property": "…clear-width"},
 "key_1": {"type": "propertyReference", "propertySet": "…space-use", "property": "…use"},
 "key_1_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
 "pair_key": {"type": "string", "value": "key_1"}}
```

Without `pair_key` nothing changes, and an `other_side` cell is refused.

Whether a window sits at the end of a corridor is not decided: it needs the corridor's axis (a medial axis of its footprint), which no service provides yet.

`clear-width` limits a door's clear width in metres, taken from the first of three steps that produces one:

1. the length `quantity_property` states (a clear width the project records on the door or its type);
2. with `clear_width_from_leaves` `passage`, the door's overall width less its lining thickness at both jambs and the thickness of every hinged leaf, as the object-frame service states the door's leaves (with IFC, `OverallWidth`, `IfcDoorLiningProperties.LiningThickness` and each `IfcDoorPanelProperties.PanelDepth`): a leaf standing open at right angles stands its thickness into the opening; with `widest-leaf`, the widest hinged leaf's own passage instead: its width less the lining at each jamb its closed edge meets (an outer leaf meets one, a single leaf both) and its own thickness, for requirements that judge the widest leaf rather than the whole opening;
3. else the length `overall_width` states (with IFC, `axioval:attributes.OverallWidth`) less the door type's `width_deduction` from `door_type_defaults` (see [Defaults per door type](#defaults-per-door-type)), else less `width_deduction`, a non-negative length the rule states for frame, lining and leaf.

| Parameter | Kind | Meaning |
|---|---|---|
| `quantity_property` | `propertyReference` | The stated clear width, a length. |
| `overall_width` | `propertyReference` | The overall width the deduction is taken from, a length. |
| `width_deduction` | `quantity` | The rule author's deduction, a length of at least zero; declared with `overall_width`, which needs it or `door_type_defaults`. |
| `clear_width_from_leaves` | `string` | `passage` or `widest-leaf`: derive the clear width from the door's lining and leaves before the deduction. |

At least one step is declared; `overall_width` and `width_deduction` are refused with any other quantity, and `floor_path` with this one. As in the [light-opening fallback](#light-opening-area), only an exact absence moves to the next step: a stated value that is null, not a length or not positive leaves the door not evaluated rather than replaced by the approximation, and so does an absent overall width or a deduction that leaves no width. The deduction is a **declared approximation of the rule author**, not a measurement: the finding says so ("clear width (… 0.9 m less the rule's deduction 0.1 m, an approximation) is 0.8 m; required at least 0.9 m …") and cites the evidence entry `axioval:derived.clear-width:<door>:step=overall-width-less-deduction;deduction=<metres>`, marked inexact; a stated width cites `…:step=stated`, exact. A deduction that differs per door type is a `door_type_defaults` row, worded and cited as the type's default. Widths are read as the decimals they display, so an end within a few units in the last binary place of a bound meets it: a 1 m door less 0.1 m meets a 0.9 m minimum. The leaves step moves on when the source states no leaves (`NotStated`), no lining thickness or a leaf without a thickness, or when a leaf slides, rolls or is fixed, which the derivation does not cover; leaves the source states but cannot place (`Refused`, `Unreadable`), a missing object-frame service and a derivation leaving no width are not evaluated. Its finding names the figures ("clear width (overall width 1 m less 2 × 0.05 m lining and 0.04 m of open leaf, as the door states them) is 0.86 m …") and cites `…:step=lining-and-leaves`, inexact: stops, rebates and hardware are not modelled. `widest-leaf` ignores fixed leaves (they open nothing, but one may stand between a leaf and a jamb), moves on for a sliding or rolling leaf, and cites `…:step=widest-leaf` ("clear width of the widest leaf (leaf 0.9 m less 0.05 m lining at one jamb and 0.04 m of open leaf, as the door states them) is 0.81 m …"): a 1.4 m double door with 0.9 m and 0.5 m leaves fails a 1 m widest-leaf minimum although its passage is 1.22 m.

`clear-height` limits a door's clear height in metres: the length `quantity_property` states, else the length `overall_height` states (with IFC, `axioval:attributes.OverallHeight`) less `lining_thickness` (the head lining) and `threshold_thickness`, lengths the door states (with IFC, `LiningThickness` and `ThresholdThickness` of `IfcDoorLiningProperties`). A thickness the rule does not declare is not deducted; one it declares and the door does not state (absent or null) is unknown, never zero: the clear height is then bounded only from above, between zero and the overall height less the stated thicknesses, so a door too low even without it is a finding and one that may pass is not evaluated. A stated value that is not a length leaves the door not evaluated, as does an absent overall height. The finding names the figures ("clear height (… OverallHeight 2.1 m less the lining 0.05 m less the threshold 0.02 m) is 2.03 m; required at least 2.05 m …") and cites `axioval:derived.clear-height:<door>:step=overall-height-less-lining-and-threshold`, inexact, since it takes the lining to run across the head; a stated clear height cites `…:step=stated`, exact.

`threshold-step` limits the step a door's threshold makes above the floor on each side, measured from geometry, in metres: `|door bottom + threshold − floor|` for each floor `floor_path` reaches, the door's bottom from `VerticalExtentService`. Without `threshold_thickness` the door's body is its threshold, so a sill 4 cm above the corridor's floor fails a 2 cm maximum with no property stated; with it, the threshold's stated thickness is added, and a door that does not state it has an unknown threshold of at least zero, so only a step already too high is decided. The floor is the reached space's bottom, unless `ramp_selector` and `ramp_reach` are declared: a selected ramp within `ramp_reach` of the door in plan (`ProximityService`, `horizontal`) that overlaps the space in plan (`plan_overlap`) is that side's floor instead, measured at its top, so a door at the top of a ramp steps onto the ramp, not onto the floor below it.

| Parameter | Kind | Meaning |
|---|---|---|
| `overall_height` | `propertyReference` | `clear-height`: the overall height, a length. |
| `lining_thickness`, `threshold_thickness` | `propertyReference` | Thicknesses the door states, non-negative lengths: deducted from the overall height (`clear-height`); the threshold also raises the door's bottom (`threshold-step`). |
| `ramp_selector` | `selector` | `threshold-step`: the ramps that may be a side's floor; declared with `ramp_reach`. |
| `ramp_reach` | `quantity` | How far in plan from the door a ramp may lie and still be its floor, such as 0.4 m. |

A threshold step is judged per side, like a sill height: one side failing is a finding naming its floor (`the step from the floor of … to the door's bottom is 0.04 m; required at most 0.02 m …`), relating the space or ramp and citing the door's and the floor's extents. A ramp that only may be near, may overlap, or whose selection is undecided leaves both it and the space's own floor possible: the side fails only when every possible floor does, and passes only when every one passes. A floor or ramp top that cannot be measured leaves its side undecided, and a door that cannot be measured, or whose `floor_path` reaches nothing, is not evaluated. Without the proximity service every ramp is possible. A definition bound to `keyed-limit` must declare `clear_width_from_leaves` (a `string`), `overall_height`, `lining_thickness`, `threshold_thickness`, `ramp_selector`, `ramp_reach`, `pair_key` (a `string`) and `door_type_defaults` (a `table` with the columns below) as optional, and the optional `other_side` column (a `textPattern`) of `limits`.

#### Defaults per door type

Clear door dimensions are often derived from a table per door type where the model states nothing. `door_type_defaults` is that table, a rule parameter for `clear-width`, `clear-height`, `threshold-step` and `glazing-ratio` (refused with any other quantity). Its values are the rule author's, never measurements: each is used only where the door states no value, and every use is named in the outcome and cited by an inexact evidence entry `axioval:default.door-type:<door>:row=<index>;<column>=<value>`.

| Column | Kind | Meaning |
|---|---|---|
| `operation` | `textPattern` | The operation type the door's leaves state (`ObjectFrameService::leaves`; with IFC, `door_operation` from the `OperationType`), such as `DOUBLE_SWING_*`. Patterns follow `case_sensitive`. |
| `applies_to` | `selector` | The doors the row applies to, such as a property selector on `axioval:attributes.OperationType` or the type object's name. |
| `width_deduction` | `quantity` | `clear-width`: deducted from `overall_width`, in place of the rule's `width_deduction`. A non-negative length. |
| `height_deduction` | `quantity` | `clear-height`: deducted from `overall_height` in place of a head lining (`lining_thickness`) the door does not state. |
| `threshold_height` | `quantity` | `clear-height` and `threshold-step`: the threshold of a door that states no `threshold_thickness`. |
| `glazing_ratio` | `number` | `glazing-ratio`: the glazed share of the leaf, between 0 and 1. |

A door's type row is the first row whose `operation` pattern and `applies_to` selector both match (a blank cell matches every door). It is looked up only when a default is needed: a door that states its clear width, lining or threshold is never asked for its type. A row that may apply before the first that surely does leaves the type undecided, and the door is then not evaluated rather than given a later row's default: an `operation` pattern on a door whose leaves cannot be read (no object-frame service, an operation type, overall width or panels not stated) or an `applies_to` the selection cannot decide. No matching row, or a row without the column needed, is no default: the quantity then behaves as without the table (the rule's `width_deduction`, an unknown lining or threshold, an unknown glazing ratio). A defaults row only ever stands in for an absent or null value: a stated value that is not a length (or not a ratio) leaves the door not evaluated, as without the table.

- `clear-width`: the overall width less the type's deduction, after the stated width and the leaves; with no type deduction, the rule's. A double-swing door without a stated clear width is judged by its overall width less its type's deduction (`clear width (… OverallWidth 1.35 m less the door type's default width deduction 0.2 m (door_type_defaults row 0), an approximation) is 1.15 m; required at least 1.2 m …`), cited by `…:step=overall-width-less-type-deduction;deduction=0.2`. `overall_width` needs `width_deduction`, `door_type_defaults` or both.
- `clear-height`: a declared lining or threshold the door does not state takes the type's `height_deduction` or `threshold_height` (`… less the door type's default threshold 0.02 m (door_type_defaults row 0)`); without one it stays unknown.
- `threshold-step`: with `threshold_thickness` declared, a door stating none takes its type's `threshold_height` (`the step from the floor of … to the door's bottom with the door type's default threshold 0.03 m (door_type_defaults row 0) is 0.03 m; required at most 0.02 m …`). Without `threshold_thickness` the door's body remains its threshold.
- `glazing-ratio`: the fraction `quantity_property` states (with IFC, `Pset_DoorCommon.GlazingAreaFraction`, a plain number), else the type's `glazing_ratio`. A value outside 0 to 1, or neither, leaves the door not evaluated. IFC records no glazing per panel (`IfcDoorPanelProperties` has none), so a leaf's own glazing is not read.

```json
{"door_type_defaults": {"type": "table", "value": [
   {"operation": {"type": "string", "value": "DOUBLE_SWING_*"},
    "width_deduction": {"type": "quantity", "value": 0.2, "unit": "m"},
    "threshold_height": {"type": "quantity", "value": 0.02, "unit": "m"}},
   {"applies_to": {"type": "selector", "value": {"kind": "property",
      "propertySet": "axioval:attributes", "property": "…operation-type",
      "operator": "equals", "value": {"type": "string", "value": "SINGLE_SWING_LEFT"}}},
    "width_deduction": {"type": "quantity", "value": 0.1, "unit": "m"},
    "glazing_ratio": {"type": "number", "value": 0.0}}]}}
```

### Exit separation

`exit-separation` requires each selected space's exits to lie far enough apart for its size: at least `fraction` (one half by default) of the space's longest plan diagonal, or `flagged_fraction` (say one third) when a boolean `flag` is `true`.

| Parameter | Type | Meaning |
|---|---|---|
| `exit_path` | string list, required | the relationship path from the space to its exits, such as `["axioval:derived.adjacent-space:backward"]` |
| `exit_selector` | selector, required | which reached objects are exits (doors, or doors marked as exits) |
| `fraction` | number, optional | share of the diagonal the exits must lie apart; `0.5` by default |
| `flag` | property reference, optional | a boolean selecting `flagged_fraction`, such as a sprinkler flag |
| `flag_path` | string list, optional | where `flag` is read: the objects this path reaches from the space (its storey, its building); the space itself without it |
| `flag_sources` | table, optional | instead of `flag` and `flag_path`: the places the flag is read, in order (below) |
| `flag_default` | boolean, optional | the flag's value when no source states one |
| `flagged_fraction` | number, optional | the share that applies when the flag is `true`; required with `flag` or `flag_sources` |
| `separation` | string, optional | `closest` (the default), `centres` or `farthest` |
| `pairs` | string, optional | `any` (the default): some pair of exits lies far enough apart; `all`: every pair does |
| `minimum_exits` | integer, optional | fewer exits than this is a finding |

The diagonal is the largest distance between two points of the space's footprint, through `PlanSpanService`. `closest` measures the plan distance between the exits' footprints through `ProximityService` (the `horizontal` projection of `distance`); `centres` and `farthest` measure between the footprints' centroids or their farthest points through `PlanSpanService`. A space with fewer than two exits is not checked, unless `minimum_exits` makes the shortfall a finding.

Each row of `flag_sources` is one place the flag is read, commonly the space, then its storey, then its building:

| Column | Kind | Meaning |
|---|---|---|
| `property_set` | string, optional | the property's set |
| `property` | string, required | the flag's property |
| `path` | string, optional | where it is read: the objects this path reaches from the space, its steps separated by spaces (`IfcRelContainedInSpatialStructure:backward IfcRelAggregates:backward`); the space itself without it |

The first source that states a value decides; `flag` with `flag_path` is one such source. A source states nothing only when its path reaches no object, or when every object it reaches exactly lacks the property; then the next source is read, and after the last `flag_default` applies. A source that cannot be read, states a null, blank or non-boolean value, disagrees between the objects it reaches, or states the flag on some of them and nothing on others leaves the flag unknown: a later source or the default is never read past it. With nothing stated and no `flag_default`, the flag is unknown.

A finding names the pair (with `any`, the pair farthest apart), its separation, the required distance, the fraction and diagonal it comes from and the flag's value; it relates the exits and the objects the flag was read from, and cites the relationship, measurement and flag evidence. Every length is an interval:

- A pair is far enough apart only when its whole interval lies at or above the whole required interval, too close only when it lies wholly below it; otherwise it is unknown, and so is a pair that cannot be measured.
- The flag must be a boolean and agree across the objects a source's path reaches. A null, non-boolean or disagreeing flag, or one stated nowhere without `flag_default`, is unknown: the required interval then spans both fractions, so a pair far enough apart for the larger or too close for the smaller is still decided. The message names the source that decided, or that the default applied.
- An object `exit_selector` cannot decide can only add exits and pairs: with `any` a pair far enough apart stands, with `all` a pair too close stands, and anything else they could change is not evaluated. The same holds for the count against `minimum_exits`.
- A diagonal that cannot be measured, or a missing service, leaves the space not evaluated.

### Escape routes

`escape-route` checks each selected space against the first row of its
`uses` table whose `spaces` selector picks it: how far it lies from an exit,
how many exits it has, and how wide its exits and passages are for its
occupants.

| Column of `uses` | Kind | Meaning |
|---|---|---|
| `spaces` | selector, required | the spaces of this use |
| `maximum_travel` | number | the longest walk in metres from the start to the nearest exit |
| `route_start` | string | `farthest-point` (default): every point of the space; `door`: each of its own doors |
| `exits` | integer | the number of exits the space needs |
| `area_per_occupant` | number | square metres per occupant, for the occupant load |
| `label` | string | named in findings |

| Column of `widths` | Kind | Meaning |
|---|---|---|
| `occupants` | integer, required | the row covers loads up to this many occupants |
| `width` | number, required | the least clear width of each exit, in metres |
| `total_width` | number | the least width of all exits together |
| `door_width` | number | the least clear width of each door on a route; required in every row by `route_door_selector` |
| `total_door_width` | number | the least width of a space's own doors together; needs `door_path` and `door_selector` |
| `passage_width` | number | the least clear width of each passage; required in every row by `passage_selector`, unless the passages only trace routes (`exit_count: routes`, `common_path_factor`) |

| Column of `sections` | Kind | Meaning |
|---|---|---|
| `objects` | selector, required | the section objects, a stair say |
| `factor` | number, required | how many times a metre walked on them counts, at least 1 |
| `shared_by` | integer | at least 2: only a section that many checked spaces reach along `section_path` multiplies |
| `label` | string | named in messages |

| Parameter | Kind | Meaning |
|---|---|---|
| `uses` | table, required | the rows above; each states at least one requirement |
| `widths` | table | required by `area_per_occupant`, and only with it |
| `sections` | table | needs a use stating `maximum_travel` |
| `section_path` | string list | from a space to the shared sections it uses; required by `shared_by`, and only with it |
| `exit_path`, `exit_selector` | string list, selector, required | the exits, reached from the space as `exit-separation` reaches them |
| `door_path`, `door_selector` | string list, selector | the space's own doors; required by `route_start: door` |
| `clear_width_property` | property reference | an exit's stated clear width, a length |
| `passage_selector` | selector | the passages (corridors); checks their widths |
| `passage_path` | string list | from a space to the passages it relies on; needs `passage_selector` |
| `walked_passages` | boolean | the passages are those the walks from each space's doors cross, not declared; needs `passage_selector`, `door_path`, `door_selector` and a walking profile, and excludes `passage_path` |
| `passage_width_property` | property reference | a passage's stated clear width, a length; needs `passage_selector` |
| `exit_door_direction` | boolean | every exit door must open in the direction of escape, out of the space |
| `no_escape_selector` | selector | objects not usable for escape (locked or staff-only doors): never an exit or a start, and every walk keeps out of them |
| `compartment_selector` | selector | the fire compartments; travel ends at the start compartment's boundary. Needs `door_path`, `door_selector` and one of the two below |
| `compartment_path` | string list | from a space to the compartments it lies in (a group membership, say) |
| `compartment_overlap` | number | a space lies in each compartment covering at least this share (above 0, at most 1) of its footprint, 0.8 say |
| `exit_count` | string | `exits` (default): `exits` counts exits; `routes`: it counts independent routes. `routes` needs `passage_selector` and a walking profile |
| `route_door_direction` | boolean | every single-swing door the walks from the space's doors cross must open along them |
| `minimum_clear_height` | number | the least clear height, in metres, of every door, opening and space the walks cross, and of the space itself |
| `clear_height_property`, `overall_height`, `lining_thickness`, `threshold_thickness` | property reference | a door's clear height as `keyed-limit`'s `clear-height` quantity reads it; need `minimum_clear_height` |
| `common_path_factor` | number | at least 1: a metre of the common path counts this many times; needs a use stating `maximum_travel` |
| `route_door_selector` | selector | the doors on routes whose loads are checked, as walked passages are; needs `door_path`, `door_selector`, a walking profile and `door_width` in every row |
| `zones` | table | ranks of zones (`objects`, a selector; `rank`, an integer; `label`); every walk keeps out of what ranks above the start |
| `walking_height`, `walking_step` | number | the headroom and the step walked over; required by `maximum_travel` |
| `stair_selector`, `ramp_selector`, `lift_selector` | selector | the vertical connectors walks may climb, by kind (see [Routes across levels](./metric-routing.md#routes-across-levels)); without any, walks stay on their level. One object in two of them is an invalid declaration, and an undecided selection leaves every walk not evaluated |
| `stair_length` | string | `slope` (default): a climb counts along its slope; `horizontal-plus-vertical`: its horizontal length plus its rise times `vertical_factor`; needs a connector selector |
| `vertical_factor` | number | what a metre of rise counts, at least 0, default 1; needs a connector selector |

- **Travel** follows the walking line of a point through the metric-routing
  service, from the farthest point of the space's walkable area
  (`farthest_point`) or from each of its doors (`nearest_target`), to the
  nearest exit. Exits and doors stand at their representative points, the
  centroid of the footprint inside it at the bottom of the vertical extent,
  as `space-distance` walks. The farthest distance is a certified bracket;
  part of the space that reaches no exit (proven with complete evidence) is
  a finding at a point of it. See [Metric routing](./metric-routing.md#many-targets).
  With a connector selector every walk may climb the selected stairs, ramps
  and lifts to exits on other storeys, a climb counting by `stair_length`
  and `vertical_factor` (see [Routes across levels](./metric-routing.md#routes-across-levels));
  a climb the service cannot measure leaves the travel not evaluated. A
  trace measures in plan and a climb is longer than its plan length, so a
  walk climbing a connector is never traced: sections bound it by the
  largest factor, and its common path is the whole walk.
- **Multiplied sections**: a metre walked on a `sections` object counts
  `factor` times; where sections overlap, the largest factor. The travel
  is the least multiplied length of any walk to an exit, bracketed: at
  least the plain walk's lower bound (every factor is at least one), at
  most the multiplied length of any one walk. From a door, the routing
  answer names a walk no longer than the plain upper bound `U`, and the
  metric-routing service traces how much of it lies over each section's
  footprint (`trace_path`); the bound is `U` plus, per section, the length
  over it times its factor less one, a section whose length is unknown
  counting the whole walk. Otherwise, and whenever it is smaller, the bound
  is `U` times the largest factor of a section the walk may cross: a walk
  of at most `U` metres stays within `U` of its start in plan, so a section
  whose horizontal distance (`ProximityService`) from the space, or from
  the door it starts at, surely exceeds `U` is not crossed; a section that
  is not measured, or undecided, may be. The farthest point's answer is a
  point, not a walk (a walk from one point bounds that point only), so from
  the farthest point only the second bound applies. Where the backend
  weighs travel (`weighs_travel`, see
  [Weighted travel](./metric-routing.md#weighted-travel-and-forced-walks)),
  the same query is asked weighted as well, from a door or from the
  farthest point: over every section that may multiply for the upper bound
  (which replaces the others where smaller), and over only the sections
  surely multiplying (surely picked, no `shared_by`) for the lower bound,
  which replaces the plain one where larger. A room whose weighted farthest
  point exceeds the maximum only because of a stair's factor is then a
  finding, naming the rows it counted. A walk that climbs is weighted too,
  across levels (each section weighing only its own level, and a climb
  counting at most the largest factor of a section meeting its connector);
  a backend that cannot weigh it (a tessellated section, crossing
  footprints, a section whose level it cannot tell) keeps the bounds
  above. A travel within the
  maximum only at the plain length is therefore not evaluated unless the
  traced or weighted walk passes, never a pass on a guess. A row with
  `shared_by` multiplies only a section at least that many checked spaces
  (including those the rule's selector cannot decide) reach along
  `section_path`; a space whose sections cannot be read may reach any.
- **Common path** (`common_path_factor`): the stretch one space's routes
  share before they part counts `common_path_factor` times, on top of any
  section's factor. From a door, the named walk to the nearest sure target
  is traced over the passages (`passage_selector`), and so is the walk to
  every other sure target; the common path is the named walk's length over
  the passages every other walk may cross too (a walk that cannot be
  traced may cross any). With no other route to a target, the whole walk
  is common, and from the farthest point, whose answer is no walk, it may
  be. This bounds the multiplied travel from above only: its lower bound
  stays the plain walk's, as with sections.
- **Doors not used for escape** (`no_escape_selector`): an exit the
  selector surely picks is no exit, a door it surely picks is no start, and
  every walk keeps out of whatever it picks (`nearest_target` with the
  objects avoided), so a locked door on the short way forces the longer
  walk. An object it may pick counts as possibly not usable: an exit or
  door only possible, and the walk bounded from below around the sure
  picks only, from above around every possible one. A backend that cannot
  walk around objects (`avoids_objects`) leaves such travel not evaluated.
  The farthest point is measured on the plain walk only; its answer bounds
  the walk around the avoided objects from below, and from above only when
  every avoided object lies surely farther from the space in plan than the
  plain walk's upper bound, beyond the reach of any such walk.
- **Compartments** (`compartment_selector`): travel is measured on the
  safe route only and ends where it leaves the start space's compartment.
  A space lies in the compartments `compartment_path` reaches from it, or
  in each whose plan overlap (`PlanAreaService::measure_plan_overlap`)
  covers at least `compartment_overlap` of its footprint; the overlap is an
  interval, and a share straddling the threshold leaves the membership
  undecided. The start must lie surely in exactly one compartment, else its
  travel is not evaluated. The doors out of it are found by walking its
  spaces through their doors (`door_path`, and the same path walked back
  from each door to the spaces it connects): a door reaching a space
  outside the compartment, or no other space (it leads outside), is a door
  out, and walks end at the nearest exit or door out, each a target
  standing in its portal like an exit door. So `door_selector` must pick
  every door and opening a walk may pass. Anything undecided on the way (a
  door's selection, a neighbour's membership) makes a door out only
  possible; a door or space whose relationships cannot be read may hide
  more doors out, and the travel then has no lower bound. A project in
  which `compartment_selector` picks nothing is an inadequate-information
  finding on each source holding a checked space, and every space's travel
  is not evaluated.
- **Zones** (`zones`): each object takes the rank of the first row
  picking it; the start takes its own space's rank, else its compartment's,
  and one no row ranks, or whose row is undecided, is not evaluated. Every
  walk keeps out of whatever surely ranks above the start (its upper bound
  also out of what may), as it keeps out of doors not used for escape; a
  door out of the compartment leading only into such a zone is no door out.
  Zone objects need a body to walk around; the farthest point measures them
  as it measures every avoided object, so zones are best walked from the
  doors (`route_start: door`).
- **Exits** are counted as `exit-separation`'s `minimum_exits` counts them.
- **Independent routes** (`exit_count: routes`): `exits` then counts
  routes to distinct targets (exits, and doors out of the compartment),
  two routes that share a passage (`passage_selector`, within the start's
  compartment where compartments are declared) counting once. Routes are
  walked from the space's representative point. At most, every target a
  walk may reach (a target proven unreachable is none), and a single route
  when the nearest walk crosses a passage around which no walk reaches any
  target: every route passes through it, as through a dead-end corridor
  with both exits at its end. At least, as many sure targets as have walks
  (around everything possibly avoided) that cross pairwise no common
  passage, as traced (`trace_path`); a walk that cannot be traced may
  share every passage. A finding needs fewer routes at most than required,
  a pass at least as many at least.
- **Widths**: the occupant load is the space's footprint (`PlanAreaService`)
  divided by `area_per_occupant`, rounded up. Each exit's stated clear width
  must reach the covering row's `width`, and together they must reach its
  `total_width`. Without a stated width, geometry decides only a failure:
  no clear width exceeds the longest plan diagonal of the exit's footprint.
- **Passages**: the passages of a checked space are the `passage_selector`
  objects `passage_path` reaches from it, and the space itself where
  `passage_selector` picks it. A passage carries the occupants of every
  checked space that reaches it, summed, and must be as wide as the
  `passage_width` of the rows covering that load. A stated
  `passage_width_property` decides both ways; without one, geometry
  decides only a failure: a body that passes stands on a disc of its width
  inside the footprint, so no clear width exceeds the shorter side of the
  rectangle of least area enclosing it (`measure_rectangle`; a tied or
  unproven orientation leaves the width unknown). The finding is the
  passage's and relates the spaces it serves. A space that may reach a
  passage but whose load is unknown (its use states no
  `area_per_occupant`, its use or selection is undecided, its footprint is
  not measured) leaves that passage not evaluated; so does a space whose
  passages cannot be read, for every passage.
- **Walked passages** (`walked_passages`): instead of declaring them, the
  passages of a checked space are derived from the walks out of each of its
  doors (`door_path`, `door_selector`) to its nearest exit. The walk the
  routing answer names is one shortest walk, not necessarily the only one,
  so a passage on it is not yet one the occupants rely on. With `U` the
  plain walk's upper bound to the sure exits, a passage is **surely**
  crossed from a door when the walk to every possible exit that keeps out of
  it (`nearest_target` with the passage avoided) is longer than `U`, or
  reaches no exit under complete evidence: then every shortest walk to
  whichever exits there are enters it, ties included. Only passages the
  named walk lies over (`trace_path`) are tried. A passage is **off** every
  shortest walk from a door when the plan distance from the door to it and
  on from it to the nearest possible exit (`ProximityService`) exceeds `U`,
  or when the shortest walk to any possible exit forced through it
  (`forced_walk`, keeping out of only what surely is avoided, every target
  placed) is longer than `U` or enters it nowhere; every other passage
  **may** be crossed. A space surely relies on a
  passage every door's walks surely cross; it may rely on one any door's
  walks may cross. A space measured from its farthest point that reaches no
  door walks its passages from its farthest point instead, as its travel
  does; a door that cannot be walked, any other space without doors, or one
  with unreadable exits may use any passage. A passage carries the loads
  of the spaces surely relying on it at least and of those that may at
  most, so ties and unknowns widen the load rather than guess it. A
  finding needs a width below what every load in that interval requires,
  and at least one space surely relying on the passage; a passage no walk
  surely crosses is never too narrow. The space itself is its own passage
  where `passage_selector` picks it, as when declared. Each detour needs a
  backend that walks around objects (`avoids_objects`); one that does not,
  or refuses, proves no passage sure.

- **Doors on routes** (`route_door_selector`): a second class of walked
  passages. The doors a space's walks rely on are derived as walked
  passages are (a door the walk starts from is relied on from it), and
  each carries the summed occupants of every space relying on it, surely
  at least and possibly at most, against the `door_width` of the rows
  for that load. Its width is its `clear_width_property`, else no more
  than its footprint's longest plan diagonal, as for exits. Separately,
  `total_door_width` asks a space's own doors (`door_path`) together to
  be that wide for its load, as `total_width` asks of its exits.

Every measure is an interval, and a verdict stands only when what is
unknown cannot change it. The travel is bounded from above through the exits
that surely are exits and from below through every one that might be; with
`door`, a finding needs one sure door too far, a pass every possible door
near enough. An exit without a representative point leaves the lower bound
at zero. A load between two rows of `widths` requires either row's widths.

With `exit_door_direction`, every exit door must open in the direction of
escape, out of the checked space. Its leaves come from the object-frame
service and the side the space lies on from the free-space service's
containment probes, as for [door swing](#door-swing): an exit door swinging
into the space is a finding (`exit door … opens into the space, against the
direction of escape`), one opening away from it or a double-acting one
passes. An exit that is no door (an opening) has no leaf and is skipped. A
sure exit door without a hinged leaf (a sliding door opens in no
direction), with leaves that cannot be read, or with neither side in the
space at its probes leaves the space not evaluated; an exit whose selection
is undecided matters only when it swings into the space. Exits are the ones
`exit_path` reaches, so the direction is judged from each checked space
towards its own exits; a door further along the route is judged from the
space it leaves.

**Along the route** (`route_door_direction`, `minimum_clear_height`;
both need `door_path`, `door_selector` and a walking profile): from each
door of the space the walk the routing answer names, to the nearest sure
target around everything possibly avoided, is traced over every
`door_selector` object and passage (`trace_path`). What it may cross is
judged, and so are the door it starts from and the space itself. The walk
is one shortest walk among perhaps several, so a failure is a finding
only when every shortest walk crosses the object too: the walk to every
possible target around it (and around only what is surely avoided) is
longer than the named walk, or reaches none. A failure on the named walk
alone leaves the space not evaluated.

- **Door direction**: a single-swing door must open along the walk. Where
  the walk crosses the line of a closed leaf (within a leaf's width of it)
  says which way it passes; crossing it towards the side the leaf opens to
  is along. A door crossed both ways or not at all is undecided. A
  double-acting, sliding or other door that swings in no one direction,
  and anything that is no door, is not judged; a door whose leaves the
  source does not state (`NotStated`, an unset operation type) opens in an
  undefined direction, a failure. The door a walk starts from is judged
  against the space with the containment probes, as exit doors are: one
  opening into the space opens against the escape.
- **Clear height**: a door's clear height is read as `keyed-limit`'s
  `clear-height` quantity reads it (the stated `clear_height_property`,
  else `overall_height` less the lining and threshold), an interval that
  decides where it lies wholly on one side; a space's measured clear height
  (`SpaceService`) decides both ways; otherwise the
  vertical extent bounds the clear height from above and decides only a
  failure. A door lower than `minimum_clear_height` is reported by name in
  a finding of its own, while the travel is still judged.

Not checked yet: the passages walked from the farthest point of a space
rather than from its doors, and the width of the route between passages.
Travel is measured for a point: a body's width is checked at the exits and
passages, not along the walk.
### Openings at corridor ends

`corridor-end-openings` finds windows (or any opening the rule selects) in the wall a selected corridor ends at. The rule selects the corridors, typically spaces of a corridor type.

| Parameter | Type | Meaning |
|---|---|---|
| `opening_path` | string list, required | the relationship path from the corridor to its openings, such as `["axioval:derived.adjacent-space:backward"]` |
| `opening_selector` | selector, required | which reached objects are checked (windows) |
| `wall_depth` | number, optional | how far, in metres, an opening's footprint may lie from the end wall's face and still sit in it; `0.5` by default |
| `facing` | number, optional | how much of the end wall, in metres, an opening must face to sit in it; more than `0.1` by default |

Through `PlanSpanService::measure_corridor_ends`, the corridor's footprint gives the ends of its paths and the straight wall each runs into, and every reached opening is measured against each such wall: its plan gap to the wall segment and the length of the segment it faces. An opening sits in an end wall when its gap is at most `wall_depth` and it faces more than `facing` of the wall, so a window in a side wall right beside the corner, which touches the end wall but faces none of it, does not. A straight corridor has two ends, an L-shaped one the far end of each leg, a corridor looping round a core none.

A finding is on the opening; it names the corridor and the wall, relates the corridor, and cites the path, the corridor ends (approximate evidence) and the gap and facing measurements.

- Gap and facing are intervals: an opening surely sits in an end wall only when the whole gap lies within `wall_depth` and the whole facing beyond `facing`, surely not when either lies wholly on the other side; otherwise it is not evaluated.
- An end whose wall the service could not decide (the path's direction too uncertain, a path stopping short of any wall, a room whose skeleton is only a stub) leaves every opening not found at another end not evaluated: it might sit in that wall.
- Ends come from an approximate skeleton of the footprint. A finding stands on the wall segment and the measured footprints; a pass relies on the skeleton having found every end, which is not certified.
- An opening whose selection is undecided is not evaluated where it would sit in an end wall, and ignored otherwise. A corridor reaching no opening passes without measurement; a missing service or a corridor whose ends cannot be measured (a tessellated footprint, whose edges are chords of a curved wall) is not evaluated.

### Distances and connections between spaces

Two capabilities judge how spaces relate to one another: `space-connection`
how they open onto each other and onto the outside, `space-distance` how far
each lies from its nearest destination. They are separate because they ask
different services: a connection is a relationship question and needs no
geometry, while a distance is measured.

Both read direct access the same way. Each door (`door_selector`) or opening
(`opening_selector`) reaches the spaces it connects through `access_path`,
among the `space_selector` objects (every object by default):

- `["axioval:derived.adjacent-space"]`, forward and alone: the spaces a probe
  first enters on each face of the element. Two spaces have direct access
  through it only on opposite faces, and a space opens to the outside
  through it when its other face enters no space. Faces are read with
  `adjacent_side`, never from the locator by hand.
- a relationship the model states, such as `["IfcRelSpaceBoundary:backward"]`
  from the element to the spaces it bounds: two spaces the element reaches
  have direct access through it. A stated relationship records no faces and
  no outside, so it cannot judge an exit.

Every answer is three-valued. A link through an element surely of the asked
type stands. An element whose type a selector cannot decide, or whose
spaces cannot be read (a refused derivation, which might connect anything),
leaves an answer it could change not evaluated, never "no".

#### Space connection

`space-connection` checks each selected space against the rows of its
`connections` table whose `from` selector picks it (every such row applies;
an undecided `from` leaves the space not evaluated).

| Column | Kind | Meaning |
|---|---|---|
| `from` | selector, required | the spaces the row applies to |
| `to` | selector | the spaces access is judged to |
| `access` | string | `allowed` (default), `required` (direct access to at least one `to` space) or `forbidden` (to none); needs `to` |
| `access_type` | string | `any` (default), `doors` or `openings`: which elements count, for access and exit alike |
| `exit` | string | `allowed` (default), `required` or `forbidden`: a direct exit to the outside; needs the derived adjacency |
| `label` | string | named in findings |

| Parameter | Kind | Meaning |
|---|---|---|
| `connections` | table, required | the rows above |
| `access_path` | string list, required | from a door or opening to the spaces it connects |
| `door_selector`, `opening_selector` | selector | the doors and the openings; at least one, and each `access_type` other than `any` needs its own |
| `space_selector` | selector | the spaces an element may reach |

A forbidden connection is found once per row, naming every linked space and
the element it is reached through, and cites the adjacency evidence; a
missing required connection or exit cites the evidence of every element that
reaches the space. A `to` selector that cannot decide a linked space, like an
undecided element, leaves the requirement not evaluated unless a sure link
already decides it.

#### Space distance

`space-distance` checks each selected space against the rows of its
`distances` table whose `from` selector picks it.

| Column | Kind | Meaning |
|---|---|---|
| `from`, `to` | selector, required | the start spaces and their destinations |
| `measure` | string | `straight` (default), `closest` or `walking` |
| `same_storey` | boolean | only destinations on the start's storey count |
| `direct_access` | boolean | only destinations the start has direct access to count |
| `minimum`, `maximum` | number | bounds in metres on the nearest destination's distance; at least one, minimum not above maximum |
| `label` | string | named in findings |

| Parameter | Kind | Meaning |
|---|---|---|
| `distances` | table, required | the rows above |
| `storey_path`, `storey_selector` | string list, selector | how a space's storeys are found, climbed as `same-container` climbs; required by `same_storey` |
| `access_path`, `door_selector`, `opening_selector`, `space_selector` | | as for `space-connection`; `access_path` is required by `direct_access` |
| `walking_radius`, `walking_height`, `walking_step` | number | the body a walking row routes, required by one; `walking_slope` defaults to level |
| `stair_selector`, `ramp_selector`, `lift_selector`, `stair_length`, `vertical_factor` | | the connectors a walking row climbs and how a climb counts, as for `escape-route`; so a walk reaches destinations on other storeys without `same_storey` |

- **Straight** is the plan distance between the footprints' centroids
  (`PlanSpanService`, `centres`), an interval exact for planar meshes. Plan
  rather than 3D, because storeys are what `same_storey` states.
- **Closest** is the shortest distance between the two spaces' bodies in
  space (`ProximityService::measure_distance`, the `minimum_3d` projection
  `distance` measures), an interval widened by any tessellation. Two long
  rooms side by side, their centroids 8 m apart, are as close as the wall
  between them is thick. A pair the service cannot measure, or whose bodies
  have no distance, is unknown.
- **Walking** is the metric route (`MetricRoutingService`) from the
  space's representative point to the nearest destination's: the centroid
  of each footprint (`PlanSpanService::measure_centre`) at the bottom of
  its vertical extent. The centroid must be exact and lie inside the
  footprint; an L- or U-shaped space whose centroid falls outside it, or on
  its boundary, is not evaluated rather than walked from a point chosen for
  it. It is one nearest-target query over all destinations at once
  (`nearest_target`); destinations proven unreachable (with complete
  evidence) are none, and a refused query is unknown. See
  [Metric routing](./metric-routing.md#many-targets).

The nearest distance is bounded from above by the destinations that surely
qualify (`to` matches, same storey, direct access) and from below by every
destination that might, a destination whose distance is unknown counting as
zero. A maximum fails only when every possible destination lies beyond it
(or there is none, or none is reachable), and holds once a sure destination
lies within it; a minimum fails once a sure destination lies nearer, and
holds only when every possible one lies at least that far. Anything else is
not evaluated with the reasons, so a route that cannot be measured decides
only what it cannot change. Walking, the upper bound is one query over the
sure destinations and the lower bound one over every possible destination;
a destination without a representative point leaves the lower bound at
zero.

### Counterpart coverage

`counterpart-coverage` checks that each selected element is covered by its counterparts, in plan and in height: architectural walls by structural walls, or the reverse. The counterparts are the objects `counterparts` picks, typically another discipline's elements of matching kinds (a [`discipline` selector](#discipline-selectors) with an entity type), so a check across two models declares `--model arch.ifc:architecture --model struct.ifc:structure`. It needs the plan-area, proximity and (for the height check) vertical-extent services.

| Parameter | Type | Meaning |
|---|---|---|
| `counterparts` | selector, required | the objects that should cover each element |
| `tolerance` | length, optional | coverage: one growth for both checks |
| `horizontal_tolerance`, `vertical_tolerance` | lengths, optional | conformity: separate growths, declared together instead of `tolerance` |
| `info_above`, `warning_above`, `error_above` | numbers, optional | uncovered shares in `[0, 1)` above which a finding has that severity; at least one, ascending in that order |
| `axis_tolerance` | plane angle, optional | only counterparts whose long axis lies within this angle of parallel to the element's count; in `[0°, 45°)` |
| `measure` | string, optional | `plan_and_height` (the default) or `elevation`: one check in the element's own elevation plane |
| `infill_counterparts` | selector, optional | `elevation` only: frame members (columns, beams) whose infill covers when enough is uncovered |
| `infill_above` | number, optional | the uncovered share above which the frame's infill covers; in `[0, 1)`, default 0.5 |

- **Plan**: the share of the element's footprint outside the union of the counterparts' footprints, each grown by the horizontal tolerance in every plan direction, through `PlanAreaService`'s uncovered area.
- **Height**: the share of the element's vertical extent outside the union of the vertical extents, each grown by the vertical tolerance below and above, of the counterparts that overlap it in plan (their grown footprint covers part of its footprint).

A negative tolerance switches its check off, so conformity with `vertical_tolerance: -1 m` checks plan only; switching every check off is an invalid declaration. A share above the lowest declared threshold is a finding of the most severe band it exceeds, and the rule's own severity is not used; a share at or below it passes. Each check reports on its own: an element with nothing under it has a plan finding and a height finding. A finding states the share, the uncovered area or height, and the tolerance; it relates the counterparts that surely cover part of the element, and adds "no counterpart overlaps it" when none may.

Shares are intervals: a tessellated body's footprint and extent are, and so is a footprint grown by a disc, which the plan-area service brackets between two polygons. A share straddling the lowest threshold is not evaluated; one above it that straddles a higher threshold is graded by the most severe band it may reach, and the message says so. A counterpart the selector cannot decide, whose extent cannot be read, or whose cover cannot be measured can only cover more, so a pass stands and anything else is not evaluated. Measured areas carry the plan overlay's rounding (around 10⁻⁸ of the extent), so a threshold of exactly 0 can flag a fully covered element; use a small positive one.

Without `axis_tolerance`, counterparts are not filtered by direction: a perpendicular wall meeting the element within the horizontal tolerance overlaps it in plan and counts towards its height. With it, only **axis-compatible** counterparts count: a long axis is the longer side of the least-area rectangle around a footprint (`PlanSpanService::measure_rectangle`, which then also needs the plan-span service), and a counterpart whose long axis lies surely further from parallel than the tolerance is left out. A counterpart whose angle straddles the tolerance, or whose axes or the element's are not their own (a square has no long axis, a footprint enclosed equally well at two orientations none, a tessellated one no proven one), may count: it is a possible cover only, so a finding it could remove is not evaluated and says why.

Plan and height checked apart miss a wall under a full-height counterpart along one half and a half-height one along the other: every part of the footprint is covered, and the full-height one spans the whole height, yet the upper quarter of the wall's face has nothing behind it. With `measure: elevation` the two are one check in the element's **elevation**: its projection onto the vertical plane along the long axis of its least-area rectangle (`PlanAreaService::measure_elevation_cover`, which also needs the plan-span service). A counterpart counts by its part within the element's depth across that axis, widened by the horizontal tolerance on both sides, and its projection grows by the horizontal tolerance along the axis and by the vertical tolerance in height; the finding states the share of the elevation left uncovered. Both tolerances must be switched on, and an element without a long axis (a square, several least-area orientations, a tessellated footprint) is not evaluated. A tessellated body, or an axis off the coordinate axes, makes the areas intervals.

A wall filling a bay of a column-and-beam frame stands against no counterpart body, only between them. `infill_counterparts` names the frame's members: when more than `infill_above` of the elevation (half by default) is left uncovered by the counterparts, the convex hull of the members' projections near the element, grown the same way, covers too, and the message names the frame. Whether the infill applies is decided on the share the counterparts leave; a share straddling `infill_above` is judged from both answers, so only what holds either way stands.

Findings report how far the share exceeds the lowest declared threshold, so a rule's severity bands may grade them.

### Parking bays

`axioval:capability.parking-bay` checks each selected parking bay along its own axes: those of the least-area rectangle around its footprint (`PlanSpanService::measure_rectangle`), never its bounding box, so a bay turned 45° whose box is long enough but which is itself too short is found. Its length is the longer side, its width the shorter; the ends are the two sides across the long axis, the sides the two along it. It needs the plan-span service and, depending on the checks declared, the vertical-extent and proximity services.

| Parameter | Type | Meaning |
|---|---|---|
| `min_width`, `max_width`, `min_length`, `max_length` | lengths, optional | inclusive bounds on the sides of the rectangle |
| `min_height`, `max_height` | lengths, optional | inclusive bounds on the bay's vertical extent |
| `aisles` | selector, optional | the aisles a bay opens onto; with `orientation` |
| `orientation` | string, optional | `parallel`, `perpendicular` or `angled`: how the bay's long axis must stand to an aisle's |
| `angle_tolerance` | plane angle | required with `orientation`: how far from parallel or perpendicular still counts as it, in `[0°, 45°)`; `angled` is anything further from both |
| `aisle_reach` | length, optional | how far in plan an aisle may lie from the bay (default 0: meeting it) |
| `obstacles` | selector, optional | what obstructs a bay, such as columns and walls |
| `obstruction_reach` | length | how far in plan from the bay an obstacle obstructs it |
| `end_obstructions`, `side_obstructions` | strings | how many ends and sides may be obstructed: `none`, `one` or `both` |
| `side_zone_length` | length, optional | a side counts as obstructed only by an obstacle overlapping its central stretch this long, so a column off a corner is none |
| `applies_when` | string, optional | `findings` (the default): orientation and obstructions are findings; `filter`: they select the bays the size bounds apply to |
| `orientations` | string list | with `filter`: the orientation states a bay must be in, any of `parallel`, `perpendicular`, `angled` and `unclear` |
| `end_states`, `side_states` | string lists | with `filter`: how many ends and sides may be obstructed, any of `none`, `one` and `both` |
| `neighbour_reach` | length, optional | with `orientations` and no `aisles`: infer the orientation from the neighbouring bays within this reach in plan |

Declaring nothing to check, an orientation without aisles, or part of the obstruction parameters is an invalid declaration. Each declared check is its own finding or not-evaluated outcome:

- **Size**: the width and length (and height) are intervals judged against the bounds; one straddling a bound is not evaluated. Sizes need a unique orientation: a footprint several rectangles of least area enclose, or a tessellated one, is not evaluated.
- **Orientation**: the bay passes when a selected aisle surely within reach stands at the required angle; it is a finding ("not perpendicular to any aisle", or "no aisle lies within") when no aisle that may be within reach may stand so, relating the aisles found. A bay or aisle without a long axis (a square) leaves the angle undecided.
- **Obstructions**: an obstacle within reach obstructs an end when it reaches past the end's line and overlaps the end's span across the bay; likewise a side. Positions come from each object's extent along the bay's axes (`VerticalExtentService::measure_directional_extent`), widened by how far the axes may be turned. More ends (or sides) surely obstructed than allowed is a finding naming the obstacles; an obstacle within the bay's rectangle, past none of its edges, is always one. An obstacle the selector cannot decide, whose extent cannot be read or whose position straddles an edge may obstruct any edge it could reach, so it leaves the count not evaluated when it could exceed the allowance. A square bay has no ends; with obstacles near it, the count is not evaluated.

**Filters.** With `applies_when: filter`, the orientation and obstruction states of a bay are no findings: a size bound applies only to bays whose states lie in the declared sets, so one rule checks perpendicular bays against a 5 m length and ignores parallel ones, another parallel bays against 6 m. `orientation`, `end_obstructions` and `side_obstructions` belong to findings mode and are refused with `filter`; `obstacles` and `obstruction_reach` go with `end_states` or `side_states`, and at least one size bound is required.

- A bay's orientation state is the alignment its long axis makes with every selected aisle surely within `aisle_reach`, within `angle_tolerance`. With `neighbour_reach` instead of `aisles`, it is read from its neighbouring bays (the rule's own selection) within reach whose long axes are parallel to its own: the direction from its centre to theirs, taken as an aisle running along the row, is perpendicular when they stand side by side, parallel when end to end, angled in between. No such aisle or neighbour, or ones that disagree, make the bay `unclear`.
- Its obstruction states are the numbers of ends and sides obstructed, counted as in findings mode (with `side_zone_length` too).
- Every state is three-valued: an aisle, neighbour or obstacle that is only possibly there, or whose angle or position straddles, leaves several states possible. A bound applies when every possible state is allowed, not when none is; otherwise its pass stands and its failure is not evaluated, saying which states are open. A finding names the states it was applied under.
- An obstacle standing within the bay stays a finding in both modes.

Angled bays drawn as parallelograms are enclosed by a rectangle that is not their own; their dimensions along the stall line wait on a parallelogram measurement.

### Wall spacing

`axioval:capability.wall-spacing` checks the parallel walls or beams on each selected storey. It needs the plan-span, proximity, vertical-extent and (with a maximum) plan-area services, and a relationship service for the paths.

| Parameter | Type | Meaning |
|---|---|---|
| `members` | selector, required | the walls or beams |
| `member_path` | stringList, required | steps from the storey to its members, as in `path`; with IFC, `IfcRelContainedInSpatialStructure` |
| `angle_tolerance` | plane angle, required | how far from parallel two long axes may be, in `[0°, 45°)` |
| `minimum` | length, optional | the least plan distance between a parallel pair |
| `maximum` | length, optional | the largest distance at which a parallel pair bounds a band |
| `footprints` | selector | with `maximum`: the objects whose footprints make the storey's gross footprint, such as its slabs |
| `footprint_path` | stringList | with `maximum`: steps from the storey to them |
| `uncovered_above` | area | with `maximum`: how much of a footprint may lie outside every band |

At least one of `minimum` and `maximum` is declared, and `maximum` with `footprints`, `footprint_path` and `uncovered_above`. Two members form a **parallel pair** when their long axes (as for parking bays) lie within `angle_tolerance` of parallel and they face each other: along the first one's long axis, their extents share a stretch of positive length. Two collinear walls meeting end to end are no pair.

- **Minimum**: a parallel pair closer in plan than `minimum` (closest points, `horizontal` distance) is a finding against the storey that names and relates both.
- **Maximum**: each parallel pair at most `maximum` apart bounds a **band**, the convex hull of both footprints cut to the stretch they share (`PlanAreaService::measure_outside_bands`). The area of each footprint object outside every band above `uncovered_above` is a finding, relating the footprint and the members bounding a band; one at or below it passes.

Everything is an interval. A pair is too close only when it is surely parallel, facing, selected and closer than the minimum; a pair that may be is not evaluated with the reason. The uncovered area's upper bound comes from the sure bands, its lower bound from every possible one, and drops to zero when a member's extent cannot be read or a possible pair has no long axis to cut its band along; a straddling area is not evaluated. A storey reaching no footprint object is not evaluated. A band bounded by a tessellated member is refused by the Axiolid service.

### Accessible route

`accessible-route` requires each selected destination (an accessible room, say) to be reachable from a start point (an entrance) through the route spaces, for a mobility profile: a body `width_metres` wide under `clear_height_metres` of headroom. It needs the [walkability service](./walkability.md), for stated widths property resolution, and for the widths of ramps and stairs that state none the walking-surface service.

| Parameter | Type | Meaning |
|---|---|---|
| `route_selector` | selector, required | the spaces a route may cross |
| `start_selector` | selector, required | the start points; a destination some start reaches passes |
| `portal_selector` | selector, optional | the doors and openings a route may pass |
| `lift_selector`, `ramp_selector`, `stair_selector` | selectors, optional | the vertical connectors between levels, by kind; one object in two of them is an invalid declaration |
| `obstacle_selector` | selector, optional | what obstructs the route spaces and portals inside the headroom band (furniture, walls around a doorway) |
| `subtract_door_swings` | selector, optional | the doors whose leaves' swing obstructs the route spaces too; a route through a door walks through its own swing, never another's |
| `width_metres` | number, required | the body's width, the route's minimum clear width |
| `clear_height_metres` | number, optional | the headroom band above each floor; without it, each space's own height |
| `door_width_metres` | number, optional | the minimum clear width of every portal on the route |
| `ramp_width_metres`, `stair_width_metres` | numbers, optional | the minimum clear width of a ramp or stair on the route |
| `forbid_stairs` | boolean, optional | `true` (the default): no route may use a stair, so a room reached by stairs only is a finding |
| `clear_width_property` | property reference, optional | where the source states a portal's or connector's clear width, as a length |
| `obstruction_depth_metres` | number, optional | how far obstacles may intrude into a route space from its boundary without obstructing it (a skirting, a pipe along the wall); default 0 |
| `surface_gap_metres` | number, optional | route spaces at most this far apart are joined across the gap, unless an obstacle fills it; default 0 |
| `passing_width_metres`, `passing_length_metres` | numbers, optional | a passing space: a free box this wide across the route and this long along it, `clear_height_metres` high |
| `passing_spacing_metres` | number, optional | the longest stretch of a route without a passing space; the three passing-space numbers go together and need `clear_height_metres` |
| `passing_reach_metres` | number, optional | how far across the route a passing space's centre may lie from it (default: half its width, so it covers the route) |

A start or destination the portal selector picks is an entrance (its two faces); any other is a walkable surface. A route may cross only route spaces and portals, besides its own start and destination: a room that is a destination but not a route space is never passed through on the way to another.

The rule takes one walkability snapshot for the profile and judges every passage on top of its width bounds:

- **Portals.** A stated clear width below `door_width_metres` blocks; so does a door the geometry shows narrower (the upper bound of its crossing). A stated width at or above the minimum, or a crossing proven at least that wide, admits it; otherwise the door is undecided. A stated width is also sent with the request (`WalkabilityRequest::with_stated_clear_widths`), so the geometry can prove the body passes the leaf and lining: without one, a door from IFC can bound a route from above only, never pass it, because its `OverallWidth` includes the lining.
- **Connectors.** With `forbid_stairs`, a stair blocks. A ramp or stair with a minimum width is admitted by a stated clear width at or above it and blocked by one below it. A connector that states none (an exact absence) is measured as `ramp-geometry` and `stair-geometry` measure it, through the walking-surface service: a ramp's narrowest run, a flight's width from every tread's sides. A measured width wholly below the minimum blocks, citing the measurement; one wholly at or above it admits; anything else, a flight with winders, a refused measurement or no walking-surface service, is undecided. An unreadable stated width stays undecided and is never measured past. A lift needs no width of its own. A route through a connector is proven where the walkability backend proves the climb or ride (see [the Axiolid backend](./walkability.md#the-axiolid-backend)): the flight or run wide enough, its headroom clear and the walks to and from its landings swept.
- **Tolerances.** `obstruction_depth_metres` and `surface_gap_metres` go into the walkability request (`with_obstruction_depth`, `with_surface_gap`); see [the Axiolid backend](./walkability.md#the-axiolid-backend).
- **Stretches.** A block inside one route space names where it lies: the snapshot's `WalkableStretch` on the blocking passage. The reason reads `<space> is too narrow near (x, y, z)`, `<space> is obstructed near (x, y, z) … by <obstacles>` or `<space> is too low near (x, y, z) …: the headroom under <obstacles> is <h> m`, and the finding relates the space and those obstacles (or swung doors). The position is in the model's coordinates at the floor's elevation.
- **Start and destination portals.** A route begins on either face of a start door, so the door's own crossing is judged separately: too narrow for the body, it blocks every route from it.

Outcomes are three-valued. A destination some decided start reaches definitely passes. One that every start is proven cut off from is a finding that relates the blocking elements: the passages leaving what the starts can reach that lead on towards the destination (`WalkabilitySnapshot::blocking_passages`), each with its reason, or "connected to the starts by stairs only" when every block is a forbidden stair, or "no route space connects" when nothing joins them at all. Anything else is not evaluated, listing the undecided elements. A start, route space, portal or connector whose selection is undecided can only add routes: it keeps its passages possible but never definite, so it cannot turn a block into a pass. An undecided obstacle could block or free any route and leaves every destination not evaluated, and so does a door the `subtract_door_swings` selection cannot decide or whose leaves cannot be read. A swing across a corridor splits the corridor into pieces (see [the Axiolid backend](./walkability.md#the-axiolid-backend)), so a destination only reachable past it is a finding, located at the swing. A backend refusal (see [the Axiolid backend](./walkability.md#the-axiolid-backend)) is not evaluated, never a pass.

#### Passing spaces

With `passing_width_metres`, `passing_length_metres` and `passing_spacing_metres`, a proven route must also offer a passing space at most every `passing_spacing_metres` along it. The route checked is the one [metric routing](./metric-routing.md) walks from the start's representative point to the destination's (the exact centre of the footprint, inside it, on the floor, as in `space-distance`), for the rule's body and `clear_height_metres`: a polyline measured in plan. Its ends count as passing spaces, so a route no longer than the spacing needs none. A passing space stands at a position of the route when its centre lies across the segment from that position, within `passing_reach_metres`, with its length along the segment; it must lie in the route spaces (and the start and destination) the route crosses and meet no obstacle of `obstacle_selector` below `clear_height_metres`.

The route is cut into equal tiles no longer than half the spacing less 1 mm, and each tile is searched segment by segment with the [placement search](./free-space.md#placement-search): a box fixed to the segment's frame in a frame-offset domain anchored on it, across the route within the reach and along it over the tile's part of the segment, widened by 1 mm at both ends. A witness in every tile but the two at the ends means no stretch longer than the spacing lacks a passing space. Consecutive tiles proven empty that together are longer than the spacing prove such a stretch: a finding relating the start, `no route to … has its passing spaces: the route from … has no passing space (1.5 m by 1.5 m) between … and … along it, a stretch longer than the 15 m allowed`. Anything else, including a gap the tiles are too coarse to decide, is not evaluated.

A destination some start reaches with its passing spaces passes. One whose proven routes all lack them, with every other start cut off, is a finding; one whose passing spaces are undecided on some proven route is not evaluated. Passing spaces need the metric-routing, plan-span, vertical-extent and free-space services; without one, or when the metric route is refused, blocked, or crosses an object that is neither a route space nor a portal, the destination is not evaluated. An entrance as start or destination has no representative point on a floor, so metric routing refuses its route and its passing spaces are not evaluated. The finding is about the route metric routing walks, not every route: another route through the same spaces might have its passing spaces where this one has none, which is why the reach should cover the corridor's width.

### Local circulation

`axioval:capability.local-circulation` requires, within each selected space, a circulation path `width_metres` wide from the space's entrances to its components (a WC, a bed, a washbasin), with free areas where the path ends and at most a given distance apart along it. It asks `FreeSpaceService` for the space's [circulation map](./free-space.md#circulation-maps) and searches free areas with the [placement search](./free-space.md#placement-search), and it reads entrances and components through relationships, so it needs a geometry adapter and relationship selection.

| Parameter | Kind | Meaning |
|---|---|---|
| `component_selector` | selector, required | the components the path must reach |
| `space_path` | string list, required | steps from a component to the spaces it stands in, such as `axioval:derived.contained-in-space` |
| `access_path` | string list, required | steps from a door or opening to the spaces it opens onto, as in `space-connection`; with `axioval:derived.adjacent-space` alone, only the spaces on its faces |
| `door_selector`, `opening_selector` | selectors, at least one | the doors and openings that are entrances |
| `space_selector` | selector, optional | the spaces `access_path` may reach (default: every object) |
| `obstacles` | selector, optional | what occupies floor (default: every other object, the space's entrances never) |
| `subtract_door_swings` | selector, optional | the doors whose leaves' swing occupies floor too; an entrance of the space is walked through, so its own swing never does |
| `width_metres` | number, required | the path's width |
| `clear_height_metres` | number, required | the height above the floor the path, its end areas and passing spaces must be free to |
| `tolerance_metres` | number, optional | how much farther than half the width an entrance or component may be from the path and still be reached (default 0.05 m) |
| `component_mode` | string, optional | `touch` (the default): each component must be reached from an entrance; `link`: the components of a space must be linked with one another, entrances aside; `link_sets`: each component must be linked with one of the `partner_selector` objects of its space |
| `partner_selector` | selector, with `link_sets` | the objects each component must be linked with, placed by `space_path` like the components |
| `merge_path` | string list, optional | a relationship path reaching the spaces mapped together with the selected one as one walkable area, such as the zones of one room; their entrances and components count too |
| `band_from_metres` | number, optional | where the obstacle band starts above the floor (default: the floor); below `clear_height_metres` |
| `end_width_metres`, `end_length_metres` | numbers, optional, together | the free area each path end needs, this wide across the path and this long along it |
| `end_reach_metres` | number, optional | how far from the end, along and across the path, the area's centre may lie (default: half its larger side) |
| `short_end_metres` | number, optional | an end whose branch is shorter than this needs no free area |
| `narrow_end_metres` | number, optional | an end where the free width is less than this needs no free area |
| `end_exempt_selector`, `end_exempt_reach_metres` | selector and number, optional, together | an end within this reach of a selected object's footprint (placed by `space_path`) needs no free area |
| `passing_width_metres`, `passing_length_metres`, `passing_spacing_metres`, `passing_reach_metres` | numbers, optional | passing spaces along the path from each entrance to each component, as in [`accessible-route`](#passing-spaces) |
| `require_entrances` | boolean, optional | `true`: a space no door or opening reaches along `access_path` is a finding on the space; default `false` |
| `check_entrance_width` | boolean, optional | `true`: an entrance whose clear width is below `width_metres` is a finding on the space; default `false` |
| `clear_width_property`, `clear_width_from_leaves`, `overall_width`, `width_deduction` | optional, with `check_entrance_width` | where an entrance's clear width comes from: the steps of `keyed-limit`'s `clear-width` ([Keyed limits](#keyed-limits)), `clear_width_property` in place of `quantity_property` |

The circulation map erodes the space's free area (its floor less what the obstacles occupy up to `clear_height_metres`) by half the width, bounded from both sides. Its **pieces** lie inside the exact erosion, so a path runs between any two points of one piece; its **possible pieces** contain it, so no path joins two different ones. An entrance or component is near a piece when the piece comes within half the width plus `tolerance_metres` of its plan footprint: proven for a piece, possibly for a possible piece. The obstacle selection includes the components by default, since they stand on the floor; the tolerance then says how close the path must pass.

- **Reaching components.** A component near a piece that a surely selected entrance is also near is reached. One whose possible pieces are all apart from every entrance's, sure or possible, is a finding relating the space and its entrances: `no entrance of … reaches it on a path 0.9 m wide`. Anything else, such as a gap exactly as wide as the path, is not evaluated. A space with no entrance at all reaches none of its components.
- **Merged spaces.** With `merge_path`, the spaces it reaches from the selected one are mapped together with it (`CirculationRequest::with_merged_scopes`): the union of their floors is one free area, they are never obstacles, and the entrances and components of every one of them count. A bed in one zone is linked with a WC in the next across their shared boundary. The end areas and passing spaces are searched over the same spaces, and a finding relates the merged spaces and cites the path. A path the source cannot answer leaves the space not evaluated.
- **Band start.** `band_from_metres` starts the band obstacles count in above the floor (`CirculationRequest::with_band_from`, and the end areas' band), so a skirting or a threshold below it leaves the path free. Passing spaces keep the band from the floor.
- **Linking components.** With `component_mode` `link`, a component sharing a piece with every other component of its space passes, one with a possible piece apart from another's is a finding naming it (`no path … wide in … links it with …`), and anything else is not evaluated.
- **Path ends.** The ends of the skeleton of every piece a sure entrance reaches must each offer a free box `end_width_metres` across by `end_length_metres` along the path, found by a placement search with a frame-offset domain anchored on the end, its forward axis the direction the path runs into the end (taken over at least the end's half width, so nodes a rounding apart do not set it), within `end_reach_metres` both ways. A proof that none fits is one finding per space listing the ends: `no free area 1.5 m by 1.5 m lies within 0.75 m of the path end at (…, …)`. An end is exempt when its free width (twice its half width) is surely below `narrow_end_metres` or its branch surely shorter than `short_end_metres`; the branch runs along the skeleton from the end to the first junction or other end, plus the half width to the wall it ends at, widened by two sample spacings each way because node positions are approximate. An exemption that may or may not apply leaves an end without a free area not evaluated. A piece whose skeleton could not be built leaves its ends not evaluated. With `end_exempt_selector`, an end without a free area within `end_exempt_reach_metres` of a selected object's footprint is exempt: the distance is measured with the proximity service (`measure_region_distance`) from a square two sample spacings to each side of the end node, since the node stands only near the true end; it is surely within reach when the square's distance plus its diagonal is, surely not when the square's distance exceeds the reach. An object whose selection is undecided can only make an end possibly exempt, and without the proximity service the exemption is undecided.
- **Linking sets.** With `component_mode` `link_sets`, a component sharing a piece with a surely selected partner of its space passes. One whose possible pieces are apart from every partner's, sure or undecided, is a finding (`no path 0.9 m wide in … links it with a partner`, noting when the space has none), unless a partner's spaces cannot be read; anything else is not evaluated.
- **Passing spaces.** The path from each sure entrance to each component it reaches is traced along the skeleton (fewest nodes from the node nearest the entrance to the node nearest the component), simplified to within a quarter of the width, and judged as in [passing spaces](#passing-spaces), its two ends counting as passing spaces. A missing stretch is a finding on the component relating the space and the entrance.

With `subtract_door_swings`, the floor the selected doors' hinged leaves sweep (their `SwingSector`s, as the object-frame service states them) is an obstacle for the map, the end areas and the passing spaces (see [Door swings as obstacles](./free-space.md#door-swings-as-obstacles)): a leaf standing open across the path cuts off what lies behind it. A door the selection cannot decide, or whose leaves cannot be read, leaves every space not evaluated. Without the option only what the obstacles occupy counts. Door widths along a route are judged by `accessible-route`; `check_entrance_width` judges the entrances of the space itself.

**Entrances.** Two space-level checks judge the space's entrances (the doors and openings `access_path` reaches it from, those of its `merge_path` spaces included), independent of any component and of the circulation map, so they apply to an empty room too. Both are off by default.

- With `require_entrances`, a space no door or opening reaches is a finding on it: `… has no entrance: no door or opening reaches it along IfcRelSpaceBoundary:backward`. One reached only by an element whose type the selectors cannot decide, or by one whose spaces cannot be read, is not evaluated.
- With `check_entrance_width`, each entrance's clear width is read exactly as `keyed-limit` reads a door's `clear-width` (a stated length, the leaves, or the overall width less the rule's deduction, each step only after an exact absence, the deduction cited as an approximation) and judged against `width_metres`, as the decimals it displays. One below it is a finding on the space relating the door: `entrance … is narrower than the path: clear width (Pset.ClearWidth) is 0.8 m; required at least 0.9 m, the path's width`. A width that cannot be read or straddles the path width leaves the space not evaluated, and so does a possible entrance (undecided type or unreadable spaces) that may be too narrow. At least one source is declared with the check, and none without it.

These findings stand beside the path checks; the space's findings win over its undecided checks, as for its end areas.

Every verdict is three-valued. An obstacle the selection cannot decide leaves every space not evaluated; a component whose selection or spaces cannot be read is not evaluated itself; a map the service refuses leaves the space and its components not evaluated. A space with no component and no end area to check has nothing more to judge and is not mapped.

### Slab stacks

`slab-stack-spacing` judges `VerticalExtentService` elevations together with `PlanAreaService` footprints, so it needs a geometry adapter. Unlike `level-spacing`, which reads storey elevations, it measures the slabs' own surfaces.

| Capability | Checks |
|---|---|
| `slab-stack-spacing` | Two selected slabs stack when their footprints overlap by at least `minimum_overlap_ratio` of the smaller footprint. Each slab is checked against the next slab up it stacks with, ordered by top elevation: `top_to_top`, `bottom_to_bottom` and `top_to_bottom` (the clear gap to the next slab's underside) each lie within their optional `<measure>_minimum` and `<measure>_maximum`. `consistent` lists the measures that must equal the prevailing one of the stack within `tolerance` (1 mm by default), as in `level-spacing`. |

Elevations and footprints are intervals, and a tessellated slab's are never points. A slab is judged only on the whole interval: a distance straddling a bound, an overlap ratio straddling the minimum, or two tops the intervals cannot order are not evaluated. An object whose selection is undecided still counts as a possible next slab up, and one whose extent cannot be measured leaves every slab not evaluated, since it could sit in any stack. A slab with nothing stacked above it has nothing to check. When a proximity service is registered, its enclosing boxes skip overlap measurements between slabs that cannot meet in plan.

### Free space around components

`axioval:capability.component-clearance` requires a free volume on a stated side of each selected component: a transfer area beside a WC, the space in front of a washbasin or a control panel. It reads the component's placement frame through `ObjectFrameService`, its extents through `VerticalExtentService`, and asks `FreeSpaceService` whether the volume is clear (and, with `within_space`, inside the space) or, for a floating volume, searches for a free position, so it needs a semantic adapter that states placements and a geometry adapter. See [Free space and clearance](./free-space.md#component-clearance).

| Parameter | Kind | Meaning |
|---|---|---|
| `side` | `string` | `front`, `back`, `left` or `right` of the component's front; left and right as seen facing along the front (the component's own left and right). Exactly one of `side` and `sides`. |
| `sides` | `stringList` | Several sides instead of one, each checked as `side` is. |
| `quantifier` | `string` | With `sides`: `all` (default) checks each side on its own; `any` needs one side free of every question, such as a bed with one free long side (below). |
| `front_axis` | `string` | Required. Which placement axis is the front: `forward`, `-forward`, `right` or `-right`, or `stated` for the front the source states. A source that states none leaves the component not evaluated; IFC states none. For a door, `swing` is the side its hinged leaves open towards and `-swing` the other side, as the door's leaves state them (see [Doors](#doors)). `against-wall`: the front faces away from the wall the component stands against (below). |
| `wall_selector` | `selector` | With `against-wall`, required: the walls (or space boundaries) a component may stand against. |
| `wall_reach` | `quantity` | With `against-wall`, required: how far from the footprint's centre lines to look for walls. |
| `wall_inset` | `quantity` | With `against-wall`: how far each side's strip is narrowed on both edges (default 0), so the wall behind the component, which touches the strips beside it along their edge, is not taken for a wall beside it. |
| `both_sides` | `boolean` | Also check the opposite side, as its own finding. |
| `width`, `depth` | `quantity` | A box: `width` across the side, `depth` away from it. With a component mode, the length added to the component's own (default 0, may be negative). |
| `width_mode`, `depth_mode`, `height_mode` | `string` | `fixed` (default): the stated length. `component_plus`: the component's own width across the side, depth along it or height, plus the stated length. `component_clamped`: that, clamped between `*_minimum` and `*_maximum` (at least one). For the depth of a door's area, `less_clear_width`: `depth` less the door's clear width, clamped the same way (below). |
| `width_minimum`, `width_maximum`, `depth_minimum`, `depth_maximum`, `height_minimum`, `height_maximum` | `quantity` | The clamps of `component_clamped` and `less_clear_width`, refused with any other mode. |
| `clear_width_property`, `clear_width_from_leaves`, `overall_width`, `width_deduction` | `propertyReference`, `string`, `propertyReference`, `quantity` | With `depth_mode` `less_clear_width`, required (at least one source) and refused otherwise: where the door's clear width comes from, the steps of `keyed-limit`'s `clear-width` ([Keyed limits](#keyed-limits)), `clear_width_property` in place of `quantity_property`. |
| `depth_from` | `string` | `face` (default): the volume starts at the component's outermost point on the side. `midline`: at its midline between that side and the opposite one, so the depth counts from the middle of the component. |
| `radius` | `quantity` | A cylinder instead of a box. |
| `height` | `quantity` | The volume's height; required unless `top_datum` is declared, and refused with it. |
| `top_datum`, `top_offset` | `string`, `quantity` | The volume's top lies `top_offset` (default 0) above `floor`, the component's `bottom` or its `top`, independently of its base: "from the floor to 50 mm above the top". |
| `offset` | `quantity` | Gap between the component's outermost point on the side and the volume (default 0; negative overlaps the component). |
| `align` | `string` | `centre` (default) on the component, or flush with its `left` or `right` edge as seen looking out of the side. For a door with one hinged leaf, `handle` or `hinge`: flush with the edge its handle (the closed leaf's free edge) or its hinge is at, on a front or back side. |
| `lateral_offset` | `quantity` | Moves the volume across the side, to the right as seen looking out of it. |
| `slide_from`, `slide_to` | `quantity` | Together: the volume floats, free when it is free at some offset between the two across the side, to the right as seen looking out of it, from where `align` and `lateral_offset` put it. Needs `space_path`. |
| `depth_slide_from`, `depth_slide_to` | `quantity` | Together: the volume floats away from the component, free when it is free at some offset between the two (not negative) beyond where `offset` puts it; with `slide_from` and `slide_to` it floats both ways. Needs `space_path`. |
| `size_mode` | `string` | `minimum` (default): the volume must be free. `maximum`: no volume `size_tolerance` larger in one dimension may be free. `fixed`: both, the first less `size_tolerance` in every dimension. |
| `size_tolerance` | `quantity` | The tolerance of the size (default 0); `maximum` and `fixed` need a positive one. A minimum is checked less the tolerance in every dimension. |
| `height_reference` | `string` | Required. `floor` (the lowest point of the spaces `space_path` reaches), `bottom` or `top` of the component. |
| `vertical_offset` | `quantity` | The volume's base above the reference (default 0). |
| `obstacles` | `selector` | Required. What may obstruct the volume, such as every object but spaces. The component itself never does. |
| `allowed_intruders` | `selector` | Objects that may stand in the volume. |
| `protrusion` | `quantity` | How far an obstacle may reach into the volume through any of its plan sides. |
| `within_space` | `boolean` | Also require the volume's plan to lie inside the union of the spaces `space_path` reaches. |
| `space_path` | `stringList` | Steps from the component to its spaces, as in `path`; with `axioval:derived.contained-in-space:forward` the spaces its body stands in. Required by a `floor` datum, `within_space` and a floating volume, and refused otherwise. |
| `support_selector`, `support_tolerance` | `selector`, `quantity` | Together, for a fixed volume: its plan must lie wholly on the tops of the selected bodies (a slab, a landing) within `support_tolerance` of its base (below). |

Exactly one of the box (`width` and `depth`) and the cylinder (`radius`) is declared. A minimum size is a box of that size: a larger free volume holds it. The WC transfer area 0.9 m deep and 0.7 m wide to the left of a WC facing along its placement's forward axis, flush with its front edge:

```json
{"side": {"type": "string", "value": "left"},
 "front_axis": {"type": "string", "value": "forward"},
 "width": {"type": "quantity", "value": 70, "unit": "cm"},
 "depth": {"type": "quantity", "value": 90, "unit": "cm"},
 "height": {"type": "quantity", "value": 2, "unit": "m"},
 "align": {"type": "string", "value": "right"},
 "height_reference": {"type": "string", "value": "floor"},
 "space_path": {"type": "stringList", "value": ["axioval:derived.contained-in-space:forward"]},
 "obstacles": {"type": "selector", "value": {"kind": "not", "operand": {"kind": "entityType", "objectType": "…space"}}}}
```

The volume starts at the component's outermost point on the side, whatever the placement origin. With `protrusion`, the volume checked is the declared one shrunk by it on every plan side: an obstacle meeting the shrunk volume reaches further in than allowed, one missing it reaches no further. `protrusion` must leave a volume (less than half the width and depth, or the radius). The containment check uses the declared volume.

**Sized from the door's clear width.** Some accessibility requirements size the area in front of a door from the door itself: its depth is the larger of a fixed depth and a total depth less the door's clear width, so a wider door needs a shallower area. `depth_mode` `less_clear_width` is that: the depth is `max(depth_minimum, depth - clear width)` (and at most `depth_maximum` when declared). With a 1.50 m `depth` and a 1.20 m `depth_minimum`, a 0.90 m door needs 1.20 m and a 0.20 m door 1.30 m. The clear width is read once per door exactly as `keyed-limit` reads it: a stated length (or a measured interval of lengths), the door's lining and leaves, or the overall width less the rule's deduction, each only after an exact absence. A clear width known only as an interval gives a depth interval, from the widest door's depth to the narrowest's (widened by a rounding step where the subtraction rounds), and the volume's edges are placed from it as from any measured position: the area is clear when it is clear at the deepest, obstructed when it is obstructed at the shallowest, and otherwise not evaluated, so a verdict that depends on where in the interval the width lies is never decided. A width that cannot be read (null, not a length, refused leaves, a missing deduction) leaves the door not evaluated (`the depth needs the clear width: …`). Every outcome names the depth's source (`front clearance (0.5 m wide, 1.3 m deep, 2 m high) is obstructed by …; the depth is 1.5 m less the clear width (Door.ClearWidth) 0.2 m, at least 1.2 m`), and a finding cites the clear width's evidence.

**Sized from the component.** A dimension in a component mode is measured on each side: the width is the component's extent across the side, the depth its extent along it (between its outermost points on the side and the opposite side) and the height its vertical extent, each an interval, and the volume's edges are placed from them, so the union and the common part of its positions account for the size too. "As wide as the component, 0.8 to 1.2 m, from the floor to 50 mm above its top" is `width_mode` `component_clamped` with `width_minimum` 0.8 m and `width_maximum` 1.2 m, `height_reference` `floor`, `top_datum` `top` and `top_offset` 50 mm; "the component's depth plus 0.3 m in front and 0.2 m behind" beside it is `width_mode` `component_plus` with `width` 0.5 m and `lateral_offset` 0.05 m. `size_tolerance` changes a top datum's height like any other.

**Any of several sides.** With `sides` and `quantifier` `any`, a side passes when every question on it passes (clear, and inside the space or supported when asked); the component passes when one side does, is one finding when every side surely fails a question (`left or right clearance has no free side: left (…) is obstructed by …; right (…) …`, relating every obstacle named), and is not evaluated otherwise.

**Supported.** With `support_selector` the declared volume's plan must lie on the tops of the selected bodies: the upward faces of their meshes, clipped to the band within `support_tolerance` of the volume's base (`FreeSpaceService::assess_support_coverage`, see [Free space and clearance](./free-space.md#support-coverage)). Supported needs the union of the volume's positions covered by sure supports within the tolerance of every elevation the base may have; unsupported, a finding (`… is not wholly supported: part of it lies over no top of the supports within … of its base`), needs part of the common part outside the tops of every possible support within the tolerance of any of them. An external door's clear area overhanging the slab edge is found this way. A floating volume has no support check.

**A front against a wall.**
 Most fixtures state no front, and their placement axes point any way, so one rule cannot say `forward` for every washbasin. With `front_axis` `against-wall` the front is derived from the footprint: the sides are those of its least-area rectangle (a footprint whose least-area orientation is tied, or a tessellated one, has none and is not evaluated), and beside each side the plan-span service finds the `wall_selector` walls within `wall_reach` of the rectangle's centre lines (`PlanSpanService::measure_side_distances`, see [Services](./services.md)). The side whose nearest wall is surely nearer, measured from the side, than every other side's is the back; the front faces away from it, horizontally. A wall the selection cannot decide, or one that may or may not lie beside a side, can only make a side nearer, so it can stop a side from being the back but never make it one. A tie (a basin in a corner, touching two walls) or no wall within reach leaves the component not evaluated (`front not decided: …`), never guessed. The derived front is cited as inexact evidence naming the wall (`axioval:derived.front:<component>:against=<wall>:side=<side>`). A rectangle turned off the coordinate axes has rounded axes; the volume is widened (union) and narrowed (common part) by the arc its points could sweep about the rectangle's centre. Two washbasins whose placements are turned a quarter against each other get the same verdict from one rule.

Each side and each question (clear, inside the space) is its own finding or not-evaluated outcome. A finding names
 and relates the obstructing objects (`left clearance (0.7 m wide, 0.9 m deep, 2 m high) is obstructed by …`) or the spaces the volume extends outside, and cites the placement, the extents and the service's evidence. An obstacle the selections cannot decide can only obstruct: a clear volume stands, and a volume obstructed only by undecided objects is not evaluated. Positions are intervals: a component whose sides are off the coordinate axes is measured within the rounding of the projection, and the volume is judged over every position it could take (see [Free space and clearance](./free-space.md#component-clearance)). A cylinder is bounded by inscribed and circumscribed polygons; an obstacle between the two leaves it not evaluated. A tilted component frame, an unmeasured obstacle or a tessellated one near the volume, and a `floor` reference whose `space_path` reaches no space are not evaluated. A `within_space` check whose `space_path` reaches no space is a finding.

**Floating volumes.** With `slide_from` and `slide_to`, the volume is searched with the [placement search](./free-space.md#placement-search) instead of asked about one position: in the spaces `space_path` reaches (the first is the scope, the rest are merged with it, so they must share its floor), as a box fixed to the side's axes in a frame-offset domain anchored on the component where `align` and `lateral_offset` put the volume, across the side between the two offsets. Obstacles count in the band from the volume's base up by its height, measured from the scope's floor, which the base must not lie below. A witness passes; a proof that no offset fits is a finding relating the spaces (`left clearance (0.7 m wide, 0.9 m deep, 2 m high) fits nowhere between 0 m and 0.3 m across the side in …`). A floating volume lies in its spaces by construction, so `within_space` adds nothing to it. Positions are intervals here too: a witness is asked for the volume grown by the interval (and 1 mm deeper, so the witness may stand up to 1 mm off the side's line), which holds the volume wherever it is; absence is proven for the volume shrunk by it. An obstacle the selections cannot decide is sent with the search, so a witness stands, and a proof of absence is asked again without it.

**Size modes.** A `minimum` is one question per side: is the volume, less `size_tolerance` in every dimension, free? A `maximum` asks, per dimension (width, depth and height, or radius and height), whether the volume `size_tolerance` larger in that dimension is free, grown the way the volume is anchored (the depth away from the component, the width away from an aligned edge or both ways when centred, the height upwards): a free one is a finding (`… is free, so the free volume exceeds the maximum width`), an obstructed one passes. `fixed` asks both. Each question is its own finding or not-evaluated outcome, fixed or floating. The largest volume that fits is not measured: a maximum is decided by the one larger volume only.

### Centre lines beside walls

`axioval:capability.centre-line-distance` requires the centre line of each selected component's footprint to lie between `minimum` and `maximum` from the walls beside it: a WC's axis 405 to 455 mm from the side wall, a basin's from the wall beside it. The centre line is one of the axes of the footprint's least-area rectangle, through its centre; the distance is measured square to it, to the nearest `wall_selector` wall in the strip beside the footprint on each side (as long as the footprint along the line, narrowed by `inset` at both ends), through `PlanSpanService::measure_side_distances` (see [Services](./services.md)). It needs the plan-span service.

| Parameter | Kind | Meaning |
|---|---|---|
| `wall_selector` | `selector` | Required. The walls (or other bodies) to measure to. |
| `centre_line` | `string` | Required. `long` or `short`: the rectangle's longer or shorter axis (a square, or sides the measurement cannot order, has neither). `against-wall`: from the wall the component stands against to its front, derived as `component-clearance`'s `front_axis` `against-wall` does; the sides are then its left and right. |
| `sides` | `string` | Required. `nearest`: the nearer wall of the two sides is judged. `both`: each side is judged on its own, so a missing second wall is a finding. |
| `minimum`, `maximum` | `quantity` | The distance range, at least one; the minimum no larger than the maximum. |
| `reach` | `quantity` | Required. How far from the centre line to look; at least `minimum` and `maximum`. A wall beyond it is none. |
| `inset` | `quantity` | How far each strip is narrowed at both ends (default 0), so the wall behind the component, which touches the strips beside it along their edge, is not taken for a wall beside it. |

Each judged side (or the nearest wall) is its own finding or not-evaluated outcome, with one of three results: `too close` (a sure wall surely nearer than `minimum`), `too far` (every wall that may be there farther than `maximum`, or no wall within it), and `no wall nearby` (none within `reach`). A pass needs every wall that may be there no nearer than `minimum` and, with a maximum, a sure wall no farther than it. A wall the selection cannot decide, or one that may or may not lie in the strip, can only be nearer: it lowers the lower bound and never passes a maximum. Distances are intervals widened by the rectangle's uncertainty and the rounding of the clipping, so a distance straddling a bound is not evaluated, and the evidence is never exact. A footprint whose least-area orientation is tied, or a tessellated one, has no sides and is not evaluated.

```json
{"wall_selector": {"type": "selector", "value": {"kind": "entityType", "objectType": "…wall"}},
 "centre_line": {"type": "string", "value": "against-wall"},
 "sides": {"type": "string", "value": "nearest"},
 "minimum": {"type": "quantity", "value": 405, "unit": "mm"},
 "maximum": {"type": "quantity", "value": 455, "unit": "mm"},
 "reach": {"type": "quantity", "value": 1, "unit": "m"},
 "inset": {"type": "quantity", "value": 1, "unit": "cm"}}
```

### Visibility of targets

`axioval:capability.component-visibility` requires targets to be in view from an eye above each selected component: a reception desk that must see the entrance doors, or a device that must be seen from nowhere. The eye stands `eye_height` above the component's base (the bottom of its vertical extent) over the centre of its footprint, read through `VerticalExtentService` and `PlanSpanService`; each target is asked about through the line-of-sight service (`SightService`), so it needs a geometry adapter.

| Parameter | Kind | Meaning |
|---|---|---|
| `targets` | `selector` | Required. What must (or must not) be seen, such as doors. |
| `blockers` | `selector` | Required. What may hide a target, such as walls and columns. The component and the target itself never do. |
| `eye_height` | `quantity` | Required. The eye's height above the component's base. |
| `radius` | `quantity` | Required. Only targets whose nearest point lies within this distance of the eye count. |
| `mode` | `string` | Required. `at-least` (at least `minimum` targets in view) or `none` (no target in view). |
| `minimum` | `integer` | With `at-least`, how many targets must be in view (default 1). |

A target is **in view** when a straight segment from the eye reaches it before any blocker; the service names a witness point on the target. It is **hidden** when the blockers cover every ray from the eye to it; the service names the occluders. Neither may be provable: a target only grazed, or covered only where two separate blockers meet, is **undecided**, never guessed. A wall of one body covers as a whole; a column of one closed convex body counts as a solid.

`at-least` passes when enough targets are surely in view and is a finding when too few could be, even counting every undecided one (`0 target(s) within 6 m of the eye 1.2 m above the base of … are in view; required at least 1; 1 hidden`, relating the hidden ones); otherwise it is not evaluated. `none` is a finding naming every target surely in view, passes when none could be, and is otherwise not evaluated. A target whose selection is undecided, whose distance straddles the radius, or which the service cannot assess is undecided. A blocker whose selection is undecided may hide a target: a target hidden only with its help is re-asked without it, and stays undecided unless the certain blockers hide it too. An eye whose centre or base is not known exactly (a tessellated component) is not evaluated.

The rule has no transparency threshold of its own: leave glazing and other see-through elements out of `blockers` with a condition on `axioval:presentation.Transparency`, as shown for selectors above.

### Effective coverage

`axioval:capability.effective-coverage` requires the union of sources' effect areas to cover enough of each selected element's footprint: how much of a room its sprinklers, detectors or extinguishers reach. It measures through the plan-area service's coverage (`PlanAreaService::measure_coverage`) and finds the sources near each element with the proximity service's broad phase.

| Parameter | Kind | Meaning |
|---|---|---|
| `sources` | `selector` | Required. The objects whose effect areas cover. |
| `mode` | `string` | Required. How far an effect reaches: `grown`, `touching`, `travel` or `visible` (below). |
| `range` | `quantity` | Required. The effect's range. |
| `minimum_ratio` | `number` | Required, in (0, 1]. The share of the footprint that must be covered. |
| `blockers` | `selector` | With `travel` and `visible`: objects whose footprints travel and sight go round. |
| `touch_tolerance` | `quantity` | With `touching`: how far apart in plan a source may stand and still touch (default 0). |
| `capacity_property`, `capacity_multiplier` | `propertyReference`, `number` | Together: also require the summed property of the sources reaching the element, times the multiplier, to reach its area. |
| `capacity_multiplier_property` | `propertyReference` | Instead of `capacity_multiplier`: each source's own multiplier, read on it. |
| `area_property` | `propertyReference` | The element's area, read on it, instead of its footprint's: a number in m² or an area quantity. |
| `access_path`, `door_selector`, `opening_selector`, `space_selector` | | With `travel` and `visible`: continue effects into the spaces the element's doors and openings join it to, read as for `space-connection` (below). |

- `grown`: a source's footprint grown by `range` in every plan direction.
- `touching`: the same, counting only sources whose footprint touches the element's, within `touch_tolerance`.
- `travel`: the points of the element's **free region** within `range` of travel from the centre of the source's footprint, going round blockers.
- `visible`: the points of the free region the source's centre sees, no farther than `range`.

The free region is the element's footprint less the blockers' footprints; a source whose centre lies outside it reaches none of it. The union of the effect areas, clipped to the footprint and divided by its area (or the area `area_property` states), must reach `minimum_ratio`: a finding says `0.5052 of the footprint (20.2083 of 40 m²) lies within the sources' effect areas (visible by 20 m); required at least 0.9`, or `… of the stated area (Pset.NetFloorArea) (…)`, and relates the sources that surely reach the element.

**Connected spaces.** With `access_path`, effects continue into connected spaces: each door or opening `door_selector` or `opening_selector` picks that reaches the element along `access_path` joins it to the other spaces it reaches (on its other face, with the derived adjacency). The free region then also holds the footprints of those spaces and of the doors and openings (for a bodiless opening, its void), less the blockers, so a sprinkler in the next room travels or sees through an open doorway into the element. The covered area is still clipped to the element's own footprint, space by space. A door or opening whose selection is undecided joins only the upper bound; one whose spaces cannot be read might join anything, and so might one the geometry cannot measure: either leaves the upper bound at the whole footprint. With connections, blockers within `range` of the element are sent, not only those overlapping it, since a walk or sight line reaching it within the range stays within the range of it. `grown` and `touching` take no `access_path`: a grown effect ignores walls already. Only one step is taken: an effect does not continue from a connected space into a third.

The covered area is an interval. Effect areas are bracketed between an inner and an outer bound (a disc has no exact polygon, and travel distance is known cell by cell), a source whose selection or touch is undecided counts only towards the upper bound, a blocker whose selection is undecided only narrows the lower bound, and a source whose effect or extent cannot be measured leaves the upper bound at the whole footprint. A share straddling the minimum is not evaluated, with the reasons.

The capacity check sums the property, each times the multiplier (the constant, or the source's own `capacity_multiplier_property`), over the sources whose effect meets the footprint (surely for the lower sum, possibly for the upper), reading a number or a quantity in its SI unit: extinguisher rating units times the floor area one unit serves must reach the room's area (the stated area with `area_property`). A source that states no usable value leaves the check undecided unless the other sources already settle it. It is its own finding (`capacity: …`) or not-evaluated outcome.

**Missing values** are findings of their own, starting `missing value:`, apart from the checks they keep from being decided:

- `missing value: its Pset.NetFloorArea is not stated`: the element's `area_property` is absent, null or blank. Neither check runs.
- `missing value: <source>'s Pset.Units is not stated`: a source that surely contributes (surely selected, its effect surely meeting the footprint) states no capacity or multiplier; the finding relates the source, and the capacity check is not evaluated unless the others settle it. A source that only possibly contributes is named in the not-evaluated reason instead.

A value of another kind (text, a negative number, a length for an area) is not a missing value: it leaves its check not evaluated.

### Stairs and ramps

`stair-geometry` and `ramp-geometry` judge what `WalkingSurfaceService` measures from each selected object's body (see [Typed host services](./services.md)), so they need a geometry adapter; declared values such as `RiserHeight` or `NumberOfRisers` are checked with `property-predicate` instead. Select the objects the measure fits: single flights (`IfcStairFlight`) and ramp flights (`IfcRampFlight`), not a whole stair whose landing would count as a tread.

`axioval:capability.stair-geometry` measures a straight or turning flight: its treads are its upward-facing level faces, its risers the height differences from its lowest point through each tread to its top (the first riser starts at the body's lowest point, so the flight is taken to stand on the level it starts from; a top above the last tread is a final riser to the upper floor), its goings the horizontal distances from nosing to nosing along its walking line. A straight flight's walking line is the direction it climbs, derived from the treads rather than the placement; a turning flight's (winders, a quarter turn) is a polyline with a vertex on every tread, midway across it or `walking_line_offset` from the side the flight turns towards. At least one check is declared:

| Parameter | Kind | Meaning |
|---|---|---|
| `riser_minimum`, `riser_maximum` | `quantity` | Every riser, inclusive. |
| `going_minimum`, `going_maximum` | `quantity` | Every going (a flight of `n` treads has `n - 1`). |
| `step_length_minimum`, `step_length_maximum` | `quantity` | `2r + g` for every tread, `r` the riser climbing onto it and `g` the going leaving it. |
| `nosing_minimum`, `nosing_maximum` | `quantity` | How far each tread reaches over the one below. |
| `minimum_risers`, `maximum_risers` | `integer` | The number of risers. |
| `maximum_rise` | `quantity` | The flight's rise, base to top of its last riser. |
| `riser_tolerance`, `going_tolerance` | `quantity` | The difference between the flight's largest and smallest riser or going. |
| `walking_line_offset` | `quantity` | Where a turning flight's walking line runs: this far from the side it turns towards, across each tread. Without it, the centre line. A straight flight's goings are the same on every line. |
| `winder_angle_maximum` | `quantity` (plane angle, `deg` or `rad`) | The plan angle between consecutive nosings, `0` for straight treads. |
| `winder_angle_minimum` | `quantity` (plane angle) | Every winder's angle at least this: an angle surely zero is a straight tread and is skipped, and a straight flight has no winder to judge. |
| `forbid_open_risers` | `boolean` | `true`: every open riser is a finding. |
| `minimum_headroom` | `quantity` | The least vertical clearance above the treads, with `headroom_obstacles`. |
| `headroom_obstacles` | `selector` | The objects that may stand above (slabs, beams, ducts); declared together with `minimum_headroom`. |
| `width_minimum`, `width_maximum` | `quantity` | The flight's width: its narrowest tread's, across the direction it climbs. |
| `landing_objects` | `selector` | The objects that may carry a landing (landing slabs, floors); declared with at least one landing check. |
| `landing_depth_minimum`, `landing_width_minimum` | `quantity` | The landing at each end: its depth along the walking direction from the first or last riser, and its width across it. |
| `landing_at_least_walking_width` | `boolean` | Each landing also at least as deep and as wide as the flight. |
| `landings_required` | `boolean` | A selected slab or landing meets both ends of the flight. |
| `landing_doors` | `selector` | Doors (or their openings) that must not stand on the landing at either end of the flight; needs `landing_objects`. |
| `landing_door_height` | `quantity` | The height of the column over a landing a door's body must not reach into; declared together with `landing_doors`. |
| `landing_door_swing` | `boolean` | No `landing_doors` door may swing over a landing either; needs `landing_doors`. |
| `minimum_headroom_below` | `quantity` | The least clearance under the flight over the floors of `headroom_below_spaces`. |
| `headroom_below_spaces` | `selector` | The spaces people walk in under the flight; declared together with `minimum_headroom_below`. |
| `handrail_objects` | `selector` | The objects that may be the flight's handrails (railings of a handrail type); declared with at least one handrail check. |
| `handrail_reach_across` | `quantity` | How far outside the flight's sides a rail may run and still belong to it. |
| `handrail_reach_above` | `quantity` | How far above the nosing line's highest point a rail's lowest point may lie and still belong to it, so the rail of a flight stacked above is not taken for this one's. |
| `handrail_height_minimum`, `handrail_height_maximum` | `quantity` | The height of each rail's top above the nosing line, wherever both run. |
| `handrail_extension_minimum` | `quantity` | How far the handrail along each side reaches beyond the first and the last nosing (its first piece at the bottom, its last at the top), running level over that stretch. |
| `handrail_extension_maximum` | `quantity` | How far it may reach beyond them at most. |
| `handrail_extension_from` | `string` | `nosing` (the default) or `riser`: measure the extension from the first and the last riser instead, with an extension bound. |
| `handrail_gap_maximum` | `quantity` | The largest gap in plan allowed between consecutive pieces of the handrail along a side. |
| `handrail_sides` | `string` | `one` or `both`: the sides of the flight a rail must run along. |
| `handrail_both_sides_above_width` | `quantity` | With `handrail_sides` `one`: a flight wider than this needs rails on both sides. |
| `end_space_depth`, `end_space_width`, `end_space_height` | `quantity` | A free space this deep (along the walking direction), wide and high before the first riser and beyond the last, standing on the level there and centred on the end tread; declared together with `end_space_obstacles`. |
| `end_space_obstacles` | `selector` | The objects that must not reach into the free space at either end. |
| `clear_width_minimum` | `quantity` | The least clear width: the narrowest free width across the flight that the `clear_width_obstacles` leave between `clear_width_band_from` and `clear_width_band_to` above its pitch line. |
| `clear_width_obstacles` | `selector` | What narrows it: handrails, walls, anything standing beside or over the flight. |
| `clear_width_band_from`, `clear_width_band_to` | `quantity` | The band's bottom and top above the pitch line, such as 0.5 m and 1.5 m; declared with the other two. |
| `landing_clear_width_minimum` | `quantity` | The least clear width of the landing at each end: the narrowest free width across the direction leaving the flight that the `clear_width_obstacles` bounding it leave within the band above its level; needs `landing_objects`, `clear_width_obstacles` and the band. |
| `total_clear_width_minimum` | `quantity` | The least of the flight's and its landings' clear widths (in whole-stair mode, of every flight's and every landing's between flights, judged on the stair); needs the same. |

**Clear width.** The clear width is measured through the walking-surface service's `measure_clear_width`: at each position along the flight (or run) between its ends, the distance across between the innermost points the obstacles reach within the band from the left and from the right, the walking surface's own side where nothing reaches in, and its least value along the stretch (`the clear width of the flight 0.5 m to 1.5 m above its pitch line is 1 m beside …; at least 1.1 m required`, relating the obstacles bounding it): a 1.2 m flight with 0.1 m rails inside both sides fails a 1.1 m clear width. An obstacle the selection cannot decide can only narrow it, so a shortfall stands and enough is not evaluated. The Axiolid adapter measures straight flights and runs; a turning flight, an obstacle reaching over the middle of the walking surface, or one that may enclose the band without a face in it leaves the check not evaluated.

**Landing clear width.** With `landing_clear_width_minimum`, the landing at each end of the flight (the level surface of a `landing_objects` object, as for the landing checks) is measured through `measure_landing_clear_width`: over its rectangle, from the arrival line to its far side, the distance across the direction leaving the flight between the innermost points the `clear_width_obstacles` reach within the band above its level, and its least value (`the clear width of the landing at the top of the flight 0.5 m to 1.5 m above its level is 1 m beside …; at least 1.1 m required`). A landing is measured between the walls or rails bounding it: a side no selected obstacle reaches is not bounded by anything selected, and the landing is not evaluated rather than measured to its slab's edge. Where the walking line turns on the landing, its clear width along the turn is its depth, which `landing_depth_minimum` bounds. No selected landing at an end is nothing to measure, unless an object the selection cannot decide may carry one. In whole-stair mode only the landings between the stair's flights are measured. With `total_clear_width_minimum`, the least of the flight's own clear width and its landings' (in whole-stair mode, of every flight's and every landing's between flights, on the stair: `the least clear width of the stair's flights and the landings between them is 1 m, at the landing at the top of flight …`) must reach it: a width not measured can only lower the least, so a shortfall stands and a pass is not evaluated. A 1.2 m flight arriving at a landing 1 m wide between its walls fails a 1.1 m landing minimum and a 1.1 m total and passes a 1.1 m `clear_width_minimum`. Both parameters are the stair's; `ramp-geometry` does not take them.

**Tactile surfaces.** With `tactile_objects`, `tactile_offset` and `tactile_depth`, a tactile warning strip `tactile_depth` deep must lie `tactile_offset` before the first riser and beyond the last, across the flight's width (its end tread's sides), covered by a selected object lying on the level there (its body reaching within 5 cm of it, a floor finish):

| Parameter | Kind | Meaning |
|---|---|---|
| `tactile_objects` | `selector` | The tactile surfaces (with IFC, coverings of a tactile type). |
| `tactile_offset` | `quantity` | How far from the riser the strip starts, zero for directly at it. |
| `tactile_depth` | `quantity` | The strip's depth along the walking direction, positive. |
| `tactile_on_intermediate_landings` | `boolean` | In whole-stair mode, strips on the landings between flights too; without it, only at the stair's own ends. Outside whole-stair mode every flight end needs one. |

The strip is placed as the end space is. A tactile object is read through the plan-span service's least-area rectangle, which encloses its footprint, and the plan-area service's footprint area: a footprint whose area reaches the rectangle's (within rounding) fills it. The strip is covered when one object surely selected, surely on the level and filling its rectangle surely holds all four corners of the strip, a strip exactly as large as required included. It is a finding when a point surely in the strip lies surely outside every object that may lie there: none at all (`no selected tactile surface lies in the tactile strip at the bottom of the flight (0.6 m deep, 0.3 m before the first riser, across the flight)`), or one too narrow or misplaced (`… is not covered: … leave part of it bare`, relating the objects near it). An object that cannot be placed in plan, or strips of several objects that may cover it together, leave it not evaluated; so do an end tread filling no rectangle and an end the service does not place.

**Whole stairs.** With `stair_path`, the rule selects whole stairs (with IFC, `IfcStair`) and reaches each one's parts along that path (`IfcRelAggregates`); the parts `stair_flights` selects are its flights. Every flight is checked once as above, its findings on the flight, and the stair as a whole, its findings on the stair:

| Parameter | Kind | Meaning |
|---|---|---|
| `stair_path` | `stringList` | Steps from a stair to its parts, as in `path`. |
| `stair_flights` | `selector` | Which reached parts are its flights; required with `stair_path`. |
| `maximum_total_rise` | `quantity` | The stair's rise, from its lowest flight's base to its highest flight's top. |
| `handrail_continuous_across_landings` | `boolean` | The handrail along each side continues across every landing between consecutive flights; needs the handrail parameters. |
| `handrail_break_doors` | `selector` | Doors that break the handrail where they stand at a landing; needs `landing_objects` and `landing_door_height`. |

Consecutive flights are the stair's flights in the order of their bases, each arriving within a millimetre of the level the next one starts from; flights that do not meet so, or start at one level, leave the handrails between them not evaluated. Across each such landing, the last piece along a side of the lower flight and the first piece along the same side of the upper one (sides seen climbing each, so the inner side of a stair turning left is the left of both) must be joined by a chain of `handrail_objects` rails, each within `handrail_gap_maximum` of the next (touching without it), as the proximity service measures them in space: a rail along the landing joins them, a rail stopping at it does not (`the handrail along the left side stops at the landing between … and …: … and … are not joined by selected rails within 0 m of each other`), and neither does a side one flight has a rail along and the other not. A side neither has is left to `handrail_sides`. Where a `handrail_break_doors` door reaches into the column `landing_door_height` high over the landing's rectangle grown by `handrail_reach_across` on every side (where a handrail would run), the rail may break there: a door that may stand there leaves a break not evaluated. A flight the selection cannot decide or the service cannot measure leaves the stair's continuity not evaluated and its rise decided only when already too high; a pair of rails whose distance is not decided, or an undecided rail that may bridge the gap, leaves the side not evaluated. `stair_flights`, `maximum_total_rise`, `handrail_continuous_across_landings` and `handrail_break_doors` without `stair_path` are an invalid declaration.

`axioval:capability.ramp-geometry` measures a ramp's sloped runs: connected upward-facing faces flatter than 45°, each planar, separated by level landings. A run's slope is its rise over its horizontal length along its steepest ascent.

| Parameter | Kind | Meaning |
|---|---|---|
| `slope_limits` | `table` | Rows of `maximum_slope` (`number`, rise over length: `0.0833` for 1:12, required), `maximum_length` and `maximum_rise` (`quantity`, optional). A run conforms when some row holds all its columns, so a slope depending on the run's length or rise is one row per step. |
| `slope_tolerance` | `number` | The difference between the ramp's steepest and shallowest run. |
| `minimum_headroom`, `headroom_obstacles` | | As for stairs, above the runs and landings. |
| `width_minimum`, `width_maximum` | `quantity` | Every run's width across its slope. |
| `landing_objects`, `landing_depth_minimum`, `landing_width_minimum`, `landing_at_least_walking_width` | | As for stairs, at both ends of every run; the ramp's own level faces count as its landings. |
| `landings_required` | `boolean` | A selected slab or landing (or the ramp's own level face) meets both ends of every run. |
| `minimum_headroom_below`, `headroom_below_spaces` | | As for stairs, under the ramp. |
| `handrail_objects`, `handrail_reach_across`, `handrail_reach_above`, `handrail_height_minimum`, `handrail_height_maximum`, `handrail_extension_minimum`, `handrail_extension_maximum`, `handrail_gap_maximum`, `handrail_sides`, `handrail_both_sides_above_width` | | As for stairs, along every run: the height above the run's surface and the extension beyond its lower and upper end. A run has no riser, so `handrail_extension_from` `riser` is refused. |
| `check_continuous_handrails` | `boolean` | The handrail along each side continues across every landing between consecutive runs; needs the handrail parameters. |
| `handrail_continuity_tolerance` | `quantity` | The largest gap allowed along a side across such a landing; without it `handrail_gap_maximum`, else none. Needs `check_continuous_handrails`. |
| `check_rails_obstruction` | `boolean` | No rail of the ramp may reach over an `accessible_surface_selector` surface in plan; needs the handrail parameters. |
| `accessible_surface_selector` | `selector` | The clear paths a rail must keep out of: the zones or spaces of the landings or corridors adjoining the ramp, not the slabs carrying its walls; declared together with `check_rails_obstruction`. |
| `clear_width_minimum`, `clear_width_obstacles`, `clear_width_band_from`, `clear_width_band_to` | | As for stairs, every run above its surface. |
| `end_landing_depth_minimum`, `end_landing_width_minimum` | `quantity` | The landing at the ramp's two ends, before its lowest run and beyond its highest, against these instead of `landing_depth_minimum` and `landing_width_minimum`, which keep judging the landings between runs. |
| `end_space_depth`, `end_space_width`, `end_space_height` | `quantity` | A free space this deep (along the run), wide and high in front of the lowest run's lower end and beyond the highest run's upper end, centred on the run across it; declared together with `end_space_obstacles`. |
| `end_space_obstacles` | `selector` | The objects that must not reach into the free space at either end. |
| `landing_doors`, `landing_door_height`, `landing_door_swing` | | As for stairs, at both ends of every run. |

**Handrail continuity on a ramp.** With `check_continuous_handrails`, across each landing between consecutive runs (the upper end of one meeting the lower end of the next within a millimetre; runs that do not meet leave it not evaluated), the last piece along a side of the lower run and the first piece along the same side of the upper one must be joined by a chain of `handrail_objects` rails, each within the tolerance of the next as the proximity service measures them in space, exactly as a stair's handrail across its landings: `the handrail along the left side stops at the landing between run 1 of 2 and run 2 of 2: … and … are not joined by selected rails within 0.1 m of each other`, relating both rails. A distance known only as an interval straddling the tolerance (a tessellated rail within its chord deviation), an unordered side, an undecided rail and a missing proximity service leave the side not evaluated, never a finding.

**Rails obstructing an accessible surface.** With `check_rails_obstruction`, the ramp's rails are the `handrail_objects` rails measured along any of its runs and the selected rails joined to them touching in space (a separate extension piece or return). Each against each `accessible_surface_selector` surface is judged by the proximity service's plan overlap, the measurement `distance` takes with `projection: plan_overlap`: footprints overlapping with positive area are a finding relating both (`handrail … of the ramp reaches over the accessible surface … in plan`), so a rail extension reaching 0.3 m into a landing's path is found while a rail turning along a wall beside the path passes. An overlap the measurement leaves open (a tessellation within its chord deviation of the surface), a rail only possibly joined to the ramp's, an undecided rail or surface and a missing proximity service leave a pass not evaluated.

A ramp limited to 1:12 and 0.76 m of rise per run, or allowed 1:10 over runs of at most 2 m:

```json
{"slope_limits": {"type": "table", "value": [
  {"maximum_slope": {"type": "number", "value": 0.0833},
   "maximum_rise": {"type": "quantity", "value": 0.76, "unit": "m"}},
  {"maximum_slope": {"type": "number", "value": 0.1},
   "maximum_length": {"type": "quantity", "value": 2, "unit": "m"}}]}}
```

Every length and slope is an interval. Each check is judged on its own and each failing check is its own finding naming the values (`riser 3 of 4 is 0.21 m; at most 0.19 m required`) and citing the measurement; a check whose interval straddles its bound, widened by a few units in the last place for the binary rounding of decimal coordinates, is not evaluated while the others still decide. Headroom is the least vertical distance from the walking surface (treads, runs and landings) to a selected obstacle's body directly above it, measured from the surface, not from the pitch line; a finding names and relates the lowest obstacle. An obstacle `headroom_obstacles` cannot decide can only lower the headroom: too little stands, enough is not evaluated. An obstacle crossing the walking surface, an unmeasured or nearby tessellated one leaves headroom not evaluated.

A width is the walking surface's own, measured only where each tread or run fills a rectangle along its walking direction; handrails, walls or anything else standing over it are not deducted, so declare the width the body itself must have. A tread or run of any other shape leaves the width not evaluated; a winder tapers, so a flight with winders leaves `width_minimum` and `width_maximum` not evaluated, never judged on its straight treads alone.

A landing is the level surface of one `landing_objects` object (or, for a ramp, of the ramp itself) at the elevation of an end, meeting that end across its width. At a flight's bottom its depth runs back from the first riser; at its top on from the last riser, so a top tread level with the landing counts towards it (the flight's own top tread is never a landing by itself). A landing is measured only when its surface is a rectangle along the walking direction; one of another shape, or two objects meeting one end, leave its size not evaluated. With `landing_at_least_walking_width`, the requirement is the larger of the declared minimum and the flight's (or run's) width, an interval when that width is one. No landing at an end has nothing to check unless `landings_required` is set, when it is a finding; an object `landing_objects` cannot decide leaves a missing landing or a shortfall not evaluated, since it might carry the landing.

Headroom below is the least height of the flight's or ramp's underside above the floor of a selected space, its body's downward-facing level faces, directly beneath; where the body rests on that floor nobody stands, so it is left out. An open underside meeting the floor at its foot has no headroom there: select the spaces people walk in, and model the low part as another space where it is closed off. As for headroom above, an undecided space can only lower it, and a flight crossing a space's floor, an unmeasured or tessellated space leave it not evaluated.

A handrail is a `handrail_objects` object whose body runs along the flight or run: it overlaps the pitch line (the line through the nosings of a flight, the surface of a run along its slope) along the walking direction, lies within `handrail_reach_across` of the walking surface's sides across it and within `handrail_reach_above` above the pitch line, and its top is not below the pitch line's lowest point. It must run parallel to the walking direction: the upward-facing faces of its body fill one rectangle along it in plan. A rail bending in plan, one with posts or brackets standing out of that rectangle, or one at an angle to the flight leaves every handrail check of that flight or run not evaluated, never measured in part. Its height is the top of its body above the pitch line, the least and the greatest wherever both run, each an interval computed with a numerical margin; a rail given as a tessellation of curved faces is widened by its chord deviation, so a bound it straddles is not evaluated. A rail runs along the side (the left or the right, seen climbing) whose half of the walking surface's width holds it wholly across; one reaching over the middle counts for neither. The rails along one side are the pieces of that side's handrail, in order from the bottom: each must start and end decidably further along than the one before. Its extension is how far its first piece reaches beyond the first nosing (a run's lower end) and its last piece beyond the last nosing (the upper end) along the walking direction, and over `handrail_extension_minimum` beyond each end its top must stay level: a piece rising or falling there by more than a micrometre is a finding, one the chord deviation leaves undecided is not evaluated. With `handrail_gap_maximum`, consecutive pieces must lie no farther apart in plan than that, the least distance between the rectangles they fill (`handrail pieces … and … along the left side of the flight leave a gap of 0.1 m in plan; at most 0.05 m allowed`, relating both). Pieces beside or within one another (a second rail below the first, two starting where the positions cannot tell apart) cannot be put in order, and leave that side's extension and gaps not evaluated; heights are each piece's. A rail reaching over the middle is checked for its extension on its own, and falling short is not evaluated, since it may be one piece of a longer rail. With `handrail_extension_maximum`, a first or last piece reaching farther than that beyond its end is a finding (`… reaches 0.3 m beyond the bottom of the flight; at most 0.25 m required`), and so is a rail over the middle, which reaches too far whatever it is part of. With `handrail_extension_from` `riser`, a flight's extensions are measured from its first and last riser: a closed first riser lies under the first nosing, one not measured anywhere under the first tread (widening the extension by the tread's depth), and the last riser at the upper floor's edge when a closed final riser arrives there, else at the back of the tread below the last, so the top extension loses the last tread's overhang (`… reaches 0.28 m beyond the top riser of the flight; at least 0.29 m required`); an open or unmeasured last riser leaves that extension not evaluated. Each failing rail is its own finding naming and relating it. An object `handrail_objects` cannot decide might be a rail: a missing side or a passing height or extension is then not evaluated, and so are a short extension and a gap it might continue or bridge, while a rail found too low stands. A flight or run whose sides are not measured (a tread or run filling no rectangle) leaves its handrails not evaluated.

At a ramp's ends, `end_space_*` places a box of the declared size in front of the lowest run's lower end and beyond the highest run's upper end, standing on the end's elevation and centred on the run, and asks the free-space service whether an `end_space_obstacles` object reaches into it; the ramp itself never obstructs. At a flight's ends the box stands before its first riser and beyond its last, along the direction leaving the flight and from the arrival line the landing measurement places (asked with no candidate, so a turning flight's end lies square to its end tread's nosing), on the level at that end and centred across the end tread: a cupboard 1 m before the bottom riser obstructs a 1.5 m end space (`… obstructs the free space at the bottom of the flight (1.5 m deep, 1.2 m wide)`). An end tread that fills no rectangle, or an end the service does not place, leaves that end space not evaluated. `landing_doors` asks the same about the column `landing_door_height` high over each landing at a run's or a flight's end (standing on the flight's base at its bottom, on its top at its top), its rectangle as measured for the landing checks: a door whose body stands on the landing is a finding. A landing that fills no rectangle leaves its doors not evaluated; no landing has nothing to check. With `landing_door_swing`, no such door may swing over a landing either: a hinged leaf's sector (bracketed as for [distance](./clash.md#distance)) overlapping the landing's rectangle in plan, for a door whose vertical extent reaches into the column, is a finding (`door … swings over the landing at …`). A door wholly above or below the column is skipped; a sliding door sweeps nothing. A door whose leaves cannot be read, whose height is unknown while its swing may reach the landing, or whose swing only may overlap it leaves the landing not evaluated, and so does a door the selection cannot decide. Without the free-space service both checks are not evaluated. An undecided obstacle or door leaves a clear box not evaluated; an obstruction stands.

A stair near a ramp is the `distance` capability's `nearest` mode, the ramps as subjects and the flights as `counterparts` within `maximum_metres` (in plan with `projection` `horizontal`), not a parameter of `ramp-geometry`.

A turning flight is measured along its walking line: the goings of its winders shrink as the line moves towards the inner side, and a winder angle is the angle its nosing turns through to the next tread's (`winder angle 4 of 8 is 30°; at most 25° required`). A riser is open where the tread below ends in a face falling away rather than a riser climbing to the next tread (`riser 2 of 4 is open; closed risers required`); whether a riser rises all the way is not measured, so a kick plate below a gap counts as closed, and a first riser set back behind its nosing is not measured, leaving `forbid_open_risers` not evaluated. A winder angle or riser that is not measured leaves its check not evaluated. A tessellated flight is measured as intervals widened by its chord deviation, so a check near its bound may be left not evaluated. A turning flight's landing is placed along the tread meeting it, square to that tread's nosing, and compared with that tread's width under `landing_at_least_walking_width`; a flight starting or ending on a winder leaves that landing not evaluated. Its handrails are measured in its straight parts, the runs of treads square to parallel nosings: each rail must run straight along one part, on one side, and its height is taken above the nosings' ends on that side; around a turn, between nosings ending on different walls, the pitch line may lie anywhere between their elevations, so a rail there is measured against both and a bound it straddles is not evaluated. The handrail along a side reaches beyond the bottom from its first piece along the first part, and beyond the top from its last along the last part; a side whose rail starts or ends along another part does not reach that end (`handrail … runs along a later straight part of the flight only, …`). A rail at an angle to every part, reaching over the middle, or near a tessellated turning flight whose parts turn with its chords, leaves the flight's handrail checks not evaluated. Its headroom above and below is checked as any flight's.

A flight or ramp the service cannot measure is not evaluated, never passed: an open or inward-facing mesh, a flight in several pieces (separate treads), a flight with a sloped walking face, a walking line from the inner side of a flight whose winders turn both ways or that runs outside a tread, a tessellated ramp, a ramp whose slopes meet without a landing, or a body with no tread or run. Door swing over a ramp's or a flight's landing is `landing_door_swing`.

### Door swing

`axioval:capability.door-swing` requires each selected door to swing into, or not into, the spaces it opens onto: a WC door must open outward, a corridor must not be swung into, an office door opens into the office. The door's spaces are what `space_path` reaches from it (with IFC, `IfcRelSpaceBoundary:backward`, or `axioval:derived.adjacent-space`). Its swing is its leaves as the object-frame service states them (`ObjectFrameService::leaves`; with IFC, the operation type, the panel properties and the placement), and which side of the door a space lies on is asked of the free-space service (`assess_containment`) at two small probes per hinged leaf: halfway through its sweep and three quarters of its width out from the hinge, one on the side it opens into and one behind it. It needs both services, so the CLI runs it with `--geometry`.

| Parameter | Kind | Meaning |
|---|---|---|
| `space_path` | `stringList` | Required. Steps from a door to the spaces it opens onto. |
| `swing_into` | `selector` | Among the reached spaces this picks, the door must swing into at least one. |
| `swing_not_into` | `selector` | The door must swing into none of the reached spaces this picks. |

At least one of the two is declared. A door swings into a space when a swing-side probe of a leaf lies in it; a double-acting leaf swings into the spaces on both sides. `swing_not_into` finds each picked space swung into (`swings into …, which `swing_not_into` forbids`); `swing_into` is a finding only when every picked space surely lies behind the door and none on its swing side (`swings away from …, which `swing_into` requires it to swing into`). A space neither probe lies in (the probes stand in a wall, or the space is not beside the door) decides nothing: it leaves `swing_into` open and cannot break `swing_not_into`. A door without a hinged leaf (sliding, rolling up) swings into no space and is not evaluated, like a door whose leaves cannot be read; a space whose selection is undecided or a probe the service refuses decides only what it cannot change. Findings relate the spaces and cite the leaves and the containment proofs.

### Doors

Door accessibility checks are compositions of the capabilities above; `door-swing` is the one specific to doors. The door type is a key: with IFC, the door's `OperationType` (`axioval:attributes.OperationType`, such as `SINGLE_SWING_LEFT` or `DOUBLE_DOOR_SINGLE_SWING`) or its type object's name (`axioval:type-attributes.Name`). Each sub-check maps to one rule:

| Sub-check | Rule |
|---|---|
| Clear width per door type | `keyed-limit` with `quantity: clear-width`, a `minimum` per type row: the clear width the door states, else its `OverallWidth` less its type's deduction (`door_type_defaults`) or the rule's `width_deduction`, a declared approximation (see [Keyed limits](#keyed-limits) and [Defaults per door type](#defaults-per-door-type)). |
| Clear width derived from panel width less frame and panel thickness | `keyed-limit` `clear-width` with `clear_width_from_leaves` `passage`: the overall width less the lining on both jambs and every open leaf's thickness, as the door's leaves state them (with IFC, `LiningThickness` and each `PanelDepth`). |
| Clear width of the widest leaf | `keyed-limit` `clear-width` with `clear_width_from_leaves` `widest-leaf`: the widest hinged leaf less the lining at its jamb and its own thickness. |
| Clear height | `keyed-limit` with `quantity: clear-height`: a stated clear height, else `OverallHeight` less the head lining and threshold the door states (`lining_thickness`, `threshold_thickness`), or its type's defaults where it states none (`height_deduction`, `threshold_height`). |
| Threshold height | Measured: `keyed-limit` with `quantity: threshold-step`, `floor_path` `axioval:derived.adjacent-space` and a `maximum`: the door's bottom plus the stated `ThresholdThickness` (where `threshold_thickness` is declared; its type's `threshold_height` where it states none) above the floor on each side, a ramp's top within `ramp_reach` where `ramp_selector` picks one. Stated only: `keyed-limit` with `quantity: property`, or a `property-requirements` row with `maximum` and `unit`. |
| A revolving door accompanied by a swing door | `related-count` from the revolving doors, `path` `axioval:derived.adjacent-space` then `axioval:derived.adjacent-space:backward`, `related_selector` the swing doors, `minimum` 1 and `same_ends` `axioval:derived.adjacent-space`: only a swing door between the same spaces counts. |
| Glazing ratio | `keyed-limit` with `quantity: glazing-ratio`: a stated fraction, such as `GlazingAreaFraction` in `Pset_DoorCommon`, else its type's `glazing_ratio` default. Or a `property-requirements` row with `minimum`/`maximum` and no `unit`. IFC states no glazing per panel, so a leaf's own share is not derived. |
| Minimum distance to other doors | `distance` with `counterparts` the doors, `mode: none_closer_than`, `projection: horizontal` and `minimum_metres`. With `relationship: axioval:derived.adjacent-space`, only doors opening into a common space count. |
| Which spaces a door connects, and their types | `opening-spaces`, or a key read along `axioval:derived.adjacent-space` (as for sill heights); the side facing `outside` is recorded in the adjacency evidence. |
| Requirements per pair of space types (office to corridor, corridor to outside) | Any `keyed-limit` quantity with `pair_key`: rows name both sides (`key_1`, `other_side`) or one side and a wildcard, the most specific wins, and the outside is the reserved key `exterior` (see [Keyed limits](#keyed-limits)). |
| Door width on an accessible route | Walkability (#76) with the clear widths the host states per portal; the CLI states none, since `OverallWidth` includes the lining. |
| Clear areas in front of, behind and beside the leaf (handle side), with a floor under them | `component-clearance` with `front_axis` `swing` (the side the leaf opens into) or `-swing`, one rule per side and size; `depth_mode` `less_clear_width` sizes the depth from the door's clear width (`max(minimum, total - clear width)`); `align` `handle` puts the area flush with the handle edge, and `lateral_offset` moves it beyond; `within_space` asks for the floor under it. A double-acting, sliding or multi-leaf door leaves what it cannot place not evaluated. |
| Opening direction relative to the space type | `door-swing` with `swing_into` or `swing_not_into` selecting spaces by type: a WC door `swing_not_into` the WC (it opens outward), a corridor door `swing_not_into` the corridor. See [Door swing](#door-swing). |

A clear width per door type, stated where the model records it and otherwise approximated with a 10 cm deduction:

```json
{"limits": {"type": "table", "value": [
   {"key_1": {"type": "string", "value": "SINGLE_SWING_*"},
    "minimum": {"type": "number", "value": 0.9}},
   {"key_1": {"type": "string", "value": "DOUBLE_DOOR_*"},
    "minimum": {"type": "number", "value": 1.2}}]},
 "quantity": {"type": "string", "value": "clear-width"},
 "quantity_property": {"type": "propertyReference", "propertySet": "…door-accessibility",
                       "property": "…clear-width"},
 "overall_width": {"type": "propertyReference", "propertySet": "axioval:attributes",
                   "property": "…overall-width"},
 "width_deduction": {"type": "quantity", "value": 0.1, "unit": "m"},
 "key_1": {"type": "propertyReference", "propertySet": "axioval:attributes",
           "property": "…operation-type"}}
```

A threshold of at most 2 cm is the same table with a `maximum` of `0.02`, `quantity: property` and `quantity_property` the threshold height; doors at least 1.5 m apart in plan:

```json
{"counterparts": {"type": "selector", "value": {"kind": "entityType",
                  "objectType": "…door", "includeSubtypes": true}},
 "mode": {"type": "string", "value": "none_closer_than"},
 "projection": {"type": "string", "value": "horizontal"},
 "minimum_metres": {"type": "number", "value": 1.5}}
```

Every door closer than the minimum to another is its own finding naming the nearest one, so a pair too close is reported from both sides. A door type no row matches is a "no limit defined" finding; a row keyed `*` catches the types without a limit of their own (`NOTDEFINED` is a type like any other). A door whose `OperationType` is unset is not evaluated, never judged by a general row, since any row keyed on the type might be the one that applies.

### Shelf capacity

`shelf-capacity` requires each selected space to hold at least `minimum_running_metres` of shelving, measured through `LinearQuantityServiceHandle` as a layout of the declared arrangement:

| Parameter | Kind | Meaning |
|---|---|---|
| `minimum_running_metres` | number | the running metres the space must hold, all tiers counted |
| `shelf_depth_metres` | number | how deep a band of shelving is |
| `horizontal_spacing_metres` | number | the aisle serving the bands, between band faces |
| `vertical_spacing_metres` | number | the height of one tier |
| `bottom_elevation_metres`, `top_elevation_metres` | number | where the lowest tier starts and the shelving ends, above the floor |
| `door_clearance_metres` | number | how far the clearance at a door or opening reaches, in every direction |
| `access_path` | `stringList` | required: steps from each door or opening to its spaces, as for `space-connection` (`axioval:derived.adjacent-space`, or with IFC `IfcRelSpaceBoundary:backward`) |
| `door_selector`, `opening_selector`, `space_selector` | selector, optional | as for `space-connection` |

The layout lays parallel bands across the footprint as a store room is set out: a band against a wall, an aisle, two bands back to back, an aisle, and so on, each band served by the aisle along one side. A band carries shelving where its whole depth lies on the floor outside every door clearance and its aisle's whole width lies on the floor; an aisle may cross a clearance, a shelf may not (1 mm of each may touch the boundary, the construction tolerance). The swing of a door is not known, so its clearance is every point within `door_clearance_metres` of the door's footprint, measured from the room's side of a thick wall (the gap between door and space is added). The bands run along either side of the footprint's minimum-area rectangle or of any door's, anchored at either wall; the longest layout counts. Tiers are whole multiples of `vertical_spacing_metres` between the bottom elevation and the lower of the top elevation and the space's clear height.

The length is an interval that holds the layout's length on the true geometry, and its lower bound is positive for a room that holds shelving, so a compliant room passes. A 6 × 4 m room with one 1 m door in a long wall holds 84 m with 0.5 m bands, 1 m aisles, 0.5 m tiers up to 2 m and a 1 m clearance: four 6 m bands, less 3 m of the band against the door's wall, in four tiers. A space whose clear height lies below `top_elevation_metres` is a separate finding, `space too low for the shelving`; a shortfall relates the doors whose clearances were taken out. A door or opening whose spaces cannot be read, or whose type is undecided and that reaches the space, leaves the space not evaluated, since its clearance could take shelving away.

### Building envelope

Three envelope checks, each a rule of its own.

**Adjacency to an external wall** is a `distance` rule: the selection is the components, `counterparts` the walls the model declares external (with IFC, `IsExternal` in `Pset_WallCommon` equal to `true`), `mode: nearest`, `projection: horizontal` and a `maximum_metres` tolerance. A wall whose declaration cannot be read is undecided, never taken as internal, and a component is found only when no undecided wall could lie within the tolerance:

```json
{"counterparts": {"type": "selector", "value": {"kind": "allOf", "operands": [
   {"kind": "entityType", "objectType": "…wall", "includeSubtypes": true},
   {"kind": "property", "propertySet": "…pset-wall-common", "property": "…is-external",
    "operator": "equals", "value": {"type": "boolean", "value": true}}]}},
 "mode": {"type": "string", "value": "nearest"},
 "projection": {"type": "string", "value": "horizontal"},
 "maximum_metres": {"type": "number", "value": 0.05}}
```

**`recess-width`** requires every recess of each selected object's footprint to be wide enough for its depth. A recess is a pocket between the footprint's outer boundary and its convex hull, through `PlanSpanServiceHandle::measure_recesses`: its width is its mouth, the hull edge closing it, and its depth the farthest the pocket reaches behind the mouth. The convex hull is the reference, not the minimum-area rectangle, because it is orientation-free and each of its pockets is a genuine indentation; a rectangle would also report the corners of a trapezoidal or rounded room. An L-shaped footprint has one recess across its inner corner. Holes (enclosed courtyards) are not recesses. The one parameter, `requirements`, is a table:

| Column | Kind | Meaning |
|---|---|---|
| `minimum_depth_metres` | number, optional | the row holds recesses deeper than this |
| `maximum_depth_metres` | number, optional | and no deeper than this |
| `minimum_width_metres` | number, optional | the width required |
| `minimum_width_per_depth` | number, optional | the width required per metre of depth; with both, the larger applies |

The first row holding a recess's depth applies; a recess no row holds has no requirement. A depth straddling a row's bound, or a width straddling the requirement, is not evaluated. A tessellated footprint is not measured: chords make and hide pockets.

**`light-well`** checks vertically stacked light-well spaces. Each selected object is a well whose spaces `member_path` reaches (with IFC, a zone's spaces through `IfcRelAssignsToGroup:forward`):

- contiguity: ordered by their bottoms, no space starts more than `gap_tolerance_metres` (optional, default 0) above the top of the one below, and the spaces share a plan section, the intersection of their footprints through `PlanSpanServiceHandle::measure_section`;
- the section's area against `minimum_area_square_metres`;
- the section's width, the short side of its least-area rectangle (the rectangle `measure_rectangle` answers for a footprint), against `minimum_width_metres`. A section whose least-area orientation is tied has no known width; the service refuses it and the well is not evaluated.

The well's height runs from its lowest bottom to its highest top, and selects the first row of `requirements` whose `maximum_height_metres` it does not exceed (a row without one holds any height); no row means no requirement. Every value is an interval, and one straddling a bound is not evaluated. Findings relate the well's spaces. A tessellated space leaves its well not evaluated: the short side of a minimum-area rectangle does not grow monotonically with the shape, so a chord band cannot bound it.

### Structural members

Rules on how structural members and walls are built read the reserved body set (`axioval:body`, see [IR](./ir.md#attribute-sets)): the source's own statement of how a body is modelled, never a mesh. They need no geometry service. A wall's allowed representation kinds, its extrusion along the vertical, and a wall without a body are ordinary property rules over it:

| Check | Rule |
|---|---|
| Allowed representation kinds | `selector-conformance` or `property-value` on `axioval:body.Kinds` with `quantifier: all` and `oneOf`. |
| Extruded along the vertical | `property-predicate` on `axioval:body.Extrusion.Inclination`, `less_or_equal` a small angle. |
| Element without a body (a site without geometry, too) | `property-required` on `axioval:body.Count`: a body the representations do not state (none, or only a footprint or an axis) is an exact absence, a body stated but unreadable is not evaluated. |
| Empty wall: its face wholly voided by its openings | `empty-host`. |
| Profile from a table | `allowed-profile`. |
| Openings within their host, clear of its ends, edges, flanges and each other | `opening-zone`. |
| Gross area equal to net area plus openings | `opening-area` on the stated gross and net side areas (with IFC, `Qto_WallBaseQuantities` `GrossSideArea` and `NetSideArea`). |
| Components penetrating a beam or wall without a modelled void | `opening-zone` with `host_path: ["axioval:derived.intersects"]`. |
| Openings clear of supports, and components penetrating a beam clear of connecting beams | `opening-zone` with `support_distance` and `support_clearance`, the supports found by `support_path` (with IFC, `IfcRelConnectsElements:either`) or by contact (`support_gap`). |
| Opening distance from supports as a fraction of the beam's span or depth, with an absolute minimum | `opening-zone` with `support_distance_ratio`, `support_distance_reference` (`span` or `depth`) and `support_distance` as the minimum, judged per beam. |

`axioval:capability.allowed-profile` requires each selected object's body to be one swept solid whose profile is a row of the `profiles` table. A row fits when its `type` pattern matches the profile family (`i-shape`, `rectangle`, …), its `name` pattern (if any) the profile's name (a catalogue designation such as `HEA300`), and every dimension it states lies within the tolerance of the profile's.

| Parameter | Kind | Meaning |
|---|---|---|
| `profiles` | `table` | Required. Columns below. |
| `tolerance` | `quantity` | A length every dimension may be off by; exact (up to binary rounding) without it. |
| `case_sensitive` | `boolean` | Whether `type` and `name` patterns match case. `false` by default. |
| `angle_tolerance` | `quantity` | A plane angle every slope may be off by; exact without it. |
| `match` | `string` | `rows` (the default): a profile fits one whole row. `per_dimension`: the rows of its type and name list each dimension's allowed values apart, and any combination fits. |

| Column | Kind | Meaning |
|---|---|---|
| `type` | `textPattern` | Required. The profile family, a whole-value wildcard pattern. |
| `name` | `textPattern` | The profile's name. A profile without a name does not fit a row stating one. |
| `width`, `depth`, `web_thickness`, `flange_thickness`, `thickness`, `wall_thickness`, `radius`, `girth`, `fillet_radius` | `quantity` | Lengths, named alike for every family: `width` is a rectangle's `XDim`, an I-section's overall width, a T, U or Z section's flange width, an angle's or C section's `Width`, an asymmetric I-section's bottom flange width, a trapezium's bottom width; `depth` is `YDim` or the (overall) depth; `flange_thickness` (and `fillet_radius`) an asymmetric I-section's bottom flange's; `thickness` an angle's or centre-line profile's; `wall_thickness` a hollow or C section's; `fillet_radius` the root (or rounding, or inner) radius. |
| `semi_axis_1`, `semi_axis_2` | `quantity` | An ellipse's semi-axes. |
| `top_width`, `top_offset` | `quantity` | A trapezium's top width and the offset of its top (which may be negative); `top_width` is also an asymmetric I-section's top flange width. |
| `top_flange_thickness`, `top_fillet_radius`, `top_edge_radius`, `top_flange_slope` | `quantity` | An asymmetric I-section's top flange. |
| `edge_radius`, `web_edge_radius`, `outer_fillet_radius` | `quantity` | Edge radii: the flange edges of I and T sections (the bottom flange of an asymmetric I), the leg or flange edges of L, U and Z sections, a T's web edge, a hollow rectangle's outer corners. |
| `flange_slope`, `leg_slope`, `web_slope` | `quantity` | Plane angles: the flange slope of I, T and U sections (an asymmetric I's bottom flange), an angle's leg slope, a T's web slope; judged within `angle_tolerance`. |
| `tolerance`, `angle_tolerance` | `quantity` | The row's own tolerances, replacing the rule's. |

Three results are told apart, each a finding on the object:

- **wrong geometry**: no body, a body of several items, or one that is no swept profile (a boundary representation, a tessellation);
- **arbitrary profile**: an arbitrary outline, centre-line or composite profile no row allows;
- **not an allowed profile**: a parameterised profile no row fits, naming the nearest row of its type (least total excess beyond the tolerance, then a matching name, then the first declared) and each dimension it is off in (`depth 0.295 m, allowed 0.29 m within 0.001 m`); or of a type no row names.

With `match: per_dimension`, allowed profiles given as any combination of listed values need one row per value, not one per combination: rows `width 0.2, depth 0.3` and `width 0.3, depth 0.4` allow a 0.2 × 0.4 rectangle. Every dimension any row of the profile's type and name states must match one of those rows' values; a finding lists each dimension that matches none and the values it could have had. A profile whose name no row of its type allows is found as such.

A row stating a dimension the family does not have, or one the source leaves unset, does not fit; the schema's default is never assumed. A mirrored profile is judged by its parent, whose dimensions it keeps; a derived one is not evaluated, since the body set does not state whether its operator scales. A dimension the source refuses leaves the object not evaluated unless another row fits.

`axioval:capability.opening-zone` requires each selected opening to lie within its host's face and inside the zone the rule allows. The host is what `host_path` reaches from the opening among the `host_selector` objects (every object by default); with IFC, `IfcRelVoidsElement` backward. A duct or pipe passing through a beam or wall with no void modelled reaches it through the derived `axioval:derived.intersects` (`host_path: ["axioval:derived.intersects"]`, with geometry), and is judged as an opening would be. An element reaching several hosts is placed and judged in each, each finding naming its host; one whose host body cannot be read is not evaluated there alone.

| Parameter | Kind | Meaning |
|---|---|---|
| `host_path` | `stringList` | Required. Relationship steps from the opening to its host. |
| `host_selector` | `selector` | The hosts checked. An opening reaching none of them is not checked. |
| `length_axis`, `height_axis` | `string` | Required, distinct. Two of `extrusion`, `profile-x` and `profile-y`: the host's axes spanning the face the openings pass through. A beam: `extrusion` and `profile-y`; a wall extruded up from its plan outline: `profile-x` and `extrusion`. |
| `end_distance` | `quantity` | A length the opening must keep from both ends of the host along `length_axis`. |
| `edge_distance` | `quantity` | A length the opening must keep from both edges along `height_axis`, or from both flanges with `zone` `web`. |
| `edge_distance_maximum` | `quantity` | The largest distance allowed from the edges (or flanges) `maximum_edges` names: a window head at most 0.5 m below the wall top. Across a free outline, a distance from the box an opening not extruded straight through may lie in is a lower bound: it can find, never pass. |
| `maximum_edges` | `string` | `top`, `bottom` or `both` (the default): the high and low ends of `height_axis` `edge_distance_maximum` applies to. |
| `zone` | `string` | `section` (default: the host's whole height) or `web`: between the flanges of an I, T, U, C or Z section, or beside the ledge of an L, across `profile-y`. |
| `opening_spacing` | `quantity` | The clear distance the opening must keep from every other opening of the same host, in the face. |
| `zones` | `table` | Allowed zones, columns below: the opening must lie inside at least one row's zone. |
| `minimum_opening_area` | `quantity` | An area: openings smaller in the face are ignored, neither judged nor anyone's neighbour or target. |
| `dimensions` | `table` | The dimensioning table, columns below: distances from an opening to the nearest other opening of the host, or to the host's top, bottom or side. |
| `support_path` | `stringList` | Relationship steps from the host to its supports and connecting members. With IFC, `IfcRelConnectsElements:either`, which takes in its subtype `IfcRelConnectsPathElements`. |
| `support_gap` | `quantity` | A length: the `support_selector` objects that come this close to the host in space (through the proximity service) are its supports too. |
| `support_selector` | `selector` | The objects that may be supports (every object by default). |
| `support_distance` | `quantity` | A length the opening must keep from each support along `length_axis`; with a ratio, the minimum. |
| `support_distance_ratio` | `number` | A positive fraction of the host's `support_distance_reference` the opening must keep from each support; declared together with it. |
| `support_distance_reference` | `string` | `span` (the host's extent along `length_axis`) or `depth` (its extent along `height_axis`). |
| `support_clearance` | `quantity` | A clear distance the opening must keep from each support's footprint in the face; `0 m` requires only that it not overlap one. |

Each row of `zones` is one allowed zone: the host's face less an inset from both ends, one from the bottom and one from the top (the low and high ends of `height_axis`, or the lower and upper flanges with `zone` `web`). Each inset is the larger of a fraction of the host's span or depth and a minimum length; an inset a row leaves out is zero, so a row with only a top inset bounds one side (a precast hole below a one-sided top inset). Rows combine as a union: an opening inside any zone passes. One surely outside every zone is one finding, naming the zone it misses least and each side it is too near (`opening lies outside any of the 2 allowed zones of its host #50: nearest is zone `ends`, where it is 0.15 m from an end (0.2 m required)`), graded by that zone's worst shortfall. Across a free outline a distance known only from below (an opening not extruded straight through) can place an opening inside a zone but never outside one; if it may lie in a zone, it is not evaluated.

| Column | Kind | Meaning |
|---|---|---|
| `name` | `string` | The zone's name in findings; `zone 2` for the second row without one. |
| `end_fraction`, `end_of`, `end_minimum` | `number`, `string`, `quantity` | The inset from both ends: `end_fraction` of the `span` (the default) or the `depth`, at least `end_minimum`. |
| `bottom_fraction`, `bottom_of`, `bottom_minimum` | `number`, `string`, `quantity` | The inset from the bottom: a fraction of the `depth` (the default) or the `span`, at least the minimum. |
| `top_fraction`, `top_of`, `top_minimum` | `number`, `string`, `quantity` | The inset from the top, alike. |

The span is the host's length along `length_axis`, the depth its height along `height_axis` (for a free outline, of the box around it).

Each row of `dimensions` applies to the openings its `source` selects and bounds one distance along a face axis, `length` or `height`:

| Column | Kind | Meaning |
|---|---|---|
| `name` | `string` | The row's name in findings; `dimension row 2` for the second row without one. |
| `source` | `selector` | Required. The openings (doors, windows, openings) measured from. |
| `target` | `selector` | The other openings of the same host measured to; the distance is to the nearest. |
| `edge` | `string` | Instead of a target: the host's `top` or `bottom` (along the height), or its nearer `side` (along the length). |
| `direction` | `string` | `length` or `height`, along which a distance to a target is measured: the gap between the two openings' extents. Required with `target`; with `edge` it must match the edge. |
| `minimum`, `maximum` | `quantity` | Bounds on the distance. |
| `fixed` | `quantity` | The distance required, instead of `minimum` and `maximum`. |
| `tolerance` | `quantity` | How far the distance may miss its bound (0 m by default). |
| `overlap` | `boolean` | With `target`: only targets overlapping the opening across the direction count, such as windows side by side. |

A row whose opening's host holds no target is not judged. The nearest sure target bounds the distance from above and every target of undecided selection from below; an opening whose place is unknown may be nearer still, so a minimum surely missed by a sure target stands beside it, and otherwise it leaves the row not evaluated. A distance to an edge across a free outline is known only from below for an opening not extruded straight through: it can miss a maximum, never a minimum. Each miss is a finding relating the host and the target (`opening is 0.2 m from the side of its host #10 along its length; at least 0.3 m required (dimension `window to side`)`), graded against the bound it misses. An opening whose `source` selection is undecided is not evaluated for that row.

Every opening is checked to lie within its host's face along both axes (`opening lies partly outside its host #10: along its length it spans 1.7 m to 2.7 m, the host -2.5 m to 2.5 m`); each declared zone is its own finding. Findings relate the host, and a spacing finding the openings too close.

Both bodies are read from the reserved body set, never from a mesh, and are judged only where they can be soundly:

- the host must be one straight extrusion, perpendicular to its profile, of a family whose outline the set bounds (rectangles, circles, ellipses and the I, asymmetric I, T, U, C, Z and L sections, centred on their position) or of a free outline the set states as vertices (`Profile.OutlineX`, `Profile.OutlineY`, and its voids'), which must be one region: no edge crossing another, every void inside the outline;
- the opening must be one straight extrusion of a rectangle, rounded rectangle, circle, ellipse or free outline. Its extent along each face axis is exact: the reach of its outline in that direction, swept along its extrusion, even when it is tilted.
- an outline with a curved edge states no vertices, so a host or opening of one is not evaluated.
- an L section is a polygon: a parameterised one (legs `Thickness` thick along its low X and Y sides, without fillet, rounded edges or sloped legs, which leave only its box) or a free outline of six vertices whose edges run along X and Y, its legs as thick as they are (a non-uniform L, such as a precast ledger beam). Its web is the leg spanning the whole height, and its web zone the part of the height beside the notch the L leaves out, clear of the other leg. An opening through the web leaves the host where the web ends: past it lies the notch, not the host's outline.

A free outline bounds the host differently across it: a wall mitred at its end is shorter on one face than on the other. Where the face's axes cross the outline (a wall's `profile-x`, or both axes of a slab pierced vertically), the opening must lie inside the outline over the whole depth at which it passes through the host (`opening lies partly outside its host #10: it crosses the edge of the host's outline`), and its distance from the host's ends and edges is the least over that depth, measured to the outline (`opening is 0.5 m from an end of its host #10; 0.6 m required` near a mitre, where the box around the wall would leave 0.7 m). Both are exact when the opening is extruded straight through the host, or is a rectangle through a slab whose sides run along the face axes; otherwise only the box its extents span is known in the section, which can pass an opening but never find one. An opening reaching past the box around the outline is outside it either way.

A host's supports are the members it rests on or that connect to it (columns, walls, other beams): what `support_path` reaches from the host and, with `support_gap`, what the proximity service measures within that gap of it, both among the `support_selector` objects. Declaring either without `support_distance`, `support_distance_ratio` or `support_clearance`, or those without a way to find the supports, is an invalid declaration. Each support is read from the body set too: one straight extrusion of any section the set bounds. Its extent along a face axis is an interval sure to hold the true one and one sure to lie within it: the same where its outline is exact (a rectangle, circle or ellipse, or a flanged section across whose width or depth the axis runs) and otherwise the section's box both ways. Its footprint in the face is its extents, and a rectangle within its projection is known only when it is extruded along a face axis from a section across it (a column under a beam) or through the host from a rectangle whose sides run along the face axes (a secondary beam framing into a web).

- **Distance from supports.** A support whose inner extent along the length lies closer to the opening than `support_distance` is a finding (`opening is 0.1 m from support #700 along its host #50; 0.5 m required`, or `at most` where the extent is known only within bounds); one whose outer extent keeps the distance passes.
- **Distance scaled by the host.** With `support_distance_ratio` and `support_distance_reference`, the distance is `max(support_distance, ratio × reference)`, `support_distance` taken as zero when it is not declared, measured per host from the body set: the span is the host's extent along `length_axis`, the depth its extent along `height_axis`. With `max(0.5 × depth, 0.25 m)`, a 0.60 m deep beam requires 0.30 m from each support (`opening is 0.28 m from support c1 along its host b; 0.3 m (the larger of 0.25 m and 0.5 × the host's depth 0.6 m) required`) and a 0.40 m deep one 0.25 m. An opening placed in several hosts keeps each one's own distance. The requirement is an interval sure to hold the exact product (widened by a rounding step each way); a support closer than its lower end is a finding, one farther than its upper end passes, and a distance it straddles is not evaluated. A host's extents are read exactly today, so the interval is only the product's rounding. Without the ratio a fixed `support_distance` behaves as before.
- **Clear of connecting members.** An exact opening (an axis-aligned rectangle through the host) overlapping a known rectangle of a footprint, or closer to it than `support_clearance`, is a finding (`opening overlaps connecting member #20 by 0.08 m in the face of its host #50`); an opening whose extents keep the clearance from a footprint's extents passes.

Anything in between is not evaluated, naming each member it could not decide: a support whose body cannot be read, one whose position straddles the limit, and one whose contact, relationship answer or selection is undecided and which may come too close. A support surely too close is found even beside undecided ones. Findings relate the host and every support surely too close, and cite the relationship or contact evidence and their bodies.

Distances between openings are clear distances in the face. They are exact between two rectangles whose sides run along the face axes and which are extruded through the host (the third axis); for any other pair only the distance of their extents is known, a lower bound that can pass a pair but never find one, so a pair it cannot pass leaves the opening not evaluated. An opening whose host cannot be read, or whose own selection is undecided, may be a neighbour of any opening and is treated as one. Positions are composed from placements in binary arithmetic, so every bound is widened by a nanometre. Findings of a distance missing its bound (ends, edges, flanges, the largest edge distance, spacing, zones) are graded by how far they miss it.

With `minimum_opening_area`, an opening's area in the face is its section's where it is extruded through the host, and otherwise bounded by the box its extents span: one surely below the minimum is ignored, one that may be either is not evaluated and remains a possible neighbour.

`axioval:capability.opening-area` requires the openings of each selected host (a wall) to account for the difference between its stated gross and net side areas.

| Parameter | Kind | Meaning |
|---|---|---|
| `opening_path` | `stringList` | Required. Relationship steps from the host to its openings; with IFC, `IfcRelVoidsElement:forward`. |
| `opening_selector` | `selector` | The openings counted (every object by default). |
| `length_axis`, `height_axis` | `string` | Required. The host's face, as for `opening-zone`. |
| `gross_area`, `net_area` | `propertyReference` | Required. The host's stated gross and net side areas, such as `Qto_WallBaseQuantities.GrossSideArea` and `NetSideArea`. |
| `area_tolerance` | `quantity` | The area the sum may differ from gross less net by (0 m² by default). |
| `minimum_opening_area` | `quantity` | An area: openings smaller in the face are left out of the sum, as quantity rules leave small openings out of the net area. |

A side area is measured on the host's middle plane, so each opening counts with the exact area of its section when it crosses that plane and not at all when it stops short of it (a recess). Its area is known only when it is one straight extrusion through the host of a rectangle, rounded rectangle, circle, ellipse or free outline lying in the face (not a hollow one; a free outline's voids are subtracted), wholly within the host's face and clear of the other openings' extents; the host is read as `opening-zone` reads it, and a host of a free outline must hold the opening inside that outline, not only inside the box around it. A mismatch is a finding on the host relating its openings (`its openings (#1130) cover 1.2 m² of its face, but its gross side area 15 m² less its net side area 15 m² is 0 m²; they must agree within 0.01 m²`). A host stating neither area is not checked; one stating only one, an opening it cannot place, openings that may overlap and an opening whose selection is undecided leave it not evaluated.

`axioval:capability.empty-host` reports each selected host (a wall) whose openings void its whole face: a wall that is one opening, or openings side by side with no wall left between them. It takes `opening_path`, `opening_selector`, `length_axis`, `height_axis` and `minimum_opening_area` as `opening-area` does, and `area_tolerance`, the face area the openings may leave uncovered and still void it (0 m² by default). The openings are placed and summed exactly as `opening-area` sums them, on the middle plane and clear of each other. The face is the host's section across its middle plane: the length times the height of its box or, for a free outline crossed by the face, the outline's chord along the face at the middle of the host's thickness times its height (the outline's area, voids subtracted, when the face is the section itself). An empty host is a finding relating its openings (`host is empty: its openings (#40) void 15 m² of its 15 m² face`); a host with no openings is not empty. An opening it cannot place, openings that may overlap, an opening whose selection is undecided, or a free outline with an edge along the middle plane leave the host not evaluated.

### Model quality

Checks on how a model is built rather than on what it designs. Each sub-check maps to one rule:

| Sub-check | Rule |
|---|---|
| Material-layer thickness against the body's thickness | `body-extent` with `axis` `forward` and `target_property` the material set's `TotalThickness`, within a `tolerance`. |
| Polygon count per element | `triangle-count` with a `maximum`. |
| A door or window on another storey than its host | `same-container` from each door or window along `IfcRelFillsElement:backward`, `IfcRelVoidsElement:backward` to its host, climbing `IfcRelContainedInSpatialStructure` `backward` to the storeys. |
| Space-boundary coverage of a space's surface | `space-boundary-coverage` with `minimum_covered_share`, `maximum_uncovered_area` and/or `maximum_overlap_area`. |
| Door swing direction | `door-swing`: the swing the declared operation type and placement give, against the spaces the model relates to the door, such as `swing_not_into` corridors or `swing_into` the rooms a corridor serves. |

`axioval:capability.body-extent` measures each selected object's body along one of its own placement axes, through `ObjectFrameService` (the frame) and `VerticalExtentService` (the extent along the frame's axis), so it needs both a semantic adapter that states placements and a geometry adapter. It runs as a [template](./templates.md): the measured `body_extent` judged by the generic range judge, its outside contract unchanged, and a rule bound to it can be forked into the `expression` rule it composes.

| Parameter | Kind | Meaning |
|---|---|---|
| `axis` | `string` | Required. `right`, `forward` or `up`: the placement's first, second or third axis. |
| `target_property` | `propertyReference` | A length the object states, which the extent must equal within `tolerance`. |
| `tolerance` | `quantity` | A length, with `target_property` only; exact (up to binary rounding) without it. |
| `minimum`, `maximum` | `quantity` | Inclusive length bounds, instead of `target_property`. |

Exactly one of `target_property` and the range is declared. With IFC, a wall's layer set runs across the wall along its placement's second axis (`IfcMaterialLayerSetUsage` `AXIS2`), so a layer thickness check is:

```json
{"axis": {"type": "string", "value": "forward"},
 "target_property": {"type": "propertyReference", "propertySet": "axioval:material",
                     "property": "…total-thickness"},
 "tolerance": {"type": "quantity", "value": 1, "unit": "mm"}}
```

The extent is the whole body's depth along the axis: the highest less the lowest point projected onto it. It is the thickness only where the body is a slab of constant thickness across the axis; a curved wall, or one with a projecting part, measures deeper, so select the straight walls the measure fits (for example by `axioval:material.Kind` `layer-set`). Extents are intervals: a tessellated body measures within its chord deviation, and an axis off the coordinate axes within the rounding of the projection. A verdict needs the whole interval on one side of the bound, widened by a few units in the last place for the binary rounding of decimal coordinates; one straddling it is not evaluated. An absent target property is a finding that names it; a target that is not a length, an unplaced object (IFC `NotPlaced`) and an unmeasurable body are not evaluated. A finding cites the placement, the measurement and the stated length.

`axioval:capability.triangle-count` requires each selected object's mesh to hold at most `maximum` triangles, through `TriangleCountService`. The count is of the mesh the host produced, not of anything the model states: a box extruded from a rectangle counts twelve triangles, and a curved face as many as the host's chord budget made of it, so another host or budget may count differently. A finding on a tessellation of curved faces carries approximate evidence and says that the count depends on the tessellation. A bodiless object counts none; an unmeasured one is not evaluated. It runs as a [template](./templates.md): the measured `triangle_count` judged by the generic range judge against `maximum`, its outside contract unchanged, and a rule bound to it can be forked into the `expression` rule it composes.

`axioval:capability.same-container` requires each selected object to lie in the same nearest containers as every counterpart `counterpart_path` (steps as in `path`) reaches from it. `counterpart_selector` restricts which reached objects count; `container_selector` names the containers, climbed to along the traversal parameters (`relationship` and `direction`, or `path`) exactly as `property-comparison`'s container modes climb, so `follow_chain` does not apply. The two sets must be equal: an object in no container while its counterpart is in one differs too. An object reaching no counterpart has nothing to agree with and passes. A container selector that cannot decide an object leaves every selected object not evaluated; an undecided counterpart leaves the object not evaluated unless a decided one already differs; a refused relationship answer leaves it not evaluated. A finding names each differing counterpart and its containers, and relates them together with the object's own. `property-comparison` cannot express this check: its candidates and targets are compared by property value, and a counterpart's container is no property of either object. With `container_relationship: axioval:derived.same-level` and a `level_property`, the sets are compared as levels: they agree when every container of each is on one level with a container of the other (see [Levels across models](#levels-across-models)).

### Levels across models

`axioval:derived.same-level` is a relationship between containers of different sources that stand for one federated level, such as an architecture model's and an MEP model's storeys at one elevation. It needs no geometry: `property-comparison`'s container modes and `same-container` take it as `container_relationship`, climb to each object's nearest containers in its own model as usual, and then match the containers by the rule's `level_property` (a property reference, usually the storey's elevation or name in `axioval:attributes`, bound through the package vocabulary like any property):

| Identity | Two containers are one level when |
|---|---|
| `axioval:derived.same-level` or `;by=elevation` | their `level_property` lengths differ by at most `tolerance` metres (default 0; one unit conversion's rounding is allowed beyond it), such as `axioval:derived.same-level;tolerance=0.01` |
| `axioval:derived.same-level;by=name` | their `level_property` texts are equal as stated |

A container is always on its own level, and two containers of one source never share one. A container whose `level_property` is absent leaves the pair undecided, and the checked object not evaluated unless another pair already decides it; a value of another kind (not a length, not text) is invalid evidence. `container_relationship` without `level_property`, or the reverse, an unknown identity, an unknown parameter, a negative tolerance, or a tolerance with `by=name` is an invalid declaration. Definitions bound to `property-comparison` or `same-container` must declare `container_relationship` (`string`) and `level_property` (`propertyReference`) as optional parameters. A path step naming `axioval:derived.same-level` is refused like any unknown derivation: the relationship matches containers already reached, never an object's neighbours.

`axioval:capability.space-boundary-coverage` measures how much of each selected space's body surface the space boundaries its source declares for it cover, through `BoundaryCoverageService`. The boundaries and their connection surfaces are source facts the host registers (with IFC, every `IfcRelSpaceBoundary` naming the space, with its `ConnectionGeometry`); the rule selects spaces, never boundaries.

| Parameter | Kind | Meaning |
|---|---|---|
| `minimum_covered_share` | `number` | The least share of the surface, from 0 to 1, the boundaries must cover. |
| `maximum_uncovered_area` | `quantity` | An area: the most of the surface the boundaries may leave uncovered. |
| `maximum_overlap_area` | `quantity` | An area: the most of the surface two or more boundaries may cover together. |
| `plane_tolerance` | `quantity` | A length: how far from a face plane of the body a boundary surface may lie and still count on it. Zero (on the plane, up to a micrometre) without it. |

At least one of the three bounds is declared; each is its own finding on the space. A boundary lying on no face plane of the body covers nothing and is always a finding, relating the element it bounds against, whatever the bounds are: the coverage is measured without it, never silently. An overlap finding names the pairs of boundaries that surely overlap and relates their elements. Areas are intervals: a space turned off the coordinate axes measures within the rounding of its projection, a boundary with curved edges within its chord deviation, and a check whose interval straddles its bound is not evaluated. A space whose body is missing or curved, or with a boundary whose surface cannot be read (IFC: no connection geometry, or a point, curve or volume connection), is not evaluated.

### Classifications

`axioval:capability.unclassified-object` reports every selected object that
the ruleset's classification `classification` (a string, its id) assigns no
class: every row surely does not match. An object whose deciding row cannot
be decided is not evaluated, and a classification the ruleset does not
declare leaves the rule not evaluated. Any other check keys on the class
itself, read as the property `<id>` in `axioval:classification`, or on the
class at a level of a hierarchical classification's tree, `<id>;level=<n>`,
and a `derivedClass` selector selects a class with its descendants (see
[Derived properties](./derived.md#class-trees)).

### Model comparison

`axioval:capability.model-comparison` compares the model of discipline
`base` with the model of discipline `revised`, both sources of the run, and
reports what was added, removed and changed as the rule's findings. Objects
are matched by an identity scheme, a property (a door number, when a
re-export regenerates identities) or both in a declared order; whole
property sets are compared through property enumeration. Objects without
stable identities are matched by their bodies or a related object, and a
revised model older than its base is an error finding. With
`compare_relationships`, each matched object's related objects are
compared per relationship kind (containment, aggregation, voids, fills,
space boundaries, type assignment, group membership and connections),
mapped through the matching: a door moved to another storey is a
`containment` change. A related object that is itself unmatched, or whose
match is undecided, is not evaluated, never a change. Geometry is compared
by bounds, or with `geometry: mesh` by the certified distance between the
two surfaces within `tolerance_metres`, which finds an opening moved inside
unchanged bounds and reports where as witness evidence; between the exact
boundaries where both bodies have one, so curved bodies are compared too.
A distance straddling the tolerance, or a tessellated body without
boundaries, is not evaluated. A
definition bound to `model-comparison` must declare `compare_relationships`,
`geometry` and `tolerance_metres` as optional. See [The comparison as a
rule](./comparison.md#the-comparison-as-a-rule), [Relationship
kinds](./comparison.md#relationship-kinds) and
[Mesh](./comparison.md#mesh).

### Coordinate consistency

`axioval:capability.coordinate-consistency` checks that the sources of a
federation share one coordinate system before their geometry is compared.
Each source's coordinate system (`CoordinateSystemServiceHandle`) is
compared with a reference source's: the one source of discipline
`reference`, or without it the first source in identity order. Compared
are the world frame (origin and orientation), true north, the map
conversion (target system, offset in metres, rotation and scale) and the
site placement, within `length_tolerance` (metres, default 0.001),
`angle_tolerance` (degrees, default 0.01) and `scale_tolerance` (absolute,
default 0). Map offsets stated in different units are compared in metres,
so a model georeferenced in millimetres agrees with one in metres. A map
unit the source leaves to its standard's default (an IFC `IfcProjectedCRS`
without `MapUnit` is in the project length unit) is compared, and a
difference says so (`map offset moved by 1.0000 m (this source's map unit
is the standard's default, not stated)`). A source
that differs is a finding against that source naming every difference
(`` `struct.ifc` does not share the coordinate system of `arch.ifc`: map
offset moved by 1.0000 m ``), citing both coordinate systems.

Nothing unstated is assumed to agree. A statement only one of the two
sources makes (true north, a site), an unknown site placement, a map unit
not known exactly and an unreadable coordinate system leave the source not
evaluated. A source without a map conversion is not georeferenced: it is not
evaluated (`NotRecorded`), or with `require_map_conversion` a finding, the
reference included. One source, or no coordinate-system service, leaves the
rule not evaluated.

Federated geometry is never re-aligned. A host meshes each source in its own
model coordinates, and the set is one frame only for sources whose world
frames agree and whose map conversions agree or are both absent
(`CoordinateConsistency::shares_frame`, with the default tolerances). The
CLI declares every physical object of a source that does not share the
first source's frame, or whose coordinate system cannot be read, unmeasured
with the reason, so a clash or distance between it and another model is not
evaluated rather than measured in the wrong place (see [Several models and
disciplines](./cli.md#several-models-and-disciplines)).

### Information takeoff

`axioval:capability.quantity-takeoff` counts the rule's selection per group
and aggregates stated, measured and related values of each group into the
report table `takeoff` (see [Report tables](./ir.md#tables)). It raises no finding:
quantity takeoff, model value reports and degree-of-detail reports read the
table.

| Parameter | Kind | Meaning |
|---|---|---|
| `group_1` to `group_3` | `propertyReference` | The group keys, outermost first, each declared only after the one before it. A key is read as a rule's [categories](./refinement.md#nested-categories) read a level: on the object, distinct values joined (`Lab, Office`), no value (absent, null, blank) `-`. A derived classification is the property `<id>` in `axioval:classification`, and a level of a hierarchical one `<id>;level=<n>` (level 1 groups by the roots; an object classified above the level has no value there), a type name `Name` in `axioval:type-attributes`, a material a property of `axioval:material`. |
| `group_<n>_path` | `stringList` | Read key `n` on the objects this path reaches instead (steps as in a `related` selector): `IfcRelContainedInSpatialStructure:backward` with `Name` groups by storey. |
| `group_<n>_name` | `string` | The group column's id; `group_<n>` without it. |
| `measure_1` to `measure_8` | `propertyReference` | The property a `property` or `related` column reads, each column declared (by this or its `_kind`) only after the one before it: a stated number, quantity or text, or a value of `axioval:measured` (`area`, `volume`, extents). |
| `measure_<n>_kind` | `string` | The column kind (below); `property` without it. |
| `measure_<n>_aggregates` | `stringList` | Any of `sum`, `min`, `max`, `mean` and `values` (the distinct values, listed), once each; without it `sum` for a `property`, `boundary_area` or `computed` column, `values` for the others. |
| `measure_<n>_name` | `string` | The column name after the aggregate (`sum_<name>`); without it the property's name after its last `.`, split at case changes, lowercase (`t.NetSideArea` is `net_side_area`), `boundary_area` or `profile`. An expanding column prefixes it to each part (`<name>_<part>`); a `property_set` column without it names its parts alone. |
| `measure_<n>_path` | `stringList` | The relationship path of a `related` column, steps as in a `related` selector (alternation and derived relationships included). |
| `measure_<n>_bounding` | `selector` | The bounding elements of a `boundary_area` column: `entityType` of walls, of windows. |
| `measure_<n>_property_set` | `string` | The property set a `property_set` column expands. |
| `measure_<n>_expression` | `string` | The expression of a `computed` column (below). |
| `measure_<n>_unit` | `string` | The unit a `property` or `related` column's values are stated in (`m2`, `EUR/m²`, `1` for a plain number), so an expression can use it: a quantity of another dimension is unreadable, and a plain number is read in this unit only when it counts a currency or is a plain number. A `related` column with a unit sums to an exact zero over no object reached. |
| `across_sources` | `boolean` | One set of groups for the whole project; per source without it. |
| `boundary_plane_tolerance` | `quantity` | How far from a face plane of the space's body a boundary surface may lie and still count on it (`boundary_area`); zero without it. |

Column kinds:

| Kind | Values |
|---|---|
| `property` | `measure_<n>` on the object. |
| `related` | `measure_<n>` on every object `measure_<n>_path` reaches: numbers of one dimension summed (each object reached must state one), texts listed; reaching none, or none stating it, is no value. `IfcRelContainedInSpatialStructure:backward` with `Name` gives the storey's name, `IfcRelDefinesByType:backward` with `Name` the type's. |
| `boundary_area` | The area (m²) of the space's declared boundaries whose bounding element `measure_<n>_bounding` selects, as the space-boundary coverage service measures them (see [Model quality](#model-quality)). A boundary bounding against no element counts in no column; one whose element's selection is undecided may add its area and leaves the space not evaluated; one of the kind on no face of the body leaves the area unknown. |
| `property_set` | Every property of `measure_<n>_property_set`, one column per property found on any member, in property-name order (`FireRating` is `fire_rating`). A part with a text value lists `values`; a numeric one takes the declared aggregates. |
| `profile` | The body's one swept profile, read as `allowed-profile` reads it: `type` and `name` listed, then every dimension found (`width`, `depth`, `web_thickness`, …, in that capability's order), lengths and plane angles taking the declared aggregates. A body that is no single swept profile has no value. |
| `computed` | `measure_<n>_expression` over the other columns of the same member, computed per member and then aggregated like any column (below). |

A `values` aggregate lists the distinct values of the group's members,
sorted and joined (`F30, F90`), numbers with their unit (`0.3 m`), `-` for
none. A possible member's value counts only where it is listed already: one
that may add a value leaves the list `unknown`, as does a value that cannot
be read.

#### Computed columns

A `computed` column is an [expression](./expressions.md), written in its
[text form](./expressions.md#text-form) as a `string` or given as the
expression itself (`{"type": "expression", "value": {…}}`). A name reads
another column of the takeoff (`area`, a profile's `profile_width`; not a
`property_set` column, nor a `computed` one declared after it), and the
column combines them with `+`, `-` (or `−`), `×` (`*`, `·`), `÷` (`/`),
`min(…)`, `max(…)`, parentheses and literals: a number, or a quantity with
its unit written right after it (`42.5 EUR/m²`, `2 m`, `0.5 m2`; a unit runs
to the next space or operator, and every factor after its `/` divides).
Units are the SI units and their common multiples (`mm`, `cm`, `km`, `l`,
`g`, `t`, `min`, `h`, `deg`, `N`, `kN`, `Pa`, `kPa`, `MPa`, `J`, `kWh`, `W`,
`kW`) and a currency, three capital letters (`EUR`). Being an expression,
a column may also branch and read the member itself: `if(…, …, …)`,
comparisons, `and`/`or`/`not`, and, as the expression tree, the member's
properties (derived classes included), derived values and aggregates. Doubling
offices' footprints:

```json
{"measure_2_expression": {"type": "expression", "value": {"kind": "if",
  "branches": [{"when": {"kind": "compare", "operator": "equals",
                         "left": {"kind": "property", "propertySet": "axioval:classification", "property": "space-use"},
                         "right": {"kind": "literal", "value": {"type": "string", "value": "office"}}},
                "then": {"kind": "multiply", "left": {"kind": "parameter", "name": "footprint"},
                         "right": {"kind": "literal", "value": {"type": "number", "value": 2.0}}}}],
  "else": {"kind": "parameter", "name": "footprint"}}}}
```

The expression is parsed and checked when the rule is bound, never run as
code: an unknown name or unit, a column whose unit is not known (declare
`measure_<n>_unit`), `+`, `-`, `min` or `max` over different units (a
length plus an area), two currencies, or a result in no reportable
dimension leave the rule not evaluated as an invalid declaration. The
column's unit is the expression's: a quantity column in its dimension, a
number column for a plain number, or a number column carrying its `unit`
(`EUR`) when it counts a currency.

It is computed for each member by the expression evaluator over the
intervals of its inputs, so the result is an interval sure to hold every
value they allow (rounding outward where a floating-point step rounds), and
the group's `sum` of the computed
values bounds the true total: `cost = area × 42.5 EUR/m²` fills per member
and sums per group. A member missing an input has no value, never zero, and
leaves the group's aggregate unknown; a divisor whose interval holds zero
gives no value either, and the member is reported not evaluated.

Every column states its exactness (`exactness` on the wire, see
[Tables](./ir.md#tables)): `exact` when every value it read is exact, stated
or measured exactly; `bounded` when one is known only within bounds (a
tessellated body, a measured interval).

The table's group columns are the keys' ids, its columns `count` and one
`<aggregate>_<name>` per column (each part of an expanding one) and
aggregate: a quantity in the dimension its values state (a plain number for
unitless values), text for `values`. Rows are keyed by the source (or the
project) and the group, in group order.

Every value is an interval sure to hold the exact one, so every aggregate is
too: a tessellated footprint sums to an interval.

- An object whose selection cannot be decided **may** belong to its group:
  its count is widened by one and its value may be added, never dropped
  (`sum` widens by `[min(0, lower), max(0, upper)]`).
- An object whose group cannot be read (its key unreadable, a
  classification row undecided) may belong to **any** group of its scope,
  and widens every one of them the same way.
- A member whose value is absent, null, not numeric or unreadable makes its
  group's numeric aggregate of that column `unknown`, never zero; its count
  stands.
- `min`, `max` and `mean` of a group that may be empty (no sure member) are
  unknown; a `mean` is bounded over every membership the undecided members
  allow.

Each such object is reported not evaluated, saying which of these applies,
so a table value never hides it. Values of one quantity stated in two
dimensions (or as a number and a quantity) leave that quantity unknown in
every group and the rule not evaluated. Without the geometry service a
measured value needs, the rule reports one `missing_service` per source.
An undeclared key or measure after a gap, an unknown or repeated aggregate,
or column ids that are not tokens or clash (`count` included) leave the rule
not evaluated as an invalid declaration.

```json
"parameters": {
  "group_1": {"type": "propertyReference", "propertySet": "axioval:type-attributes", "property": "Name"},
  "group_1_name": {"type": "string", "value": "type"},
  "group_2": {"type": "propertyReference", "propertySet": "axioval:attributes", "property": "Name"},
  "group_2_path": {"type": "stringList", "value": ["IfcRelContainedInSpatialStructure:backward"]},
  "group_2_name": {"type": "string", "value": "storey"},
  "measure_1": {"type": "propertyReference", "propertySet": "Qto_WallBaseQuantities", "property": "NetSideArea"}
}
```

reports one row per wall type and storey with `count` and
`sum_net_side_area` (m²). A space takeoff lists its boundary areas against
walls, windows and doors with three `boundary_area` columns:

```json
"measure_1_kind": {"type": "string", "value": "boundary_area"},
"measure_1_bounding": {"type": "selector", "value": {"kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true}},
"measure_1_name": {"type": "string", "value": "wall_area"},
"measure_2_kind": {"type": "string", "value": "property_set"},
"measure_2_property_set": {"type": "string", "value": "Pset_SpaceCommon"}
```

gives `sum_wall_area` (m²) and one `values_<property>` column per property
of `Pset_SpaceCommon` found. `axioval report RESULT --section tables` lists the
rows with their groups, and `--csv` exports the table (see
[Command line](./cli.md)).

## Adding a capability

1. Define or reuse canonical schema concepts and parameters.
2. Add failing contract and behavior tests.
3. Implement policy only; put source interpretation in an adapter.
4. Declare all evidence requirements.
5. Add deterministic and missing-evidence tests.
6. Register in the built-in registry.
   Mark every bound that may sensibly differ per checked object `per_object`, and keep population bounds constant.
7. Record legacy parity and cutover in the migration ledger when applicable.
