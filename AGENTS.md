# Axioval Engine

This repository owns the source-neutral Rust rule runtime, its portable contracts, built-in capabilities, and separately layered adapters.

## Boundaries

- Core crates must not depend on IFC, STEP, ICDD, Axiolid, OpenCascade, CGAL, or vendor types.
- `axioval-openbim` and `axioval-axiolid` are independent adapters. Never create an adapter combining both organizations.
- Source adapters map external identities and semantics into `axioval-ir`; they do not implement checking policy.
- Geometry adapters provide typed evidence and exactness/provenance. They do not emit policy findings.
- Rulesets select trusted capability IDs. Never execute package-provided code.
- Missing evidence and unsupported capabilities fail closed.
- Stable ordering and source-qualified identities are compatibility contracts.

## Direct children

- `crates/` — Rust crates; descend into each crate's `AGENTS.md` before editing.
- `docs/` — mdBook sources and architecture/migration documentation.
- `scripts/` — repository validation and architecture checks.
- `.github/workflows/` — CI and GitHub Pages deployment.

## Gates

Run `./scripts/check.sh`. It formats, lints, tests, checks architecture boundaries, builds rustdoc, and builds the mdBook. Do not mark a Solibri capability migrated without its parity evidence and cutover reference in `docs/src/migration.md`.
