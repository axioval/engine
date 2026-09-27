# IDS import

`axioval-ids` translates a buildingSMART IDS 1.0 document into a definition package and a ruleset that select the built-in capabilities. It lives in `staging/ids`: it reads IDS through the unreleased `openbim-ids` reader and is neither a workspace member nor published. It is a package importer, not a source adapter; it never reads a model.

## Exact or not at all

A facet becomes a rule only when a capability decides it exactly as IDS does. Anything else is a reported `Gap` naming the part of the specification and the reason, so a caller can refuse an incomplete translation. An untranslatable applicability facet leaves the whole specification without rules, since dropping it would widen the checked population; an untranslatable requirement leaves only itself out, since dropping it can only miss failures.

## What each facet becomes

The applicability is one selector: the entity's classes (never their subclasses), its predefined type, and a selector for every other facet, all of which must hold. Requirements become rules over that selector.

| IDS | Applicability | Requirement |
|---|---|---|
| entity | `entityType` per class, `includeSubtypes: false`; an enumeration or pattern is matched against the release's classes | `selector-conformance`; the applicability's own entity needs no rule |
| predefined type | property selectors over `axioval:type-attributes` and `axioval:attributes`, resolved as below | as the entity |
| attribute | property selectors in `axioval:attributes`, typed by the attribute's declared type | `property-required` or `property-value` in `axioval:attributes`; prohibited without a value: a `property-requirements` row excluding `not-empty`; prohibited with a value, or a restricted name: `selector-conformance` |
| property | gap | `property-required`, `property-data-type`, `property-value` (with `quantifier` and `si_units`); a set or name given as a pattern or an enumeration of several: `property-value` with `property_set_pattern`/`property_pattern`, or a required `property-requirements` row with the pattern columns; prohibited without a value: `property-requirements` rows excluding `not-empty`, one per enumerated set and name, or one pattern row |
| classification | `classification` selectors with `includeDescendants` | `classification`: systems and codes as literals or patterns, a system alone, `optional` or `prohibited` |
| material without a value | `exists` on `axioval:material.Kind` | `selector-conformance`, negated when prohibited |
| material with a value | the value against `axioval:material.Names` with `quantifier: any` | `selector-conformance`, negated when prohibited; optional: no material, or one going by the value |
| part of | `related` with `Relationship:backward+` to the whole's entity selector | `selector-conformance`, negated when prohibited |
| `minOccurs`/`maxOccurs` | — | `object-count` per source: required reports a model without applicable objects, prohibited a model with any |

The predefined type follows IDS: the type object's own, or its element or process type when that is user-defined or unset, unless the result is empty or `NOTDEFINED`; otherwise the occurrence's own, or its object type when user-defined or unset. A literal `USERDEFINED` asks whether the type is user-defined at all. A classification requirement is met by one assignment in a matching system whose code, or an ancestor's, matches; an optional one holds for an object with no classification at all and must be met by one with any. A material value is matched against every name and category the material goes by (the set's, each member's, each member material's), as `axioval:material.Names` lists them. A part-of relation is followed from the part to its wholes one or more times, and only that relation; a pattern for the whole drops classes that can never be the relation's relating end.

A property value is judged as IDS judges list, bounded, table and enumerated values: `quantifier` `any` (one of its values) for a literal, an enumeration or a pattern, and `all` for a range restriction, so a bounded value must lie within the range as a whole. `si_units` reads the literal of a measure in SI, the unit IDS states measures in. A set or property named by an `xs:pattern`, or by an enumeration of several names (written as an escaped alternation), is enumerated through the property service: one property must match in every set the set name matches, and every matching property must satisfy the facet, as the buildingSMART cases require. An enumeration narrowed by patterns is the names that match them. A matching set that holds no property at all cannot be seen by enumeration, so a required facet does not fail on it; IDS would.

Values keep IDS casting where a capability casts: `property-value` casts literals to the resolved value's kind, including `totalDigits` and `fractionDigits`. A selector compares one declared type, so an attribute value in a selector is translated only when every applicable class declares the attribute as text, an enumeration, a boolean or an integer; XML Schema patterns go through `axioval_rules::translate_xsd_pattern`, which refuses what it cannot map exactly.

## Gaps

These parts stay explicit gaps:

- a property facet in the applicability: IDS casts its literal to each property's own type, and a selector compares one declared type and cannot tell null or blank from a value without it;
- a prohibited property facet with a value or a data type;
- an attribute declared as a real, a measure, a date, a select, a reference or an aggregate, in a selector;
- in the applicability, a classification without a value (the system alone) and a classification pattern: the classification selector states both now (a selector without `code`, and `codePattern`), but the importer does not translate them yet;
- a material value restricted by several facets at once (an enumeration and patterns, a length), which one name must meet together;
- part of without a relation (every relation, mixed along the chain) or through `IFCRELVOIDSELEMENT IFCRELFILLSELEMENT`;
- requirements on a prohibited specification, which IDS declares invalid;
- entities that are not `IfcObject` occurrences, which a model session does not check, and `IFC4X3_ADD2`.

## Conformance corpus

`IDS_TEST_CASES=<IDS>/Documentation/ImplementersDocumentation/TestCases cargo test -- --ignored corpus` runs every buildingSMART test case through the IFC adapter and the engine. It asserts that no translated rule fails a `pass-` case and that every `fail-` case either produces a finding or is explained by a reported gap. `IDS_CORPUS_VERBOSE=1` lists every case with its findings.

Some facets translate but cannot be decided on some models, and are reported not evaluated rather than as gaps: a property whose value is an `IfcPropertyReferenceValue` or a complex property, which the adapter refuses; a table value checked with a `dataType`, since a table whose columns differ in type reports none; and a measure whose unit the model does not resolve.
