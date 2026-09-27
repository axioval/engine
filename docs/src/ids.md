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
| property | gap | `property-required`, `property-data-type`, `property-value`; prohibited without a value: `property-requirements` rows excluding `not-empty`, one per enumerated set and name |
| classification | `classification` selectors with `includeDescendants` | `selector-conformance`, negated when prohibited |
| material without a value | `exists` on `axioval:material.Kind` | `selector-conformance`, negated when prohibited |
| part of | `related` with `Relationship:backward+` to the whole's entity selector | `selector-conformance`, negated when prohibited |
| `minOccurs`/`maxOccurs` | — | `object-count` per source: required reports a model without applicable objects, prohibited a model with any |

The predefined type follows IDS: the type object's own, or its element or process type when that is user-defined or unset, unless the result is empty or `NOTDEFINED`; otherwise the occurrence's own, or its object type when user-defined or unset. A literal `USERDEFINED` asks whether the type is user-defined at all. A part-of relation is followed from the part to its wholes one or more times, and only that relation; a pattern for the whole drops classes that can never be the relation's relating end.

Values keep IDS casting where a capability casts: `property-value` casts literals to the resolved value's kind, including `totalDigits` and `fractionDigits`. A selector compares one declared type, so an attribute value in a selector is translated only when every applicable class declares the attribute as text, an enumeration, a boolean or an integer; XML Schema patterns go through `axioval_rules::translate_xsd_pattern`, which refuses what it cannot map exactly.

## Gaps

These parts stay explicit gaps:

- a property facet in the applicability: IDS casts its literal to each property's own type, and a selector compares one declared type and cannot tell null or blank from a value without it;
- property set and property name patterns, which need the property service to enumerate an object's properties (openbimrs/ifc#78), and name enumerations anywhere but a prohibited facet without a value;
- a prohibited property facet with a value or a data type;
- an attribute declared as a real, a measure, a date, a select, a reference or an aggregate, in a selector;
- a classification without a value (the system alone), a classification pattern, and an optional classification with a value;
- a material with a value, which IDS matches against every member and every member material's name and category;
- part of without a relation (every relation, mixed along the chain) or through `IFCRELVOIDSELEMENT IFCRELFILLSELEMENT`;
- requirements on a prohibited specification, which IDS declares invalid;
- entities that are not `IfcObject` occurrences, which a model session does not check, and `IFC4X3_ADD2`.

## Conformance corpus

`IDS_TEST_CASES=<IDS>/Documentation/ImplementersDocumentation/TestCases cargo test -- --ignored corpus` runs every buildingSMART test case through the IFC adapter and the engine. It asserts that no translated rule fails a `pass-` case and that every `fail-` case either produces a finding or is explained by a reported gap. `IDS_CORPUS_VERBOSE=1` lists every case with its findings.
