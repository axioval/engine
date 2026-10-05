# Source-neutral IR

## Package compatibility

The compiler currently accepts normalized Axioval Schema `0.1.0` packages only. Both definition packages and rulesets are checked before binding; unknown versions fail with `UnsupportedSchemaVersion` rather than being interpreted as the current contract.

Both package forms are published as generated [JSON Schemas](./json-schema.md), so an editor can check a draft's structure without Rust.

The IR describes what a checker can observe without mirroring any source schema.

## Identity

- `ProjectId` identifies a validation project.
- `SourceId` identifies one contribution.
- `ViewId` identifies a stable composition used for a run.
- `ObjectId` is opaque and valid only with its `SourceId`.
- `ObjectRef` combines source and object identity.

Adapters may expose external aliases, but aliases never replace source-qualified identity.

A `Discipline` names the role a source plays in a check (`architecture`, `structure`): a lowercase token of 1 to 64 ASCII letters, digits, `-` or `_`, starting with a letter or digit, validated when constructed or deserialized. It is a host declaration about a source, never read from it, and the IR attaches no vocabulary to it. The `discipline` selector (`{"kind": "discipline", "value": "structure"}`) selects by it; see [Discipline selectors](./capabilities.md#discipline-selectors).

An alias is an `ExternalId`: an adapter-defined `scheme` and a `value`, listed in `Object::external_ids` and read with `Object::external_id(scheme)`. It exists so output formats and cross-revision tools can name an object the way other software does; capabilities never key on it. `Project::new` rejects an object with two ids in one scheme and two objects of one source sharing an id, because any consumer resolving that alias would pick one silently. Two sources may share an alias: two revisions of one model do.

### Resource objects

A project's objects are what an adapter calls its checked objects: in IFC every occurrence, context and type object. A source holds more: materials, classifications, relationships, task times, surface styles. These are *resource objects*. They have the same identity as objects (an `ObjectId`, source-qualified and stable, with an `ExternalId` alias where the source gives one, such as an IFC relationship's GlobalId) and the same `Object` shape (a kind, aliases, and no facts of their own), but they are never in the project and never in the default population: `Project::objects` does not list them, so a rule selecting every object, an object count over all objects, BCF viewpoints and geometry never see them.

A rule reaches resource objects only by naming their class in an `entityType` selector (see [Entity types and resource objects](./capabilities.md#entity-types-and-resource-objects)). The runtime asks the source's `ResourceService` for exactly the classes the rules name, so a class no rule names is never listed. Their facts are resolved through the same services as an object's, keyed by the resource object's identity.

## Semantic data

Objects expose canonical concepts, typed properties, classifications and directed relationships. Canonical concept IDs are package vocabulary identifiers; source-specific names are adapter bindings. Stored property values are observations, not proof that an omitted key is absent. A property may carry `data_type`, the value's type as the source declares it in its own vocabulary (IFC: `IFCLABEL`); `None` means unreported, never any particular type. A table value whose two columns declare different types has no `data_type`; it may carry `column_types` instead, the defining and the defined column's declared type. Conclusive property checks use the typed property-resolution service and exact request-bound evidence.

Values distinguish null/unavailable from concrete values and preserve units where relevant. Adapters must not silently coerce malformed source values.

A quantity is stated in the coherent SI unit of its `QuantityDimension`:
- `Length`, `Area` and `Volume` in metres, square metres and cubic metres;
- `PlaneAngle` in radians;
- `Other { exponents }` for any other dimension, given as SI base-unit exponents. A thermal transmittance in W/(m²·K) is `[0, 1, -3, 0, -1, 0, 0]`.

Quantities compare only when their dimensions are equal. A dimensionless measure, such as a ratio, is a plain decimal.

A `Date` is a calendar day of the proleptic Gregorian calendar in the years 0000 to 9999, optionally with the time zone it was stated in (`Date::offset_minutes`), and a `DateTime` an instant to the nanosecond together with the UTC offset it was stated in. Both are validated on construction and when read, so a value is always a real day. On the wire they are ISO 8601 extended strings, as `xs:date` and `xs:dateTime` write them: `{"type": "date", "value": "2026-09-27"}` and `{"type": "dateTime", "value": "2026-09-27T10:00:00.5+02:00"}`, a fraction of up to nine digits, and `Z` or `±hh:mm` up to 14 hours. A zoned date appends its zone as `xs:date` does (`{"type": "date", "value": "2022-01-01Z"}`, `2022-01-01+01:00`); a date without one is written exactly as before. A zoned date is a different value from the unzoned date of the same day, and `Date::cmp_timeline` is XML Schema's partial order of the two: they are ordered only more than 14 hours apart, and never equal. A date-time is written in its canonical form: no trailing fraction zeros, and `Z` for a zero offset. A date-time without an offset is not representable: it names a wall-clock time in an unknown zone and cannot be ordered against any instant, so an adapter reports it as incomplete evidence rather than guessing a zone; `-00:00`, `24:00:00` and leap seconds are refused for the same reason. Equality of values is structural (the same instant in two offsets is two values, as is the same day in two zones); comparisons order instants and read a date-time as the calendar day it states only at `day` precision (see [Dates and date-times](./capabilities.md#dates-and-date-times)). Package parameters carry `date` and `dateTime` literals (`ParameterKind::Date`, `ParameterKind::DateTime`), and a property definition may declare `valueKind` `date` or `dateTime`.

A `List` holds several values of one property, in the order the source states them, such as every presentation layer of an object. Its elements are scalar values: never `null` and never another list; the property resolver rejects any other list as an invalid value. A list is never compared as a whole or as if it were one of its elements: a property selector states a `quantifier` (see [Property selectors](./capabilities.md#property-selectors)). Its wire form is `{"type": "list", "value": [{"type": "string", "value": "A-WALL"}]}`.

A `Reference` value (`{"type": "reference", "value": {"source": {…}, "local_id": "#14"}}`) names another instance of the same source by its `ObjectId`: an IFC attribute referencing an entity, such as the colour a surface style's `DiffuseColour` selects or the wall a relationship's `RelatingElement` names. It states that the value is set and what it names; it is scalar, never null, and never compared with a literal: a comparison is undecided (`property-value` is not evaluated), while presence (`property-required`, `exists`, `notEmpty`) holds. The property handle refuses a reference to another source.

A `Measured` value (`{"type": "measured", "value": {"lower": …, "upper": …, "dimension": …}}`) is one quantity known only to lie in `[lower, upper]`, finite and ordered, in SI units: a value measured from a tessellated body. Without a `dimension` it is a plain number known only to an interval, such as a count of objects some of which are undecided. It is scalar; a comparison it may pass or fail is undecided. A `Bounded` value is a range a source states as one value: an optional `lower` and `upper` bound and an optional `set_point`, at least one of them stated, each a scalar of one kind. An unstated bound leaves the range open on that side; it is never zero or infinity. A `Table` holds rows of a `defining` and a `defined` value (`PropertyTableRow`), in source order, at least one row, every cell a scalar. Their wire forms are `{"type": "bounded", "value": {"lower": {…}, "set_point": {…}}}` and `{"type": "table", "value": [{"defining": {…}, "defined": {…}}]}`. `PropertyValue::stated_values` lists the scalar values a list, bounded value or table states (a list's elements; lower, upper and set point as far as stated; each row's defining then defined value), which is what a quantified comparison compares; a capability judging a range against bounds also considers its open ends. `is_scalar` is true for every other value but `null` and `complex`.

A `Complex` value (`{"type": "complex"}`) is a property that groups named member properties and is no value of its own, such as an IFC complex property or physical complex quantity. It is present and not empty, declares no type (the property handle refuses one that does, and a complex inside another value) and holds no value of any simple type: `property-required` and `exists` hold, `property-data-type` and every value constraint of `property-value` fail, `property-predicate` and `property-comparison` never pass, and a property selector comparing its value leaves the object undecided. Its members are not carried.

### Attribute sets

Some facts about an object are not in any property set but in fields of the object itself, such as a space's number and name, or the name of its construction type. Reserved property-set names reach them through the same property resolver, and so do the object's presentation layer, material and body:

- `ATTRIBUTE_SET` (`axioval:attributes`) names the object's own attributes, by the source's attribute name.
- `TYPE_ATTRIBUTE_SET` (`axioval:type-attributes`) names the attributes of the type object the source assigns to the object. An object with no type has none (an exact absence). An object with several types is a conflict, not a choice.
- `PRESENTATION_SET` (`axioval:presentation`) names how the object is presented. Its property names are matched ignoring ASCII case. `Layer` (`PRESENTATION_LAYER`) lists every presentation (CAD) layer of the object's shape, as a list of strings sorted by name. An object on no layer has none (an exact absence), but only in a source that assigns layers at all: a source with no layer assignment answers that it records no layers, so a layer rule over it is not evaluated (`NotRecorded`, once per rule and source) instead of flagging or passing every object.
  `Transparency` (`PRESENTATION_TRANSPARENCY`) lists how transparent the object's styled surfaces are, as a list of decimals from `0.0` (opaque) to `1.0` (fully transparent): every distinct value, ascending, so a comparison states whether `any` or `all` surfaces must satisfy it. An object with no styled surface has none (an exact absence); a source's missing styles are never read as opaque or as transparent.

- `MATERIAL_SET` (`axioval:material`) names the material the object is made of: its own, or else its type object's. Its properties, matched ignoring ASCII case:
  - `Kind`: `material`, `layer-set`, `constituent-set`, `profile-set` or `list`.
  - `Name`: the name of a single material, or of the set. `Category`: the category of a single material.
  - `TotalThickness`: the summed layer thickness of a layer set, a length in metres.
  - `Count`: the number of layers, constituents, profiles or listed materials.
  - Members numbered from 1 in the source's order: `Layer<n>.Material`, `.Thickness` (a length), `.Name` and `.Category`; `Constituent<n>.Material`, `.Name`, `.Category` and `.Fraction` (a decimal); `Profile<n>.Material`, `.Name` and `.Category`; `Material<n>.Name` and `.Category` for a list. `.Material` is the name of the member's material.
  - `Names`: every name the material goes by, as a list of strings, distinct and sorted: the material's or the set's name and category, each member's name and category, and each member's material's name and category. A selector with `quantifier: any` asks whether one of them is a given name without enumerating members.

  An object without material has none of them (an exact absence), and neither does a member that does not exist or states no value. Two material assignments on the object, or on its type, are a conflict.

- `BODY_SET` (`axioval:body`) states how the object's body is modelled, read from the body representation the source authors, never from a mesh. Lengths are in metres, angles in radians, positions and directions in the source's model coordinates. Its properties, matched ignoring ASCII case:
  - `Count`: the number of geometric items, mapped items resolved. `Kinds`: every distinct item kind, as a list of strings, sorted. `Mapped`: whether any item is reached through a mapping (a type's shared geometry placed at the occurrence). `Mirrored`: whether the transform placing the body reverses its orientation (a negative determinant); the IFC adapter states it for every item kind from the sign of the determinant of each item's frame (`ifc-geometry`'s `BodyItem::item_world`, which composes every mapping), and refuses it when a frame is degenerate or some items are mirrored and others not.
  - Items numbered from 1 in the source's order, `Item<n>.` followed by `Kind` and `Mapped`. The kinds are `extrusion`, `tapered-extrusion`, `revolution`, `tapered-revolution`, `directrix-sweep`, `swept-disk`, `sectioned-spine`, `brep`, `csg`, `csg-primitive`, `half-space`, `bounding-box`, `tessellation`, `surface-model`, `face`, `geometric-set`, `curve`, `surface` and `point`.
  - A swept-area item (the extrusions, revolutions and directrix sweeps) adds its `Profile.` facts, `EndProfile.` for a tapered sweep, and its placement: `Placement.OriginX`, `.OriginY`, `.OriginZ` (lengths) and the axes `Placement.XAxisX` … `Placement.ZAxisZ` (decimals); the profile lies in the placement's X-Y plane. An extrusion adds `Extrusion.Depth` (a length), `Extrusion.DirectionX`, `.DirectionY`, `.DirectionZ` (a unit vector) and `Extrusion.Inclination`, the angle between the extrusion's line (either sense) and the vertical, from 0 (a wall extruded up) to π/2 (a beam). A revolution adds `Revolution.Angle`, `Revolution.OriginX` … `.OriginZ` and `Revolution.AxisX` … `.AxisZ`.
  - `Profile.Type` names the family: `rectangle`, `rounded-rectangle`, `rectangle-hollow`, `circle`, `circle-hollow`, `ellipse`, `i-shape`, `asymmetric-i-shape`, `l-shape`, `t-shape`, `u-shape`, `c-shape`, `z-shape`, `trapezium`, `arbitrary-closed`, `arbitrary-with-voids`, `center-line`, `composite`, `derived` or `mirrored`. `Profile.Name` is its name (often a catalogue designation such as `HEA300`); `Profile.PositionX`, `.PositionY` and `.PositionAngle` place it where it states a position. The family's parameters follow under their dimension names: `XDim`, `YDim`, `RoundingRadius`, `WallThickness`, `InnerFilletRadius`, `OuterFilletRadius`, `Radius`, `SemiAxis1`, `SemiAxis2`, `OverallWidth`, `OverallDepth`, `WebThickness`, `FlangeThickness`, `FilletRadius`, `FlangeEdgeRadius`, `FlangeSlope`, the asymmetric I's `BottomFlange…` and `TopFlange…`, `Depth`, `Width`, `Thickness`, `EdgeRadius`, `LegSlope`, `FlangeWidth`, `WebEdgeRadius`, `WebSlope`, `Girth`, `InternalFilletRadius`, the trapezium's `BottomXDim`, `TopXDim`, `TopXOffset`, and `VoidCount`. An arbitrary closed profile states its outline as `Profile.OutlineX` and `Profile.OutlineY`, two lists of lengths: its vertices' coordinates in the profile's X-Y plane, in the order the source states them, the closing vertex not repeated; one with voids adds `Void<n>.OutlineX` and `Void<n>.OutlineY` per void. Only straight edges are stated so: an outline with a curved segment is refused, never approximated by chords, and the profile's other facts stand. A composite profile states `Count`, `Label` and `Member<n>.` profile facts; a derived or mirrored one `Label` and `Parent.` profile facts. Every parameterised section except the trapezium is centred on its position: the centre of its bounding box lies at the position's origin, and its depth runs along the position's Y axis.
  - Without the `Item<n>.` prefix, a name reads the body's only item: `Kind` and `Profile.OverallDepth` of a beam. On a body of several items such a name is a conflict, never a choice of item.

  An object without a body has none of them (an exact absence), and neither has an item of another kind, a parameter its family does not have, or one the source leaves unset, even where the schema defines a default: an equal-leg angle that states no `Width` has none, not its `Depth`. A body the source states but that cannot be read exactly (an item or profile family not read, a dangling reference, a mapping that scales a swept solid) is refused as a whole, never absent and never read in part.

- `CLASSIFICATION_SET` (`axioval:classification`) holds the classes a ruleset's classifications derive, one property per classification id: a string for a first-match classification, a list of strings for an all-match one, none for an object no row matches. A hierarchical classification (one declaring `classes`, a tree of ids, codes, localized names and parents) also answers `<id>;level=<n>`, the class at level `n` of the tree on the way from the assigned class to its root; `contract::ClassTree` checks the tree and answers levels and ancestry, and `contract::ClassificationProperty` parses the name. A `derivedClass` selector selects a derived class, with `includeDescendants` together with the classes below it. The engine answers it, never a source; see [Derived properties](./derived.md). `GROUP_SET` (`axioval:group`) states a derived group's `key` and `members` count (see [Derived groups](./derived.md#derived-groups)); a derived group is an object of kind `DERIVED_GROUP_KIND` (`axioval:group`) the engine derives, never a source. `is_derived_set` names the sets the engine derives.

- `VALUE_SET` (`axioval:value`) holds the values a ruleset derives from expressions (`RuleSetPackage::values`, `contract::ValueDefinition`: a localized `name`, an optional `description` and the `expression`; omitted when empty, so rulesets serialize as before). The engine answers it; see [Derived values](./derived.md#derived-values).
- `MEMBER_SET` (`axioval:member`) holds the fields of a measured member (a flight's step, a ramp's run, a rail), read only inside an aggregate over a measured member list (`measured::MEASURED_MEMBERS`, parsed by `measured::parse_members`); see [Measured members](./derived.md#measured-members).
- `SOURCE_SET` (`axioval:source`) states what an evidence session knows about the object's source: `discipline` (`SOURCE_DISCIPLINE`) and the source metadata `fileName`, `application`, `schema`, `project`, `timestamp`. The engine answers it; see [Source facts](./derived.md#source-facts).
- `MEASURED_SET` (`axioval:measured`) holds values measured from geometry: `extent_x`, `extent_y`, `extent_z`, `bottom`, `top`, `area`, `volume`, `x`, `y`, `z` and the parameterised `bottom_above_level;path=…` and `boundary_area;kind=…`, and the stated `level_height`, each declared in the registry `measured::MEASURED_VALUES` (parameters, dimension, services, exactness, labels), each a quantity when measured exactly and a `measured` interval otherwise. The engine answers it through the host's geometry services; see [Derived properties](./derived.md#measured-values).

These sets are engine vocabulary. They name no property set of any source, a package cannot redeclare them, and concept binding passes them through unchanged, while the property name inside them is still bound per source. A request without a set never searches attributes.

## Views and layers

A project can expose raw source views and composed views. This supports today's single IFC model and ICDD federation as well as future IFCX-style layers without changing rule capability APIs.

## Provenance

Every imported fact and computed evidence can reference its source record, adapter, derivation and precision. Findings carry the provenance needed to explain or reproduce a decision.

## Reports

`Report` keeps conclusive `findings` separate from `not_evaluated` outcomes, and both separate from the `tables` of measured values (see [Tables](#tables)). Every not-evaluated record identifies its rule and its scope, carries a typed reason, and includes a diagnostic. Runtime ordering is deterministic. An empty findings list is not a pass when not-evaluated outcomes exist.

`resources` carries every [resource object](#resource-objects) the report's findings, not-evaluated outcomes and tables name, sorted by identity, since the project cannot resolve them. `Report::object(project, id)` resolves an outcome's object either way, and finding identities, BCF and the CLI's object labels use it. It is omitted when empty, so a report naming no resource object serializes byte for byte as before.

### Scope

Every finding and not-evaluated outcome has a `Scope`: what it is about.

| Scope | Meaning | Example |
|---|---|---|
| `Scope::Object(ObjectId)` | one object | "wall `#42` has no fire rating" |
| `Scope::Source(SourceId)` | one source as a whole | "this model has no building" |
| `Scope::Project` | every source together; for a not-evaluated outcome, the rule as a whole | "no storey anywhere has a fire compartment" |

A result about the whole model has no object to be reported against, and reporting nothing would read as a pass. A source may hold no objects at all (an empty model); it is still a source of the run, and a source-scoped result can be about it (see [empty sources](./services.md)). Scoped findings follow the same evidence rules as object findings: they are conclusive, carry exact source evidence for what decided them (sorted by source and locator, deduplicated), and may name `related` objects, such as the objects a count found. `Finding::new(rule, scope, severity, message)` with `with_evidence` and `with_related` builds one; `Finding::object_id()` and `NotEvaluated::object_id()` return the object when there is one.

The runtime orders findings by rule, then scope, then message, and not-evaluated outcomes by rule, scope, reason and message. Scopes order project first, then sources, then objects, each by identity.

**Serialized form.** The scope is written as the fields reports have always had, so a report without scoped entries serializes byte for byte as before:

- an object finding has `object_id`; a source finding has `source` instead; a project finding has neither;
- a not-evaluated outcome always has `object_id` (`null` unless it is about one object), and `source` only when it is about one source.

A record naming both an object and a source is rejected: an object id already names its source. Readers written before scopes existed reject a finding without `object_id` or with `source`, so a report containing scoped entries needs a reader of this version.

### Location

A finding or not-evaluated outcome may carry a `location`: the storeys and
spaces (`Place`: id and name) its objects lie in, and `unresolved` when part
of it could not be derived. Only a host asking for it gets one (see
[Locations](./refinement.md#locations)); otherwise the field is absent and a
report serializes exactly as before.

### Categories

A finding carries `categories`, the nested category levels its rule's
`categories` read on its subject, outermost first (see
[Nested categories](./refinement.md#nested-categories)). Empty and absent on
the wire when the rule declares none, so a report serializes exactly as
before.

### Identity and decisions

A finding may carry `id`, its stable `FindingId`, when the host derived it
(`Report::identify_findings`), and `decision`, the reviewer's decision
carried over to it (`Report::apply_decisions`); `Report::stale_decisions`
lists decisions whose finding is gone. All three are absent on the wire
unless set, so a report serializes exactly as before. See
[Review decisions](./decisions.md).

### Explanations

A finding or not-evaluated outcome of an expression rule carries an `explanation` (`axioval_ir::Explanation`, absent on the wire for every other rule, so reports serialize as before): its `entries` are the subexpressions the evaluation went through, in evaluation order (operands before the node they feed), each with its `path`, `kind`, `label`, rendered `value` or why it was `notEvaluated`, and whether it is `deciding`. The deciding path is the subexpression that failed or was not evaluated, its ancestors and everything below it, and is always kept whole; at most `MAX_EXPLANATION_ENTRIES` (64) other entries follow, and `truncated` says when some were left out. The order is the evaluation's, so the same run explains the same way. The HTML and BCF sinks show the deciding path under the message.

### Rule status

`Report::rules` holds one `RuleSummary` per rule when the host asked for
them: the decided selection size (`checked`), the objects with findings
(`failed`) and not evaluated, and a `RuleStatus` (`passed`, `failed`,
`not_evaluated`, `nothing_selected`, or `skipped` for a rule whose gate on another
rule's outcome was closed). An auxiliary rule (see
[Auxiliary rules](./gates.md#auxiliary-rules)) has none. See [Rule status](./refinement.md#rule-status).
The field is omitted when empty.

### Tables

A finding says what is wrong; a table says what was measured, whether it passed or not: one row per storey with its elevation and height, one per anchor with its areas and their ratio. `Report::tables` holds `ReportTable`s, each reported by one rule under a name unique for that rule.

- **Columns** have a lowercase id (1 to 64 ASCII letters, digits, `-` or `_`) unique in the table and a kind: `quantity` with a `QuantityDimension` (values in its coherent SI unit, as for properties), `number` (dimensionless, such as a ratio) or `text`. A column may state its `exactness`: `exact` (every value derives from exact inputs; an interval only where an undecided input widens it) or `bounded` (some input is known only within bounds); a column without it says nothing either way. A `number` column counting a unit outside SI, such as a currency, carries it as `unit` (`{"id": "sum_cost", "kind": "number", "unit": "EUR"}`, `ReportColumn::amount`); summaries and CSV headers show it as they show a quantity's unit.
- **Rows** are keyed by `Scope`, at most one per scope, and hold one value per column: `exact` (a finite number), `interval` (`lower < upper`, finite, sure to hold the exact value), `text`, or `unknown`. A measured interval is exact exactly when it is a point; `ReportValue::measured` writes a point as `exact` and non-finite bounds as `unknown`.
- **Groups.** A grouped table (`ReportTable::grouped`, such as a takeoff) declares group columns, ids like column ids and distinct from them, and keys each row by its scope and its `group`: one text value per group column, in their order (a type name and a storey, say). At most one row per scope and group (`push_group_row`, `group_row`).
- **Ordering.** The runtime sorts tables by rule, then name; rows are always in scope order (project, sources, objects), then by group, whatever order they were added or read in. Rows of several rules' tables join on their scope.
- **Validation.** Names, row widths, group widths, value kinds, finiteness and duplicate keys are checked when a table is built and when it is read. A capability adds tables with `CapabilityEvaluation::push_table`; a table without rows is dropped, the runtime binds each table to the compiled rule, and a rule reporting one name twice fails the run.

Tables are informative: they never stand in for a finding or a not-evaluated outcome, and sinks that write issues (BCF) ignore them.

```json
"tables": [{
  "rule_id": "storey-heights", "name": "levels",
  "columns": [{"id": "elevation", "kind": "quantity", "dimension": "length"},
              {"id": "height", "kind": "quantity", "dimension": "length"}],
  "rows": [{"object_id": {"source": {"system": "ifc-step", "document": "model.ifc"}, "local_id": "#102"},
            "values": [{"type": "exact", "value": 3.0},
                       {"type": "interval", "lower": 3.49, "upper": 3.51}]}]
}]
```

A row names its scope as a finding does: `object_id`, `source`, or neither for the project. The `tables` field is omitted when empty, so a report without tables serializes byte for byte as before; readers written before tables existed reject a report containing them.

A grouped table writes its group column ids as `group_by` and each row's group values as `group`; an ungrouped table writes neither, as before groups existed, and readers written before groups existed reject a grouped table:

```json
{"rule_id": "wall-takeoff", "name": "takeoff", "group_by": ["group_1", "group_2"],
 "columns": [{"id": "count", "kind": "number"},
             {"id": "sum_net_side_area", "kind": "quantity", "dimension": "area"}],
 "rows": [{"group": ["Basic Wall 200", "Level 1"],
           "values": [{"type": "exact", "value": 3.0}, {"type": "exact", "value": 36.5}]}]}
```


## Expressions

`contract::Expression` is a typed expression tree: values a rule computes and combines, stated as data. It is tagged by `kind` like a selector, refuses unknown kinds and fields when a package is read, and serializes in a stable order (a lookup's keys by column ID). The language is total: no loops, no recursion, no user-defined functions and no package-provided code, so a finite tree always finishes evaluating. A parameter value of type `expression` (`{"type": "expression", "value": {…}}`, `ParameterValue::Expression`) carries one, for a parameter of kind `expression` (`ParameterKind::Expression`); [Expression requirements](./capabilities.md#expression-requirements) bind it.

Every node may carry a `label`, which findings name in place of the node's rendered form. `Expression::validate` refuses what serde cannot state: nesting deeper than `MAX_EXPRESSION_DEPTH` (64), an empty operand list, an `if` without a branch, a lookup without keys, a blank name, unit or label, and a number that is not finite. Types and units are checked when a ruleset is compiled. The engine evaluates every value as an interval with Kleene's three-valued truth over true, false and `null`, not evaluated apart; `null` is a value the source states as absent, never a value that could not be read or measured, and the two never mix (see [Truth](./expressions.md#truth)).

| Kind | Fields | Meaning |
| --- | --- | --- |
| `literal` | `value` | a constant: a `ScalarValue` of type `boolean`, `integer`, `number`, `quantity` (with `unit`), `string`, `enum`, `date` or `dateTime`, in the wire form of the same parameter value; lists, tables, references and selectors are not literals |
| `null` | | the source states no value |
| `property` | `propertySet`?, `property`, `of`? | a property of the object in scope, or with `of: subject` of the rule's checked object inside an aggregate; `axioval:measured` names carry their parameters (`bottom_above_level;path=…`) |
| `parameter` | `name` | a parameter of the rule |
| `derived` | `name` | a derived value the ruleset names |
| `lookup` | `table`, `keys`, `column` | the `column` cell of the most specific row of the `table` parameter whose key columns match `keys` (column ID → expression), as `keyed-limit` selects a row |
| `not` | `operand` | negation |
| `and`, `or` | `operands` | conjunction, disjunction |
| `implies` | `antecedent`, `consequent` | implication |
| `xor` | `left`, `right` | exclusive or |
| `compare` | `operator`, `left`, `right`, `caseSensitive`? | `equals`, `notEquals`, `lessThan`, `lessThanOrEquals`, `greaterThan`, `greaterThanOrEquals` (dates chronologically), `like` (`*`, `?`, `\` escapes), `matches` (a regular expression over the whole text), `contains` |
| `between` | `operand`, `low`, `high`, `lowInclusive`?, `highInclusive`? | a range test; bounds are inclusive unless stated `false` |
| `oneOf`, `noneOf` | `operand`, `values`, `caseSensitive`? | membership in a list of expressions |
| `isDefined`, `isUndefined` | `operand` | whether the value is not `null`, or is |
| `if` | `branches` (`when`, `then`), `else` | the `then` of the first branch whose `when` holds; a ternary is one branch |
| `coalesce` | `operands` | the first operand that is not `null` |
| `add`, `subtract`, `multiply`, `divide` | `left`, `right` | arithmetic |
| `negate`, `abs`, `floor`, `ceil`, `sqrt` | `operand` | unary arithmetic |
| `min`, `max` | `operands` | the least or greatest operand |
| `round` | `operand`, `step` | the nearest multiple of `step` |
| `sin`, `cos`, `tan` | `operand` | trigonometry of a plane angle |
| `atan2` | `y`, `x` | the plane angle of the vector `(x, y)` |
| `convertSlope` | `operand`, `from`, `to` | a slope restated between `ratio`, `percent` and `angle` |
| `aggregate` | `function`, `over`, `where`?, `value`? | `count`, `sum`, `min`, `max`, `average`, `any`, `all`, `none` or `distinctCount` over the objects a `path`, a derived `group` or a `selector` reaches, filtered by `where`, `value` evaluated per member; see [Aggregates](./expressions.md#aggregates) |
| `ruleOutcome`, `selected`, `findingCount`, `deviation` | `rule` | another rule's verdict, whether it selected the object in scope, its finding count or its greatest graded deviation about the object; see [Other rules' outcomes](./expressions.md#other-rules-outcomes) |
| `concat` | `operands` | joined text |
| `length`, `lower`, `upper`, `trim` | `operand` | text functions |

A three-branch cover requirement reads:

```json
{"kind": "if", "label": "required cover",
 "branches": [
   {"when": {"kind": "compare", "operator": "equals",
             "left": {"kind": "property", "propertySet": "Pset_WallCommon", "property": "ExposureClass"},
             "right": {"kind": "literal", "value": {"type": "string", "value": "XC4"}}},
    "then": {"kind": "literal", "value": {"type": "quantity", "value": 40.0, "unit": "mm"}}}],
 "else": {"kind": "literal", "value": {"type": "quantity", "value": 25.0, "unit": "mm"}}}
```

`crates/contracts/ir/tests/fixtures/expression` holds one golden fixture per kind, and one per literal type.

## No source leakage

Core architecture checks reject references to `IfcModel`, STEP entity handles, ICDD container types, Axiolid meshes, OpenCascade, CGAL, and the legacy runtime types.
