# JSON Schemas

The package contract is published as two JSON Schema (draft 2020-12)
documents, so an editor, a form or a pre-commit hook can validate a draft
without linking Rust:

| Package | Schema |
| --- | --- |
| Definition package (`DefinitionPackage`) | [`schema/definitions.schema.json`](./schema/definitions.schema.json) |
| Ruleset package (`RuleSetPackage`) | [`schema/ruleset.schema.json`](./schema/ruleset.schema.json) |

Each schema's `$id` is its address on this site,
`https://axioval.github.io/engine/schema/<file>`. Every type a package
reaches is under `$defs` by its Rust name, so one part of the contract can be
used on its own: `ruleset.schema.json#/$defs/Expression` is the expression
contract, `#/$defs/Selector` a selector and `#/$defs/ParameterValue` a
parameter value.

## Generated, never written

The schemas are generated from the `axioval-ir` contract types by
`axioval_ir::schema` (the crate's opt-in `schema` feature, through
`schemars`). They state the serde wire form, so they cannot drift from it:

- field names, `kind` and `type` tags, defaults and optional fields are the
  ones the reader accepts;
- every object refuses unknown fields (`additionalProperties: false`), as
  the reader does;
- types read from text state their lexical form: a date is
  `YYYY-MM-DD` with an optional zone, a date-time carries its UTC offset, a
  discipline is a lowercase token;
- what the host supplies at run time and never appears in a package (a
  selector of explicit objects, the rows of a loaded table file) is absent.

The `ir` test `json_schema` regenerates both schemas and compares them byte
for byte with the published copies in `docs/src/schema/`, and validates every
package fixture (`fixtures/schema-v0.1.0`, the not-evaluated inventory
packages) and every golden expression fixture against them. The gate runs it
on every commit, so a contract change that is not regenerated fails CI.

## Versioning

The schemas are versioned with the `axioval-ir` crate: their `$comment`
names the crate version that generated them, and the site is deployed from
minor and major release tags only, so the published schemas are those of the
latest release. The `schemaVersion` field inside a package is the package
contract version the compiler accepts; the schema does not pin it.

## What a schema does not check

A schema checks structure. Whether a package means something is decided
when it is compiled, and a draft that validates can still be refused:

- expressions are not type checked: comparing a length with a text, a
  parameter that does not exist or an aggregate nested too deeply all
  validate;
- references are not resolved: a rule's `definitionId`, a target group,
  a classification class, a grouping or a derived value may name nothing;
- concepts are not bound: whether an object type or property names a
  concept the source declares is decided against the model;
- capability ids are not checked against the trusted registry, and a
  capability's parameters against its definition;
- a date that does not exist (`2026-02-30`) matches the pattern and is
  refused when the package is read;
- table files are not loaded, and their hashes not compared.

## Regenerating

After changing a contract type, regenerate the published copies and commit
them with the change:

```sh
AXIOVAL_BLESS=1 cargo test -p axioval-ir --features schema --test json_schema
```

A release that bumps the crate version regenerates them the same way.
