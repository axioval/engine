# Changelog

All notable changes are documented here. This project follows Semantic Versioning and Keep a Changelog.

## [Unreleased]

### Added

- **IFC2X3 support.** `import_ifc_session` accepts IFC2X3 TC1 as well as IFC4
  ADD2 TC1. The header's release is decided once (`IfcRelease`) and every
  service reads that release's schema: property resolution
  (`ifc-properties` 0.2.1), entity inheritance, relationship end slots and
  integrity. The snapshot declares `IFC2X3_TYPE_SYSTEM`, so a package binds
  on IFC2X3 data only through IFC2X3 external names. IFC4X3, several
  releases, or none are still refused. On six real models (one IFC4, five
  IFC2X3), 4,378 relationship answers match `ifc-spatial`'s independent
  reader. Of 22,392 real property assignments, every one is either resolved
  or refused for a stated reason; the 636 refusals are measure-typed values
  (`IFCAREAMEASURE`, `IFCENERGYMEASURE`, ...) the adapter does not yet map.

### Fixed

- **Package concepts were never bound to source data, so a checker could
  report a false pass.** A ruleset names canonical concepts
  (`axioval:fire.ifc4.wall`); the engine compared them verbatim with source
  kinds (`IFCWALL`) and property names (`Reference`). Nothing matched, the
  selector selected nothing, and the report came back empty: zero findings,
  zero not-evaluated. The compiler now builds a concept catalog, and every
  entity-type selector and property reference is translated per source through
  an external name in the type system that source's snapshot declares. A
  concept that cannot bind, binds ambiguously, or names a source with no
  declared type system is reported as not evaluated, never skipped.
- `includeSubtypes` was ignored: an `IfcWallStandardCase` was never selected
  by a wall rule. Subtype membership now comes from a
  `TypeHierarchyServiceHandle`; without one, a different kind is not evaluated
  rather than treated as a non-member.
- The package contract drifted from MCS `0.1.0` normalized output: localized
  package names, source catalogs, citations, requirements, target-group
  applicability, parameter citations and explanatory images were rejected, so
  every published Axioval package failed to load. The IR now accepts the
  complete contract, and the compatibility fixture is regenerated from the
  current MCS `minimal` example.

### Added

- `axioval-ifc` registers an exact `RelationshipSelectionService` over the
  model's objectified relationships (containment, aggregation, voids, fills,
  space boundaries, type and group assignment, and any other IFC4
  relationship with object ends). Relationship identities are IFC4 entity
  names; supertypes include every subtype. Checked against `ifc-spatial`'s
  independent spatial tree on a real IFC4 model: 323 containment answers, all
  equal.
  IFC2X3 sources are refused until upstream exact property resolution
  supports them (openbimrs/ifc#48).
- `RelationshipSelectionService::source_snapshots`, so a relationship service
  can be bound to an `EvidenceSession`.
- `AbsentEndPolicy` and `RelationshipSelectionRequest::with_absent_ends`: a
  relationship instance that leaves a schema-required end empty refuses the
  answer by default; `Skip` answers from the existing edges and cites each
  skipped instance. `property-comparison` exposes it as the optional
  `skip_absent_relationship_ends` parameter (default `false`). On the IFC4
  reference model, 98 virtual space boundaries without a
  `RelatedBuildingElement` previously made every space-boundary query refuse.
- `SourceIntegrityService` / `SourceIntegrityServiceHandle`, `IntegrityIssue`
  and `IntegritySeverity`: a channel for source irregularities that are
  neither findings nor not-evaluated outcomes. `axioval-ifc` reports absent
  required relationship ends as warnings (`ABSENT_REQUIRED_END`) and malformed
  relationships as errors (`MALFORMED_RELATIONSHIP`).
- `ConceptCatalog`, `ConceptBindings` and `BindingError`; the compiler rejects
  references to concepts no loaded definition package declares.
- `SourceSnapshot::with_type_system`, and `TypeHierarchyService` /
  `TypeHierarchyServiceHandle`.
- Target-group applicability: one group compiles as its selector; several
  groups are carried in `ExecutionPlan::deferred` and reported as not
  evaluated, because a one-selector capability cannot evaluate them.
- `axioval-ifc` declares the IFC4 type system (`IFC4_TYPE_SYSTEM`) and
  registers IFC4 entity inheritance from the bundled normative schema. An
  entity the schema does not declare is an error, not a non-member.

### Changed

- **Breaking.** `RuleInstance::applicability` is `RuleApplicability`, and
  `PackageMetadata::name`/`description` are `LocalizedText`.
- **Breaking.** `Runtime::run` over a bare `Project` declares no type systems,
  so a compiled package's concepts bind to nothing there; run an
  `EvidenceSession` to evaluate packages.

## [0.2.0] - 2026-09-24

A minor release rather than 0.1.19: the dependency upgrade changes a public
signature. Under Cargo's 0.x rules `cargo update` would pull a patch release
into every `axioval = "0.1"` build, breaking any consumer that builds its own
meshes with `axiolid-mesh` 0.1.

### Changed

- **Breaking.** `AxiolidGeometry::with_mesh` and `AxiolidGeometry::mesh` take
  and return `axiolid_mesh::TriMesh` from `axiolid-mesh` 0.3. A consumer that
  builds meshes with `axiolid-mesh` 0.1 gets a type mismatch; move both
  `axioval` and `axiolid-mesh` together. Verified: a 0.1 mesh fails to compile
  against this release, and the same scenario built on 0.3 produces output
  identical to 0.1.18.
- Axiolid dependencies move from 0.1 to 0.3 (`axiolid-core`, `-mesh`,
  `-measure`, `-overlay`) and IFC dependencies from 0.1 to 0.2
  (`ifc-model` 0.2.2, `ifc-schema` 0.2.2, `ifc-step` 0.2.1,
  `ifc-properties` 0.2.0). No other Axioval API changes; `axioval-ifc`
  exposes no upstream types. The IFC crates bring the STEP parser
  `openbim-step` with them, transitively, from 0.4.0 to 0.5.1.
- IFC error enums upstream are now `#[non_exhaustive]`. Unrecognised property
  errors, including the new `AuthoringInvalid`, resolve to
  `PropertyResolutionError::Unavailable`, so a new upstream failure mode can
  never pass as evidence.
- `CC0-1.0` is allowed in `deny.toml`. It reaches the tree through
  `tiny-keccak` <- `const-random` <- `ahash`, now used by both upstreams.
  Entity iteration order is unaffected: `ifc-model` iterates an explicit
  insertion-order vector, not its hash map.

## [0.1.18] - 2026-09-07

### Added

- `AxiolidGuardService` measures guard edges: what stands above a walking
  surface's exposed edge (barriers), what lies below it (landings), and what
  sits beside a barrier low enough to climb it. Edge coverage is reported as a
  parameterised interval so a policy can union overlapping protection.
- `AxiolidFreeSpaceService` measures clearance and free floor area. `Clear` is
  returned only when every named obstacle was measured and none intersects, so
  the completeness claim is earned rather than assumed.
- `SpaceService::measure_boundary_gaps` is now implemented. Unioning the
  triangle soup collapses interior edges, leaving the real perimeter to walk;
  previously this aspect reported `Unavailable`.
- Internal `planar::boundary_rings`, `ring_segments` and `ring_perimeter`,
  shared by the services that walk a footprint edge.

### Changed

- Guard proximity is measured between plan footprints, not between vertices.
  Two boxes whose faces are 50 mm apart have corner vertices a metre apart, so
  vertex distance put a touching climbing aid outside a 0.5 m search radius.
- An object can be both a barrier and a climbing aid for a taller barrier. The
  climbable pass previously excluded everything already classed as a barrier,
  which hid the classic defeat case of a low parapet beside a tall railing.

### Not implemented

- `FreeSpaceService::find_placement` returns `Unavailable`. `NoPlacement`
  asserts an exhaustive search proving the shape fits nowhere; a sampled sweep
  can only fail to find a witness, which is a weaker claim. Returning it would
  launder "did not find" into "does not exist".


### Changed

- The retired `axioval-openbim` name now carries a deprecation notice on
  crates.io, published as `0.2.0`. It contains no functionality and points at
  `axioval-ifc`.

  Released as `0.2.0` rather than `0.1.12` deliberately: under Cargo's semver
  rules a `0.1.12` would be selected automatically by anyone depending on
  `"0.1"`, turning a routine update into an empty crate. Verified against the
  live registry -- a `"0.1"` dependency still resolves to `0.1.11`.

  `0.1.11` is intentionally not yanked. Yanking would break existing lockfiles
  while explaining nothing about where the code went.


## [0.1.17] - 2026-09-07

### Added

- `AxiolidSpaceService` measures all seven space aspects from supplied geometry:
  duplicates, clear height, boundary gaps, overlaps, cap coverage, storey
  residuals and support counts. Each aspect fails independently, so an
  unmeasurable clear height no longer hides a measurable overlap.
- `AxiolidGeometry` gained storey assignment and element roles (space, slab,
  roof), declared by the host rather than inferred from a mesh.

### Removed

- The cap-coverage clamp `covered.min(whole)`. Coverage is the intersection of
  the merged cap elements with the space footprint, so it is bounded by that
  footprint by construction; the clamp could never change a result.


## [0.1.16] - 2026-09-07

### Added

- `AxiolidEnvelopeMembershipService` derives building-envelope membership from
  geometry: an object bounds the envelope when its plan footprint meets a
  declared bounding space. Both `EnvelopeDerivation` variants are served from
  separate declared space sets, so a model can be checked against all spaces or
  against gross-area groups only.

### Changed

- Plan-projection helpers moved to an internal `planar` module shared by the
  services, and the mesh/triangle vocabulary moved to `geometry`. No public API
  changed; `AxiolidGeometry` gains `with_space` and `with_gross_area_space`.

### Removed

- A redundant empty-envelope guard in the envelope derivation. An empty
  bounding set always produced empty bounding geometry, which the following
  guard already rejected with the same error, so the branch could never be
  observed to matter. Proven by a mutant that no test could kill until the
  duplicate was removed.

## [0.1.15] - 2026-09-07

### Added

- `AxiolidGeometry` is `Clone`, so one geometry set can be registered with
  several services. Found by an application registering contact and
  linear-quantity services over the same model: without it every consumer has
  to rebuild the store, which is how two services end up measuring different
  geometry.

## [0.1.14] - 2026-09-07

### Added

- `axioval-axiolid` implements `LinearQuantityService`, measuring shelf running
  length from an object's footprint, ceiling height and declared doorways. The
  `shelf-capacity` capability now runs against real geometry.
- `AxiolidGeometry::with_doorways` records how many openings interrupt an
  object's perimeter. Openings are a semantic fact a mesh does not carry, so
  the host declares them rather than the adapter inferring them.

### Changed

- The mesh store moved from `contact` to a shared `geometry` module and is now
  `AxiolidGeometry`: it was never contact-specific, and a second service needs
  the same lookup. `AxiolidContactGeometry` is renamed accordingly.
- Shelf length is reported as an upper-bounded interval rather than an exact
  value, because it derives from a bounding footprint. The capability already
  fails closed on an interval that straddles its minimum.

## [0.1.13] - 2026-09-07

### Added

- `axioval-axiolid` implements the engine's `ContactService` over the published
  Axiolid kernel (`axiolid-mesh`, `axiolid-measure`, `axiolid-overlay`). An
  application can now measure real contact from meshes using published crates
  only -- previously every geometry service had to be supplied by the host.
  Hosts register a `TriMesh` per `ObjectId`, so proprietary CAD sources use this
  without any IFC dependency.

  Contact area is measured in plan by intersecting projected geometry, and the
  nearest separation stays a 3D measurement.


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
