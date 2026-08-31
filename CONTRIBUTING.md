# Contributing

Read `AGENTS.md` and the nearest nested `AGENTS.md` before editing.

1. Add a failing test for behavior changes.
2. Keep core crates free of source-format and geometry-backend dependencies.
3. Update architecture and migration documentation with contract changes.
4. Run `./scripts/check.sh`.
5. Use scoped Conventional Commits on the linear `main` branch.

A capability is migrated only when the old Solibri implementation is no longer the production owner and parity evidence is recorded.
