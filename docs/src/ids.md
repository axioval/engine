# IDS import and export

`axioval-ids` translates a buildingSMART IDS 1.0 document into a definition package and a ruleset that select the built-in capabilities, and [exports](#export) the rules IDS states exactly back into an IDS document. It lives in `crates/packages/ids`, reads IDS through the `openbim-ids` reader, and is published with the workspace (see [Publishing](#publishing)). It is a package importer, not a source adapter; it never reads a model.

## Exact or not at all

A facet becomes a rule only when a capability decides it exactly as IDS does. Anything else is a reported `Gap` naming the part of the specification and the reason, so a caller can refuse an incomplete translation. An untranslatable applicability facet leaves the whole specification without rules, since dropping it would widen the checked population; an untranslatable requirement leaves only itself out, since dropping it can only miss failures.

## Audit

Before anything is translated, `axioval_ids::audit` checks the document
against the IFC schemas of the releases each specification lists, with
`openbim_ids::audit`: entity names (upper case, defined by the release),
predefined types, requirement entities an applicable object can be,
attribute names (explicit, not derived or inverse) and values comparable
with them, values castable to the type they are compared with, patterns on
strings only, property `dataType`s, `partOf` wholes the relation can have,
requirements on a prohibited specification, and contradictory bounds.
These are the documents the buildingSMART `invalid-` test cases describe:
valid against `ids.xsd`, but unable to work as written.

- An **error** refuses the whole document: `translate` returns
  `TranslateError::Invalid` with every finding (`Invalid::findings`, each
  with its stable code such as `entity-unknown`, the specification's index,
  the path within it, the release and a message), and nothing is
  translated. A document is never partly checked because one specification
  is meaningless.
- A **warning** (`pattern-unverified`: a pattern using `\i`, `\c`,
  character-class subtraction or a `\p{Is…}` block, which the audit does
  not evaluate) refuses nothing and is kept in `Translation::warnings`.

The audit runs every check except those against the standard property and
quantity set templates (`Pset_`/`Qto_` properties and their data types),
which need `openbim-ids`'s `audit` feature and its embedded template data
(see [Publishing](#publishing)). A misspelt standard property is therefore
not refused; it is a property no model holds.

From `openbim-ids` 0.2.2 on, the audit compares a required entity with the
applicable one through the IFC2X3 occurrence/type mapping table, so an
IFC2X3 specification applying to `IFCFLOWTERMINAL` and requiring
`IFCAIRTERMINAL` is translated and flags the flow terminals not typed by an
air terminal type (openbimrs/ids#15, #215). Two mapped names, or a mapped
name and a different occurrence class, still contradict and are refused.

## What each facet becomes

The applicability is one selector: the entity's classes (never their subclasses), its predefined type, and a selector for every other facet a selector states exactly, all of which must hold. A facet no selector states exactly is checked as a requirement by an [auxiliary rule](./gates.md#auxiliary-rules) (`spec<n>.applicability<k>`) over the rest of the applicability, and the applicable objects are those it passed: every rule of the specification selects `ruleOutcome` `passed` of it. The auxiliary rule reports nothing itself; an object it could not decide is not evaluated by every rule of the specification, never applicable and never left out. Requirements become rules over that selector.

| IDS | Applicability | Requirement |
|---|---|---|
| entity | `entityType` per class, `includeSubtypes: false`; an enumeration or pattern is matched against the release's classes; in IFC2X3, a class the IDS type mapping table renames is its occurrence class `related` by `IfcRelDefinesByType:backward` to its type object class (below) | `selector-conformance`; the applicability's own entity needs no rule |
| predefined type | property selectors over `axioval:type-attributes` and `axioval:attributes`, resolved as below | as the entity |
| attribute | property selectors in `axioval:attributes`, typed by the attribute's declared type; an attribute declared as a real, a measure, a date, a select, a reference or an aggregate: an auxiliary rule checking it as required | `property-required` or `property-value` in `axioval:attributes`; prohibited without a value: a `property-requirements` row excluding `not-empty`; prohibited with a value, or a restricted name: `selector-conformance`, or the prohibited value's negation (below) when no selector compares it |
| property | an auxiliary rule checking it as required | `property-required`, `property-data-type`, `property-value` (with `quantifier` and `si_units`); a set or name given as a pattern or an enumeration of several: `property-value` with `property_set_pattern`/`property_pattern`, or a required `property-requirements` row with the pattern columns; prohibited without a value: `property-requirements` rows excluding `not-empty`, one per enumerated set and name, or one pattern row; prohibited with a value or a data type: the negation (below) |
| classification | `classification` selectors: a literal or enumerated system with a code (`includeDescendants`), a `codePattern` per pattern (`includeDescendants`), or neither for the system alone; a system given as a pattern: an auxiliary `classification` rule | `classification`: systems and codes as literals or patterns, a system alone, `optional` or `prohibited` |
| material without a value | `exists` on `axioval:material.Kind` | `selector-conformance`, negated when prohibited |
| material with a value | the value against `axioval:material.Names` with `quantifier: any`; restricted by several facets one name must meet together: an auxiliary rule checking it as required | `selector-conformance`, negated when prohibited; optional: no material, or one going by the value; restricted by several facets: `property-value` over `axioval:material.Names` with `quantifier: any` (`optional` when optional), and the negation when prohibited |
| part of | `related` with `Relationship:backward+` to the whole's entity selector, one step naming every relation it follows (`IfcRelFillsElement\|IfcRelVoidsElement:backward+`) | `selector-conformance`, negated when prohibited |
| `minOccurs`/`maxOccurs` | — | `object-count` per source: required reports a model without applicable objects, prohibited a model with any |

A prohibited facet no capability negates is written as two rules: an auxiliary rule `spec<n>.facet<k>.required` checking the facet as required over the applicability, and `spec<n>.facet<k>`, a `selector-conformance` rule over the objects that auxiliary rule passed, which fails every one of them: meeting the facet breaks the prohibition. An object the auxiliary rule could not decide is not evaluated.

The predefined type follows IDS: the type object's own, or its element or process type when that is user-defined or unset, unless the result is empty or `NOTDEFINED`; otherwise the occurrence's own, or its object type when user-defined or unset. A literal `USERDEFINED` asks whether the type is user-defined at all. A classification requirement is met by one assignment in a matching system whose code, or an ancestor's, matches; an optional one holds for an object with no classification at all and must be met by one with any. A material value is matched against every name and category the material goes by (the set's, each member's, each member material's), as `axioval:material.Names` lists them. A part-of relation is followed from the part to its wholes one or more times, and only that relation; `IFCRELVOIDSELEMENT IFCRELFILLSELEMENT` follows both, from a filling element through its opening to the element the opening voids (the opening is a whole along the way), and a part-of without a relation follows all six relations IDS names, mixed along the chain. A pattern for the whole drops classes that can never be a followed relation's relating end.

In IFC2X3 a class the IDS IFC2X3 occurrence and type mapping table renames (`IFCAIRTERMINAL`, which IFC2X3 does not define) matches an occurrence of the mapped class typed by a type object of the mapped type class: an `IfcFlowTerminal` whose `IfcRelDefinesByType` type object is an `IfcAirTerminalType`, both exactly. Its predefined type is then its type object's, as for any typed occurrence. Only the two classes are read, never `IfcTypeObject.ApplicableOccurrence`. The class itself is matched in the other releases, and the mapped objects only in a source whose schema is `IFC2X3` (a `source` selector), since in IFC4 a flow terminal typed by an air terminal type is no air terminal. A pattern that also matches the occurrence class needs no mapping, since the mapped objects are among that class's.

A logical unknown (`IFCLOGICAL(.U.)`) matches no value and no boolean, as the buildingSMART case "a logical unknown is considered as not matching" requires: the IFC adapter reads it as a property holding no value, like `$`, so a required facet on it fails, its `dataType` included, and a prohibited one holds. An attribute's `.U.` is absent the same way.

A property value is judged as IDS judges list, bounded, table and enumerated values (a `dataType` on a table whose columns differ selects the cells of the column declaring it): `quantifier` `any` (one of its values) for a literal, an enumeration or a pattern, and `all` for a range restriction, so a bounded value must lie within the range as a whole. `si_units` reads the literal of a measure in SI, the unit IDS states measures in. A set or property named by an `xs:pattern`, or by an enumeration of several names (written as an escaped alternation), is enumerated through the property service: one property must match in every set the set name matches, and every matching property must satisfy the facet, as the buildingSMART cases require. An enumeration narrowed by patterns is the names that match them. A matching set that holds no property at all is listed by the enumeration as empty, so a required facet fails on it as IDS requires, and a property named in it exactly is absent.

Values keep IDS casting where a capability casts: `property-value` casts literals to the resolved value's kind, including `totalDigits` and `fractionDigits`, which is why a property facet in the applicability is decided by an auxiliary `property-value` rule rather than a selector, which compares one declared type. An attribute value in a selector is translated only when every applicable class declares the attribute as text, an enumeration, a boolean or an integer; XML Schema patterns go through `axioval_rules::translate_xsd_pattern`, which refuses what it cannot map exactly.

### Expressions

Where an attribute facet is tested by a selector (in the applicability,
and prohibited with a value), a selector cannot state `totalDigits` or
`fractionDigits`. On an attribute every applicable class declares an
integer (`IfcTask.Priority`), the facet becomes an
[expression](./expressions.md) rule (`axioval:capability.expression`,
definition `<package>.expression`) instead of a gap, stating exactly the
test the selector would:

```json
{"kind": "not", "operand": {"kind": "and", "operands": [
  {"kind": "isDefined", "operand": {"kind": "property", "propertySet": "axioval:attributes", "property": "ids:x.integer-attribute-1"}},
  {"kind": "compare", "operator": "greaterThanOrEquals", "left": …, "right": {"kind": "literal", "value": {"type": "integer", "value": 0}}},
  {"kind": "between", "operand": …, "low": {"kind": "literal", "value": {"type": "integer", "value": -99}}, "high": {"kind": "literal", "value": {"type": "integer", "value": 99}}}]}}
```

That is a prohibited `<xs:minInclusive value="0"/><xs:totalDigits value="2"/>`:
the attribute holds a value, the enumeration is `oneOf`, each bound a
comparison, and `totalDigits` n the range `-(10^n - 1)` to `10^n - 1`
(from 19 on every integer meets it, so nothing is written); an integer has
no fraction digits, so `fractionDigits` always holds. Prohibited, the
whole is negated by `not`; in the applicability it is the auxiliary rule
`spec<n>.applicability<k>`. A required or optional facet stays
`property-value`, which counts digits the same way, so a prohibited facet
fails exactly the objects its required form passes. The expression reads
the attribute through its own concept, `integer-attribute`, declared an
integer so it type checks; it binds to the same names as the attribute's
nominal `string` concept. An absent or `$` attribute is `null`, which
`isDefined` reads as no value, and a value that cannot be read leaves the
object not evaluated.

Everything else stays as it was. A property's value restrictions, bounds
and digits included, are `property-value`, which judges lists, bounded
values, tables, units and data types as IDS does; an expression reads none
of those (a list or bounded value is no single value to it). Digits or
bounds on a text, enumeration or boolean attribute in those places stay a
`RestrictionFacet` gap: no expression compares them as IDS casts.

## Command line

`axioval check --ids rules.ids --model model.ifc` translates a document in
memory and runs it; `axioval ids translate rules.ids --definitions d.json
--ruleset r.json` writes the packages (see [Command line](./cli.md#ids-documents)).
Both run only complete specifications: a specification with any gap runs
none of its rules and is listed with its gaps, and a check with one never
exits 0. A document the [audit](#audit) refuses runs and writes nothing
and exits 1, every finding listed; audit warnings are listed on stderr and
in the result's `ids` field.

## Prefilter

`Options::filter` restricts every specification to part of the model, beyond
its own applicability: one storey, one discipline, one selection. The
selector is combined with each specification's applicability through
`allOf`, so every rule of the specification, the auxiliary and count rules
included, sees only the objects it selects; `None` translates what the
document states, unchanged.

The prefilter is written in IFC names, as IDS writes its facets, and bound
per specification like the specification's own names: an `entityType`
names a class (`IfcBuildingStorey`), a `property` a property set and
property (`Pset_WallCommon`, `FireRating`) or a reserved set and a property
in it (`axioval:attributes`, `Name`), and each becomes a concept named in
the specification's releases. Relationship paths, classification systems,
name patterns, disciplines and source fields are used as written. A
`ruleOutcome` selector is refused (`OptionsError::FilterRuleOutcome`): the
rules it could name are the translation's own. The walls of one storey:

```json
{"kind": "related", "path": ["IfcRelContainedInSpatialStructure:backward"],
 "selector": {"kind": "allOf", "operands": [
   {"kind": "entityType", "objectType": "IfcBuildingStorey"},
   {"kind": "property", "propertySet": "axioval:attributes", "property": "Name",
    "operator": "equals", "value": {"type": "string", "value": "Level 1"}}]}}
```

On the command line it is `--ids-filter selector.json`, for `check --ids`
and `ids translate`.

## Export

`axioval_ids::export(definitions, ruleset)` writes a ruleset back as IDS
1.0, and `axioval ids export` on the [command line](./cli.md#axioval-ids-export).
`IdsProfile` is the same export behind the shared
[export profile](./export.md) with id `ids`, which `axioval export --profile
ids` runs: its artifact is the document, and every `NotExported` rule is a
refused `Loss` at the rule's id with its `Refusal` as the reason. IDS
degrades nothing; a rule it holds, it holds exactly.
Most rules cannot be stated in IDS (geometry, routing, counts of related
objects, comparisons between objects), so the export is selective and
explicit: a rule is exported only when IDS states it exactly, and every
other rule is listed as `NotExported` with a `Refusal` saying why. Nothing
is approximated and nothing is dropped silently; `Export::is_complete`
says whether anything was left out.

Exact means the translation above is the judge, compared with the
comparator every profile shares (`axioval_export::compare`, with object-type
names ignoring case and a `selector-conformance` message left out). A rule
is exported as a specification only when translating that specification
gives the rule again: the same capability and parameters (with the definition's
defaults), the same applicability, gates, severity and grading, every
concept bound to the same names and every `ruleOutcome` to the same rule.
Names, descriptions, messages and tags are presentation and not compared.
A specification comes from one of two places:

- **Its origin.** Every folder a translation writes keeps the
  specification it came from in its `ids:specification` annotation, as the
  IDS XML of the `<specification>` element, and the root folder keeps the
  document's `<info>` under `ids:info.title`, `ids:info.author` and the
  other field names. That folder is exported as the specification it keeps
  (names, identifier, description, instructions, `ifcVersion`, facet
  cardinalities, `uri`s and instructions included), with the folder's own
  name and description, when translating it gives exactly the folder's
  rules, auxiliary rules included. A complete specification whose facets
  always hold writes an empty folder for this, so a translated document is
  exported again specification by specification. A folder whose rules were
  edited, that was translated with a prefilter, or whose specification had
  a gap is refused as a whole (`Refusal::Origin`).
- **One rule.** Any other rule is read as one specification: its
  applicability as an entity facet (exact classes, a predefined type as the
  translation resolves it) followed by the part-of, classification,
  attribute and material facets its selector's operands state, and its
  capability as one requirement: `property-required`,
  `property-data-type`, `property-value` (values, patterns, bounds,
  lengths and digits; `optional`), `property-requirements` rows forbidding
  a value or requiring every property a pattern matches, `classification`
  (`optional`, `prohibited`), and `selector-conformance` over an entity,
  attribute, material or part-of selector, negated when prohibited, and
  `expression` stating an integer attribute facet as the translation
  writes it ([Expressions](#expressions)).
  `object-count` is the applicability's `minOccurs`/`maxOccurs` with no
  requirement. The specification is named after the rule, identified by
  its id, and lists all three releases, since every concept a translation
  writes is named in all three.

Among the refusals: a capability no facet states (`clash`, `distance`,
every geometric or counting capability), a selector no facet states (a
subtype-inclusive entity type, a rule outcome, a discipline), a concept not
named alike in IFC2X3, IFC4 and IFC4X3_ADD2, a severity other than error,
a disabled, auxiliary, gated or graded rule, target groups, and a reading
whose translation differs (`Refusal::Differs` names the first differing
path, such as `rules[0].parameters.si_units` for a `property-value` rule
that reads values in the model's units rather than SI).

### Expression rules

`IdsProfile` declares the expression nodes an IDS facet may state
(`expression_kinds`): `and`, `between`, `compare`, `isDefined`, `literal`,
`not`, `oneOf` and `property`. An expression rule is read as one attribute
facet only in the form the translation writes: `and` whose first operand
is `isDefined` of an attribute in `axioval:attributes` read through a
concept declared an integer, and whose others are `oneOf`, an ordered
`compare` or a `totalDigits` `between` of that attribute and integer
literals, negated by `not` when prohibited. The reading is then checked
like any other: exported only when translating it gives the rule again.
A prohibited one is exported and round-trips, alone or in its folder; a
required one reads as a required facet, which translates to
`property-value`, and is refused as `Differs`; a `deviation` is refused as
a parameter, and a `message` is presentation.

Every other expression rule is refused with `Refusal::Expression { path,
kind, why }`, naming the first node no facet states by the path the
engine names it with: first any node of a kind outside the declared set
(an `aggregate`, arithmetic, a rule outcome), then any declared node in a
place or with operands no facet has (a measured value, a `like`
comparison, a range that is no digit count, a concept declared as text):

```text
expression node `requirement.and[1]` (`aggregate`) has no IDS facet: IDS states no such computation
expression node `requirement.and[0].isDefined.operand` (`property`) has no IDS facet: it reads the set axioval:measured; …
```

An expression inside a selector (an `expression` selector in the
applicability) is refused as a selector, as before.

### Round trip

A package translated from IDS re-exports the same specifications: for every
buildingSMART test case that translates without a gap, exporting the
translation gives a document that reads back to the original
specifications and info, translates to identical packages, and reports
identical findings on the case's model. The same rules exported without
their origin, one specification per rule, translate back to rules that
report on the same objects. The [conformance corpus](#conformance-corpus)
test asserts both and validates every exported document against the
corpus checkout's `ids.xsd`.

### Writing IDS

Documents are written by the `openbim-ids` writer (`openbim_ids::to_string`,
0.1.4 or later), which checks the whole model against `ids.xsd` 1.0 before
writing and escapes every text so it reads back unchanged. A rule read as
one specification lists its applicability facets in the schema's sequence
(entity, partOf, classification, attribute, property, material) before its
translation is checked. The writer also refuses two schema-valid shapes
that express nothing, an applicability without facets and a restriction
without facets; a specification it refuses is not exported
(`Refusal::Unwritable`, with the location in the specification, such as
`requirements/facets[1]/value/xs:restriction`). `Export::to_xml` returns
`DocumentError::Write`, the writer's `WriteError`, when the `<info>` a root
folder keeps cannot be written (an author that is no e-mail address, a
date that is no `xs:date`); the `ids` profile then writes no document and
refuses every rule, naming the location.

No invalid IDS is ever written. Every specification a rule or a folder
reads as is [audited](#audit) on its own before it is exported, and one
the audit refuses is not (`Refusal::Invalid`, with the findings): a rule
read as one specification lists all three releases, so a rule over a
class one of them lacks (`IFCCHIMNEY`, which IFC2X3 does not define) is
refused rather than written as a specification that cannot work for
IFC2X3. `Export::to_xml` audits the whole document again before writing
it; an error then is an exporter bug, and it returns
`DocumentError::Invalid` with the findings instead of a document, which
the `ids` profile turns into a refused loss for every rule.

The one thing `axioval-ids` still writes itself (`src/write.rs`) is the
`ids:specification` annotation: the `<specification>` element cut out of a
one-specification document the upstream writer wrote, since `openbim-ids`
reads and writes whole documents only. It is read back by wrapping it in a
document, so annotations written by earlier releases, on one line, still
read. A folder whose specification the writer refuses keeps
`ids:unwritable` (`<location>: <why>`) instead, and the export refuses its
rules with it rather than exporting them one by one. The schema is CC BY-ND
4.0 and not vendored: unit tests check what they write by reading it back,
and the corpus test validates against the checkout's schema with Python's
`lxml`.

### Folder annotations

The origin travels in `annotations`, an optional map on every rule folder
of the normalized package contract: namespaced keys (`scheme:name`) with
text values, omitted when empty. The engine never reads it, so it changes
no selection, evidence or outcome; an importer or authoring tool keeps its
provenance there. The MCS rule folder needs the same optional field for a
package authored in MCS to carry it.

## Gaps

These parts stay explicit gaps:

- an attribute facet whose name is a restriction, with a value or a cardinality other than required, and one whose named attributes a selector cannot compare;
- in the applicability, or prohibited with a value, an attribute facet whose restriction a selector cannot apply (`RestrictionFacet`): bounds or digits on a text, enumeration or boolean attribute, lengths or patterns on an integer one, and a length beyond 1000 characters; digits on an integer attribute become an [expression](#expressions);
- an applicability without an entity facet, which in IDS covers every resource of every class;
- a part-of whole that is neither an `IfcObject` occurrence, an `IfcContext` nor an `IfcTypeObject`, which no relationship traversal reaches.

An entity that is neither an `IfcObject` occurrence, an `IfcContext` nor an `IfcTypeObject` (`IFCMATERIAL`, `IFCTASKTIME`, `IFCRELCONNECTSPATHELEMENTS`) is translated like any class: its `entityType` selector selects the model's [resource objects](./capabilities.md#entity-types-and-resource-objects) of that class, which are judged through the same services as objects. Their attributes follow IDS: an empty list or set and a logical unknown hold no value, and a select or reference naming an entity holds one. A material's property facet is decided on its own material property sets (IFC4 `IfcMaterialProperties`, IFC2X3 `IfcExtendedMaterialProperties` and the typed subtypes, see [adapters](./adapters.md)), as the buildingSMART cases "material properties are supported" require; a property facet on any other resource is not evaluated. A classification facet reads a resource's `IfcExternalReferenceRelationship`s and a material's `IfcMaterialClassificationRelationship`s. A type object is checked as itself: its own property sets, attributes, classifications and materials, and its predefined type is its own, or its `ElementType`, `ProcessType` or `ResourceType` when user-defined. An abstract class has no instances and matches nothing, as in IDS.

A property set that holds no property is malformed IFC (`HasProperties` and `Quantities` are `SET [1:?]`), but IDS still judges it: a required property in a matching set that is empty fails. The IFC adapter lists such a set with no members (`empty_sets` of the enumeration), a property named in it is absent, and `property-value` and `property-requirements` count it among the sets that need a match, so a required facet on it is a finding, as IDS requires. Only an empty set sharing its name with a set that holds members stays refused, and a facet on such an object is not evaluated, never passed.

A specification's `ifcVersion` is metadata that never changes a verdict, as the buildingSMART case "specification version is purely metadata" requires: every concept is named in the IFC adapter's type systems of all three releases IDS names, so an `IFC2X3` specification checks an `IFC4` or `IFC4X3_ADD2` model too. A class some release lacks matches nothing in its models, as in IDS; a specification naming a class one of its listed releases does not define, an entity name in mixed case, or requirements on a prohibited specification is refused by the [audit](#audit). The translation keeps a gap for each (`UnknownEntity`, `EntityCase`, `ProhibitedRequirements`) as a second line of defence, which an audited document never reaches. The IFC2X3 type mapping therefore applies to every specification, in IFC2X3 sources only.

`IFC4X3_ADD2` models are checked through the IFC adapter's IFC4X3 type system. Classification facets are decided on them as on IFC4 models, read with the IFC4X3 table.

## Conformance corpus

`IDS_TEST_CASES=<IDS>/Documentation/ImplementersDocumentation/TestCases cargo test -p axioval-ids -- --ignored corpus` runs every buildingSMART test case through the IFC adapter and the engine. It asserts that no translated rule fails a `pass-` case and that every `fail-` case either produces a finding or is explained by a reported gap. `IDS_CORPUS_VERBOSE=1` lists every case with its findings.

It also asserts the [audit](#audit): every one of the 27 `invalid-` cases is refused, and the 307 `pass-` and `fail-` cases audit without a single finding, warnings included. None of the `invalid-` cases needs the template checks the audit leaves out; one that did would be listed in `TEMPLATE_ONLY` with the check it needs, and skipped.

The same run checks the [round trip](#round-trip) of every case that translates without a gap: all 307 are exported again, read back to the original specifications and info, translate to identical packages and report identical findings, and 565 of their 603 rules are also exported one by one without their origin, reporting on the same objects. The other 38 read as specifications for all three releases over a class or attribute IFC2X3 lacks (`IFCTASKTIME`, `IfcPerson.Identification`), which the audit refuses. Every exported document is validated against `Schema/ids.xsd` of the checkout, which needs `python3` with `lxml`.

Some facets translate but cannot be decided on some models, and are reported not evaluated rather than as gaps: a property whose value is an `IfcPropertyReferenceValue` referencing an entity, which the adapter refuses (one referencing nothing is no value, and fails a required facet as IDS requires); and the value of a measure whose unit the model does not resolve. Such a measure's declared type is still exact, so its `dataType` is judged: `IFCMASSMEASURE(2.)` fails `dataType="IFCTIMEMEASURE"` whatever its value, as the buildingSMART case "measures are used to specify an IFC data type" requires, and a required facet without a value is met.

A complex property or quantity (`IfcComplexProperty`, `IfcPhysicalComplexQuantity`) is present and holds no value of any type, as the buildingSMART case "complex properties are not supported" requires: it meets a required facet, and fails a `dataType` or a value restriction.

## Publishing

`axioval-ids` is a workspace member under `crates/packages/ids` and is published with the other crates at the workspace version. It reads IDS with the `openbim-ids` reader (`openbim_ids::read`, `from_str`, `from_slice`) and writes it with its writer (`openbim_ids::to_string`), and audits it (`openbim_ids::audit`) with the `audit-schema` feature, from 0.2.2 on. `scripts/package.sh` packages and verifies it with the rest of the workspace (`EXPECTED` in `scripts/check_package_contents.py`).

`audit-schema` runs every audit check against the IFC schema tables of `ifc-schema` (AGPL-3.0-or-later, as the crate already used it), and adds only `regex`. The `audit` feature is not enabled: it adds the standard property and quantity set template checks and pulls in `ifc-template-catalog`, whose licence is `AGPL-3.0-or-later AND CC-BY-ND-4.0`: its embedded template data (`data/*.bin`, compiled in through `include_bytes!`) is buildingSMART's PSD/QTO content under CC BY-ND 4.0. `cargo deny` does not allow CC-BY-ND-4.0, and whether distributing that data inside the published crates is acceptable is the maintainers' decision, not a dependency bump's. `cargo tree -i ifc-template-catalog -e normal` prints nothing.

It is a package importer, not a source adapter, so the architecture gate treats it as core: it may not depend on any source adapter, format library or geometry kernel, with one narrow exemption in `scripts/architecture.py` (`PERMITTED_COUPLINGS`). The importer alone may depend on `openbim-ids`, to parse IDS, and `ifc-schema`, to ask which IDS classes are occurrences in each IFC release, and may use `openbim_ids::` and `ifc_schema::` paths; every other coupling still fails it, and both crates stay forbidden to every other core crate.

Its tests run the translated rules through the facade's IFC adapter. The facade is a path-only dev-dependency, which is left out of the published manifest; `cargo deny` permits it through `allow-wildcard-paths`, which exempts dev-dependencies only. `./scripts/check.sh test` runs the [conformance corpus](#conformance-corpus) too when `IDS_TEST_CASES` is set, and skips it otherwise.
