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

### Attribute sets

Some facts about an object are not in any property set but in fields of the object itself, such as a space's number and name, or the name of its construction type. Two reserved property-set names reach them through the same property resolver:

- `ATTRIBUTE_SET` (`axioval:attributes`) names the object's own attributes, by the source's attribute name.
- `TYPE_ATTRIBUTE_SET` (`axioval:type-attributes`) names the attributes of the type object the source assigns to the object. An object with no type has none (an exact absence). An object with several types is a conflict, not a choice.
- `PRESENTATION_SET` (`axioval:presentation`) names how the object is presented. Its property `Layer` (`PRESENTATION_LAYER`) is the presentation (CAD) layer of the object's shape. An object on no layer has none; one on several distinct layers is a conflict.

Both are engine vocabulary. They name no property set of any source, a package cannot redeclare them, and concept binding passes them through unchanged, while the property name inside them is still bound per source. A request without a set never searches attributes.

## Views and layers

A project can expose raw source views and composed views. This supports today's single IFC model and ICDD federation as well as future IFCX-style layers without changing rule capability APIs.

## Provenance

Every imported fact and computed evidence can reference its source record, adapter, derivation and precision. Findings carry the provenance needed to explain or reproduce a decision.

## Reports

`Report` keeps conclusive `findings` separate from `not_evaluated` outcomes. Every not-evaluated record identifies its rule, optionally identifies the affected object, carries a typed reason, and includes a diagnostic. Runtime ordering is deterministic. An empty findings list is not a pass when not-evaluated outcomes exist.

## No source leakage

Core architecture checks reject references to `IfcModel`, STEP entity handles, ICDD container types, Axiolid meshes, OpenCascade, CGAL, and the legacy runtime types.
