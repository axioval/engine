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

## Semantic data

Objects expose canonical concepts, typed properties, classifications and directed relationships. Canonical concept IDs are package vocabulary identifiers; source-specific names are adapter bindings.

Values distinguish null/unavailable from concrete values and preserve units where relevant. Adapters must not silently coerce malformed source values.

## Views and layers

A project can expose raw source views and composed views. This supports today's single IFC model and ICDD federation as well as future IFCX-style layers without changing rule capability APIs.

## Provenance

Every imported fact and computed evidence can reference its source record, adapter, derivation and precision. Findings carry the provenance needed to explain or reproduce a decision.

## Reports

`Report` keeps conclusive `findings` separate from `not_evaluated` outcomes. Every not-evaluated record identifies its rule, optionally identifies the affected object, carries a typed reason, and includes a diagnostic. Runtime ordering is deterministic. An empty findings list is not a pass when not-evaluated outcomes exist.

## No source leakage

Core architecture checks reject references to `IfcModel`, STEP entity handles, ICDD container types, Axiolid meshes, OpenCascade, CGAL, and Solibri types.
