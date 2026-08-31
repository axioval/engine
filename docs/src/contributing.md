# Contributing

See the repository [contributor guide](https://github.com/axioval/engine/blob/main/CONTRIBUTING.md).

The shortest acceptable loop is RED–GREEN–REFACTOR followed by `./scripts/check.sh`. Architecture-boundary changes require an ADR or an architecture-document update and a test that proves the forbidden dependency gate can fail.

Use scoped Conventional Commits on linear `main`. Keep logical changes atomic so `git revert` is a safe rollback mechanism.
