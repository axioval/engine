# Axioval Engine

This repository owns the source-neutral Rust rule runtime, its portable contracts, built-in capabilities, and separately layered adapters.

## Boundaries

- Core crates must not depend on IFC, STEP, ICDD, Axiolid, OpenCascade, CGAL, or vendor types.
- `axioval-ifc` and `axioval-axiolid` are independent adapters. Never create an adapter combining both organizations.
- Source adapters map external identities and semantics into `axioval-ir`; they do not implement checking policy.
- Geometry adapters provide typed evidence and exactness/provenance. They do not emit policy findings.
- Rulesets select trusted capability IDs. Never execute package-provided code.
- Missing evidence and unsupported capabilities fail closed.
- Stable ordering and source-qualified identities are compatibility contracts.

## Neutrality is a publication contract

This repository and every crate published from it are public. Axioval is
source-neutral, so naming a particular commercial product, downstream consumer,
or private integration in tracked text both leaks a private relationship and
contradicts the neutrality the crates claim.

- Never name a specific commercial checking product, client, or private
  consumer in tracked files, commit messages, or published artifacts.
- Describe capabilities by what they do, not by whose product they came from:
  "the capability migration ledger", not a vendor's name.
- Interoperability *formats* may be named where they are genuine public
  standards or file extensions the code reads (`.cset`, IDS, IFC, OpenBimRL).
  Naming the format is a technical fact; naming the company is a leak.
- Private material lives in `private/` (untracked). Gates read it through an
  environment variable and fail closed when it is missing.

## Development

Development is hot on `main`: use scoped Conventional Commits, keep history
linear, and stage explicit paths only.

The public documentation system is mdBook plus workspace rustdoc, deployed by
`.github/workflows/pages.yml` on minor and major release tags (`v0.3.0`,
`v1.0.0`), never on patch or pre-release tags, or by hand. The `github-pages`
environment allows `v*` tags to deploy. Update the relevant page with every
public contract or architecture change and keep `docs/src/SUMMARY.md`
complete; `./scripts/check.sh` builds the book on every commit, so a broken
page still fails before release.

## Direct children

- `crates/` — Rust crates; descend into each crate's `AGENTS.md` before editing.
- `docs/` — mdBook sources and architecture documentation.
- `scripts/` — repository validation and architecture checks.
- `staging/` — crates developed against unpublished path dependencies.
- `attic/` — retired crate names kept only to serve a deprecation notice.
- `.github/workflows/` — CI and GitHub Pages deployment.
- `private/` — untracked maintainer-only inputs (capability ledger, publish denylist).

## Gates

Run `./scripts/check.sh`. It formats, lints, tests, checks architecture
boundaries, builds rustdoc, and builds the mdBook. The architecture gate is
mutation-proven and binds to `scripts/rule_vocabulary.json`; a capability is
never marked migrated without its parity evidence and cutover reference.
