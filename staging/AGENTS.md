# `staging/`

Work that depends on **unpublished** crates, kept deliberately outside the
shipping workspace and outside every release. `axioval-bcf` graduated from
here to `crates/sinks/bcf/` once the `openbim-bcf` writer was published.

## Why this exists

`axiolid-route` and `axiolid-predicates` are not on crates.io. Cargo refuses to
publish a crate carrying a path dependency:

```
all dependencies must have a version requirement specified when publishing
```

Adding `version` alongside `path` does not help — Cargo then resolves the
version from the registry and fails with `no matching package`. So work needing
those crates cannot live in `crates/` at all until they land. It lives here.

## The hard rule: one source per crate graph

**Every `axiolid-*` dependency in a staging crate must come from the same
place.** Mixing published and local copies compiles, then fails at the type
level with:

> two different versions of crate `axiolid_overlay` are being used; two types
> coming from two different versions of the same crate are different types even
> if they look the same

The local kernel is at `0.14.x` while the registry publishes `0.1.0`, so a
published `Point2` cannot be passed to a local `shortest_path`. Pin the whole
graph to the local kernel while staging.

## Isolation guarantees

`scripts/staging_isolation.py` runs in `check.sh` and fails if:

- a staging crate becomes a workspace member,
- a staging manifest loses `publish = false`,
- a staging manifest loses its empty `[workspace]` table,
- a published crate under `crates/` depends on anything in `staging/`.

All four scenarios were verified to fail the gate before it was trusted.

## Rewiring when the crates land

1. Confirm the dependency is actually published:
   `curl -sS -H 'User-Agent: <you>' https://crates.io/api/v1/crates/axiolid-route`
2. Change every `{ path = ... }` to a version requirement, in one commit — do
   not mix sources.
3. `cargo update && cargo test` — expect real breakage: the published version
   will not match local `0.14.x` API, and that gap is the actual porting work.
4. Move the crate from `staging/` into `crates/sources/geometry/`, add it to
   `workspace.members`, and bump `EXPECTED_MEMBERS` in
   `scripts/staging_isolation.py`.
5. Re-run `./scripts/check.sh` and publish normally.

## Current contents

- `axiolid-routing/` — `MetricRoutingService` / `WalkabilityService` groundwork
  over `axiolid-route`. Proven working locally: 8.0000 m direct vs 8.2462 m
  around a barrier, with a visibility graph of 6 and 8 vertices.
