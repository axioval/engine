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

`axioval:capability.property-exists`, `axioval:capability.property-required`, `axioval:capability.property-value-equals`, and `axioval:capability.property-predicate` resolve values through `PropertyResolutionServiceHandle`. The integer predicate accepts `equal`, `not_equal`, `greater_than`, `greater_or_equal`, `less_than`, and `less_or_equal`; failed predicates retain exact source evidence. A present value must be bound to the complete request—including its source-qualified object identity—and carry exact reviewable evidence. A missing value is conclusive only when the provider returns exact request-bound `CompletePropertyAbsenceEvidence`; service absence, partial extraction, cross-object substitution, mismatched responses, or inexact provenance remain not evaluated. Property selectors use the same resolver, so incomplete applicability data cannot silently skip an object. `property-exists` checks exact presence and intentionally does not reinterpret a present typed value. `property-required` applies the stronger required-value contract: exact absence, `null`, or blank text emits an evidence-backed finding, while booleans, integers, finite numbers/quantities, and nonblank text satisfy it. `property-value-equals` accepts a typed `property` reference and boolean `expected` value; a non-boolean resolution is invalid evidence rather than a pass or violation.

`axioval:capability.property-data-type` takes a `property` reference and a `data_type` string. It applies the `property-required` contract and additionally requires the value's type as the source declares it (`Property::data_type`, for IFC `IFCLABEL`, `IFCBOOLEAN`, ...) to equal `data_type`, compared ASCII case-insensitively. A different declared type is a finding. A present value whose type the source does not report is not evaluated, never taken to match. It is what an IDS property facet with a `dataType` and no value translates to.

`axioval:capability.attribute-value` applies the same constraints to a direct attribute read through `AttributeServiceHandle`. Its `attribute` parameter is a property reference without a set; with no other parameter the attribute must merely hold a non-empty value. An unset attribute fails unless `optional`; a reference or list with value constraints, or an attribute the class lacks, is not evaluated.

`axioval:capability.predefined-type` requires each selected object's predefined type to be one of `values` or to match one of `patterns`, or, with `user_defined`, to be user-defined at all. An object without one fails a value or pattern requirement.

`axioval:capability.classification` requires a classification whose code (the assigned item or any ancestor) and whose system each meet `codes`/`code_patterns` and `systems`/`system_patterns`; the two may be met by different assignments. `axioval:capability.material` requires a material known by a name in `values` or matching `patterns`. `axioval:capability.part-of` requires a whole of a class (and predefined type) through a `relation`; the nearest whole of a required class decides. `axioval:capability.entity` requires the object's own class and, optionally, predefined type. Each takes `optional` and/or `prohibited` where IDS allows them, and a missing service, an unstated system that could decide, or an ambiguous whole is not evaluated.

`axioval:capability.population` requires at least `min` and at most `max` selected objects. Too few is one rule finding; too many is one rule finding naming the objects, except that with `max` zero each selected object is its own finding. When objects whose membership selection could not decide could change the verdict, the rule is not evaluated.

`property-value` and `attribute-value` also take `prohibited`: the requirement is judged as if required and meeting it is the violation; an absent value passes.

`axioval:capability.property-value` checks a value against lexical constraints cast to the kind of the value the source resolved: `values` (any of), `patterns` (XML Schema regular expressions matching the whole value), `min_inclusive`/`max_inclusive`/`min_exclusive`/`max_exclusive`, `length`/`min_length`/`max_length`, and optionally `data_type`. Text compares exactly and case-sensitively and alone takes patterns and lengths; booleans accept `true`/`1` and `false`/`0`; integers take integer literals; decimals take `xs:double` literals and are equal within `|x - v| <= |v|·1e-6 + 1e-6` (the IDS tolerance, boundaries included), while bounds compare exactly. Without `optional`, absence, `null` and blank text are violations; with it, an absent or `null` property passes and any present value is checked. A literal that cannot be cast, a constraint the value's kind does not take, or a pattern that cannot be translated exactly (class subtraction, `\i`/`\c`, block escapes) makes the object not evaluated (`InvalidDeclaration`); a quantity is not evaluated until units are handled.

`axioval:capability.property-comparison` currently covers exact property-to-property targets with an independent selector-valued candidate scope, checked/shared/related modes, target-side factors, and `each` or `at_least_one` quantifiers. It compares booleans, strings, exact integers, finite decimals, and canonical quantities with matching dimensions. Missing properties emit evidence-backed missing-information findings; incompatible types or unavailable evidence remain not evaluated. Constant targets, `count`, and `sum` are deliberately rejected until their oracle fixtures and issue contracts land, so this registration is not a full legacy parity claim.

`axioval:capability.free-floor-circle` and `axioval:capability.free-floor-rectangle` check whether each selected spatial scope can contain an exact supported vertical shape. Circle parameters are `diameter_metres` and `height_metres`; rectangle parameters are `width_metres`, `length_metres`, and `height_metres`, all in canonical metres. Each request covers every other project object as a candidate obstacle and requires exact whole-base support on the selected scope with zero hidden gap. A complete exact no-placement proof emits the shape-specific `NO_FREE_FLOOR_SPACE_*` finding; missing services, backend outages, or invalid/incomplete evidence emit not-evaluated outcomes instead.

`axioval:capability.clash` and `axioval:capability.distance` check selected subjects against a selector-valued `counterparts` group through `ProximityServiceHandle`, after the engine's complete broad-phase candidate search. A hard clash is a witnessed penetration beyond `penetration_tolerance_metres`, or containment. A clearance clash is a separation below the optional `clearance_metres`. Distance bounds the nearest counterpart with `minimum_metres` and/or `maximum_metres`. Findings on tessellated geometry carry inexact evidence and say so. See [Clash, interference and distance](./clash.md).

## Selecting with a capability

A `meets` selector (`{"kind": "meets", "capability": ..., "parameters": {...}}`) selects the objects that meet a capability's requirement on their own: the capability is evaluated over a project of just that object, and a finding is a non-match while an undecided outcome leaves membership undecided. Only capabilities that judge each object independently declare themselves `selectable` (the property, attribute, predefined-type, classification, material, part-of and entity capabilities); counting ones such as `population` never are. The compiler checks the capability and binds its parameters as for a rule, and the runtime evaluates it against the registry the plan was compiled with. It is how an IDS applicability facet other than the entity is expressed.

## Adding a capability

1. Define or reuse canonical schema concepts and parameters.
2. Add failing contract and behavior tests.
3. Implement policy only; put source interpretation in an adapter.
4. Declare all evidence requirements.
5. Add deterministic and missing-evidence tests.
6. Register in the built-in registry.
7. Record legacy parity and cutover in the migration ledger when applicable.
