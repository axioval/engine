# Scripts

Repository gates and deterministic maintenance tools.

Scripts must fail closed, propagate subprocess exit status, avoid machine-specific paths, and include a discriminating self-test when they enforce an invariant.

Use `snapshot_hash.py` whenever a review needs an exact candidate identity; do not substitute an ad hoc archive or file-selection algorithm.

`publish-workspace.py` is the only path to crates.io, driven by `.github/workflows/release.yml`. Without `--execute` it only prints the plan. Keep it network-free under test (`test_publish_workspace.py`, run in the `lint` gate). Never add a stored registry token.

`parity_models.py` fetches the public IFC models of `fixtures/parity/models.json` into a cache (`AXIOVAL_PARITY_MODELS`, else `~/.cache/axioval/parity-models`, never a RAM-backed temporary directory) and keeps a file only at its pinned SHA-256; `check` validates the manifest offline (URLs pinned to a commit, licences from `LICENCES`, an attribution each). `test_parity_models.py` (run in the `lint` gate) is its network-free self-test. Never vendor a model, and never pin one by branch.

`not_evaluated_inventory.py` ranks the causes of not-evaluated outcomes across saved `check --report` results; `inventory/` holds the geometric rule set it is run with. It reads results only, never models, and its output is deterministic (`test_not_evaluated_inventory.py`, run in the `lint` gate). Keep model names out of anything tracked: label results neutrally (`m01=…`). The packages' parameter signatures must equal the registry's; `crates/apps/cli/tests/check.rs` enforces it, so regenerate them when a capability gains a parameter. Document the method and the latest ranking in `docs/src/not-evaluated-inventory.md`.

`measurement_gate.py` is the composability half of `architecture.py` (#225, #274): service-trait markers, per-call-site `measurement_ledger.json` entries checked against `crates/engine/rules/tests/golden/catalogue.json`, and the closed `SEARCH_KINDS` vocabulary. Each bypass it closes is a mutation in its `self_test`; keep it that way when changing it, and keep `UNREGISTERED_BUDGET` falling.

`bench.py` runs the template benchmark (`crates/apps/cli/benches/templates.rs`) and judges its records against `bench_budget.json` (#279): `gate` fails a template over its budget, its parity, a missing fixture or public model; `report` (CI's non-blocking `bench` job) never fails on a ratio. Run time is judged by a t interval over rounds (separate bench processes), never by one median (#293): it decides only at the planned `looks` (round counts), measuring an input whose interval straddles the ceiling in further rounds up to the last, and one still undecided fails; never pass an undecided input, never judge between looks (measuring until an interval clears passes by chance), and never pool runs of one process as independent. Keep it out of `check.sh`: timings depend on the machine's load. `test_bench.py` (run in the `lint` gate) is its self-test; never loosen the budget to pass a rebuild. An input may have its own ceiling only as a recorded `exceptions` entry with its reason, on the generated fixture only.
