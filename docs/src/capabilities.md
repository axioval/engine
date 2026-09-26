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

At runtime, missing evidence does not become a pass. `CapabilityEvaluation` carries conclusive findings separately from object- or rule-level not-evaluated outcomes. The runtime binds every such outcome to the compiled `RuleId`, sorts it deterministically, and exposes it through `Report::not_evaluated()`. Reasons distinguish an invalid declaration, a missing service, backend outage, incomplete evidence, invalid evidence, and resource exhaustion.

A capability may return findings and not-evaluated outcomes together when only part of its selected universe was computable. Consumers must not interpret an empty findings list as a pass while `not_evaluated` is non-empty.

## Built-ins

`axioval-rules` contains reusable, vendor-neutral implementations. Vendor identity, proprietary format handling, localized vendor text and oracle-only ordering remain adapters in the legacy runtime.

`axioval:capability.property-exists`, `axioval:capability.property-required`, `axioval:capability.property-value-equals`, and `axioval:capability.property-predicate` resolve values through `PropertyResolutionServiceHandle`. `property-predicate` takes exactly one target: `value` (integer), `number`, `quantity`, `text`, `texts` or `boolean`. A `quantity` is written with a unit and compared in SI; it compares only with a quantity of the same dimension. With a numeric target it accepts `equal`, `not_equal`, `greater_than`, `greater_or_equal`, `less_than` and `less_or_equal`. With `text` it accepts `equal`, `not_equal`, `contains` and `matches`, a regular expression that must match the whole value. With `texts` it accepts `one_of` and `none_of`, and with `boolean` it accepts `equal` and `not_equal`. `is_defined` and `is_undefined` take no target; a blank or null value counts as undefined. Text comparisons are case-sensitive unless `case_sensitive` is `false`. A comparison presupposes a value, so an exactly absent property fails every operator except `is_undefined`. A quantity is never compared with a unit-less target and is reported as not evaluated. Failed predicates retain exact source evidence and name the actual value. A present value must be bound to the complete request—including its source-qualified object identity—and carry exact reviewable evidence. A missing value is conclusive only when the provider returns exact request-bound `CompletePropertyAbsenceEvidence`; service absence, partial extraction, cross-object substitution, mismatched responses, or inexact provenance remain not evaluated. Property selectors use the same resolver, so incomplete applicability data cannot silently skip an object. `property-exists` checks exact presence and intentionally does not reinterpret a present typed value. `property-required` applies the stronger required-value contract: exact absence, `null`, or blank text emits an evidence-backed finding, while booleans, integers, finite numbers/quantities, and nonblank text satisfy it. `property-value-equals` accepts a typed `property` reference and boolean `expected` value; a non-boolean resolution is invalid evidence rather than a pass or violation.

`axioval:capability.property-data-type` takes a `property` reference and a `data_type` string. It applies the `property-required` contract and additionally requires the value's type as the source declares it (`Property::data_type`, for IFC `IFCLABEL`, `IFCBOOLEAN`, ...) to equal `data_type`, compared ASCII case-insensitively. A different declared type is a finding. A present value whose type the source does not report is not evaluated, never taken to match. It is what an IDS property facet with a `dataType` and no value translates to.

`axioval:capability.property-value` checks a value against lexical constraints cast to the kind of the value the source resolved: `values` (any of), `patterns` (XML Schema regular expressions matching the whole value), `min_inclusive`/`max_inclusive`/`min_exclusive`/`max_exclusive`, `length`/`min_length`/`max_length`, and optionally `data_type`. Text compares exactly and case-sensitively and alone takes patterns and lengths; booleans accept `true`/`1` and `false`/`0`; integers take integer literals; decimals take `xs:double` literals and are equal within `|x - v| <= |v|·1e-6 + 1e-6` (the IDS tolerance, boundaries included), while bounds compare exactly. Without `optional`, absence, `null` and blank text are violations; with it, an absent or `null` property passes and any present value is checked. A literal that cannot be cast, a constraint the value's kind does not take, or a pattern that cannot be translated exactly (class subtraction, `\i`/`\c`, block escapes) makes the object not evaluated (`InvalidDeclaration`); a quantity is not evaluated until units are handled.

`axioval:capability.property-comparison` currently covers exact property-to-property targets with an independent selector-valued candidate scope, checked/shared/related modes, target-side factors, and `each` or `at_least_one` quantifiers. It compares booleans, strings, exact integers, finite decimals, and canonical quantities with matching dimensions. Missing properties emit evidence-backed missing-information findings; incompatible types or unavailable evidence remain not evaluated. The target is exactly one of:

- `target_property`, a property of the checked object;
- a constant: `target_number`, `target_quantity`, `target_text` or `target_boolean`;
- `target_texts`, a list that only `one_of` and `none_of` take.

Two more quantifiers compare a single number with the target:

- `count` compares the number of candidates, zero included, and needs no `compared_property`.
- `sum` compares the total of the candidates' compared values. The values must all be integers, all numbers, or all quantities of one dimension; otherwise the checked object is not evaluated. A candidate without the value gets its missing-property finding, and no verdict is drawn from the partial total.

`axioval:capability.free-floor-circle` and `axioval:capability.free-floor-rectangle` check whether each selected spatial scope can contain an exact supported vertical shape. Circle parameters are `diameter_metres` and `height_metres`; rectangle parameters are `width_metres`, `length_metres`, and `height_metres`, all in canonical metres. Each request covers every other project object as a candidate obstacle and requires exact whole-base support on the selected scope with zero hidden gap. A complete exact no-placement proof emits the shape-specific `NO_FREE_FLOOR_SPACE_*` finding; missing services, backend outages, or invalid/incomplete evidence emit not-evaluated outcomes instead.

`axioval:capability.clash` and `axioval:capability.distance` check selected subjects against a selector-valued `counterparts` group through `ProximityServiceHandle`, after the engine's complete broad-phase candidate search. A hard clash is a witnessed penetration beyond `penetration_tolerance_metres`, or containment. A clearance clash is a separation below the optional `clearance_metres`. Distance bounds the nearest counterpart with `minimum_metres` and/or `maximum_metres`. Findings on tessellated geometry carry inexact evidence and say so. See [Clash, interference and distance](./clash.md).

### Semantic capabilities

These capabilities judge exact properties, classifications and relationships only, so they need no geometry. Relationship parameters (`relationship`, `direction`, `follow_chain`, `skip_absent_relationship_ends`) mean the same as in `property-comparison`. With the IFC adapter, a relationship is an IFC relationship entity name such as `IfcRelAggregates`. An object whose selection or value cannot be decided is not evaluated, never passed.

| Capability | Checks |
|---|---|
| `selector-conformance` | Each selected object satisfies the `requirement` selector. An agreed list is an `anyOf` of `allOf` rows. The finding cites every property fact consulted. |
| `unique-value` | `property` does not repeat among the selected objects of one source, or of the project with `across_sources`, or among objects reaching the same related objects through `relationship` (for example one storey). Text is trimmed and compared ignoring case unless `trim` or `case_sensitive` says otherwise. A missing value is a finding unless `require_value` is `false`. |
| `consistent-value` | Objects of one kind that share a `key` value share their `value` too. An absent value is a value of its own. Each member of a disagreeing group is reported, naming the others. |
| `related-count` | Each anchor has between `minimum` and `maximum` related objects that `related_selector` picks. Without `relationship`, the anchor's whole source is counted. Undecided related objects are counted as unknown, and the anchor is judged only when they cannot change the verdict. |
| `relative-count` | At each anchor, `provided / provided_unit` stands in `operator` (`equal`, `not_equal`, `greater`, `at_least`, `less`, `at_most`) to `required / required_unit`, computed in exact integers. |
| `name-sequence` | The members of each anchor, ordered by the numeric `order` property, carry whole-number names from `first` stepping by `increment`. A name is a number only when it is exactly one. A member without an order value leaves the anchor not evaluated. |
| `manual-issue` | Raises the declared `title`, `category` and `description` against every selected object, for checks owed by hand. |

| `level-spacing` | Each level's height, the rise of its `order` length to the next level up, lies within `minimum` and `maximum` and, with `consistent`, matches the prevailing height within `tolerance`. The highest level is not evaluated unless `ignore_highest`, since its height needs geometry; `ignore_lowest` leaves out a basement. |

Most names and numbers these rules read are attributes of the object, not properties; see [attribute sets](./ir.md#attribute-sets). Quantities are written with a unit: `m`, `cm`, `mm`, `km`, `m2`, `cm2`, `mm2`, `m3`, `cm3`, `mm3`, `l`, `rad` or `deg`. `²`, `³` and `°` are accepted too.

A traversal is either one `relationship` (with `direction` and `follow_chain`) or a `path` of steps walked in order, each `Relationship` or `Relationship:direction`. For example, `IfcRelVoidsElement:forward` then `IfcRelFillsElement:forward` goes from a wall to the doors and windows filling its openings.

`relative-count` also has a table mode. `table` lists rows `R:P`, meaning "from R required objects on, at least P provided". The row with the largest R not above the required count applies. Beyond the last row, `additional_required` / `additional_provided` add P for every further R. Parameters have no table type, so rows are text, and a malformed row is a declaration error.

### Plan-area capabilities

These judge `PlanAreaService` measurements, so they need a geometry adapter.

| Capability | Checks |
|---|---|
| `area-ratio` | At each anchor, the summed footprints of `numerator_selector` over those of `denominator_selector` (or the anchor's own footprint) lie within `minimum` and `maximum`. `numerator_property` / `denominator_property` take a population's areas from an area-quantity property instead, e.g. glazing areas. |
| `plan-coverage` | Each subject's footprint lies within one `candidate_selector` object by at least `minimum_ratio`, e.g. a space within a fire compartment. |

## Adding a capability

1. Define or reuse canonical schema concepts and parameters.
2. Add failing contract and behavior tests.
3. Implement policy only; put source interpretation in an adapter.
4. Declare all evidence requirements.
5. Add deterministic and missing-evidence tests.
6. Register in the built-in registry.
7. Record legacy parity and cutover in the migration ledger when applicable.
