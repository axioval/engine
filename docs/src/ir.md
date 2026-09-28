# Source-neutral IR

## Package compatibility

The compiler currently accepts normalized Axioval Schema `0.1.0` packages only. Both definition packages and rulesets are checked before binding; unknown versions fail with `UnsupportedSchemaVersion` rather than being interpreted as the current contract.

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

## Semantic data

Objects expose canonical concepts, typed properties, classifications and directed relationships. Canonical concept IDs are package vocabulary identifiers; source-specific names are adapter bindings. Stored property values are observations, not proof that an omitted key is absent. A property may carry `data_type`, the value's type as the source declares it in its own vocabulary (IFC: `IFCLABEL`); `None` means unreported, never any particular type. Conclusive property checks use the typed property-resolution service and exact request-bound evidence.

Values distinguish null/unavailable from concrete values and preserve units where relevant. Adapters must not silently coerce malformed source values.

A quantity is stated in the coherent SI unit of its `QuantityDimension`:
- `Length`, `Area` and `Volume` in metres, square metres and cubic metres;
- `PlaneAngle` in radians;
- `Other { exponents }` for any other dimension, given as SI base-unit exponents. A thermal transmittance in W/(m²·K) is `[0, 1, -3, 0, -1, 0, 0]`.

Quantities compare only when their dimensions are equal. A dimensionless measure, such as a ratio, is a plain decimal.

A `Date` is a calendar day of the proleptic Gregorian calendar in the years 0000 to 9999, and a `DateTime` an instant to the nanosecond together with the UTC offset it was stated in. Both are validated on construction and when read, so a value is always a real day. On the wire they are ISO 8601 extended strings, as `xs:date` and `xs:dateTime` write them: `{"type": "date", "value": "2026-09-27"}` and `{"type": "dateTime", "value": "2026-09-27T10:00:00.5+02:00"}`, a fraction of up to nine digits, and `Z` or `±hh:mm` up to 14 hours. A date-time is written in its canonical form: no trailing fraction zeros, and `Z` for a zero offset. A date-time without an offset is not representable: it names a wall-clock time in an unknown zone and cannot be ordered against any instant, so an adapter reports it as incomplete evidence rather than guessing a zone; `-00:00`, `24:00:00` and leap seconds are refused for the same reason. Equality of values is structural (the same instant in two offsets is two values); comparisons order instants and read a date-time as the calendar day it states only at `day` precision (see [Dates and date-times](./capabilities.md#dates-and-date-times)). Package parameters carry `date` and `dateTime` literals (`ParameterKind::Date`, `ParameterKind::DateTime`), and a property definition may declare `valueKind` `date` or `dateTime`.

A `List` holds several values of one property, in the order the source states them, such as every presentation layer of an object. Its elements are scalar values: never `null` and never another list; the property resolver rejects any other list as an invalid value. A list is never compared as a whole or as if it were one of its elements: a property selector states a `quantifier` (see [Property selectors](./capabilities.md#property-selectors)). Its wire form is `{"type": "list", "value": [{"type": "string", "value": "A-WALL"}]}`.

A `Measured` value (`{"type": "measured", "value": {"lower": …, "upper": …, "dimension": …}}`) is one quantity known only to lie in `[lower, upper]`, finite and ordered, in SI units: a value measured from a tessellated body. It is scalar; a comparison it may pass or fail is undecided. A `Bounded` value is a range a source states as one value: an optional `lower` and `upper` bound and an optional `set_point`, at least one of them stated, each a scalar of one kind. An unstated bound leaves the range open on that side; it is never zero or infinity. A `Table` holds rows of a `defining` and a `defined` value (`PropertyTableRow`), in source order, at least one row, every cell a scalar. Their wire forms are `{"type": "bounded", "value": {"lower": {…}, "set_point": {…}}}` and `{"type": "table", "value": [{"defining": {…}, "defined": {…}}]}`. `PropertyValue::stated_values` lists the scalar values a list, bounded value or table states (a list's elements; lower, upper and set point as far as stated; each row's defining then defined value), which is what a quantified comparison compares; a capability judging a range against bounds also considers its open ends. `is_scalar` is true for every other value but `null`.

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
  - `Count`: the number of geometric items, mapped items resolved. `Kinds`: every distinct item kind, as a list of strings, sorted. `Mapped`: whether any item is reached through a mapping (a type's shared geometry placed at the occurrence). `Mirrored`: whether the transform placing the body reverses its orientation (a negative determinant); the IFC adapter states `false` when every item is authored in place or is a swept solid (whose mapping is rigid by construction) and refuses it otherwise, since the mapping transforms of other items are not described yet.
  - Items numbered from 1 in the source's order, `Item<n>.` followed by `Kind` and `Mapped`. The kinds are `extrusion`, `tapered-extrusion`, `revolution`, `tapered-revolution`, `directrix-sweep`, `swept-disk`, `sectioned-spine`, `brep`, `csg`, `csg-primitive`, `half-space`, `bounding-box`, `tessellation`, `surface-model`, `face`, `geometric-set`, `curve`, `surface` and `point`.
  - A swept-area item (the extrusions, revolutions and directrix sweeps) adds its `Profile.` facts, `EndProfile.` for a tapered sweep, and its placement: `Placement.OriginX`, `.OriginY`, `.OriginZ` (lengths) and the axes `Placement.XAxisX` … `Placement.ZAxisZ` (decimals); the profile lies in the placement's X-Y plane. An extrusion adds `Extrusion.Depth` (a length), `Extrusion.DirectionX`, `.DirectionY`, `.DirectionZ` (a unit vector) and `Extrusion.Inclination`, the angle between the extrusion's line (either sense) and the vertical, from 0 (a wall extruded up) to π/2 (a beam). A revolution adds `Revolution.Angle`, `Revolution.OriginX` … `.OriginZ` and `Revolution.AxisX` … `.AxisZ`.
  - `Profile.Type` names the family: `rectangle`, `rounded-rectangle`, `rectangle-hollow`, `circle`, `circle-hollow`, `ellipse`, `i-shape`, `asymmetric-i-shape`, `l-shape`, `t-shape`, `u-shape`, `c-shape`, `z-shape`, `trapezium`, `arbitrary-closed`, `arbitrary-with-voids`, `center-line`, `composite`, `derived` or `mirrored`. `Profile.Name` is its name (often a catalogue designation such as `HEA300`); `Profile.PositionX`, `.PositionY` and `.PositionAngle` place it where it states a position. The family's parameters follow under their dimension names: `XDim`, `YDim`, `RoundingRadius`, `WallThickness`, `InnerFilletRadius`, `OuterFilletRadius`, `Radius`, `SemiAxis1`, `SemiAxis2`, `OverallWidth`, `OverallDepth`, `WebThickness`, `FlangeThickness`, `FilletRadius`, `FlangeEdgeRadius`, `FlangeSlope`, the asymmetric I's `BottomFlange…` and `TopFlange…`, `Depth`, `Width`, `Thickness`, `EdgeRadius`, `LegSlope`, `FlangeWidth`, `WebEdgeRadius`, `WebSlope`, `Girth`, `InternalFilletRadius`, the trapezium's `BottomXDim`, `TopXDim`, `TopXOffset`, and `VoidCount`. An arbitrary closed profile states its outline as `Profile.OutlineX` and `Profile.OutlineY`, two lists of lengths: its vertices' coordinates in the profile's X-Y plane, in the order the source states them, the closing vertex not repeated; one with voids adds `Void<n>.OutlineX` and `Void<n>.OutlineY` per void. Only straight edges are stated so: an outline with a curved segment is refused, never approximated by chords, and the profile's other facts stand. A composite profile states `Count`, `Label` and `Member<n>.` profile facts; a derived or mirrored one `Label` and `Parent.` profile facts. Every parameterised section except the trapezium is centred on its position: the centre of its bounding box lies at the position's origin, and its depth runs along the position's Y axis.
  - Without the `Item<n>.` prefix, a name reads the body's only item: `Kind` and `Profile.OverallDepth` of a beam. On a body of several items such a name is a conflict, never a choice of item.

  An object without a body has none of them (an exact absence), and neither has an item of another kind, a parameter its family does not have, or one the source leaves unset, even where the schema defines a default: an equal-leg angle that states no `Width` has none, not its `Depth`. A body the source states but that cannot be read exactly (an item or profile family not read, a dangling reference, a mapping that scales a swept solid) is refused as a whole, never absent and never read in part.

- `CLASSIFICATION_SET` (`axioval:classification`) holds the classes a ruleset's classifications derive, one property per classification id: a string for a first-match classification, a list of strings for an all-match one, none for an object no row matches. The engine answers it, never a source; see [Derived properties](./derived.md). `is_derived_set` names the sets the engine derives.

- `MEASURED_SET` (`axioval:measured`) holds values measured from geometry: `extent_x`, `extent_y`, `extent_z`, `bottom`, `top`, `area`, `volume`, `x`, `y`, `z` and the parameterised `bottom_above_level;path=…` and `boundary_area;kind=…`, and the stated `level_height` (`MEASURED_NAMES`), each a quantity when measured exactly and a `measured` interval otherwise. The engine answers it through the host's geometry services; see [Derived properties](./derived.md#measured-values).

These sets are engine vocabulary. They name no property set of any source, a package cannot redeclare them, and concept binding passes them through unchanged, while the property name inside them is still bound per source. A request without a set never searches attributes.

## Views and layers

A project can expose raw source views and composed views. This supports today's single IFC model and ICDD federation as well as future IFCX-style layers without changing rule capability APIs.

## Provenance

Every imported fact and computed evidence can reference its source record, adapter, derivation and precision. Findings carry the provenance needed to explain or reproduce a decision.

## Reports

`Report` keeps conclusive `findings` separate from `not_evaluated` outcomes, and both separate from the `tables` of measured values (see [Tables](#tables)). Every not-evaluated record identifies its rule and its scope, carries a typed reason, and includes a diagnostic. Runtime ordering is deterministic. An empty findings list is not a pass when not-evaluated outcomes exist.

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

- **Columns** have a lowercase id (1 to 64 ASCII letters, digits, `-` or `_`) unique in the table and a kind: `quantity` with a `QuantityDimension` (values in its coherent SI unit, as for properties), `number` (dimensionless, such as a ratio) or `text`.
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


## No source leakage

Core architecture checks reject references to `IfcModel`, STEP entity handles, ICDD container types, Axiolid meshes, OpenCascade, CGAL, and the legacy runtime types.
