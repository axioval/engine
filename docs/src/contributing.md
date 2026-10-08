# Contributing

See the repository [contributor guide](https://github.com/axioval/engine/blob/main/CONTRIBUTING.md).

The shortest acceptable loop is RED–GREEN–REFACTOR followed by `./scripts/check.sh`. Where [cargo-nextest](https://nexte.st) is installed, its `test` section runs the workspace's test binaries in parallel with it (doctests still through `cargo test --doc`); without it, and on CI, it uses `cargo test`. The `dev` and `test` profiles compile dependencies at `opt-level = 2` (debug assertions and overflow checks stay on), so the geometry tests, which spend their time in the axiolid kernels, stay short; the workspace's own crates stay unoptimized. Architecture-boundary changes require an ADR or an architecture-document update and a test that proves the forbidden dependency gate can fail.

Use scoped Conventional Commits on linear `main`. Keep logical changes atomic so `git revert` is a safe rollback mechanism.

Before a release, commit the candidate and run `./scripts/package.sh`. It packages and verifies every workspace crate through Cargo's temporary local registry, including unpublished intra-workspace dependencies.

Crates reach crates.io only through the `Release` workflow (`.github/workflows/release.yml`), started by hand. `verify-only` tests the release tooling, prints the publication order, and verifies every archive. `publish` then runs in the `crates.io` environment: it scans every archive for the private forbidden terms, read from the environment secret `AXIOVAL_FORBIDDEN_TERMS_JSON`, and publishes the workspace with `scripts/publish-workspace.py` in dependency order. The upload uses a short-lived crates.io token that GitHub OIDC exchanges per attempt. No registry token is stored anywhere. A version already on crates.io is skipped, so a rerun resumes an interrupted release. A crate's first publish must also name this workflow and environment as its trusted publisher on crates.io.

## Layering a new check

A requirement is built in layers, so every rule a capability decides can
also be stated by an author, and a capability is a
[template](./templates.md) over the same parts
([ADR 0005](./adr-0005.md)):

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
`--self-test`) holds this, call site by call site
(`scripts/measurement_gate.py`):

- **Service traits mark their measuring methods.** Every method of a
  `pub trait …Service` in a core crate carries a marker comment:
  `// gate: measures <kinds>` when it returns a measured quantity
  (`length`, `area`, `volume`, `plane_angle`, `number`, `truth`), or
  `// gate: reads` when it returns stated facts, identities, relations or
  configuration. A method without a marker fails, and one named `measure_*`
  cannot be marked `reads`. The gate reads the set of measuring methods from
  these markers, not from method names, so renaming a method does not hide
  it.
- **Every call site is in the ledger.** Each call of a measuring method (by
  name and arity, or as a `…Service::method` path) and each piece of inline
  geometry (`hypot`, `sqrt`, `atan2`, trigonometry) in the rules crate,
  outside a file that only implements registered `MeasuredProvider`s, is a
  call site keyed `module::function::method` in
  `scripts/measurement_ledger.json`, with the number of such calls in that
  function. A new call, in a new function or beside an existing one, fails
  until it is mapped, and an entry for a call that is gone is stale.
- **`providedBy` must match.** It names the registered measured values or
  member lists that expose the quantity. Each must be in the authoring
  catalogue (`crates/engine/rules/tests/golden/catalogue.json`), list the
  called method's service among its `services`, and have a dimension the
  method's marker declares (a plain number, a count or share, may be taken
  from any).
- **`searchOnly` is reviewed.** It names one kind from the closed vocabulary
  `SEARCH_KINDS` in the gate (`candidate-filter`, `pairing`, `route-search`,
  `obstruction-search`, `description`, `classification`,
  `revision-comparison`, `derivation` for inline arithmetic only, and
  `unregistered`), with a reason of at least four words. Adding a kind is a
  change to the gate. `unregistered` marks a measurement no value exposes
  yet: its reason names the issue that registers it, and their number is
  capped by `UNREGISTERED_BUDGET`, which only falls.

The gate also fails a capability without labels and help in
`crates/engine/rules/src/catalogue_texts.rs`; see the
[authoring catalogue](./catalogue.md).

**One implementation per quantity.** A measured value reproducing a
capability's measurement calls the capability's own function (or both call
one in the service contract), never a copy: a copy drifts apart the day
one side changes. The gate catches the simplest copy, a tolerance defined
twice: every `const <NAME>: f64 = <value>;` in `crates/engine/*/src` whose
name holds `EPSILON`, `TOLERANCE`, `SLACK` or `EPS` as a word is defined
once per name and value (values compared as numbers, so `1e-8` and
`1.0e-8` agree). One name with two meanings, or two names for one value,
passes; import the constant or call the function applying it instead of
repeating it.
