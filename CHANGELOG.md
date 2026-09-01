# Changelog

All notable changes are documented here. This project follows Semantic Versioning and Keep a Changelog.

## [Unreleased]

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
