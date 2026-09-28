# Scripts

Repository gates and deterministic maintenance tools.

Scripts must fail closed, propagate subprocess exit status, avoid machine-specific paths, and include a discriminating self-test when they enforce an invariant.

Use `snapshot_hash.py` whenever a review needs an exact candidate identity; do not substitute an ad hoc archive or file-selection algorithm.

`publish-workspace.py` is the only path to crates.io, driven by `.github/workflows/release.yml`. Without `--execute` it only prints the plan. Keep it network-free under test (`test_publish_workspace.py`, run in the `lint` gate). Never add a stored registry token.
