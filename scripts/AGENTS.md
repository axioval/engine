# Scripts

Repository gates and deterministic maintenance tools.

Scripts must fail closed, propagate subprocess exit status, avoid machine-specific paths, and include a discriminating self-test when they enforce an invariant.

Use `snapshot_hash.py` whenever a review needs an exact candidate identity; do not substitute an ad hoc archive or file-selection algorithm.

`publish-workspace.py` is the only path to crates.io, driven by `.github/workflows/release.yml`. Without `--execute` it only prints the plan. Keep it network-free under test (`test_publish_workspace.py`, run in the `lint` gate). Never add a stored registry token.

`not_evaluated_inventory.py` ranks the causes of not-evaluated outcomes across saved `check --report` results; `inventory/` holds the geometric rule set it is run with. It reads results only, never models, and its output is deterministic (`test_not_evaluated_inventory.py`, run in the `lint` gate). Keep model names out of anything tracked: label results neutrally (`m01=…`). The packages' parameter signatures must equal the registry's; `crates/apps/cli/tests/check.rs` enforces it, so regenerate them when a capability gains a parameter. Document the method and the latest ranking in `docs/src/not-evaluated-inventory.md`.

`measurement_gate.py` is the composability half of `architecture.py` (#225, #274): service-trait markers, per-call-site `measurement_ledger.json` entries checked against `crates/engine/rules/tests/golden/catalogue.json`, and the closed `SEARCH_KINDS` vocabulary. Each bypass it closes is a mutation in its `self_test`; keep it that way when changing it, and keep `UNREGISTERED_BUDGET` falling.
