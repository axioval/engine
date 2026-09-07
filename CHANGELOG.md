# Changelog

All notable changes are documented here. This project follows Semantic Versioning and Keep a Changelog.

## [Unreleased]

## [0.1.12] - 2026-09-07

### Changed

- **Renamed `axioval-openbim` to `axioval-ifc`**, and the facade's `openbim`
  feature to `ifc`. The crate adapts IFC specifically -- it never covered the
  rest of the OpenBIM ecosystem -- so it was named for a dependency family
  rather than its content. Migration: replace the dependency name, the
  `openbim` feature with `ifc`, and `axioval::openbim` with `axioval::ifc`.
  `axioval-openbim` is discontinued at 0.1.11 and will not receive updates.
- Crates are now grouped by role: `contracts/`, `engine/`, `sources/`,
  `facade/`, and `apps/`. Published crate names are unchanged apart from the
  rename above, so this is a source-tree change only. A new geometry backend
  or source format is a sibling directory under `sources/` and needs no change
  to the engine or contracts.

### Fixed

- The architecture gate discovers crates recursively by declared package name
  rather than by a fixed `crates/*` glob, and fails closed when discovery
  returns nothing. The previous glob would have silently passed every
  neutrality check under the nested layout.

## [0.1.11] - 2026-09-07

### Fixed

- A barrier is treated as present on an edge only when it runs along more than
  half of it, matching the native gate. A stub of railing beside a long open
  edge now reports `missing_barrier` rather than `hole_in_barrier`: the first
  tells a reviewer the railing was never built, the second sends them looking
  for a gap to close.

## [0.1.10] - 2026-09-07

### Fixed

- Horizontal guard reports every distinct defect on a surface instead of only
  the worst one. Edges are sample points along a boundary, so a slab with a
  short rail on one side and no rail on another has two separate problems and
  a reviewer must see both. Findings are grouped by surface and defect, and
  the elements responsible are merged and sorted across the edges that share
  a defect.

## [0.1.9] - 2026-09-07

### Changed

- The horizontal-guard capability names which fall-protection defect an edge has
  instead of reporting one generic verdict. Nine diagnoses are distinguished,
  ordered worst-first, and a surface reports its worst defect rather than one
  finding per edge. Findings carry the barrier, landing or climbable object
  responsible.

### Fixed

- A barrier that is only tall enough when measured from its curb is reported as
  `barrier_too_low_due_to_curb` rather than the generic `barrier_too_low`. The
  curb reason was previously unreachable.
- Landing shortfalls name their own cause (`landing_too_low`,
  `landings_too_small`, `landing_too_far_away`) instead of collapsing into
  `insufficient_landings`.

## [0.1.8] - 2026-09-07

### Fixed

- Space-validation findings carry their related objects: the body a space
  overlaps, the elements along an uncovered boundary run, and the elements
  covering a cap. Only duplicate findings did before, so the rest said
  something was wrong without saying where.

## [0.1.7] - 2026-09-07

### Fixed

- `CapCoverage` normalises negative zero. A geometry kernel can return `-0.0`
  for an empty intersection; it compares equal to zero and so passed every
  guard, but rendered as `-0.0`, reporting an uncovered cap as
  "bottom cap only -0.0% covered".

### Added

- `EnvelopeDerivation` derives `Hash`, so it can key a `HashMap` in an adapter.

## [0.1.6] - 2026-09-07

### Added

- `Finding::related` carries the other objects that participate in a finding --
  the slab a wall rests on, the spaces a body duplicates. A finding a reviewer
  cannot act on is a finding that gets ignored: "insufficient contact" is only
  useful alongside what the face fails to rest on. `Finding::with_related`
  sorts, deduplicates, and drops the subject, so ordering never depends on the
  order an adapter walked the model.
- Slab contact reports the touching surfaces; space validation reports
  coincident space bodies and the body a space overlaps.

### Compatibility

- `related` is `#[serde(default)]` and omitted when empty, so existing payloads
  deserialize unchanged.

## [0.1.5] - 2026-09-07

### Added

- `ContactService` and the `axioval:capability.slab-contact` capability: how
  much of a face rests on another element is measured; whether that suffices
  is decided by policy. The measurement is exact, so a ratio can no longer be
  rounded across a declared minimum.
- `EnvelopeMembershipService` and `axioval:capability.external-wall-validation`:
  declared and derived envelope sets are compared per element. Applicability
  is not carried as evidence, and one request names one derivation.
- `SpaceService` and `axioval:capability.space-validation`: duplicates, clear
  height, boundary gaps, overlaps, cap coverage and storey residuals are
  measured independently, so one unavailable aspect no longer sinks the rest.
- `GuardService` and `axioval:capability.horizontal-guard`: exposed edges and
  nearby barriers, landings and climbable objects are measured; search radii
  travel with the request. Edge coverage unions intervals rather than summing
  them, so overlapping rails cannot fake a guarded edge.

## [0.1.4] - 2026-09-07

### Fixed

- Published crates no longer name a downstream vendor or internal repository
  path. `0.1.3` shipped 36 such references across 21 files, including every
  crate README, and is yanked; `0.1.0`-`0.1.2` carry the same leak.
- Doc comments that explained the neutral model by reference to one vendor now
  attribute it to "a source format", matching the source-neutrality these
  crates claim. The reasoning is retained, not deleted.

### Changed

- `check_package_contents.py` reads every shipped text member of every `.crate`
  and rejects forbidden vendor terms, so the check runs against what a registry
  would actually receive.

## [0.1.3] - 2026-09-07

### Added

- Source-neutral `LinearQuantityService`: adapters report a measured
  `LinearInterval` plus supporting evidence, never a verdict (ADR 0004).
- `ShelfCapacity` capability compares a measured shelf run against a declared
  minimum. A measurement spanning the minimum is reported as incomplete
  evidence rather than a violation, because it has not been shown to fail.

### Changed

- `reviewable_exact_evidence` now has a single owner in `services`, so every
  evidence service applies the same admission test.

## [0.1.2] - 2026-09-01

### Added

- Immutable `EvidenceSession` snapshots bind each project source to revision,
  fingerprint, optional schema, and a session-authoritative service registry;
  registration rejects unbound or stale service snapshot identities.
- Production IFC4 STEP import in `axioval-openbim`, with exact direct-property
  presence/absence, occurrence/type provenance, and fingerprint-bound evidence.

### Changed

- Direct-property failures now preserve incomplete and conflicting source states;
  exact evidence from a different source is rejected.
- Integer property predicates now support evidence-preserving `not_equal` directly.
- Relationship property completeness remains deliberately unavailable and
  fail-closed in production IFC sessions.
- Cargo-deny narrowly ignores RUSTSEC-2025-0141 for `ifc-schema`'s bundled
  `bincode 2.0.1` decoder; the advisory is project discontinuation, not a vulnerability.

## [0.1.1] - 2026-09-01

### Added

- Source-neutral `axioval:capability.property-required`, with exact absence/null/blank findings and fail-closed property-resolution errors.

## [0.1.0] - 2026-08-31

### Added

- Initial source-neutral engine workspace and architecture contract.
- Fail-closed runtime registry-drift and duplicate package rejection.
- Dependency-policy, canonical snapshot identity, and workspace package-verification gates.
- Exact source-neutral connectivity graphs with deterministic width-constrained traversal.
- Backend-neutral metric-routing requests, bounded shortest-distance evidence, and request-bound blocked verdicts.
- Backend-neutral free-area, directional-clearance, and constrained placement-search evidence contracts.
- Complete walkable-region snapshots with deterministic three-valued width-constrained routing.
- Explicit deterministic report outcomes for missing services, backend outages, incomplete/invalid evidence, and resource limits.
- Exact grounded free-floor circle and rectangle capabilities backed by the source-neutral free-space service.
- Exact property-to-property comparison capability slice with independent candidate selectors, request-bound relationship selection, target factors, `each` / `at_least_one`, and evidence-backed missing-information behavior.
- Exact request-bound property resolution for both present values and conclusive absence evidence, including cross-object substitution rejection, finite numeric validation, and fail-closed property selector evaluation.
- Source-neutral canonical SI quantity dimensions, boxed selector-valued parameters, and exact relationship-selection requests bound to a checked object and complete candidate universe.

### Fixed

- Placement offset bounds no longer admit tolerance-expanded witnesses; supported found placements now require exact frame-bound whole-base support evidence.

[Unreleased]: https://github.com/axioval/engine/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/axioval/engine/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/axioval/engine/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/axioval/engine/releases/tag/v0.1.0
