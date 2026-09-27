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

`axioval:capability.property-value` checks a value against lexical constraints cast to the kind of the value the source resolved: `values` (any of), `patterns` (XML Schema regular expressions matching the whole value), `min_inclusive`/`max_inclusive`/`min_exclusive`/`max_exclusive`, `length`/`min_length`/`max_length`, and optionally `data_type`. Text compares exactly and case-sensitively and alone takes patterns and lengths; booleans accept `true`/`1` and `false`/`0`; integers take integer literals; decimals take `xs:double` literals and are equal within `|x - v| <= |v|·1e-6 + 1e-6` (the IDS tolerance, boundaries included), while bounds compare exactly. A date takes `xs:date` literals and a date-time `xs:dateTime` literals with a UTC offset, for `values` and bounds alike; with `precision` `day` either value takes either literal, compared by the day it states (see [Dates and date-times](#dates-and-date-times)). `precision` on any other value is an invalid declaration. Without `optional`, absence, `null` and blank text are violations; with it, an absent or `null` property passes and any present value is checked. A literal that cannot be cast, a constraint the value's kind does not take, or a pattern that cannot be translated exactly (class subtraction, `\i`/`\c`, block escapes) makes the object not evaluated (`InvalidDeclaration`); a quantity is not evaluated until units are handled.

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

A `related` selector selects an object by the objects a relationship `path` reaches from it. The `path` is written as the `path` parameter of the relationship-scoped capabilities: steps `Relationship` or `Relationship:direction` (`forward`, the default, `backward` or `either`), walked one after another through `RelationshipSelectionServiceHandle`, never reaching the object itself. The reached objects are tested against the nested `selector` under a `quantifier`:

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
| `manual-issue` | Raises the declared `title`, `category` and `description` once per rule, for checks owed by hand: the finding is against the first selected object and relates the others. An empty selection raises nothing. |

| `level-spacing` | Each level's height, the rise of its `order` length to the next level up, lies within `minimum` and `maximum` and, with `consistent`, matches the prevailing height within `tolerance`. The highest level is not evaluated unless `ignore_highest`, since its height needs geometry; `ignore_lowest` leaves out a basement. |

Deliberate differences from other checkers:

- `name-sequence` orders members only by the declared `order` property. When a member has no order value (a storey without `Elevation`), the anchor stays not evaluated rather than falling back to placement height: the engine exposes no exact source-neutral service for an object's placement height, and a storey has no body whose extent could stand in for it.
- `property-required` reports each property in its own rule, so an object missing several identity fields gets one finding per field. Combining them into one finding per object would add a parameter every `property-required` definition must then declare; grouping findings per object is left to the report consumer.
- `manual-issue` raises nothing for an empty selection. A model-level finding for that case needs findings that are not reported against an object, which the report contract does not have yet.

Most names and numbers these rules read are attributes of the object, not properties; see [attribute sets](./ir.md#attribute-sets). Quantities are written with a unit: `m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`. `²`, `³` and `°` are accepted too.

A traversal is either one `relationship` (with `direction` and `follow_chain`) or a `path` of steps walked in order, each `Relationship` or `Relationship:direction`. For example, `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` goes from a wall to the doors and windows filling its openings.

### Relationships derived from geometry

Every traversal parameter also accepts a derived relationship identity, answered from geometry rather than from what the model states: `axioval:derived.contained-in-space` (an element to the space containing it, or the nearest within `horizontal` and `vertical` tolerances), `axioval:derived.adjacent-space` (a door, window or opening to the spaces on each side of it) and `axioval:derived.overlapping-group-space` (a space to the larger spaces covering at least `ratio` of its footprint). Tolerances follow the name, for example `axioval:derived.contained-in-space;horizontal=0.3;vertical=0.5`. Edges run from the element to the space, so counting components per space is `related-count` from each space `backward`. The service needs a geometry adapter; see [typed host services](./services.md). An undecided derivation leaves the anchor not evaluated, never counted as unrelated.

`relative-count` also has a table mode. `table` lists rows `R:P`, meaning "from R required objects on, at least P provided". The row with the largest R not above the required count applies. Beyond the last row, `additional_required` / `additional_provided` add P for every further R. Below the first row the table sets no requirement: the anchor or group is skipped, not extrapolated from zero. A table of increments alone applies them from zero. Parameters have no table type, so rows are text, and a malformed row is a declaration error.

In ratio mode, `small_required_below` n and `small_provided` k, declared together, replace the ratio for small counts: a required count from 1 up to but excluding n is judged as `provided operator k`. "With fewer than four workplaces, at least one washbasin" is n = 4, k = 1 with `at_least`; "below ten workplaces nothing is required" is n = 10, k = 0. A required count of zero is always judged by the ratio. Neither applies in table mode.

With `group_property`, `relative-count` groups instead of walking from anchors. The rule's selection is then the set of objects counted, and each object `provided_selector` or `required_selector` picks is counted in the group of its `group_property` value (a location code, say), within one source unless `across_sources`. Text is trimmed and compared ignoring case unless `case_sensitive`. Relationship parameters do not apply, and `across_sources` and `case_sensitive` apply only here. A group with required objects and no provided object is reported as present only in the required set, whatever the ratio, small-count case or table would say. A group's finding is raised against its lowest required object (its lowest provided object when it has none) and relates every other member. A counted object with no value (absent, null or blank) is a finding of its own; one whose value cannot be read is not evaluated and leaves every group of its scope not evaluated, since it could belong to any of them. A group with an object whose membership is undecided is not evaluated.

### Plan-area capabilities

These judge `PlanAreaService` measurements, so they need a geometry adapter.

| Capability | Checks |
|---|---|
| `area-ratio` | At each anchor, the summed footprints of `numerator_selector` over those of `denominator_selector` (or the anchor's own footprint) lie within `minimum` and `maximum`. `numerator_property` / `denominator_property` take a population's areas from an area-quantity property instead, e.g. glazing areas. |
| `plan-coverage` | Each subject's footprint lies within one `candidate_selector` object by at least `minimum_ratio`, e.g. a space within a fire compartment. |
| `plan-area` | Each selected object's footprint lies within `minimum` and `maximum` square metres, e.g. a space of at least 8 m² or a fire compartment (a zone, measured as the union of its members) of at most 400 m². With `member_selector`, the summed footprints of the members each anchor reaches lie within the range instead, e.g. the space area of each storey. |

All bounds are inclusive, and `area-ratio` and `plan-area` need at least one of `minimum` and `maximum`. Areas are intervals: a tessellated body measures within a bound derived from its chord deviation. A verdict needs the whole interval on one side of a bound, so an area that straddles one is not evaluated rather than judged from a midpoint.

`plan-area` reaches members as `related-count` does: through the declared traversal (`relationship` or `path`, as above; with IFC, `IfcRelAssignsToGroup` from a zone or `IfcRelAggregates` from a storey), or everywhere in the anchor's source without one; a traversal without `member_selector` is an invalid declaration. Footprints are summed, so members that overlap count twice; select members that tile the floor, such as spaces. An object with an empty footprint has no body and is not evaluated, and so is an anchor with such a member, since its sum is unknown. A member whose selection is undecided can only add area: a sum already above the maximum is still a finding, anything else is not evaluated. A finding relates the members summed. A definition bound to `plan-area` declares `minimum`, `maximum`, `member_selector` and the traversal parameters, all optional.

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

### Keyed limits

`keyed-limit` looks a limit up in a table keyed by facts about each selected object, then checks one of its quantities against it. A fire compartment's area limit depends on the building's fire class, the compartment's use class and whether its storey is sprinklered; one rule carries the whole table.

Up to four keys, `key_1` (required) to `key_4`, are property references. A key is read from the object itself, or, with `key_<n>_path` (steps as in the `path` parameter), from the objects that path reaches from it: `IfcRelContainedInSpatialStructure:backward` then `IfcRelAggregates:backward` climbs from an element to its building. A key's value is matched as text: a string as stated, a boolean as `true` or `false`, an integer in decimal. It is unknown when it is absent, null, blank or of another type, when the path reaches no object, or when the reached objects state different values; a refused property or relationship answer leaves the object not evaluated outright.

The `limits` table has the optional columns `key_1` … `key_4` (text patterns, as in [Table parameters](#table-parameters); a blank cell accepts any value) and `minimum` and `maximum` (numbers). A row may key only the keys the rule declares. The single most specific matching row applies, through the shared row matcher:

- No row matches: a finding that no limit is defined for the object's keys.
- A row that tests an unknown key could apply: not evaluated. A row another key already rules out does not matter, so an unknown key no row can reach is harmless.
- Rows tie for most specific: not evaluated as an invalid declaration, naming the rows; never broken by order.
- The row has neither bound: no limit applies, and the object passes.

`quantity` names what the row limits. `plan-area` is the object's measured footprint in square metres through `PlanAreaService` (a zone's is the union of its members); like `plan-area`, a footprint straddling a bound, or an empty one, is not evaluated. `property` is the number or quantity `quantity_property` states, compared in canonical SI units; an absent or non-numeric value is not evaluated. Bounds are inclusive. Key patterns are case-sensitive unless `case_sensitive` is `false`. A finding names the row (zero-based) and the key values, relates the objects the keys were read from, and cites the key, relationship and measurement evidence.

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
