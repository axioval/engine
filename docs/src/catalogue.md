# Authoring catalogue

A rule editor, such as a block editor, is generated from what the engine
supports, never written by hand. The authoring catalogue is that
description: one versioned JSON document listing everything a rule may be
built from, every entry labelled and explained in English and German.

```sh
axioval catalogue > catalogue.json
axioval catalogue --definitions definitions.json --output catalogue.json
```

The library function is `axioval::catalogue(&packages)`, or
`axioval_rules::catalogue::catalogue(&registry, &packages)` for a host
registry. The engine part is `axioval_engine::catalogue::catalogue`, which
takes the host's texts for every capability it registered.

## Contents

| Field | What it lists |
| --- | --- |
| `schemaVersion` | The version of this JSON form (see below). |
| `languages` | `["en", "de"]`: every `label` and `help` states both, in this order. |
| `valueTypes` | The value types expression fields take and nodes yield. |
| `units` | The base units, every unit symbol a written unit may use (scale, exponents, aliases), and the dimensions measured values are answered in. |
| `capabilities` | Every registered capability by id: whether it grades deviations or takes authored parameters, and each parameter's package kind, requirement, whether it may be per object, the type an expression parameter must have, and a table's columns. |
| `measuredValues` | The registry of measured values: parameters, dimension and unit, services, exactness, what leaves them not evaluated, and whether this host measures them (`available`). |
| `measuredMembers` | The measured member lists with their fields. |
| `expressionCategories`, `expressionKinds` | Every expression node kind: its `kind` tag, category, fields (each an expression of the value types it accepts, a list of them, a name and what it names, a choice, a flag, a selector, a path or a literal), and how its result type follows (`resultRule`). |
| `expressionComparisons` | The operators of `compare`, with the value types each compares. |
| `aggregateFunctions`, `aggregateSources` | What an aggregate computes and what it ranges over. |
| `slopeForms` | How `convertSlope` states a slope. |
| `selectorKinds`, `selectorComparisons` | Every selector kind a package writes, with its fields, and the operators of `property` and `source` selectors. |
| `relationships` | Path step directions, the source-neutral stated relationships, the derived relationships with their parameters and defaults, and the relation a ruleset declares. |
| `concepts` | For each `--definitions` package: its object types, property sets, properties and rule definitions. |

Every list is in a stable order: capabilities by id, measured values and
member lists as their registries order them, node kinds and selector kinds
in the order of the contract, packages as given. Two runs over the same
inputs write the same bytes.

## Versioning

`schemaVersion` is `major.minor.patch`. A reader accepts any catalogue of
its own major version: a minor version only adds entries or optional
fields, a patch only changes texts. Removing or renaming an entry or a
field raises the major version.

## What the gate holds

The catalogue is built only when every registered capability, each of its
parameters, every measured value and member list, and each of their
parameters and fields carries texts in English and German; otherwise
`catalogue` refuses with the first one missing, so a capability is never
offered to an editor undescribed. The contract's tables fail a test when a
kind of the contract (an expression node, a selector, an operator, an
aggregate function or source, a slope form) is missing from them.

The golden copy `crates/engine/rules/tests/golden/catalogue.json` makes
every change to the catalogue visible in review; regenerate it with
`AXIOVAL_BLESS=1 cargo test -p axioval-rules --test catalogue`.
Capability texts live in `crates/engine/rules/src/catalogue_texts.rs`.
