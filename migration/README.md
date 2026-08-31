# Solibri migration ledger

The ledger contains every runtime entry from the source snapshot, including the non-discoverable read-only executable.

Allowed states:

- `pending` — no complete Axioval cutover.
- `in_progress` — implementation exists but one or more proof obligations remain.
- `ported` — Axioval owns policy, Solibri consumes it, parity passes, and duplicate ownership is removed.
- `blocked` — an explicit external prerequisite is recorded.

A script or review that changes a state must add repository-relative proof paths. Runtime fidelity in the source is historical data and must never be used as proof that the migration itself is complete.
