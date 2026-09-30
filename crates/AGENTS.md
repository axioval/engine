# Crates

Grouped by **role**, not by the dependency a crate happens to wrap. The
directory a crate lives in tells you what it is allowed to do; its name tells
you what it contains.

```
contracts/   source-neutral vocabulary — depends on nothing else in this tree
engine/      capability execution over those contracts
sources/     adapters that feed the engine, one directory per port kind
sinks/       writers that turn a finished report into an exchange format
packages/    importers and exporters between rule packages and other formats
facade/      feature-gated re-export surface
apps/        executables
```

## contracts/

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

## sinks/

One subdirectory per exchange format. A sink reads `axioval-ir` reports and
projects only: never the engine, never a source adapter. It must not depend on
an adapter at runtime; agreeing on an alias scheme is pinned by a test instead.

- `bcf` (`axioval-bcf`) — BCF 2.1 and 3.0 issue archives, and decisions
  read back from them.
- `bcf-api` (`axioval-bcf-api`) — BCF API 3.0 client; maps topics through
  `axioval-bcf`, never a second way.
- `bcf-snapshot` (`axioval-bcf-snapshot`) — illustrative PNG snapshots of
  BCF viewpoints from meshes, through the sink's `SnapshotRenderer`; no
  source or kernel types.

## packages/

One subdirectory per rule format a package is imported from, beside the
export framework every target shares. An importer
writes `axioval-ir` definition packages and rulesets and never reads a model,
so it adapts no source and depends on no adapter. It is core to the
architecture gate: only the libraries named for it in `PERMITTED_COUPLINGS`
of `scripts/architecture.py` are exempted, and only for that crate.

- `export` (`axioval-export`) — export profiles, losses (refused or
  degraded) and the shared rule comparator; depends on `axioval-ir` only.
- `ids` (`axioval-ids`) — rule packages from buildingSMART IDS documents, and exported back to them (the `ids` export profile).

`axioval-spec` is retired: its notice lives in `attic/axioval-spec-notice`.

## facade/ and apps/

- `axioval` — feature-gated facade. One feature per source adapter or sink,
  named for the format or library it adapts (`ifc`, `axiolid`, `icdd`, `bcf`,
  `bcf-api`, `bcf-snapshot`).
- `cli` (`axioval-cli`) — command-line frontend.

## Adding a source adapter or sink

1. Create `sources/<port-kind>/<content-name>/` or `sinks/<format>/` — name
   it for the format or library, never for the ecosystem it was found in.
2. Add a matching optional dependency and same-named feature to
   `facade/axioval/Cargo.toml`, and a `#[cfg(feature = "…")]` re-export.
3. Add the crate name to `ADAPTER_CRATES` in `scripts/architecture.py` and to
   `EXPECTED` in `scripts/check_package_contents.py`, and bump
   `EXPECTED_MEMBERS` in `scripts/staging_isolation.py`.

Crates not listed in `ADAPTER_CRATES` are treated as core and must stay
source-neutral; the architecture gate enforces that by package name, so a crate
cannot escape the guard by moving between directories.
