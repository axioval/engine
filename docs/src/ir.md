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

An alias is an `ExternalId`: an adapter-defined `scheme` and a `value`, listed in `Object::external_ids` and read with `Object::external_id(scheme)`. It exists so output formats and cross-revision tools can name an object the way other software does; capabilities never key on it. `Project::new` rejects an object with two ids in one scheme and two objects of one source sharing an id, because any consumer resolving that alias would pick one silently. Two sources may share an alias: two revisions of one model do.

## Semantic data

Objects expose canonical concepts, typed properties, classifications and directed relationships. Canonical concept IDs are package vocabulary identifiers; source-specific names are adapter bindings. Stored property values are observations, not proof that an omitted key is absent. A property may carry `data_type`, the value's type as the source declares it in its own vocabulary (IFC: `IFCLABEL`); `None` means unreported, never any particular type. Conclusive property checks use the typed property-resolution service and exact request-bound evidence.

Values distinguish null/unavailable from concrete values and preserve units where relevant. Adapters must not silently coerce malformed source values.

A quantity is stated in the coherent SI unit of its `QuantityDimension`:
- `Length`, `Area` and `Volume` in metres, square metres and cubic metres;
- `PlaneAngle` in radians;
- `Other { exponents }` for any other dimension, given as SI base-unit exponents. A thermal transmittance in W/(m²·K) is `[0, 1, -3, 0, -1, 0, 0]`.

Quantities compare only when their dimensions are equal. A dimensionless measure, such as a ratio, is a plain decimal.

A `List` holds several values of one property, in the order the source states them, such as every presentation layer of an object. Its elements are scalar values: never `null` and never another list; the property resolver rejects any other list as an invalid value. A list is never compared as a whole or as if it were one of its elements: a property selector states a `quantifier` (see [Property selectors](./capabilities.md#property-selectors)). Its wire form is `{"type": "list", "value": [{"type": "string", "value": "A-WALL"}]}`.

### Attribute sets

Some facts about an object are not in any property set but in fields of the object itself, such as a space's number and name, or the name of its construction type. Reserved property-set names reach them through the same property resolver, and so do the object's presentation layer and material:

- `ATTRIBUTE_SET` (`axioval:attributes`) names the object's own attributes, by the source's attribute name.
- `TYPE_ATTRIBUTE_SET` (`axioval:type-attributes`) names the attributes of the type object the source assigns to the object. An object with no type has none (an exact absence). An object with several types is a conflict, not a choice.
- `PRESENTATION_SET` (`axioval:presentation`) names how the object is presented. Its property `Layer` (`PRESENTATION_LAYER`) lists every presentation (CAD) layer of the object's shape, as a list of strings sorted by name. An object on no layer has none (an exact absence), but only in a source that assigns layers at all: a source with no layer assignment answers that it records no layers, so a layer rule over it is not evaluated (`NotRecorded`, once per rule and source) instead of flagging or passing every object.

- `MATERIAL_SET` (`axioval:material`) names the material the object is made of: its own, or else its type object's. Its properties, matched ignoring ASCII case:
  - `Kind`: `material`, `layer-set`, `constituent-set`, `profile-set` or `list`.
  - `Name`: the name of a single material, or of the set. `Category`: the category of a single material.
  - `TotalThickness`: the summed layer thickness of a layer set, a length in metres.
  - `Count`: the number of layers, constituents, profiles or listed materials.
  - Members numbered from 1 in the source's order: `Layer<n>.Material`, `.Thickness` (a length), `.Name` and `.Category`; `Constituent<n>.Material`, `.Name`, `.Category` and `.Fraction` (a decimal); `Profile<n>.Material`, `.Name` and `.Category`; `Material<n>.Name` and `.Category` for a list. `.Material` is the name of the member's material.

  An object without material has none of them (an exact absence), and neither does a member that does not exist or states no value. Two material assignments on the object, or on its type, are a conflict.

These sets are engine vocabulary. They name no property set of any source, a package cannot redeclare them, and concept binding passes them through unchanged, while the property name inside them is still bound per source. A request without a set never searches attributes.

## Views and layers

A project can expose raw source views and composed views. This supports today's single IFC model and ICDD federation as well as future IFCX-style layers without changing rule capability APIs.

## Provenance

Every imported fact and computed evidence can reference its source record, adapter, derivation and precision. Findings carry the provenance needed to explain or reproduce a decision.

## Reports

`Report` keeps conclusive `findings` separate from `not_evaluated` outcomes. Every not-evaluated record identifies its rule and its scope, carries a typed reason, and includes a diagnostic. Runtime ordering is deterministic. An empty findings list is not a pass when not-evaluated outcomes exist.

### Scope

Every finding and not-evaluated outcome has a `Scope`: what it is about.

| Scope | Meaning | Example |
|---|---|---|
| `Scope::Object(ObjectId)` | one object | "wall `#42` has no fire rating" |
| `Scope::Source(SourceId)` | one source as a whole | "this model has no building" |
| `Scope::Project` | every source together; for a not-evaluated outcome, the rule as a whole | "no storey anywhere has a fire compartment" |

A result about the whole model has no object to be reported against, and reporting nothing would read as a pass. Scoped findings follow the same evidence rules as object findings: they are conclusive, carry exact source evidence for what decided them (sorted by source and locator, deduplicated), and may name `related` objects, such as the objects a count found. `Finding::new(rule, scope, severity, message)` with `with_evidence` and `with_related` builds one; `Finding::object_id()` and `NotEvaluated::object_id()` return the object when there is one.

The runtime orders findings by rule, then scope, then message, and not-evaluated outcomes by rule, scope, reason and message. Scopes order project first, then sources, then objects, each by identity.

**Serialized form.** The scope is written as the fields reports have always had, so a report without scoped entries serializes byte for byte as before:

- an object finding has `object_id`; a source finding has `source` instead; a project finding has neither;
- a not-evaluated outcome always has `object_id` (`null` unless it is about one object), and `source` only when it is about one source.

A record naming both an object and a source is rejected: an object id already names its source. Readers written before scopes existed reject a finding without `object_id` or with `source`, so a report containing scoped entries needs a reader of this version.

## No source leakage

Core architecture checks reject references to `IfcModel`, STEP entity handles, ICDD container types, Axiolid meshes, OpenCascade, CGAL, and the legacy runtime types.
