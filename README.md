# Axioval Engine

A source-neutral, pure-Rust rule engine for validating federated engineering and construction data.

Axioval Engine compiles portable [Axioval MCS](https://github.com/axioval/mcs) packages into deterministic execution plans. Rules operate on an engine-specific semantic IR rather than IFC, STEP, ICDD, or a geometry-kernel object model.

## Design boundaries

- **Source neutral:** IFC/OpenBIM is one adapter; proprietary CAD and future layered formats are peers.
- **Geometry neutral:** Axiolid is the default pure-Rust geometry adapter, not part of the engine IR.
- **Federation neutral:** ICDD assembles current multi-model projects; future IFCX/layered compositions map to the same project-view contract.
- **Fail closed:** missing capabilities, incomplete evidence, invalid bindings, and backend failures are explicit outcomes—not implicit passes.
- **Deterministic:** stable ordering, source-qualified identities, and reproducible diagnostics are public contracts.
- **Trusted code boundary:** packages select registered capability IDs; package-authored executable code never runs in the checker.

There is deliberately no combined OpenBIM–Axiolid adapter. `axioval-openbim` maps semantic model data, while `axioval-axiolid` supplies geometry capabilities for any source that can provide Axiolid geometry handles.

## Workspace

| Crate | Responsibility |
|---|---|
| `axioval-ir` | Stable project/object IDs, semantic values, selectors, provenance, evidence and findings |
| `axioval-engine` | Capability registry, package compiler, execution plans and deterministic runtime |
| `axioval-rules` | Reusable built-in capability implementations |
| `axioval-openbim` | Independent OpenBIM/IFC semantic source adapter |
| `axioval-axiolid` | Independent Axiolid geometry evidence adapter |
| `axioval-icdd` | ICDD project assembly adapter |
| `axioval` | Batteries-included facade |
| `axioval-cli` | Portable command-line runner |

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
mdbook build docs
```

See [PLAN.md](PLAN.md), [architecture](docs/src/architecture.md), and the [contributor guide](CONTRIBUTING.md).

## Status

Active extraction from `projects/vendor/solibri`. Compatibility is tracked capability-by-capability in [the migration ledger](docs/src/migration.md). A capability is not marked migrated until Solibri consumes the Axioval implementation and parity gates pass.

The published `0.1.1` facade contains the bounded property-comparison slice and `axioval:capability.property-required`, which treats exact absence, null, and blank text as violations while preserving adapter failures as not-evaluated. Solibri consumes the former through `0.1.0`; required-property remains unmigrated until its production cutover is certified.

## License

AGPL-3.0-or-later. Dependencies and adapters retain their own licenses; Axiolid is MPL-2.0.
