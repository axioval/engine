# Contributing

See the repository [contributor guide](https://github.com/axioval/engine/blob/main/CONTRIBUTING.md).

The shortest acceptable loop is RED–GREEN–REFACTOR followed by `./scripts/check.sh`. Architecture-boundary changes require an ADR or an architecture-document update and a test that proves the forbidden dependency gate can fail.

Use scoped Conventional Commits on linear `main`. Keep logical changes atomic so `git revert` is a safe rollback mechanism.

Before a release, commit the candidate and run `./scripts/package.sh`. It packages and verifies every workspace crate through Cargo's temporary local registry, including unpublished intra-workspace dependencies.

Crates reach crates.io only through the `Release` workflow (`.github/workflows/release.yml`), started by hand. `verify-only` tests the release tooling, prints the publication order, and verifies every archive. `publish` then runs in the `crates.io` environment: it scans every archive for the private forbidden terms, read from the environment secret `AXIOVAL_FORBIDDEN_TERMS_JSON`, and publishes the workspace with `scripts/publish-workspace.py` in dependency order. The upload uses a short-lived crates.io token that GitHub OIDC exchanges per attempt. No registry token is stored anywhere. A version already on crates.io is skipped, so a rerun resumes an interrupted release. A crate's first publish must also name this workflow and environment as its trusted publisher on crates.io.

## Layering a new check

A requirement is built in layers, so every rule a capability decides can
also be stated by an author:

- **Measurement in a measured value.** A quantity read through a service
  (a distance, a share, a count of what a search found) is registered in
  `axioval_ir::measured` and answered by a `MeasuredProvider`, usually a
  `measured.rs` beside the capability, reusing the capability's own code.
- **Decision in an expression or a generic judge.** Comparing a measurement
  with a limit, a band or a table row is an expression (or a shared judge
  such as `keyed-limit`), never new code per requirement.
- **Search-only logic in a capability.** A capability keeps what no single
  value states: routes, pairings, sight lines, the candidates a search
  narrows.

The architecture gate (`scripts/architecture.py`, mutation-proven by its
`--self-test`) holds this. A rules module that calls a measuring service
method (`measure_*`) must register its own `measured.rs` provider, or have
an entry in `scripts/measurement_ledger.json`: `answeredBy` names the
registered measured values that already expose what it measures,
`searchOnly` says why it only searches. Every named value must be
registered, and an entry for a module that no longer measures or now has
its own provider is stale and fails, so the ledger only shrinks. The gate
also fails a capability without labels and help in
`crates/engine/rules/src/catalogue_texts.rs`; see the
[authoring catalogue](./catalogue.md).
