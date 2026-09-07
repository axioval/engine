# Crates

Grouped by **role**, not by the dependency a crate happens to wrap. The
directory a crate lives in tells you what it is allowed to do; its name tells
you what it contains.

```
contracts/   source-neutral vocabulary — depends on nothing else in this tree
engine/      capability execution over those contracts
sources/     adapters that feed the engine, one directory per port kind
facade/      feature-gated re-export surface
apps/        executables
```

## contracts/

- `spec` (`axioval-spec`) — source-neutral rule and classification vocabulary.
- `ir` (`axioval-ir`) — source-neutral data and normalized package contracts.

## engine/

- `core` (`axioval-engine`) — compiler, trusted registry, services, runtime.
- `rules` (`axioval-rules`) — reusable capability policy and algorithms.

## sources/

One subdirectory per **kind of port** the engine exposes. A new backend is a
new sibling directory — nothing outside it changes.

- `semantic/ifc` (`axioval-ifc`) — IFC semantic source adapter.
- `geometry/axiolid` (`axioval-axiolid`) — Axiolid geometry adapter.
- `assembly/icdd` (`axioval-icdd`) — ICDD project assembly adapter.

Adapters are peers. They must not depend on each other: a combined
`ifc`-plus-`axiolid` adapter would re-couple the two halves this layout
separates. The host composes them.

## facade/ and apps/

- `axioval` — feature-gated facade. One feature per source adapter, named for
  the format or library it adapts (`ifc`, `axiolid`, `icdd`).
- `cli` (`axioval-cli`) — command-line frontend.

## Adding a source adapter

1. Create `sources/<port-kind>/<content-name>/` — name it for the format or
   library, never for the ecosystem it was found in.
2. Add a matching optional dependency and same-named feature to
   `facade/axioval/Cargo.toml`, and a `#[cfg(feature = "…")]` re-export.
3. Add the crate name to `ADAPTER_CRATES` in `scripts/architecture.py` and to
   `EXPECTED` in `scripts/check_package_contents.py`.

Crates not listed in `ADAPTER_CRATES` are treated as core and must stay
source-neutral; the architecture gate enforces that by package name, so a crate
cannot escape the guard by moving between directories.
