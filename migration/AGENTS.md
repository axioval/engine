# Migration artifacts

This directory tracks the behavior-preserving cutover from `projects/vendor/solibri`.

- `solibri-capabilities.json` is the machine-readable completion ledger.
- An entry may move to `ported` only when every item in `completionContract` has concrete proof paths.
- Never promote a capability from compilation alone.
- Preserve the source commit and legacy fidelity separately from Axioval completion status.
