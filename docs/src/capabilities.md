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

## Table parameters

Many checks are naturally a table: rows of patterns and limits, such as a minimum area per space type or the components required per class. A `table` parameter carries such a table in one rule instead of one rule per row.

The definition declares the table's columns, each with an `id`, a localized `name`, a `kind` and whether it is `required` (the default). A column kind is one of `string`, `textPattern`, `number`, `quantity` (with a `unitDimension`), `integer`, `boolean`, `selector` and `reference`. A table value is a list of rows; each row maps column IDs to cells written as the scalar value of the column's kind (a `textPattern` cell is a `string` value):

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

## Built-ins

`axioval-rules` contains reusable, vendor-neutral implementations. Vendor identity, proprietary format handling, localized vendor text and oracle-only ordering remain adapters in the legacy runtime.

`axioval:capability.property-exists`, `axioval:capability.property-required`, `axioval:capability.property-value-equals`, and `axioval:capability.property-predicate` resolve values through `PropertyResolutionServiceHandle`. `property-predicate` takes exactly one target: `value` (integer), `number`, `quantity`, `text`, `texts`, `boolean`, `date` or `date_time`. A `quantity` is written with a unit and compared in SI; it compares only with a quantity of the same dimension. With a numeric target it accepts `equal`, `not_equal`, `greater_than`, `greater_or_equal`, `less_than` and `less_or_equal`. With `text` it accepts `equal`, `not_equal`, `contains` and `matches`, a regular expression that must match the whole value. With `texts` it accepts `one_of` and `none_of`, and with `boolean` it accepts `equal` and `not_equal`. A `date` or `date_time` target accepts the ordered operators and compares chronologically (see [Dates and date-times](#dates-and-date-times)); `precision` applies to it alone. `is_defined` and `is_undefined` take no target; a blank or null value counts as undefined. Text comparisons are case-sensitive unless `case_sensitive` is `false`. A numeric target may be compared under a `tolerance`, `relative_tolerance` or `decimals` (see [Numeric tolerance](#numeric-tolerance)). A comparison presupposes a value, so an exactly absent property fails every operator except `is_undefined`. A quantity is never compared with a unit-less target and is reported as not evaluated. Failed predicates retain exact source evidence and name the actual value. A present value must be bound to the complete request—including its source-qualified object identity—and carry exact reviewable evidence. A missing value is conclusive only when the provider returns exact request-bound `CompletePropertyAbsenceEvidence`; service absence, partial extraction, cross-object substitution, mismatched responses, or inexact provenance remain not evaluated. Property selectors use the same resolver, so incomplete applicability data cannot silently skip an object. `property-exists` checks exact presence and intentionally does not reinterpret a present typed value. `property-required` applies the stronger required-value contract: exact absence, `null`, or blank text emits an evidence-backed finding, while booleans, integers, finite numbers/quantities, and nonblank text satisfy it. `property-value-equals` accepts a typed `property` reference and boolean `expected` value; a non-boolean resolution is invalid evidence rather than a pass or violation.

`axioval:capability.property-data-type` takes a `property` reference and a `data_type` string. It applies the `property-required` contract and additionally requires the value's type as the source declares it (`Property::data_type`, for IFC `IFCLABEL`, `IFCBOOLEAN`, ...) to equal `data_type`, compared ASCII case-insensitively. A different declared type is a finding. A present value whose type the source does not report is not evaluated, never taken to match. It is what an IDS property facet with a `dataType` and no value translates to.

`axioval:capability.property-value` checks a value against lexical constraints cast to the kind of the value the source resolved: `values` (any of), `patterns` (XML Schema regular expressions matching the whole value), `min_inclusive`/`max_inclusive`/`min_exclusive`/`max_exclusive`, `length`/`min_length`/`max_length`, `total_digits`/`fraction_digits`, and optionally `data_type`. Text compares exactly and case-sensitively and alone takes patterns and lengths; booleans accept `true`/`1` and `false`/`0`; integers take integer literals; decimals take `xs:double` literals and are equal within `|x - v| <= |v|·1e-6 + 1e-6` (the IDS tolerance, boundaries included), while bounds compare exactly. `total_digits` and `fraction_digits` take a number only and count as XML Schema does, leading zeros of the whole part and trailing zeros of the fraction excluded; a decimal is counted on the shortest decimal that reads back as the same double, the form a model states it in (`0.1`, not its binary expansion). A date takes `xs:date` literals and a date-time `xs:dateTime` literals with a UTC offset, for `values` and bounds alike; with `precision` `day` either value takes either literal, compared by the day it states (see [Dates and date-times](#dates-and-date-times)). `precision` on any other value is an invalid declaration. Without `optional`, absence, `null` and blank text are violations; with it, an absent or `null` property passes and any present value is checked. A literal that cannot be cast, a constraint the value's kind does not take, or a pattern that cannot be translated exactly (class subtraction, `\i`/`\c`, block escapes) makes the object not evaluated (`InvalidDeclaration`); a quantity is not evaluated until units are handled.

`axioval:capability.property-comparison` compares a property of candidate objects, picked by the `compared_selector`, with a target for each checked object. It compares booleans, strings, exact integers, finite decimals, canonical quantities with matching dimensions, and dates and date-times (see [Dates and date-times](#dates-and-date-times)); a `factor` other than 1 or a tolerance does not apply to a date, and `precision` applies to nothing else. Missing properties emit evidence-backed missing-information findings; incompatible types or unavailable evidence remain not evaluated. Numbers may be compared under a tolerance (see [Numeric tolerance](#numeric-tolerance)).

`component_mode` says which objects are candidates:

- `checked`: the checked object itself.
- `shared`: the members of a group the checked object belongs to, through one `relationship`.
- `related`: the objects a traversal (`relationship`, or a `path` of steps, as below) reaches from the checked object. `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` compares a wall with the doors and windows filling its openings.
- `same_space` and `same_building`: the other objects that share a nearest container with the checked object. Containers are the `container_selector` objects (spaces, or buildings). An object's nearest containers are found by climbing the declared steps (`relationship` and `direction`, or every step of a `path`) in any order and any number of times, stopping at each container reached, so a building is found from a door through its storey, and a space nested in another is the nearer one. Climbing is always transitive, so `follow_chain` does not apply. An object that reaches no container shares none. A container the selector cannot decide leaves every checked object not evaluated, since it may be the one two objects share. For the geometric variant, the space an element lies in by its position rather than by a stated containment, declare `relationship: axioval:derived.contained-in-space` with `direction: forward` (see [relationships derived from geometry](#relationships-derived-from-geometry)).

The target is exactly one of:

- `target_property`, a property of the checked object;
- a constant: `target_number`, `target_quantity`, `target_text`, `target_boolean`, `target_date` or `target_date_time`;
- `target_texts`, a list of texts or patterns, which only `one_of`, `none_of`, `contains`, `like` and `matches` take, and which is satisfied by any one entry;
- a range, `minimum_number` and `maximum_number` or `minimum_quantity` and `maximum_quantity` (both bounds, of one dimension, minimum not above maximum), which only `between` takes;
- none, for `is_defined` and `is_undefined`.

The operators are `equals`, `not_equals`, `greater`, `greater_or_equal`, `less`, `less_or_equal`, `between` (inclusive at both ends, with the tolerance applied at each), the text operators `contains`, `one_of`, `none_of`, `like` and `matches`, and the presence operators `is_defined` and `is_undefined`. The text operators mean what they mean in property selectors. `like` compares the whole value with a wildcard pattern: `*` stands for any run of characters, none included, `?` for exactly one, and a backslash makes the next character literal (`\*`, `\?`, `\\`). `matches` compares the whole value with a regular expression, anchored at both ends, so `EI\d+` does not match `EI30-T1`. A declared pattern that does not compile is an invalid declaration; one read from `target_property` that does not compile leaves the object not evaluated. Text comparisons respect case unless `case_sensitive` is `false`. `is_undefined` holds for an absent property, null or blank text, `is_defined` for anything else; neither reports a missing property as missing information.

Two more quantifiers compare a single number with the target:

- `count` compares the number of candidates, zero included, and needs no `compared_property`.
- `sum` compares the total of the candidates' compared values. The values must all be integers, all numbers, or all quantities of one dimension; otherwise the checked object is not evaluated. A candidate without the value gets its missing-property finding, and no verdict is drawn from the partial total.

`between` gives a count or a sum a minimum and a maximum in one rule. Text and presence operators do not apply to a count or sum.

With `category_property`, every finding of a checked object starts with that object's value of the property in brackets, such as `[F90] `, and cites it, so findings can be grouped by fire rating, discipline or similar. An absent, null or blank value adds no category. The property is read only for objects with findings; when it cannot be read, the object is not evaluated rather than reported uncategorised.

A definition bound to `property-comparison` must declare every parameter above, including the optional `path`, `container_selector`, `case_sensitive`, `minimum_number`, `maximum_number`, `minimum_quantity`, `maximum_quantity`, `target_date`, `target_date_time`, `precision` and `category_property`.

`axioval:capability.free-floor-circle` and `axioval:capability.free-floor-rectangle` check whether each selected spatial scope can contain an exact supported vertical shape. Circle parameters are `diameter_metres` and `height_metres`; rectangle parameters are `width_metres`, `length_metres`, and `height_metres`, all in canonical metres, and an `orientation`. The rectangle `orientation` must be `any` (every rotation counts). A rule without it, or with an orientation that no service can yet ground in a frame (such as `fixed-to-room` or `fixed-to-door`), is reported not evaluated as an invalid declaration, because a rectangle placement is only meaningful once it says which rotations it allows. Each request covers every other project object as a candidate obstacle and requires exact whole-base support on the selected scope with zero hidden gap. A complete exact no-placement proof emits the shape-specific `NO_FREE_FLOOR_SPACE_*` finding; missing services, backend outages, or invalid/incomplete evidence emit not-evaluated outcomes instead.

`axioval:capability.slab-contact` requires at least `minimum_contact_ratio` of each selected subject's face to rest on something on its `contact_side` (`above` or `below`), measured through `ContactServiceHandle` with the `maximum_gap_metres`, `maximum_intersection_metres` and `minimum_polygon_area_square_metres` tolerances. What the face may rest on is the optional `counterparts` selector; without it every other project object is a candidate. The capability resolves the selector and sends the candidates in the `ContactRequest`, and the adapter measures exactly those: an object outside them neither supports the face nor blocks the measurement, while an unmeasured candidate makes the subject not evaluated. A counterpart the selector cannot decide leaves a shortfall not evaluated, since it might support the face; a pass stands, because more candidates can only add contact. `skip_top_storey` and `skip_bottom_storey` leave out subjects on the highest or lowest storey of their source. Storeys are the `storey_selector` objects, ordered by their `Elevation` attribute (a length), and a subject's storey is the one the declared traversal (`relationship` or `path`, as below) reaches from it. Skipping needs both. Every storey needs a known elevation and every subject exactly one storey; otherwise the subjects concerned are not evaluated rather than guessed at. A shortfall is graded by how far the ratio falls below the minimum, no contact at all by the distance to the nearest candidate.

`axioval:capability.clash` and `axioval:capability.distance` check selected subjects against a selector-valued `counterparts` group through `ProximityServiceHandle`, after the engine's complete broad-phase candidate search. Clash puts each pair in one class: a duplicate (surfaces within `duplicate_tolerance_metres` of each other), one body inside the other, or an intersection (a witnessed penetration beyond `penetration_tolerance_metres` whose extent exceeds `horizontal_tolerance_metres` along both plan axes and `vertical_tolerance_metres` in height). `report_duplicates`, `report_containment` and `report_intersections` switch the classes off. A clearance clash is a separation below the optional `clearance_metres`. `exclude_paths` (relationship paths, steps separated by spaces) and `exclude_same_layer` leave out pairs that reach a shared target or share a presentation layer. A definition bound to `clash` must declare all eleven parameters. Distance takes a `mode`: `nearest` (the default) bounds the nearest counterpart with `minimum_metres` and/or `maximum_metres`, `none_closer_than` keeps every counterpart at least `minimum_metres` away, and `at_least` requires `count` counterparts within `maximum_metres` (and no nearer than an optional `minimum_metres`). A `projection` measures in space (`minimum_3d`, the default), in plan (`horizontal`), between bodies above one another (`vertical`, with an optional `footprint_offset_metres`), or as overlap in plan (`plan_overlap`). The traversal parameters (`relationship` or `path`) scope counterparts to those sharing a container with the subject. Distances are intervals; a counterpart that could fall either side of a bound counts as unknown, and a verdict is given only when the unknowns cannot change it. A definition bound to `distance` must declare `mode`, `count`, `projection`, `footprint_offset_metres` and the traversal parameters as optional. Findings on tessellated geometry carry inexact evidence and say so. See [Clash, interference and distance](./clash.md).

`axioval:capability.clash-matrix` judges each pair with the tolerance profile and severity of one cell of a `cells` table. A cell keys both sides of the pair (`subject_*`, `counterpart_*`) by discipline pattern, by patterns over the text properties `key_1` to `key_3` name, and by a selector, and gives the `clash` tolerances and switches as columns, with an optional `severity` and `label`. The single most specific cell applies, either way round unless `symmetric` is false; tied cells or an unreadable category leave the pair not evaluated, and a pair no cell covers is ignored or, with `report_unmatched`, reported. Same-system exclusion (`exclude_same_system`, through `system_path`) is on by default and same-layer exclusion off. A definition bound to it must declare all twelve parameters. See [Clash matrix](./clash.md#clash-matrix).

`axioval:capability.horizontal-guard` checks that the exposed edges of the selected walking surfaces are guarded, through `GuardServiceHandle`: by barriers tall and close enough, or by a short fall onto a landing wide enough to stand on, and not defeated by a climbable object beside the barrier. The rule's selection is the walking-surface profile. The optional selectors `barrier_selector`, `landing_selector` and `climbable_selector` name the objects that may play each role; their resolved sets travel in the `GuardSearch`, and only their members are counted for that role, so a cupboard along an edge is not taken for a railing. An absent selector leaves its role open to any nearby body. An object a role selector cannot decide makes the rule not evaluated, since it might be the barrier that guards an edge. A definition bound to this capability must declare all three selectors, as optional `selector` parameters.

`axioval:capability.external-wall-validation` compares the objects a model declares external with the objects on the envelope geometry derives, through `EnvelopeMembershipServiceHandle`, and reports each selected object that disagrees against itself. Which objects bound the envelope is the rule's choice, carried in every `EnvelopeMembershipRequest`; the service derives around exactly those, and the host declares none of its own.

| Parameter | Kind | Meaning |
|---|---|---|
| `derivations` | `stringList` | Required. `all-spaces`, `gross-area-groups`, or both. Each is measured and reported on its own; every finding and not-evaluated outcome names its derivation. An unknown or repeated name is an invalid declaration. |
| `bounding_selector` | `selector` | The objects the `all-spaces` envelope is derived around, such as every `IfcSpace`, or the spaces a `related` selector finds in one zone. Required by `all-spaces`. |
| `gross_area_group_selector` | `selector` | The gross-area groups, such as `IfcZone`s with a given name. Required by `gross-area-groups`, together with the path. |
| `gross_area_group_path` | `stringList` | Steps from each group to its members, as in `path`; with IFC, `IfcRelAssignsToGroup:forward`. The members of every selected group, together, bound the `gross-area-groups` envelope. |

A derivation whose bounding input is not declared is an invalid declaration of the whole rule, never measured around nothing. A bounding or group selector that cannot decide an object leaves that derivation not evaluated, since the undecided object might bound the envelope; so does a selector that selects nothing, groups with no member, or a refused relationship answer along the path. The other derivation is still measured. A bounding object is the envelope's inside and takes no part in the comparison. An object the model declares neither external nor internal, or whose body could not be measured, is not evaluated, never read as internal. A definition bound to this capability must declare all four parameters, the last three as optional.

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

### Property selectors

A `property` selector (in a rule's selection or any selector-valued parameter) resolves one property exactly, as `property-predicate` does, and compares it with its `value` under an `operator`:

| Operator | `value` | Selects when the resolved value |
|---|---|---|
| `exists` | none | is present, even `null` or blank. |
| `equals`, `notEquals` | boolean, integer, number, quantity, string, enum, reference, date or dateTime | equals, or does not equal, the value. |
| `lessThan`, `lessThanOrEquals`, `greaterThan`, `greaterThanOrEquals` | integer, number, quantity, string, date or dateTime | orders against the value; strings by code point, dates chronologically. |
| `matches` | string | matches the regular expression as a whole: `EI\d+` selects `EI30`, not `EI30-T1`. |
| `like` | string | matches the wildcard pattern as a whole: `*` is any run of characters, `?` one character, and `\` makes the next character literal. |
| `contains` | string | contains the text. |
| `oneOf`, `noneOf` | string list | is, or is not, one of the texts. |

Integers and numbers compare with each other. A `quantity` value is written with a unit (`m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`) and compared in SI with a quantity of the same dimension, allowing the few units in the last place one unit conversion can introduce. Text comparisons are case-sensitive unless `caseSensitive` is `false`, which folds both sides (Unicode lowercase) and makes `matches` and `like` case-insensitive. `trim: true` drops surrounding whitespace from the resolved text first. Both options apply to text comparisons only. `precision: day` applies to a `date` or `dateTime` value only (see [Dates and date-times](#dates-and-date-times)).

A list value, such as the presentation layers of an object, is compared element by element under a `quantifier`: `any` selects when at least one element satisfies the operator, `all` when every element does. `all` never holds vacuously, so an empty list satisfies neither. A scalar value under a quantifier counts as a list of one. "Every layer is agreed" is `oneOf` with `quantifier: all`, "at least one layer is agreed" the same with `any`, and "no layer is forbidden" `noneOf` with `all`. An element that cannot be compared (an integer against text) leaves the object not evaluated unless the other elements already decide it.

The selector fails closed:

- an exactly absent property, or a `null` value, matches every operator but `exists` as "no", as a comparison presupposes a value;
- a value of another type than the operator compares (text against an integer, a quantity against a unit-less number or one of another dimension, a boolean with `contains`) leaves the object not evaluated (`InvalidEvidence`), never silently out of the selection; `notEquals` and `noneOf` are no exception;
- a list value compared without a `quantifier` is not evaluated (`InvalidEvidence`), except by `exists`;
- a date-time compared with a `date` value without `precision: day` is not evaluated (`InvalidEvidence`), as is text compared with a date;
- a selector whose `value` does not fit its operator, an unknown unit, an invalid pattern, a text option on a non-text comparison, a `precision` on anything but a date comparison, or a `quantifier` on `exists` is an invalid declaration.

### Dates and date-times

A date (`PropertyValue::Date`) is a calendar day; a date-time (`PropertyValue::DateTime`) is an instant with the UTC offset it was stated in. Package literals are `{"type": "date", "value": "2026-09-27"}` and `{"type": "dateTime", "value": "2026-09-27T10:00:00+02:00"}`; one that is not a real day, or a date-time without an offset, is refused when the package is read. `property-predicate`, `property-value`, `property-comparison` and property selectors compare them the same way:

- a date with a date by day;
- a date-time with a date-time as instants, whatever their offsets: `10:00:00+02:00` equals `08:00:00Z`;
- a date-time with a date only when the rule states `precision` `day` (a `precision` parameter, or `precision: day` on a selector). Day precision reads every date-time as the calendar day it states in its own offset, not the UTC day: `2026-09-27T22:30:00-05:00` is on the 27th. Two date-times at day precision compare their stated days.

Without day precision a date-time neither precedes nor follows the day it falls on, so that pair is not evaluated instead of guessed. Parameter names stay snake_case like every capability parameter (`date_time`, `target_date_time`), while the value tags are `date` and `dateTime`. Property selectors support dates in full: `date`/`dateTime` values with `equals`, `notEquals` and the ordered operators, `precision: day`, and list values under a `quantifier`. Not yet supported: date bounds for `between` in `property-comparison` (its ranges are numbers or quantities) and date columns in `table` parameters. Dates never compare with text, numbers or quantities, and take no tolerance; `unique-value` and `consistent-value` treat two date-times as one value when they name the same instant.

### Related selectors

A `related` selector selects an object by the objects a relationship `path` reaches from it. The `path` is written as the `path` parameter of the relationship-scoped capabilities: steps `Relationship` or `Relationship:direction` (`forward`, the default, `backward` or `either`), each optionally followed by `+` to take it one or more times (`IfcRelAggregates:backward+` reaches every whole above a part), walked one after another through `RelationshipSelectionServiceHandle`, never reaching the object itself. The reached objects are tested against the nested `selector` under a `quantifier`:

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

### Discipline selectors

A `discipline` selector selects the objects of the sources that play a discipline in the check, such as `architecture` or `structure`:

```json
{ "kind": "discipline", "value": "structure" }
```

IFC carries no discipline, so the host declares one per source (the CLI's `--model PATH:DISCIPLINE`, or `EvidenceSession::with_discipline`); the engine keeps it next to the source's snapshot and installs it per run as `SourceDisciplines`, which any capability may read. The value is a lowercase token: 1 to 64 ASCII letters, digits, `-` or `_`, starting with a letter or digit. A package with any other value does not load. Names compare exactly and the engine attaches no vocabulary to them; a project agrees on its names as it agrees on its rules.

Every object of a source matches or none does. An object whose source declares no discipline is not evaluated, never a non-match, so a discipline-scoped rule cannot pass over a model nobody classified; the outcome is reported once per rule and source (`not-recorded`), not once per object. `clash-matrix` keys its cells by discipline patterns directly; a `clash` rule between two disciplines selects its subjects and its `counterparts` with one discipline each:

```json
"applicability": { "kind": "allOf", "operands": [
  { "kind": "entityType", "objectType": "…wall", "includeSubtypes": true },
  { "kind": "discipline", "value": "architecture" } ] },
"counterparts": { "type": "selector",
  "value": { "kind": "discipline", "value": "structure" } }
```

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
| `selector-conformance` | Each selected object satisfies the `requirement` selector. An agreed list is an `anyOf` of `allOf` rows. An object none of whose consulted properties has a value gets its own "no value" finding. Other failing objects are grouped by their combination of values: one finding per unknown combination, against the first object, naming the values and relating the others. `message` replaces the default text and is followed by the values. Every finding cites the property facts consulted. A layer agreement ("walls of this type must be on one of these layers") selects by class and type in the rule's selection and requires `axioval:presentation.Layer` `oneOf` the agreed layers with `quantifier: all`; over a source without layers it is not evaluated once for that source. |
| `unique-value` | `property` does not repeat among the selected objects of one source, or of the project with `across_sources`, or among objects reaching the same related objects through `relationship` (for example one storey). Text is trimmed and compared ignoring case unless `trim` or `case_sensitive` says otherwise. A missing value is a finding unless `require_value` is `false`. Numbers may be compared under a `tolerance`, `relative_tolerance` or `decimals` (see [Numeric tolerance](#numeric-tolerance)). |
| `consistent-value` | Objects of one kind that share a `key` value share their `value` too. An absent value is a value of its own. Each member of a disagreeing group is reported, naming the others. Objects without a key form one group of their own, so a missing key is reported only when those objects disagree. |
| `object-count` | The rule's selection holds between `minimum` and `maximum` objects in each source, or in the whole project with `across_sources`. With neither bound it is an existence check: at least one. A violation is reported against the source or the project, not an object, so an empty selection reads "no object matches the selection" instead of passing. The finding relates the objects found and cites the facts that selected them. Undecided objects count as unknown: the scope is judged only when they cannot change the verdict, and is otherwise not evaluated together with each undecided object. Per source, a project with no objects at all is not evaluated. |
| `related-count` | Each anchor has between `minimum` and `maximum` related objects that `related_selector` picks. Without `relationship`, the anchor's whole source is counted. Undecided related objects are counted as unknown, and the anchor is judged only when they cannot change the verdict. |
| `relative-count` | At each anchor, or in each group of objects sharing a `group_property` value, `provided / provided_unit` stands in `operator` (`equal`, `not_equal`, `greater`, `at_least`, `less`, `at_most`) to `required / required_unit`, computed in exact integers. |
| `name-sequence` | The members of each anchor, ordered by the numeric `order` property, carry whole-number names from `first` stepping by `increment`. A name is a number only when it is exactly one. A number below `first` and a number out of order are separate findings; like a name that is not a number, a number below `first` does not interrupt the sequence. A member without an order value leaves the anchor not evaluated. |
| `numbering-consistency` | The numbers `pattern` (an XML Schema pattern over the whole value with exactly one group of digits) reads from `property` agree within each scope, formed as in `unique-value`. With `prefix_length`, their first digits match: objects departing from the predominant prefix are reported, and every object when none predominates. With `gap_free`, the distinct numbers step by one, and the objects above each gap are reported. At least one check must be declared. A value that is missing or does not match is not evaluated, and a gap an unreadable object could fill is not evaluated rather than reported. |
| `manual-issue` | Raises the declared `title`, `category` and `description` once per rule, for checks owed by hand: the finding is against the first selected object and relates the others. A selection that decidedly picks nothing raises the check once for the project, saying that no object matched. |

| `level-spacing` | Each level's height, the rise of its `order` length to the next level up, lies within `minimum` and `maximum` and, with `consistent`, matches the prevailing height within `tolerance`. The highest level is not evaluated unless `ignore_highest`, or unless `content_path` measures it from geometry (see [Storey metrics](#storey-metrics)); `ignore_lowest` leaves out a basement. With `space_selector`, each level's spaces must be as high as the level within `space_tolerance`. |

Deliberate differences from other checkers:

- `name-sequence` orders members only by the declared `order` property. When a member has no order value (a storey without `Elevation`), the anchor stays not evaluated rather than falling back to placement height: the engine exposes no exact source-neutral service for an object's placement height, and a storey has no body whose extent could stand in for it.
- `property-required` reports each property in its own rule, so an object missing several identity fields gets one finding per field. Combining them into one finding per object would add a parameter every `property-required` definition must then declare; grouping findings per object is left to the report consumer.

Most names and numbers these rules read are attributes of the object, not properties; see [attribute sets](./ir.md#attribute-sets). Quantities are written with a unit: `m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`. `²`, `³` and `°` are accepted too.

A traversal is either one `relationship` (with `direction` and `follow_chain`) or a `path` of steps walked in order, each `Relationship` or `Relationship:direction`. For example, `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` goes from a wall to the doors and windows filling its openings. A step ending in `+` is taken one or more times, reaching every object along the relationship's chain: `IfcRelAggregates:backward+` reaches every whole above an object (its assembly, storey, building and site), where `follow_chain` would apply to the whole traversal. The anchor is never among the objects reached, even around a cycle.

### Relationships derived from geometry

Every traversal parameter also accepts a derived relationship identity, answered from geometry rather than from what the model states: `axioval:derived.contained-in-space` (an element to the space containing it, or the nearest within `horizontal` and `vertical` tolerances), `axioval:derived.adjacent-space` (a door, window or opening to the spaces on each side of it) and `axioval:derived.overlapping-group-space` (a space to the larger spaces covering at least `ratio` of its footprint). Tolerances follow the name, for example `axioval:derived.contained-in-space;horizontal=0.3;vertical=0.5`. Edges run from the element to the space, so counting components per space is `related-count` from each space `backward`. The service needs a geometry adapter; see [typed host services](./services.md). An undecided derivation leaves the anchor not evaluated, never counted as unrelated.

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
| `area-ratio` | At each anchor, the summed footprints of `numerator_selector` over those of `denominator_selector` (or the anchor's own footprint) lie within `minimum` and `maximum`. `numerator_property` / `denominator_property` take a population's areas from an area-quantity property instead, e.g. glazing areas. `numerator_derivation` `light-area` derives each numerator member's light-transmitting area by a fallback (below). |
| `plan-coverage` | Each subject's footprint lies within one `candidate_selector` object by at least `minimum_ratio`, e.g. a space within a fire compartment. How much of an element the union of several counterparts leaves uncovered is `counterpart-coverage` (below). |
| `plan-area` | Each selected object's footprint lies within `minimum` and `maximum` square metres, e.g. a space of at least 8 m² or a fire compartment (a zone, measured as the union of its members) of at most 400 m². With `member_selector`, the summed footprints of the members each anchor reaches lie within the range instead, e.g. the space area of each storey. |

All bounds are inclusive, and `area-ratio` and `plan-area` need at least one of `minimum` and `maximum`. Areas are intervals: a tessellated body measures within a bound derived from its chord deviation. A verdict needs the whole interval on one side of a bound, so an area that straddles one is not evaluated rather than judged from a midpoint.

`area-ratio` and `plan-area` take `measure`: `footprint` (the default) or `facade`. `facade` measures each object's outward-facing surface through `FacadeAreaService` instead of its footprint (see [typed host services](./services.md)); a population with a declared area property still reads the property. A facade area may be zero, so `plan-area` does not treat an empty facade as a missing body.

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

This is a numerator mode of `area-ratio` rather than a capability of its own: the traversal, the undecided-member rule, the denominator and the interval judgement are all `area-ratio`'s, and only where each numerator member's area comes from differs. A separate capability would have to repeat all of them, and capabilities cannot feed one another. A definition bound to `area-ratio` declares `numerator_selector` (required), `denominator_selector`, `minimum`, `maximum`, `numerator_property`, `denominator_property`, `measure`, `numerator_derivation`, `empty_numerator_finding`, the light-area parameters above and the traversal parameters.

### Property requirements

`property-requirements` checks each selected object against a `requirements` table: which properties it must, may or must not carry, and which values they may hold. One rule carries a whole requirement sheet.

| Column | Kind | Meaning |
|---|---|---|
| `applies_to` | `selector` | Objects the row applies to, such as `{"kind": "entityType", "objectType": "IfcWall", "includeSubtypes": false}` for one exact class or `true` for the class and its subtypes. Blank applies the row to every selected object. |
| `property_set` | `textPattern` | The property set's exact name. Blank asks for the property by name in any set. |
| `property` | `textPattern` | The property's exact name. |
| `requirement` | `string` | `required`, `optional` or `forbidden`. A row declares `requirement` or `state`. |
| `state` | `string` | `include` (the row's statement must hold), `exclude` (it must not hold) or `ignore` (the row is skipped). On a `requirement` row only `include` (the row as written) and `ignore` are allowed. |
| `presence` | `string` | The statement of a `state` row about presence: `defined` (present, null included), `undefined` (exactly absent), `empty` (present but null, blank or an empty list) or `not-empty` (present with a value). It takes no value condition. |
| `value_like` | `textPattern` | A whole-value wildcard pattern the value must match. |
| `one_of` | `string` | Allowed values separated by `\|`; a backslash escapes the next character (`a\\|b` is one value). |
| `one_of_like` | `string` | Allowed whole-value wildcard patterns separated by `\|` (`Exist*\|New`); `\|` is a literal bar and `\*`, `\?` literal wildcards. |
| `contains` | `string` | Text the value contains: a substring of a text value, or an element of a list value (text, booleans and integers compared as text). |
| `minimum`, `maximum` | `number` | Inclusive numeric bounds, in `unit`. |
| `unit` | `string` | The bounds' unit, as for quantity parameters (`mm`, `m2`, `l`, `deg` …). Without it, the bounds compare with unit-free numbers only. |
| `per` | `string` | Divides the value before it is bounded: `measured-area` (the measured plan footprint through `PlanAreaService`, in m²), `stated-area` (the area quantity `area_property` names) or `stated-volume` (`volume_property`, in m³). |
| `decimals` | `integer` | Rounds the value (or quotient), read in `unit`, half away from zero to 0–15 decimals before it is bounded, as the rule-level `decimals` of the comparison capabilities does: `299.6 mm` rounded to 0 decimals meets `minimum` 300. |

The rule-level `case_sensitive` (default `true`) applies to `value_like` and `one_of`. Every row that applies to an object is checked, through the shared row matcher with all rows selected: an `applies_to` selector that cannot be decided leaves the object not evaluated. Each failing row is one finding against the object, and its message begins with the result:

- `missing property`: a `required` property is exactly absent.
- `missing value`: a `required` property is present but null, blank or an empty list.
- `forbidden property present`: a `forbidden` row without a value condition, and the property is present with any value, null included.
- `forbidden value`: a `forbidden` row with a value condition, and the value meets it.
- `wrong value`: a `required` or `optional` value that does not meet the row's conditions.

An `optional` property may be absent, null or blank. Conditions must all hold. `value_like`, `one_of` and `one_of_like` compare text, booleans (`true`/`false`) and integers as text; any other value is not evaluated. A range compares integers and decimals when the row has no `unit`, and quantities of the unit's dimension in canonical SI units when it has one; any other pairing is not evaluated, never a pass. A list value must meet the conditions with every element, and a forbidden value is present when any element meets them; `contains` alone reads the list as a whole. A divided value is an interval when the measured footprint is: a quotient straddling a bound, a footprint that is not positive, or a missing stated area or volume is not evaluated.

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

A definition bound to `property-requirements` declares `requirements` with all fifteen columns above (only the column set, kinds and optionality are compared) and the optional `case_sensitive`, `area_property`, `volume_property`, `group_by_value` and `category_property`.

Names are resolved exactly. The property service answers requests for one named property and cannot list an object's property sets or properties, so a row whose set or property name contains a wildcard (`Pset_*Common`), or that names a set without a property (set presence), cannot be decided: each such row is reported once as not evaluated for the rule (`MissingService`), and the other rows are still checked. A backslash-escaped `*` or `?` is part of an exact name. Missing property sets are therefore reported as missing properties. An unknown requirement, bounds in the wrong order, a `unit` or `per` without bounds, `stated-area` without `area_property`, an empty `one_of` value, or a value condition on a set-only row is an invalid declaration.

### Storey metrics

Storey checks are compositions of the capabilities above; no capability is specific to storeys. With IFC, storeys hang off the building through `IfcRelAggregates`, spaces off each storey through `IfcRelAggregates`, and elements through `IfcRelContainedInSpatialStructure`.

| Metric | Rule |
|---|---|
| Storey height | `level-spacing` from each building over its storeys, `order` the `Elevation` attribute, bounded by `minimum` / `maximum` or `consistent`. |
| Height of the highest storey | The same rule with `content_path: ["IfcRelContainedInSpatialStructure"]` (and optionally `content_selector`, e.g. walls): the highest top of the storey's contents, through `VerticalExtentService`, less its elevation. The height is an interval when a content is tessellated. A highest storey with no contents, an undecided content or an unmeasurable one is not evaluated. |
| Space height matches its storey | The same rule with `space_selector` (spaces), `space_path: ["IfcRelAggregates"]` and `space_tolerance`: each space's rise from bottom to top must lie within the tolerance of its storey's height, highest storey included when it is measured. A space is not evaluated when its extent is unknown or the difference straddles the tolerance. |
| Facade area per storey | `plan-area` from each storey with `measure: facade`, `member_selector` the external walls (`IsExternal` true in `Pset_WallCommon`) and `relationship: IfcRelContainedInSpatialStructure`. |
| Window-to-wall ratio per storey | `area-ratio` from each storey with `measure: facade`, `numerator_selector` the windows and `denominator_selector` an `anyOf` of the external walls and the windows: walls are measured with their openings cut out, so gross wall area is the walls' facade plus the windows'. Select the windows on external walls with a `related` selector through `IfcRelFillsElement` then `IfcRelVoidsElement` backwards when internal windows exist. |
| Window-to-wall ratio per building | The same rule from each building, with `path: ["IfcRelAggregates:forward", "IfcRelContainedInSpatialStructure:forward"]`. |
| Net-to-gross ratio | `area-ratio` from each storey: spaces (their footprints, or `numerator_property` such as `Qto_SpaceBaseQuantities.NetFloorArea`) over the storey's own `denominator_property`, e.g. `Qto_BuildingStoreyBaseQuantities.GrossFloorArea`, through `IfcRelAggregates`. |
| Empty-area ratio | The same rule with the empty (void) spaces as the numerator, selected by name, type or classification. |
| Storeys with no elements or no external walls | `related-count` from each storey through `IfcRelContainedInSpatialStructure`, `minimum: 1`, with `related_selector` everything or the external walls. |
| Compartment area against its gross-area group | Where the model assigns the compartments to the group (`IfcRelAssignsToGroup`), `area-ratio` from the group over its compartments with `denominator_selector` omitted, so the denominator is the group's own footprint (the union of its members), bounded around 1, e.g. `minimum: 0.95` and `maximum: 1.05` for 5 %. Whether a compartment lies within a group at all is `plan-coverage`. |

A tabular report alongside the findings (one row per storey with these values) is not part of the report contract yet; each metric reports only its findings and not-evaluated outcomes.

### Table allocation

`table-allocation` assigns each selected object to exactly one row of its `rows` table and then checks every row's objects together: "two offices and one meeting room per storey", "an archive of 30 m² ± 1 m²". One rule per row cannot express this, because an object would count in every row it matches.

| Column | Kind | Meaning |
|---|---|---|
| `key_1`, `key_2`, `key_3` | `textPattern` | Pattern over the property the same-named rule parameter names. |
| `label` | `string` | How findings name the row; otherwise by its number and patterns. |
| `count` | `integer` | Exactly this many objects are assigned to the row. |
| `area` | `number` | Their summed plan area, in square metres. |
| `area_tolerance` | `number` | Allowed deviation from `area`, in square metres (0 by default). |

All columns are optional. The keyed properties are rule parameters, not cells: `key_1` might be `{"type": "propertyReference", "propertySet": "Pset_SpaceCommon", "property": "Reference"}` and `key_2` the name, and each row fills the pattern columns it tests. A row matches an object when every key cell it fills matches the object's value, compared case-sensitively unless `case_sensitive` is false; a row with no key cell matches everything, which makes a catch-all last row. A key value that is absent or null matches no pattern, not even `*`; one that is not text, or cannot be read, leaves the rows testing it undecided. A key cell whose parameter is not declared, a negative count, area or tolerance, and a tolerance without an area are invalid declarations.

`mode` is `first` (the default), the first matching row in declared order, or `most_specific`, the matching row with the most literal pattern characters, the specificities of a row's keys adding up. Matching goes through the shared row matcher above, so it fails closed: an object whose row an undecided key could change, or whose most specific rows tie, is not evaluated.

Rows are judged per group. With `anchor_selector`, each anchor is a group of the objects it reaches through the traversal parameters (or all of its source's objects without them), such as every storey through `IfcRelAggregates`; its outcomes go against the anchor, and an object that no anchor reaches is not evaluated. Without anchors, each source is a group, or the whole project with `across_sources`, and outcomes go against the source or the project; a traversal without `anchor_selector` is an invalid declaration.

- An object no row matches is a finding of its own, the "extra", naming its key values.
- A row that matched no object in a group is a finding, unless its `count` is 0.
- A row's `count` must equal the number of objects assigned to it.
- The summed area must lie within `area` ± `area_tolerance`. Areas are measured footprints through `PlanAreaService`, as in `plan-area`, or the area quantity `area_property` names. Measured areas are intervals, and a sum straddling a bound is not evaluated; an object with an empty footprint has no body and leaves its row's area not evaluated.

Objects that may belong to a row without being decided (a tie, an undecided key, or an undecided selection) make its count and area not evaluated, unless the row already exceeds its count or area, since they could only add to it. A finding relates the objects assigned to the row.

### Group composition

`group-composition` requires each selected group to hold a multiset of members: "two bedrooms, one kitchen and one bathroom per apartment". `related-count` checks one entry at a time and counts a member for every entry it fits; this capability allocates members across entries.

The rule's selection are the groups. Each reaches its members through the traversal parameters, which are required: `relationship` (such as `IfcRelAssignsToGroup`, or the derived `axioval:derived.overlapping-group-space` walked `backward`, from a group space to the spaces it covers) or a `path`. `member_selector` restricts which reached objects are members, every one by default. The `requirements` table lists the member entries:

| Column | Kind | Meaning |
|---|---|---|
| `key_1`, `key_2`, `key_3` | `textPattern` | Pattern over the member property the same-named rule parameter names, as in `table-allocation`. |
| `group` | `textPattern` | Pattern over the group's `group_key` property; the row applies only to groups it matches. |
| `label` | `string` | How findings name the entry; otherwise by its number and patterns. |
| `count` | `integer` | Required: how many members the entry takes. |

A member fits an entry when every key cell the entry fills matches its value (absent or null matches no pattern); a row with no key cell fits every member. A member may fit several entries but fills one place, so members are allocated to places by a **maximum bipartite matching**: as many places as possible are filled, whatever order members and rows come in. A bedroom that fits both `*room` and `Bed*` goes to `Bed*` when a living room can take only `*room`, where filling entries in declared order would strand the living room.

Maximum allocations need not be unique, so outcomes are reported only as far as all of them agree:

- An entry that every maximum allocation leaves short is a finding naming how many of its members it has and how many are missing. Entries that compete for the same members, when any of them could be the short one, are reported together with their joint shortfall ("row 1 `bedroom` and row 2 `sleeping room` together have 1 of 2 required member(s); 1 missing"), never blamed one by one.
- Members beyond the places they fit are a surplus, reported per set of full entries they compete for ("row 1 `bedroom` takes 2 member(s), but 3 fit; 1 surplus"). A member that fits no entry is a surplus finding of its own, naming its key values.

Findings go against the group and relate the members concerned. With `group_key`, one table carries every kind of group: a row filling `group` applies only to groups whose `group_key` value it matches, and a row without it applies to every group. A group that no row with a `group` cell matches is a finding ("no requirement row matches the group"), and its members are not judged. A `group` cell without `group_key`, `group_key` with no `group` cell, a key cell without its property, a missing or negative `count`, an empty table and a missing traversal are invalid declarations.

Nothing undecided is guessed: a reached object whose membership in `member_selector` is undecided, a member whose key cannot be read or is not text when a row tests it, or a group whose `group_key` cannot be read leaves the group not evaluated, and the member is reported once as not evaluated. A group whose members cannot be walked is not evaluated.

With `ungrouped_selector`, each object it picks that no selected group reaches is a finding, "in no group". It is not evaluated when a group that could not be walked, or whose own selection is undecided, might reach it, or when its selection by `ungrouped_selector` is undecided. Spaces that must belong to a group are thus checked in the same rule as the groups, over the same relationship; a separate `related-count` rule walking the relationship backwards from each space checks the same where one rule per concern is preferred.

### Keyed limits

`keyed-limit` looks a limit up in a table keyed by facts about each selected object, then checks one of its quantities against it. A fire compartment's area limit depends on the building's fire class, the compartment's use class and whether its storey is sprinklered; one rule carries the whole table.

Up to four keys, `key_1` (required) to `key_4`, are property references. A key is read from the object itself, or, with `key_<n>_path` (steps as in the `path` parameter), from the objects that path reaches from it: `IfcRelContainedInSpatialStructure:backward` then `IfcRelAggregates:backward` climbs from an element to its building. A key's value is matched as text: a string as stated, a boolean as `true` or `false`, an integer in decimal. It is unknown when it is absent, null, blank or of another type, when the path reaches no object, or when the reached objects state different values; a refused property or relationship answer leaves the object not evaluated outright.

The `limits` table has the optional columns `key_1` … `key_4` (text patterns, as in [Table parameters](#table-parameters); a blank cell accepts any value) and `minimum` and `maximum` (numbers). A row may key only the keys the rule declares. The single most specific matching row applies, through the shared row matcher:

- No row matches: a finding that no limit is defined for the object's keys.
- A row that tests an unknown key could apply: not evaluated. A row another key already rules out does not matter, so an unknown key no row can reach is harmless.
- Rows tie for most specific: not evaluated as an invalid declaration, naming the rows; never broken by order.
- The row has neither bound: no limit applies, and the object passes.

`quantity` names what the row limits: `plan-area`, `property`, `sill-height` or `clear-width` (the last two below). `plan-area` is the object's measured footprint in square metres through `PlanAreaService` (a zone's is the union of its members); like `plan-area`, a footprint straddling a bound, or an empty one, is not evaluated. `property` is the number or quantity `quantity_property` states, compared in canonical SI units; an absent or non-numeric value is not evaluated. Bounds are inclusive. Key patterns are case-sensitive unless `case_sensitive` is `false`. A finding names the row (zero-based) and the key values, relates the objects the keys were read from, and cites the key, relationship and measurement evidence.

`sill-height` limits a window's sill per space type: the object's bottom elevation above the floor of each object `floor_path` reaches from it, in metres, through `VerticalExtentService`. A floor is the reached object's own bottom, so `floor_path: ["axioval:derived.adjacent-space"]` measures a window against the spaces on each side of it, and `key_1_path` along the same path keys the table on those spaces' use. `floor_path` is required with `sill-height` and refused with any other quantity, as `quantity_property` is outside `property`.

- Each reached floor is judged on its own against the one row the keys select. A window between two spaces whose floors lie at different elevations is too high when it is too high above either: one failing floor is a finding naming that floor, relating its space and citing the window's and that floor's extents.
- Elevations are intervals, and a tessellated body's are never points. The sill height runs from the window's lowest possible bottom less the floor's highest possible bottom to the reverse, widened by one rounding step wherever the subtraction rounded, so it always holds the exact difference. A sill height straddling a bound is not evaluated.
- A floor that cannot be measured, or straddles a bound, leaves the window not evaluated unless another floor already fails it. A window that cannot be measured, or whose `floor_path` reaches nothing, is not evaluated.
- The keys must agree across the reached spaces, as for any key: a window between an office and a corridor, whose rows differ, is not evaluated rather than judged against either.

Whether a window sits at the end of a corridor is not decided: it needs the corridor's axis (a medial axis of its footprint), which no service provides yet.

`clear-width` limits a door's clear width in metres, taken from the first of two steps that produces one:

1. the length `quantity_property` states (a clear width the project records on the door or its type);
2. else the length `overall_width` states (with IFC, `axioval:attributes.OverallWidth`) less `width_deduction`, a non-negative length the rule states for frame, lining and leaf.

| Parameter | Kind | Meaning |
|---|---|---|
| `quantity_property` | `propertyReference` | The stated clear width, a length. |
| `overall_width` | `propertyReference` | The overall width the deduction is taken from, a length. |
| `width_deduction` | `quantity` | The rule author's deduction, a length of at least zero; declared together with `overall_width`. |

At least one step is declared; `overall_width` and `width_deduction` are refused with any other quantity, and `floor_path` with this one. As in the [light-opening fallback](#light-opening-area), only an exact absence moves to the next step: a stated value that is null, not a length or not positive leaves the door not evaluated rather than replaced by the approximation, and so does an absent overall width or a deduction that leaves no width. The deduction is a **declared approximation of the rule author**, not a measurement: the finding says so ("clear width (… 0.9 m less the rule's deduction 0.1 m, an approximation) is 0.8 m; required at least 0.9 m …") and cites the evidence entry `axioval:derived.clear-width:<door>:step=overall-width-less-deduction;deduction=<metres>`, marked inexact; a stated width cites `…:step=stated`, exact. A deduction that differs per door type is one rule per type. Widths are read as the decimals they display, so an end within a few units in the last binary place of a bound meets it: a 1 m door less 0.1 m meets a 0.9 m minimum. Deriving the clear width from the lining and panel properties the model states (lining offset, thickness, panel width) waits on openbimrs/ifc#149.

### Exit separation

`exit-separation` requires each selected space's exits to lie far enough apart for its size: at least `fraction` (one half by default) of the space's longest plan diagonal, or `flagged_fraction` (say one third) when a boolean `flag` is `true`.

| Parameter | Type | Meaning |
|---|---|---|
| `exit_path` | string list, required | the relationship path from the space to its exits, such as `["axioval:derived.adjacent-space:backward"]` |
| `exit_selector` | selector, required | which reached objects are exits (doors, or doors marked as exits) |
| `fraction` | number, optional | share of the diagonal the exits must lie apart; `0.5` by default |
| `flag` | property reference, optional | a boolean selecting `flagged_fraction`, such as a sprinkler flag |
| `flag_path` | string list, optional | where `flag` is read: the objects this path reaches from the space (its storey, its building); the space itself without it |
| `flagged_fraction` | number, optional | the share that applies when `flag` is `true`; required with `flag` |
| `separation` | string, optional | `closest` (the default), `centres` or `farthest` |
| `pairs` | string, optional | `any` (the default): some pair of exits lies far enough apart; `all`: every pair does |
| `minimum_exits` | integer, optional | fewer exits than this is a finding |

The diagonal is the largest distance between two points of the space's footprint, through `PlanSpanService`. `closest` measures the plan distance between the exits' footprints through `ProximityService` (the `horizontal` projection of `distance`); `centres` and `farthest` measure between the footprints' centroids or their farthest points through `PlanSpanService`. A space with fewer than two exits is not checked, unless `minimum_exits` makes the shortfall a finding.

A finding names the pair (with `any`, the pair farthest apart), its separation, the required distance, the fraction and diagonal it comes from and the flag's value; it relates the exits and the objects the flag was read from, and cites the relationship, measurement and flag evidence. Every length is an interval:

- A pair is far enough apart only when its whole interval lies at or above the whole required interval, too close only when it lies wholly below it; otherwise it is unknown, and so is a pair that cannot be measured.
- The flag must be a boolean and agree across the objects `flag_path` reaches. An absent, null, non-boolean or disagreeing flag, or a path reaching nothing, is unknown: the required interval then spans both fractions, so a pair far enough apart for the larger or too close for the smaller is still decided.
- An object `exit_selector` cannot decide can only add exits and pairs: with `any` a pair far enough apart stands, with `all` a pair too close stands, and anything else they could change is not evaluated. The same holds for the count against `minimum_exits`.
- A diagonal that cannot be measured, or a missing service, leaves the space not evaluated.

### Counterpart coverage

`counterpart-coverage` checks that each selected element is covered by its counterparts, in plan and in height: architectural walls by structural walls, or the reverse. The counterparts are the objects `counterparts` picks, typically another discipline's elements of matching kinds (a [`discipline` selector](#discipline-selectors) with an entity type), so a check across two models declares `--model arch.ifc:architecture --model struct.ifc:structure`. It needs the plan-area, proximity and (for the height check) vertical-extent services.

| Parameter | Type | Meaning |
|---|---|---|
| `counterparts` | selector, required | the objects that should cover each element |
| `tolerance` | length, optional | coverage: one growth for both checks |
| `horizontal_tolerance`, `vertical_tolerance` | lengths, optional | conformity: separate growths, declared together instead of `tolerance` |
| `info_above`, `warning_above`, `error_above` | numbers, optional | uncovered shares in `[0, 1)` above which a finding has that severity; at least one, ascending in that order |

- **Plan**: the share of the element's footprint outside the union of the counterparts' footprints, each grown by the horizontal tolerance in every plan direction, through `PlanAreaService`'s uncovered area.
- **Height**: the share of the element's vertical extent outside the union of the vertical extents, each grown by the vertical tolerance below and above, of the counterparts that overlap it in plan (their grown footprint covers part of its footprint).

A negative tolerance switches its check off, so conformity with `vertical_tolerance: -1 m` checks plan only; switching every check off is an invalid declaration. A share above the lowest declared threshold is a finding of the most severe band it exceeds, and the rule's own severity is not used; a share at or below it passes. Each check reports on its own: an element with nothing under it has a plan finding and a height finding. A finding states the share, the uncovered area or height, and the tolerance; it relates the counterparts that surely cover part of the element, and adds "no counterpart overlaps it" when none may.

Shares are intervals: a tessellated body's footprint and extent are, and so is a footprint grown by a disc, which the plan-area service brackets between two polygons. A share straddling the lowest threshold is not evaluated; one above it that straddles a higher threshold is graded by the most severe band it may reach, and the message says so. A counterpart the selector cannot decide, whose extent cannot be read, or whose cover cannot be measured can only cover more, so a pass stands and anything else is not evaluated. Measured areas carry the plan overlay's rounding (around 10⁻⁸ of the extent), so a threshold of exactly 0 can flag a fully covered element; use a small positive one.

Counterparts are not filtered by direction: a perpendicular wall meeting the element within the horizontal tolerance overlaps it in plan and counts towards its height. Keeping only axis-compatible counterparts needs a minimum-area rectangle per footprint, which no geometry service provides yet.

### Accessible route

`accessible-route` requires each selected destination (an accessible room, say) to be reachable from a start point (an entrance) through the route spaces, for a mobility profile: a body `width_metres` wide under `clear_height_metres` of headroom. It needs the [walkability service](./walkability.md) and, for stated widths, property resolution.

| Parameter | Type | Meaning |
|---|---|---|
| `route_selector` | selector, required | the spaces a route may cross |
| `start_selector` | selector, required | the start points; a destination some start reaches passes |
| `portal_selector` | selector, optional | the doors and openings a route may pass |
| `lift_selector`, `ramp_selector`, `stair_selector` | selectors, optional | the vertical connectors between levels, by kind; one object in two of them is an invalid declaration |
| `obstacle_selector` | selector, optional | what obstructs the route spaces and portals inside the headroom band (furniture, walls around a doorway) |
| `width_metres` | number, required | the body's width, the route's minimum clear width |
| `clear_height_metres` | number, optional | the headroom band above each floor; without it, each space's own height |
| `door_width_metres` | number, optional | the minimum clear width of every portal on the route |
| `ramp_width_metres`, `stair_width_metres` | numbers, optional | the minimum clear width of a ramp or stair on the route |
| `forbid_stairs` | boolean, optional | `true` (the default): no route may use a stair, so a room reached by stairs only is a finding |
| `clear_width_property` | property reference, optional | where the source states a portal's or connector's clear width, as a length |

A start or destination the portal selector picks is an entrance (its two faces); any other is a walkable surface. A route may cross only route spaces and portals, besides its own start and destination: a room that is a destination but not a route space is never passed through on the way to another.

The rule takes one walkability snapshot for the profile and judges every passage on top of its width bounds:

- **Portals.** A stated clear width below `door_width_metres` blocks; so does a door the geometry shows narrower (the upper bound of its crossing). A stated width at or above the minimum, or a crossing proven at least that wide, admits it; otherwise the door is undecided. A stated width is also sent with the request (`WalkabilityRequest::with_stated_clear_widths`), so the geometry can prove the body passes the leaf and lining: without one, a door from IFC can bound a route from above only, never pass it, because its `OverallWidth` includes the lining.
- **Connectors.** With `forbid_stairs`, a stair blocks. A ramp or stair with a minimum width is admitted by a stated clear width at or above it, blocked by one below it, and undecided without one; the geometry does not measure connector widths. Climbs are not measured either, so a route through any connector is never proven.
- **Start and destination portals.** A route begins on either face of a start door, so the door's own crossing is judged separately: too narrow for the body, it blocks every route from it.

Outcomes are three-valued. A destination some decided start reaches definitely passes. One that every start is proven cut off from is a finding that relates the blocking elements: the passages leaving what the starts can reach that lead on towards the destination (`WalkabilitySnapshot::blocking_passages`), each with its reason, or "connected to the starts by stairs only" when every block is a forbidden stair, or "no route space connects" when nothing joins them at all. Anything else is not evaluated, listing the undecided elements. A start, route space, portal or connector whose selection is undecided can only add routes: it keeps its passages possible but never definite, so it cannot turn a block into a pass. An undecided obstacle could block or free any route and leaves every destination not evaluated. A backend refusal (see [the Axiolid backend](./walkability.md#the-axiolid-backend)) is not evaluated, never a pass.

Passing spaces (turning areas at a maximum spacing along the route) are not checked yet: they need the placement search, which no geometry service performs.

### Slab stacks

`slab-stack-spacing` judges `VerticalExtentService` elevations together with `PlanAreaService` footprints, so it needs a geometry adapter. Unlike `level-spacing`, which reads storey elevations, it measures the slabs' own surfaces.

| Capability | Checks |
|---|---|
| `slab-stack-spacing` | Two selected slabs stack when their footprints overlap by at least `minimum_overlap_ratio` of the smaller footprint. Each slab is checked against the next slab up it stacks with, ordered by top elevation: `top_to_top`, `bottom_to_bottom` and `top_to_bottom` (the clear gap to the next slab's underside) each lie within their optional `<measure>_minimum` and `<measure>_maximum`. `consistent` lists the measures that must equal the prevailing one of the stack within `tolerance` (1 mm by default), as in `level-spacing`. |

Elevations and footprints are intervals, and a tessellated slab's are never points. A slab is judged only on the whole interval: a distance straddling a bound, an overlap ratio straddling the minimum, or two tops the intervals cannot order are not evaluated. An object whose selection is undecided still counts as a possible next slab up, and one whose extent cannot be measured leaves every slab not evaluated, since it could sit in any stack. A slab with nothing stacked above it has nothing to check. When a proximity service is registered, its enclosing boxes skip overlap measurements between slabs that cannot meet in plan.

### Stairs and ramps

`stair-geometry` and `ramp-geometry` judge what `WalkingSurfaceService` measures from each selected object's body (see [Typed host services](./services.md)), so they need a geometry adapter; declared values such as `RiserHeight` or `NumberOfRisers` are checked with `property-predicate` instead. Select the objects the measure fits: single flights (`IfcStairFlight`) and ramp flights (`IfcRampFlight`), not a whole stair whose landing would count as a tread.

`axioval:capability.stair-geometry` measures a straight flight: its treads are its upward-facing level faces, its risers the height differences from its lowest point through each tread to its top (the first riser starts at the body's lowest point, so the flight is taken to stand on the level it starts from; a top above the last tread is a final riser to the upper floor), its goings the horizontal distances from nosing to nosing along the direction it climbs, which is derived from the treads rather than the placement. At least one check is declared:

| Parameter | Kind | Meaning |
|---|---|---|
| `riser_minimum`, `riser_maximum` | `quantity` | Every riser, inclusive. |
| `going_minimum`, `going_maximum` | `quantity` | Every going (a flight of `n` treads has `n - 1`). |
| `step_length_minimum`, `step_length_maximum` | `quantity` | `2r + g` for every tread, `r` the riser climbing onto it and `g` the going leaving it. |
| `nosing_minimum`, `nosing_maximum` | `quantity` | How far each tread reaches over the one below. |
| `minimum_risers`, `maximum_risers` | `integer` | The number of risers. |
| `maximum_rise` | `quantity` | The flight's rise, base to top of its last riser. |
| `riser_tolerance`, `going_tolerance` | `quantity` | The difference between the flight's largest and smallest riser or going. |
| `minimum_headroom` | `quantity` | The least vertical clearance above the treads, with `headroom_obstacles`. |
| `headroom_obstacles` | `selector` | The objects that may stand above (slabs, beams, ducts); declared together with `minimum_headroom`. |

`axioval:capability.ramp-geometry` measures a ramp's sloped runs: connected upward-facing faces flatter than 45°, each planar, separated by level landings. A run's slope is its rise over its horizontal length along its steepest ascent.

| Parameter | Kind | Meaning |
|---|---|---|
| `slope_limits` | `table` | Rows of `maximum_slope` (`number`, rise over length: `0.0833` for 1:12, required), `maximum_length` and `maximum_rise` (`quantity`, optional). A run conforms when some row holds all its columns, so a slope depending on the run's length or rise is one row per step. |
| `slope_tolerance` | `number` | The difference between the ramp's steepest and shallowest run. |
| `minimum_headroom`, `headroom_obstacles` | | As for stairs, above the runs and landings. |

A ramp limited to 1:12 and 0.76 m of rise per run, or allowed 1:10 over runs of at most 2 m:

```json
{"slope_limits": {"type": "table", "value": [
  {"maximum_slope": {"type": "number", "value": 0.0833},
   "maximum_rise": {"type": "quantity", "value": 0.76, "unit": "m"}},
  {"maximum_slope": {"type": "number", "value": 0.1},
   "maximum_length": {"type": "quantity", "value": 2, "unit": "m"}}]}}
```

Every length and slope is an interval. Each check is judged on its own and each failing check is its own finding naming the values (`riser 3 of 4 is 0.21 m; at most 0.19 m required`) and citing the measurement; a check whose interval straddles its bound, widened by a few units in the last place for the binary rounding of decimal coordinates, is not evaluated while the others still decide. Headroom is the least vertical distance from the walking surface (treads, runs and landings) to a selected obstacle's body directly above it, measured from the surface, not from the pitch line; a finding names and relates the lowest obstacle. An obstacle `headroom_obstacles` cannot decide can only lower the headroom: too little stands, enough is not evaluated. An obstacle crossing the walking surface, an unmeasured or nearby tessellated one leaves headroom not evaluated.

A flight or ramp the service cannot measure is not evaluated, never passed: a tessellated body, an open or inward-facing mesh, winders or a turning flight, a flight in several pieces (open risers), a flight with a sloped walking face, a ramp whose slopes meet without a landing, or a body with no tread or run. Not measured yet (#85): winders, open risers, headroom under a flight, landing sizes, clear width, handrails (height, extension, continuity, side), the slab connection, doors on landings, free space at a ramp's ends and a stair nearby.

### Doors

Door accessibility checks are compositions of the capabilities above; no capability is specific to doors. The door type is a key: with IFC, the door's `OperationType` (`axioval:attributes.OperationType`, such as `SINGLE_SWING_LEFT` or `DOUBLE_DOOR_SINGLE_SWING`) or its type object's name (`axioval:type-attributes.Name`). Each sub-check maps to one rule:

| Sub-check | Rule |
|---|---|
| Clear width per door type | `keyed-limit` with `quantity: clear-width`, a `minimum` per type row: the clear width the door states, else its `OverallWidth` less the rule's `width_deduction`, a declared approximation (see [Keyed limits](#keyed-limits)). |
| Clear width derived from panel width less frame and panel thickness | Not decided yet: the lining and panel properties are not exposed (upstream openbimrs/ifc#149). The rule's deduction stands in for them until then. |
| Threshold height | A stated length: `keyed-limit` with `quantity: property` and a `maximum` per type row, or a `property-requirements` row with `maximum` and `unit`. The threshold's body is not measured; models rarely carry one. |
| Glazing ratio | A stated fraction, such as `GlazingAreaFraction` in `Pset_DoorCommon`: a `property-requirements` row with `minimum`/`maximum` and no `unit`, or `keyed-limit` with `quantity: property` per type. Deriving it from the panels waits on openbimrs/ifc#149. |
| Minimum distance to other doors | `distance` with `counterparts` the doors, `mode: none_closer_than`, `projection: horizontal` and `minimum_metres`. With `relationship: axioval:derived.adjacent-space`, only doors opening into a common space count. |
| Which spaces a door connects, and their types | `opening-spaces`, or a key read along `axioval:derived.adjacent-space` (as for sill heights); the side facing `outside` is recorded in the adjacency evidence. |
| Door width on an accessible route | Walkability (#76) with the clear widths the host states per portal; the CLI states none, since `OverallWidth` includes the lining. |
| Clear areas in front of, behind and beside the leaf (handle side), with a floor under them | Not decided yet: they need the door's leaf, hinge side and front (upstream openbimrs/ifc#148) and rectangles fixed to the door, which `free-floor-rectangle` refuses until the placement search grounds them (#18, #23 to #25). Clearance zones around components are #83. |
| Opening direction relative to the space type | Not decided yet: it needs the swing direction (upstream openbimrs/ifc#148). The spaces on each side are known; which way the leaf opens is not. |

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

### Model quality

Checks on how a model is built rather than on what it designs. Each sub-check maps to one rule:

| Sub-check | Rule |
|---|---|
| Material-layer thickness against the body's thickness | `body-extent` with `axis` `forward` and `target_property` the material set's `TotalThickness`, within a `tolerance`. |
| Polygon count per element | `triangle-count` with a `maximum`. |
| A door or window on another storey than its host | `same-container` from each door or window along `IfcRelFillsElement:backward`, `IfcRelVoidsElement:backward` to its host, climbing `IfcRelContainedInSpatialStructure` `backward` to the storeys. |
| Space-boundary coverage of a space's surface | Not decided yet (below). |
| Door swing direction | Not decided yet: it needs door leaves (hinge side and swing), which the object-frame contract does not carry yet (upstream openbimrs/ifc#148). |

`axioval:capability.body-extent` measures each selected object's body along one of its own placement axes, through `ObjectFrameService` (the frame) and `VerticalExtentService` (the extent along the frame's axis), so it needs both a semantic adapter that states placements and a geometry adapter.

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

`axioval:capability.triangle-count` requires each selected object's mesh to hold at most `maximum` triangles, through `TriangleCountService`. The count is of the mesh the host produced, not of anything the model states: a box extruded from a rectangle counts twelve triangles, and a curved face as many as the host's chord budget made of it, so another host or budget may count differently. A finding on a tessellation of curved faces carries approximate evidence and says that the count depends on the tessellation. A bodiless object counts none; an unmeasured one is not evaluated.

`axioval:capability.same-container` requires each selected object to lie in the same nearest containers as every counterpart `counterpart_path` (steps as in `path`) reaches from it. `counterpart_selector` restricts which reached objects count; `container_selector` names the containers, climbed to along the traversal parameters (`relationship` and `direction`, or `path`) exactly as `property-comparison`'s container modes climb, so `follow_chain` does not apply. The two sets must be equal: an object in no container while its counterpart is in one differs too. An object reaching no counterpart has nothing to agree with and passes. A container selector that cannot decide an object leaves every selected object not evaluated; an undecided counterpart leaves the object not evaluated unless a decided one already differs; a refused relationship answer leaves it not evaluated. A finding names each differing counterpart and its containers, and relates them together with the object's own. `property-comparison` cannot express this check: its candidates and targets are compared by property value, and a counterpart's container is no property of either object.

Space-boundary coverage, the share of a space's surface its `IfcRelSpaceBoundary` connection geometry covers, is left open. The relationship service answers which elements bound a space, not the boundaries' surfaces: the connection geometry is neither an object nor meshed by the geometry bridge, so no service measures the covered area, and a rule over the bounding elements' bodies would decide something else.

## Adding a capability

1. Define or reuse canonical schema concepts and parameters.
2. Add failing contract and behavior tests.
3. Implement policy only; put source interpretation in an adapter.
4. Declare all evidence requirements.
5. Add deterministic and missing-evidence tests.
6. Register in the built-in registry.
7. Record legacy parity and cutover in the migration ledger when applicable.
