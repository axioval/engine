# Scripts

Repository gates and deterministic maintenance tools.

Scripts must fail closed, propagate subprocess exit status, avoid machine-specific paths, and include a discriminating self-test when they enforce an invariant.

Use `snapshot_hash.py` whenever a review needs an exact candidate identity; do not substitute an ad hoc archive or file-selection algorithm.
