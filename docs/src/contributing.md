# Contributing

See the repository [contributor guide](https://github.com/axioval/engine/blob/main/CONTRIBUTING.md).

The shortest acceptable loop is RED–GREEN–REFACTOR followed by `./scripts/check.sh`. Architecture-boundary changes require an ADR or an architecture-document update and a test that proves the forbidden dependency gate can fail.

Use scoped Conventional Commits on linear `main`. Keep logical changes atomic so `git revert` is a safe rollback mechanism.

Before a release, commit the candidate and run `./scripts/package.sh`. It packages and verifies every workspace crate through Cargo's temporary local registry, including unpublished intra-workspace dependencies.

Crates reach crates.io only through the `Release` workflow (`.github/workflows/release.yml`), started by hand. `verify-only` tests the release tooling, prints the publication order, and verifies every archive. `publish` then runs in the `crates.io` environment: it scans every archive for the private forbidden terms, read from the environment secret `AXIOVAL_FORBIDDEN_TERMS_JSON`, and publishes the workspace with `scripts/publish-workspace.py` in dependency order. The upload uses a short-lived crates.io token that GitHub OIDC exchanges per attempt. No registry token is stored anywhere. A version already on crates.io is skipped, so a rerun resumes an interrupted release. A crate's first publish must also name this workflow and environment as its trusted publisher on crates.io.
